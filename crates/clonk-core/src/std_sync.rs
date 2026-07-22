use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

pub const INFINITE: u32 = u32::MAX;

#[derive(Debug)]
struct LockState {
    owner: Option<ThreadId>,
    recursion: usize,
}

pub struct CriticalSection {
    state: Mutex<LockState>,
    cvar: Condvar,
}

impl Default for CriticalSection {
    fn default() -> Self {
        Self::new()
    }
}

impl CriticalSection {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(LockState {
                owner: None,
                recursion: 0,
            }),
            cvar: Condvar::new(),
        }
    }

    pub fn enter(&self) -> CriticalSectionGuard<'_> {
        let current = std::thread::current().id();
        let mut state = self.state.lock().unwrap();
        loop {
            match state.owner {
                None => {
                    state.owner = Some(current);
                    state.recursion = 1;
                    break;
                }
                Some(owner) if owner == current => {
                    state.recursion += 1;
                    break;
                }
                _ => {
                    state = self.cvar.wait(state).unwrap();
                }
            }
        }
        CriticalSectionGuard {
            section: self,
            active: true,
        }
    }

    pub fn try_enter(&self) -> Option<CriticalSectionGuard<'_>> {
        let current = std::thread::current().id();
        if let Ok(mut state) = self.state.try_lock() {
            match state.owner {
                None => {
                    state.owner = Some(current);
                    state.recursion = 1;
                    Some(CriticalSectionGuard {
                        section: self,
                        active: true,
                    })
                }
                Some(owner) if owner == current => {
                    state.recursion += 1;
                    Some(CriticalSectionGuard {
                        section: self,
                        active: true,
                    })
                }
                _ => None,
            }
        } else {
            None
        }
    }

    fn leave(&self) {
        let current = std::thread::current().id();
        let mut state = self.state.lock().unwrap();
        debug_assert_eq!(state.owner, Some(current));
        if state.recursion > 1 {
            state.recursion -= 1;
            return;
        }
        state.owner = None;
        state.recursion = 0;
        self.cvar.notify_all();
    }
}

pub struct CriticalSectionGuard<'a> {
    section: &'a CriticalSection,
    active: bool,
}

impl<'a> CriticalSectionGuard<'a> {
    pub fn clear(mut self) {
        self.section.leave();
        self.active = false;
    }
}

impl Drop for CriticalSectionGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.section.leave();
            self.active = false;
        }
    }
}

pub struct StdLock<'a> {
    guard: Option<CriticalSectionGuard<'a>>,
}

impl<'a> StdLock<'a> {
    pub fn new(section: &'a CriticalSection) -> Self {
        Self {
            guard: Some(section.enter()),
        }
    }

    pub fn clear(&mut self) {
        if let Some(guard) = self.guard.take() {
            guard.clear();
        }
    }
}

impl Drop for StdLock<'_> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug)]
struct EventState {
    signaled: bool,
}

pub struct StdEvent {
    state: Mutex<EventState>,
    cvar: Condvar,
    manual_reset: bool,
}

impl StdEvent {
    pub fn new(initial_state: bool) -> Self {
        Self {
            state: Mutex::new(EventState {
                signaled: initial_state,
            }),
            cvar: Condvar::new(),
            manual_reset: true,
        }
    }

    pub fn auto_reset(initial_state: bool) -> Self {
        Self {
            manual_reset: false,
            ..Self::new(initial_state)
        }
    }

    pub fn set(&self) {
        let mut state = self.state.lock().unwrap();
        state.signaled = true;
        if self.manual_reset {
            self.cvar.notify_all();
        } else {
            self.cvar.notify_one();
        }
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.signaled = false;
    }

    pub fn wait_for(&self, milliseconds: u32) -> bool {
        let mut state = self.state.lock().unwrap();
        if milliseconds == INFINITE {
            while !state.signaled {
                state = self.cvar.wait(state).unwrap();
            }
            if !self.manual_reset {
                state.signaled = false;
            }
            return true;
        }

        let timeout = Duration::from_millis(milliseconds as u64);
        let deadline = Instant::now() + timeout;
        while !state.signaled {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let (guard, result) = self.cvar.wait_timeout(state, remaining).unwrap();
            state = guard;
            if result.timed_out() && !state.signaled {
                return false;
            }
        }
        if !self.manual_reset {
            state.signaled = false;
        }
        true
    }
}

pub trait ShareFreeCallback: Send + Sync {
    fn on_share_free(&self, sec: &SharedCriticalSection);
}

struct SharedState {
    share_count: usize,
}

pub struct SharedCriticalSection {
    base: CriticalSection,
    shared: Mutex<SharedState>,
    waiters: Condvar,
    callback: Mutex<Option<Arc<dyn ShareFreeCallback>>>,
}

impl Default for SharedCriticalSection {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedCriticalSection {
    pub fn new() -> Self {
        Self {
            base: CriticalSection::new(),
            shared: Mutex::new(SharedState { share_count: 0 }),
            waiters: Condvar::new(),
            callback: Mutex::new(None),
        }
    }

    pub fn with_callback(callback: Arc<dyn ShareFreeCallback>) -> Self {
        Self {
            callback: Mutex::new(Some(callback)),
            ..Self::new()
        }
    }

    pub fn set_callback(&self, callback: Option<Arc<dyn ShareFreeCallback>>) {
        let mut slot = self.callback.lock().unwrap();
        *slot = callback;
    }

    pub fn enter(&self) -> SharedExclusiveGuard<'_> {
        loop {
            let guard = self.base.enter();
            let share_free = {
                let shared = self.shared.lock().unwrap();
                shared.share_count == 0
            };
            if share_free {
                return SharedExclusiveGuard {
                    section: self,
                    guard: Some(guard),
                };
            }
            drop(guard);
            let mut shared = self.shared.lock().unwrap();
            while shared.share_count != 0 {
                shared = self.waiters.wait(shared).unwrap();
            }
        }
    }

    pub fn enter_shared(&self) -> SharedLock<'_> {
        let guard = self.base.enter();
        let mut shared = self.shared.lock().unwrap();
        shared.share_count += 1;
        drop(shared);
        drop(guard);
        SharedLock {
            section: self,
            active: true,
        }
    }
}

pub struct SharedExclusiveGuard<'a> {
    section: &'a SharedCriticalSection,
    guard: Option<CriticalSectionGuard<'a>>,
}

impl<'a> SharedExclusiveGuard<'a> {
    pub fn clear(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);
            self.section.waiters.notify_all();
        }
    }
}

impl Drop for SharedExclusiveGuard<'_> {
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct SharedLock<'a> {
    section: &'a SharedCriticalSection,
    active: bool,
}

impl<'a> SharedLock<'a> {
    pub fn clear(&mut self) {
        if self.active {
            self.release();
        }
    }

    fn release(&mut self) {
        let guard = self.section.base.enter();
        let mut shared = self.section.shared.lock().unwrap();
        debug_assert!(shared.share_count > 0);
        shared.share_count -= 1;
        let last = shared.share_count == 0;
        drop(shared);
        let callback = if last {
            self.section.callback.lock().unwrap().clone()
        } else {
            None
        };
        drop(guard);
        if let Some(cb) = callback {
            cb.on_share_free(self.section);
        }
        if last {
            self.section.waiters.notify_all();
        }
        self.active = false;
    }
}

impl Drop for SharedLock<'_> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn critical_section_reentrant() {
        let cs = CriticalSection::new();
        let guard1 = cs.enter();
        let guard2 = cs.enter();
        drop(guard2);
        drop(guard1);
    }

    #[test]
    fn critical_section_synchronizes_threads() {
        let cs = Arc::new(CriticalSection::new());
        let barrier = Arc::new(Barrier::new(2));
        let cs_clone = cs.clone();
        let barrier_clone = barrier.clone();
        let handle = thread::spawn(move || {
            barrier_clone.wait();
            let _guard = cs_clone.enter();
        });

        let guard = cs.enter();
        barrier.wait();
        thread::sleep(Duration::from_millis(50));
        drop(guard);
        handle.join().unwrap();
    }

    #[test]
    fn std_event_manual_reset() {
        let event = StdEvent::new(false);
        assert!(!event.wait_for(10));
        event.set();
        assert!(event.wait_for(10));
        assert!(event.wait_for(10));
        event.reset();
        assert!(!event.wait_for(10));
    }

    #[test]
    fn std_event_auto_reset_wakes_one() {
        let event = Arc::new(StdEvent::auto_reset(false));
        let barrier = Arc::new(Barrier::new(3));
        let woke = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let event_clone = Arc::clone(&event);
            let barrier_clone = Arc::clone(&barrier);
            let woke_clone = Arc::clone(&woke);
            handles.push(thread::spawn(move || {
                barrier_clone.wait();
                if event_clone.wait_for(100) {
                    woke_clone.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }

        barrier.wait();
        event.set();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(woke.load(Ordering::SeqCst), 1);
        assert!(!event.wait_for(10));
    }

    #[test]
    fn shared_section_blocks_exclusive_until_shared_released() {
        let section = Arc::new(SharedCriticalSection::new());
        let shared_guard = section.enter_shared();
        let section_clone = Arc::clone(&section);
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = thread::spawn(move || {
            let _exclusive = section_clone.enter();
            tx.send(()).unwrap();
        });

        thread::sleep(Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
        drop(shared_guard);
        handle.join().unwrap();
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    struct TestCallback {
        hits: AtomicUsize,
    }

    impl ShareFreeCallback for TestCallback {
        fn on_share_free(&self, _sec: &SharedCriticalSection) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn shared_section_invokes_callback_on_last_release() {
        let callback = Arc::new(TestCallback {
            hits: AtomicUsize::new(0),
        });
        let section = SharedCriticalSection::new();
        section.set_callback(Some(callback.clone()));
        {
            let _s1 = section.enter_shared();
            let _s2 = section.enter_shared();
        }
        assert_eq!(callback.hits.load(Ordering::SeqCst), 1);
    }
}
