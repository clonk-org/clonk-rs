//! Cached input-device metadata. Enumerating devices never belongs on the GUI
//! thread, and does not need to open a microphone stream.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{VoiceCaptureError, VoiceInputDevice};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceInputDeviceInventory {
    Scanning,
    Ready(Vec<VoiceInputDevice>),
    Unavailable(String),
}

pub(crate) struct VoiceInputDeviceCatalog {
    state: Arc<CatalogState>,
}

struct CatalogState {
    snapshot: Mutex<VoiceInputDeviceInventory>,
    running: AtomicBool,
    refresh: AtomicBool,
}

impl Drop for VoiceInputDeviceCatalog {
    fn drop(&mut self) {
        self.state.running.store(false, Ordering::Release);
    }
}

impl VoiceInputDeviceCatalog {
    #[cfg(feature = "cpal")]
    pub(crate) fn new() -> Self {
        let mut host = crate::sound_host::SoundHost::default();
        Self::with_enumerator(move || host.with(crate::voice::offered_voice_input_devices))
    }

    fn with_enumerator(
        mut enumerate: impl FnMut() -> Result<Vec<VoiceInputDevice>, VoiceCaptureError> + Send + 'static,
    ) -> Self {
        let state = Arc::new(CatalogState {
            snapshot: Mutex::new(VoiceInputDeviceInventory::Scanning),
            running: AtomicBool::new(true),
            refresh: AtomicBool::new(false),
        });
        let worker_state = state.clone();
        let worker = std::thread::Builder::new()
            .name("voice-input-devices".into())
            .spawn(move || {
                let mut next_refresh = Instant::now();
                while worker_state.running.load(Ordering::Acquire) {
                    if worker_state.refresh.swap(false, Ordering::AcqRel)
                        || Instant::now() >= next_refresh
                    {
                        let snapshot = match enumerate() {
                            Ok(devices) => VoiceInputDeviceInventory::Ready(devices),
                            Err(error) => VoiceInputDeviceInventory::Unavailable(error.to_string()),
                        };
                        if !worker_state.running.load(Ordering::Acquire) {
                            break;
                        }
                        *worker_state.snapshot.lock().unwrap() = snapshot;
                        next_refresh = Instant::now() + Duration::from_secs(1);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
        if let Err(error) = worker {
            *state.snapshot.lock().unwrap() =
                VoiceInputDeviceInventory::Unavailable(error.to_string());
        }
        Self { state }
    }

    pub(crate) fn refresh(&self) {
        self.state.refresh.store(true, Ordering::Release);
    }

    pub(crate) fn snapshot(&self) -> VoiceInputDeviceInventory {
        self.state.snapshot.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_inventory_can_be_refreshed_after_an_enumeration_error() {
        let mut calls = 0;
        let catalog = VoiceInputDeviceCatalog::with_enumerator(move || {
            calls += 1;
            if calls == 1 {
                Err(VoiceCaptureError::NoInputDevice)
            } else {
                Ok(vec![VoiceInputDevice {
                    id: "mock:microphone".parse().unwrap(),
                    name: "Microphone".into(),
                }])
            }
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while catalog.snapshot() == VoiceInputDeviceInventory::Scanning && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            catalog.snapshot(),
            VoiceInputDeviceInventory::Unavailable(_)
        ));
        catalog.refresh();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(catalog.snapshot(), VoiceInputDeviceInventory::Ready(_))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let VoiceInputDeviceInventory::Ready(devices) = catalog.snapshot() else {
            panic!("refresh never completed");
        };
        assert_eq!(devices[0].id.as_str(), "mock:microphone");
    }

    #[test]
    fn closing_input_inventory_does_not_wait_for_native_enumeration() {
        struct Completion(std::sync::mpsc::Sender<()>);
        impl Drop for Completion {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (started, entering) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let completion = Completion(done);
        let catalog = VoiceInputDeviceCatalog::with_enumerator(move || {
            let _ = &completion;
            started.send(()).unwrap();
            resumed.recv().unwrap();
            Ok(Vec::new())
        });
        entering.recv_timeout(Duration::from_secs(2)).unwrap();
        let state = catalog.state.clone();
        let stopped = Instant::now();
        drop(catalog);
        assert!(stopped.elapsed() < Duration::from_millis(100));
        resume.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            *state.snapshot.lock().unwrap(),
            VoiceInputDeviceInventory::Scanning
        );
    }

    #[test]
    fn slow_device_enumeration_never_blocks_the_settings_caller() {
        let started = Instant::now();
        let catalog = VoiceInputDeviceCatalog::with_enumerator(|| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(Vec::new())
        });
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(catalog.snapshot(), VoiceInputDeviceInventory::Scanning);
        let deadline = Instant::now() + Duration::from_secs(2);
        while catalog.snapshot() == VoiceInputDeviceInventory::Scanning && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            catalog.snapshot(),
            VoiceInputDeviceInventory::Ready(Vec::new())
        );
    }
}
