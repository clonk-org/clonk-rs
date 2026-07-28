use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(windows)]
use std::ptr;
#[cfg(all(not(unix), not(windows)))]
use std::sync::Condvar;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED_0, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};

pub const INFINITE_TIMEOUT: i32 = -1;

#[cfg(unix)]
use libc::{self, c_int, c_short, nfds_t, pollfd, POLLIN};
#[cfg(unix)]
use std::os::unix::io::RawFd;

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FdInterest {
    fd: RawFd,
    events: c_short,
}

#[cfg(unix)]
impl FdInterest {
    pub fn new(fd: RawFd, events: c_short) -> Self {
        Self { fd, events }
    }

    pub fn read(fd: RawFd) -> Self {
        Self::new(fd, POLLIN)
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    pub fn events(&self) -> c_short {
        self.events
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandleInterest {
    handle: HANDLE,
}

#[cfg(windows)]
impl HandleInterest {
    pub fn new(handle: HANDLE) -> Self {
        Self { handle }
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

pub trait StdSchedulerProc: Send + Sync {
    fn execute(&self, timeout: i32) -> bool;

    #[cfg(unix)]
    fn get_fds(&self) -> Vec<FdInterest> {
        Vec::new()
    }

    #[cfg(windows)]
    fn get_handles(&self) -> Vec<HandleInterest> {
        Vec::new()
    }

    fn get_timeout(&self) -> i32 {
        INFINITE_TIMEOUT
    }
}

type ProcHandle = Arc<dyn StdSchedulerProc>;

struct ProcEntry {
    proc: ProcHandle,
}

#[cfg(unix)]
#[derive(Debug)]
struct Unblocker {
    read_fd: c_int,
    write_fd: c_int,
}

#[cfg(unix)]
impl Unblocker {
    fn new() -> io::Result<Self> {
        let mut fds = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        for &fd in &fds {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags == -1
                || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
            {
                unsafe {
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                }
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            read_fd: fds[0],
            write_fd: fds[1],
        })
    }

    fn poll_fd(&self) -> c_int {
        self.read_fd
    }

    fn notify(&self) -> io::Result<()> {
        let byte = [42u8];
        loop {
            let written = unsafe { libc::write(self.write_fd, byte.as_ptr().cast(), 1) };
            if written == -1 {
                let err = io::Error::last_os_error();
                match err.kind() {
                    io::ErrorKind::Interrupted => continue,
                    io::ErrorKind::WouldBlock => return Ok(()),
                    _ => return Err(err),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn reset(&self) -> io::Result<()> {
        let mut buffer = [0u8; 64];
        loop {
            let read =
                unsafe { libc::read(self.read_fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read == -1 {
                let err = io::Error::last_os_error();
                match err.kind() {
                    io::ErrorKind::Interrupted => continue,
                    io::ErrorKind::WouldBlock => return Ok(()),
                    _ => return Err(err),
                }
            } else if read == 0 || read < buffer.len() as isize {
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
impl Drop for Unblocker {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read_fd);
            libc::close(self.write_fd);
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct Unblocker {
    handle: HANDLE,
}

#[cfg(windows)]
impl Unblocker {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateEventW(ptr::null_mut(), 0, 0, ptr::null()) };
        if handle == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self { handle })
        }
    }

    fn handle(&self) -> HANDLE {
        self.handle
    }

    fn notify(&self) -> io::Result<()> {
        let result = unsafe { SetEvent(self.handle) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn reset(&self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for Unblocker {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(all(not(unix), not(windows)))]
#[derive(Debug)]
struct Unblocker {
    state: Mutex<bool>,
    condvar: Condvar,
}

#[cfg(all(not(unix), not(windows)))]
impl Unblocker {
    fn new() -> io::Result<Self> {
        Ok(Self {
            state: Mutex::new(false),
            condvar: Condvar::new(),
        })
    }

    fn notify(&self) -> io::Result<()> {
        if let Ok(mut flag) = self.state.lock() {
            *flag = true;
            self.condvar.notify_all();
        }
        Ok(())
    }

    fn reset(&self) -> io::Result<()> {
        if let Ok(mut flag) = self.state.lock() {
            *flag = false;
        }
        Ok(())
    }

    fn wait(&self, timeout: Option<Duration>) {
        if let Ok(mut flag) = self.state.lock() {
            if *flag {
                *flag = false;
                return;
            }
            if let Some(dur) = timeout {
                let _ = self.condvar.wait_timeout(flag, dur);
            } else {
                let _ = self.condvar.wait(flag);
            }
        }
    }
}

type SchedulerErrorHook = Box<dyn Fn(&dyn StdSchedulerProc) + Send + Sync>;

pub struct StdScheduler {
    procs: Vec<ProcEntry>,
    on_error: Option<SchedulerErrorHook>,
    unblocker: Arc<Unblocker>,
}

impl StdScheduler {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            procs: Vec::new(),
            on_error: None,
            unblocker: Arc::new(Unblocker::new()?),
        })
    }

    pub fn set_on_error<F>(&mut self, handler: Option<F>)
    where
        F: Fn(&dyn StdSchedulerProc) + Send + Sync + 'static,
    {
        self.on_error = handler.map(|f| Box::new(f) as _);
    }

    pub fn proc_count(&self) -> usize {
        self.procs.len()
    }

    pub fn clear(&mut self) {
        self.procs.clear();
    }

    pub fn add(&mut self, proc: ProcHandle) {
        if !self
            .procs
            .iter()
            .any(|entry| Arc::ptr_eq(&entry.proc, &proc))
        {
            self.procs.push(ProcEntry { proc });
        }
    }

    pub fn remove(&mut self, proc: &ProcHandle) -> bool {
        if let Some(idx) = self
            .procs
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.proc, proc))
        {
            self.procs.swap_remove(idx);
            true
        } else {
            false
        }
    }

    #[cfg(unix)]
    pub fn execute(&mut self, mut timeout: i32) -> bool {
        if self.procs.is_empty() {
            return false;
        }

        for entry in &self.procs {
            let proc_timeout = entry.proc.get_timeout();
            if proc_timeout >= 0 && (timeout == INFINITE_TIMEOUT || timeout > proc_timeout) {
                timeout = proc_timeout;
            }
        }

        let unblocker = Arc::clone(&self.unblocker);
        let mut poll_fds: Vec<pollfd> = Vec::with_capacity(1 + self.procs.len() * 2);
        poll_fds.push(pollfd {
            fd: unblocker.poll_fd(),
            events: POLLIN,
            revents: 0,
        });

        let mut ranges: Vec<(ProcHandle, usize, usize)> = Vec::new();
        for entry in &self.procs {
            let start = poll_fds.len();
            for fd in entry.proc.get_fds() {
                poll_fds.push(pollfd {
                    fd: fd.fd(),
                    events: fd.events(),
                    revents: 0,
                });
            }
            let end = poll_fds.len();
            if end > start {
                ranges.push((Arc::clone(&entry.proc), start, end));
            }
        }

        let timeout_ms = if timeout < 0 { -1 } else { timeout };
        let poll_result =
            unsafe { libc::poll(poll_fds.as_mut_ptr(), poll_fds.len() as nfds_t, timeout_ms) };

        let mut success = true;

        if poll_result > 0 {
            if poll_fds[0].revents & POLLIN != 0 {
                let _ = unblocker.reset();
            }

            for (proc, start, end) in &ranges {
                if poll_fds[*start..*end].iter().any(|fd| fd.revents != 0) && !proc.execute(0) {
                    success = false;
                    self.handle_error(proc);
                }
            }
        } else if poll_result < 0 {
            let err = io::Error::last_os_error();
            tracing::error!(error = %err, "StdScheduler::execute poll failed");
            success = false;
        }

        for entry in &self.procs {
            if entry.proc.get_timeout() == 0 && !entry.proc.execute(INFINITE_TIMEOUT) {
                success = false;
                self.handle_error(&entry.proc);
            }
        }

        success
    }

    #[cfg(windows)]
    pub fn execute(&mut self, mut timeout: i32) -> bool {
        if self.procs.is_empty() {
            return false;
        }

        for entry in &self.procs {
            let proc_timeout = entry.proc.get_timeout();
            if proc_timeout >= 0 && (timeout == INFINITE_TIMEOUT || timeout > proc_timeout) {
                timeout = proc_timeout;
            }
        }

        let wait_timeout = if timeout < 0 {
            INFINITE
        } else {
            timeout as u32
        };

        let mut handles: Vec<HANDLE> = Vec::with_capacity(1 + self.procs.len());
        handles.push(self.unblocker.handle());

        struct HandleRange {
            proc: ProcHandle,
            start: usize,
            end: usize,
        }

        let mut ranges: Vec<HandleRange> = Vec::new();

        for entry in &self.procs {
            let proc = Arc::clone(&entry.proc);
            let handle_list = proc.get_handles();
            // C++ collects only procs that expose an event and never executes a
            // handle-less proc from this arm (`StdScheduler.cpp:86-119`). Such
            // procs run solely from the zero-timeout sweep below, so executing
            // them here too would run them twice per `execute`.
            if !handle_list.is_empty() {
                let start = handles.len();
                handles.extend(handle_list.iter().map(|interest| interest.handle()));
                let end = handles.len();
                ranges.push(HandleRange { proc, start, end });
            }
        }

        let mut success = true;
        let mut signalled: Vec<usize> = Vec::new();

        if !handles.is_empty() {
            let wait_result = unsafe {
                WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, wait_timeout)
            };

            match wait_result {
                WAIT_FAILED => {
                    let err = io::Error::last_os_error();
                    tracing::error!(error = %err, "StdScheduler::execute wait failed");
                    success = false;
                }
                WAIT_TIMEOUT => {}
                result => {
                    if let Some(idx) = decode_wait_index(result, handles.len()) {
                        signalled.push(idx);
                    }
                }
            }
        }

        if handles.len() > 1 {
            for (index, handle) in handles.iter().enumerate().skip(1) {
                if signalled.contains(&index) {
                    continue;
                }
                let status = unsafe { WaitForSingleObject(*handle, 0) };
                match status {
                    WAIT_FAILED => {
                        let err = io::Error::last_os_error();
                        tracing::error!(error = %err, "StdScheduler::execute wait failed");
                        success = false;
                    }
                    WAIT_OBJECT_0 | WAIT_ABANDONED_0 => {
                        signalled.push(index);
                    }
                    _ => {}
                }
            }
        }

        for idx in signalled {
            if idx == 0 {
                let _ = self.unblocker.reset();
                continue;
            }
            for range in &ranges {
                if idx >= range.start && idx < range.end {
                    if !range.proc.execute(0) {
                        success = false;
                        self.handle_error(&range.proc);
                    }
                    break;
                }
            }
        }

        for entry in &self.procs {
            if entry.proc.get_timeout() == 0 && !entry.proc.execute(INFINITE_TIMEOUT) {
                success = false;
                self.handle_error(&entry.proc);
            }
        }

        success
    }

    #[cfg(all(not(unix), not(windows)))]
    pub fn execute(&mut self, mut timeout: i32) -> bool {
        if self.procs.is_empty() {
            return false;
        }

        for entry in &self.procs {
            let proc_timeout = entry.proc.get_timeout();
            if proc_timeout >= 0 && (timeout == INFINITE_TIMEOUT || timeout > proc_timeout) {
                timeout = proc_timeout;
            }
        }

        let wait_duration = if timeout < 0 {
            None
        } else {
            Some(Duration::from_millis(timeout as u64))
        };

        self.unblocker.wait(wait_duration);
        let _ = self.unblocker.reset();

        let mut success = true;
        for entry in &self.procs {
            let should_run =
                timeout == 0 || entry.proc.get_timeout() == 0 || wait_duration.is_some();
            if should_run && !entry.proc.execute(timeout) {
                success = false;
                self.handle_error(&entry.proc);
            }
        }
        success
    }

    pub fn unblock(&self) {
        let _ = self.unblocker.notify();
    }

    fn handle_error(&self, proc: &ProcHandle) {
        if let Some(handler) = &self.on_error {
            handler(proc.as_ref());
        }
    }
}

#[cfg(windows)]
fn decode_wait_index(result: u32, handle_count: usize) -> Option<usize> {
    let count = handle_count as u32;
    // `WAIT_OBJECT_0` is 0, so the lower bound of the documented
    // `WAIT_OBJECT_0 <= result < WAIT_OBJECT_0 + count` range holds for every
    // `u32` and is left implicit rather than written as a vacuous comparison.
    if result < WAIT_OBJECT_0 + count {
        Some((result - WAIT_OBJECT_0) as usize)
    } else if result >= WAIT_ABANDONED_0 && result < WAIT_ABANDONED_0 + count {
        Some((result - WAIT_ABANDONED_0) as usize)
    } else {
        None
    }
}

pub struct StdSchedulerThread {
    scheduler: Arc<Mutex<StdScheduler>>,
    run_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    unblocker: Arc<Unblocker>,
}

impl StdSchedulerThread {
    pub fn new() -> io::Result<Self> {
        let scheduler = StdScheduler::new()?;
        let unblocker = Arc::clone(&scheduler.unblocker);
        Ok(Self {
            scheduler: Arc::new(Mutex::new(scheduler)),
            run_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
            unblocker,
        })
    }

    pub fn scheduler(&self) -> Arc<Mutex<StdScheduler>> {
        Arc::clone(&self.scheduler)
    }

    pub fn add(&mut self, proc: ProcHandle) -> io::Result<()> {
        let was_running = self.is_running();
        if was_running {
            self.stop()?;
        }
        self.scheduler.lock().unwrap().add(proc);
        if was_running {
            self.start()?;
        }
        Ok(())
    }

    pub fn remove(&mut self, proc: &ProcHandle) -> io::Result<bool> {
        let was_running = self.is_running();
        if was_running {
            self.stop()?;
        }
        let removed = self.scheduler.lock().unwrap().remove(proc);
        if was_running {
            self.start()?;
        }
        Ok(removed)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        let was_running = self.is_running();
        if was_running {
            self.stop()?;
        }
        self.scheduler.lock().unwrap().clear();
        Ok(())
    }

    pub fn start(&mut self) -> io::Result<()> {
        if self.is_running() {
            self.stop()?;
        }
        self.run_flag.store(true, Ordering::Release);
        let run_flag = Arc::clone(&self.run_flag);
        let scheduler = Arc::clone(&self.scheduler);
        let handle = thread::Builder::new()
            .name("StdSchedulerThread".into())
            .spawn(move || {
                while run_flag.load(Ordering::Acquire) {
                    let executed = {
                        let mut guard = scheduler.lock().unwrap();
                        guard.execute(INFINITE_TIMEOUT)
                    };
                    if !executed {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            })?;
        self.thread = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(handle) = self.thread.take() {
            self.run_flag.store(false, Ordering::Release);
            let _ = self.unblocker.notify();
            match handle.join() {
                Ok(_) => Ok(()),
                Err(_) => Err(io::Error::other("StdSchedulerThread panicked")),
            }
        } else {
            Ok(())
        }
    }

    fn is_running(&self) -> bool {
        self.thread.is_some()
    }
}

impl Drop for StdSchedulerThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct CountingProc {
        hits: AtomicUsize,
        timeout: i32,
    }

    impl CountingProc {
        fn new(timeout: i32) -> Self {
            Self {
                hits: AtomicUsize::new(0),
                timeout,
            }
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    impl StdSchedulerProc for CountingProc {
        fn execute(&self, _timeout: i32) -> bool {
            self.hits.fetch_add(1, Ordering::SeqCst);
            true
        }

        fn get_timeout(&self) -> i32 {
            self.timeout
        }
    }

    #[test]
    fn executes_zero_timeout_proc() {
        let mut scheduler = StdScheduler::new().unwrap();
        let proc = Arc::new(CountingProc::new(0));
        let handle: Arc<dyn StdSchedulerProc> = proc.clone();
        scheduler.add(handle.clone());
        assert!(scheduler.execute(INFINITE_TIMEOUT));
        assert_eq!(proc.hits(), 1);
    }

    #[test]
    fn add_ignores_duplicates() {
        let mut scheduler = StdScheduler::new().unwrap();
        let proc = Arc::new(CountingProc::new(-1));
        let handle: Arc<dyn StdSchedulerProc> = proc.clone();
        scheduler.add(handle.clone());
        scheduler.add(handle.clone());
        assert_eq!(scheduler.proc_count(), 1);
    }

    #[test]
    fn remove_returns_status() {
        let mut scheduler = StdScheduler::new().unwrap();
        let proc = Arc::new(CountingProc::new(-1));
        let handle: Arc<dyn StdSchedulerProc> = proc.clone();
        scheduler.add(handle.clone());
        assert!(scheduler.remove(&handle));
        assert!(!scheduler.remove(&handle));
    }

    #[cfg(unix)]
    struct PipeProc {
        read_fd: c_int,
        write_fd: c_int,
        hits: AtomicUsize,
    }

    #[cfg(unix)]
    impl PipeProc {
        fn new() -> io::Result<Self> {
            let mut fds = [0; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
                return Err(io::Error::last_os_error());
            }
            for &fd in &fds {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags == -1
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
                {
                    unsafe {
                        libc::close(fds[0]);
                        libc::close(fds[1]);
                    }
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(Self {
                read_fd: fds[0],
                write_fd: fds[1],
                hits: AtomicUsize::new(0),
            })
        }

        fn trigger(&self) -> io::Result<()> {
            let byte = [1u8];
            loop {
                let written = unsafe { libc::write(self.write_fd, byte.as_ptr().cast(), 1) };
                if written == -1 {
                    let err = io::Error::last_os_error();
                    match err.kind() {
                        io::ErrorKind::Interrupted => continue,
                        io::ErrorKind::WouldBlock => return Ok(()),
                        _ => return Err(err),
                    }
                } else {
                    return Ok(());
                }
            }
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    #[cfg(unix)]
    impl StdSchedulerProc for PipeProc {
        fn execute(&self, _timeout: i32) -> bool {
            let mut buffer = [0u8; 64];
            loop {
                let read =
                    unsafe { libc::read(self.read_fd, buffer.as_mut_ptr().cast(), buffer.len()) };
                if read == -1 {
                    let err = io::Error::last_os_error();
                    match err.kind() {
                        io::ErrorKind::Interrupted => continue,
                        io::ErrorKind::WouldBlock => break,
                        _ => return false,
                    }
                } else if read == 0 {
                    break;
                }
            }
            self.hits.fetch_add(1, Ordering::SeqCst);
            true
        }

        fn get_fds(&self) -> Vec<FdInterest> {
            vec![FdInterest::read(self.read_fd)]
        }
    }

    #[cfg(unix)]
    impl Drop for PipeProc {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.read_fd);
                libc::close(self.write_fd);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn executes_when_fd_ready() {
        let mut scheduler = StdScheduler::new().unwrap();
        let proc = Arc::new(PipeProc::new().unwrap());
        let handle: Arc<dyn StdSchedulerProc> = proc.clone();
        scheduler.add(handle);
        proc.trigger().unwrap();
        assert!(scheduler.execute(250));
        assert_eq!(proc.hits(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn scheduler_thread_processes_events() {
        let proc = Arc::new(PipeProc::new().unwrap());
        let mut thread = StdSchedulerThread::new().unwrap();
        let handle: Arc<dyn StdSchedulerProc> = proc.clone();
        thread.add(handle).unwrap();
        thread.start().unwrap();
        proc.trigger().unwrap();

        let start = Instant::now();
        while proc.hits() == 0 && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(proc.hits() >= 1);
        thread.stop().unwrap();
    }
}
