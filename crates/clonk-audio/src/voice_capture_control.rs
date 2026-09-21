//! A microphone boundary shared with the input callback, without callback locks.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct VoiceCaptureControl(Arc<CaptureControl>);

#[derive(Debug)]
struct CaptureControl {
    origin: Instant,
    finish_nanos: AtomicU64,
    aborted: AtomicBool,
    device_closed: AtomicBool,
    processors: AtomicUsize,
}

impl Default for VoiceCaptureControl {
    fn default() -> Self {
        Self(Arc::new(CaptureControl {
            origin: Instant::now(),
            finish_nanos: AtomicU64::new(u64::MAX),
            aborted: AtomicBool::new(false),
            device_closed: AtomicBool::new(false),
            processors: AtomicUsize::new(0),
        }))
    }
}

impl VoiceCaptureControl {
    /// Stop recording at this instant, retaining only the authorized tail.
    pub fn finish_at(&self, at: Instant) {
        let nanos = at.saturating_duration_since(self.0.origin).as_nanos();
        self.0.finish_nanos.fetch_min(
            u64::try_from(nanos)
                .unwrap_or(u64::MAX - 1)
                .min(u64::MAX - 1),
            Ordering::AcqRel,
        );
    }

    /// Privacy revocation discards the tail as well as closing the device.
    pub fn abort(&self) {
        self.0.aborted.store(true, Ordering::Release);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.aborted.load(Ordering::Acquire)
    }

    pub fn finish_time(&self) -> Option<Instant> {
        let nanos = self.0.finish_nanos.load(Ordering::Acquire);
        (nanos != u64::MAX).then(|| self.0.origin + Duration::from_nanos(nanos))
    }

    pub fn is_recording(&self) -> bool {
        !self.is_aborted() && self.finish_time().is_none()
    }

    #[cfg(any(feature = "cpal", test))]
    pub(crate) fn accepts_sample_at(&self, captured_at: Instant) -> bool {
        !self.is_aborted() && self.finish_time().is_none_or(|end| captured_at < end)
    }

    pub fn is_finished(&self) -> bool {
        self.0.device_closed.load(Ordering::Acquire)
            && self.0.processors.load(Ordering::Acquire) == 0
    }

    #[cfg(any(feature = "cpal", test))]
    pub(crate) fn device_closed(&self) {
        self.0.device_closed.store(true, Ordering::Release);
    }

    #[cfg(any(feature = "cpal", test))]
    pub(crate) fn processing_guard(&self) -> CaptureProcessingGuard {
        self.0.processors.fetch_add(1, Ordering::AcqRel);
        CaptureProcessingGuard(self.clone())
    }
}

#[cfg(any(feature = "cpal", test))]
pub(crate) struct CaptureProcessingGuard(VoiceCaptureControl);

#[cfg(any(feature = "cpal", test))]
impl Drop for CaptureProcessingGuard {
    fn drop(&mut self) {
        self.0 .0.processors.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_capture_keeps_only_samples_before_the_original_cutoff() {
        let control = VoiceCaptureControl::default();
        let before = Instant::now();
        let release = before + Duration::from_millis(10);
        control.finish_at(release);
        control.finish_at(release + Duration::from_secs(1));
        assert!(!control.is_recording());
        assert!(control.accepts_sample_at(before));
        assert!(!control.accepts_sample_at(release));
        assert!(!control.accepts_sample_at(release + Duration::from_millis(1)));
        assert!(!control.is_finished());
        let processing = control.processing_guard();
        control.device_closed();
        assert!(!control.is_finished(), "the DSP tail still owns the stream");
        drop(processing);
        assert!(control.is_finished());
        control.abort();
        assert!(
            !control.accepts_sample_at(before),
            "privacy cancellation discards the tail"
        );
    }
}
