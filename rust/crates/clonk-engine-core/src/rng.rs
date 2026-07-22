/// C++ LCG random number generator (C4Random.h / C4Random.cpp).
///
/// Replaces the previous ChaCha8Rng port with the exact algorithm used by the
/// C++ engine so that deterministic simulation (lockstep / replay) can match
/// bit-for-bit.
///
/// C++ oracle: `src/C4Random.h` lines 29-75, `src/C4Random.cpp` lines 24-42.
use serde::{Deserialize, Serialize};

/// Number of entries in the Rnd3 circular buffer. C4Random.cpp:24.
const RND3_SIZE: usize = 500;

/// Deterministic LCG matching C++ `RandomHold`/`RandomCount` + `FRndBuf3`.
///
/// `hold`  = `RandomHold`  (unsigned int in C++)
/// `count` = `RandomCount` (int in C++)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcgRng {
    pub hold: u32,
    pub count: i32,
    /// Temp forensics: when set, draws append to LC_RUST_RNG_TRACE.
    #[serde(skip, default)]
    pub trace: bool,
    /// Pre-computed Rnd3 circular buffer (FRndBuf3). C4Random.cpp:26.
    rnd3_buf: Vec<i32>,
    /// Current buffer index (FRndPtr3). C4Random.cpp:27.
    rnd3_ptr: usize,
}

impl LcgRng {
    /// Create with explicit seed (`FixedRandom` equivalent).
    /// Rnd3 buffer is zero-initialized; call `randomize3()` after seeding to
    /// match C++ game-start behaviour. C4Random.h:32-38.
    pub fn new(seed: u32) -> Self {
        Self {
            hold: seed,
            count: 0,
            trace: false,
            rnd3_buf: vec![0i32; RND3_SIZE],
            rnd3_ptr: 0,
        }
    }

    /// Engine-start seeding: `FixedRandom(seed); Randomize3();`.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut rng = Self::new(seed as u32);
        rng.randomize3();
        rng
    }

    /// Equivalent to C++ `Random(range)`. C4Random.h:40-61.
    /// Advances `hold` and `count`; returns `(hold >> 16) % range`.
    pub fn random(&mut self, range: i32) -> i32 {
        self.count = self.count.wrapping_add(1);
        if range == 0 {
            return 0;
        }
        self.hold = self.hold.wrapping_mul(214013).wrapping_add(2531011);
        let value = ((self.hold >> 16) % range as u32) as i32;
        if self.trace {
            rng_trace(self.count, range, value);
        }
        value
    }

    /// Stateless one-shot LCG step. C4Random.h:64-69.
    pub fn seeded_random(seed: u32, range: u32) -> u32 {
        if range == 0 {
            return 0;
        }
        let s = seed.wrapping_mul(214013).wrapping_add(2531011);
        (s >> 16) % range
    }

    /// Fill the Rnd3 circular buffer by calling `random(3)` 500 times.
    /// Must be called once after seeding. C4Random.cpp:29-33.
    pub fn randomize3(&mut self) {
        self.rnd3_ptr = 0;
        if self.rnd3_buf.len() != RND3_SIZE {
            self.rnd3_buf.resize(RND3_SIZE, 0);
        }
        for index in 0..RND3_SIZE {
            self.rnd3_buf[index] = self.random(3) - 1; // maps {0,1,2} → {-1,0,1}
        }
    }

    /// Current Rnd3 ring pointer (`FRndPtr3`) — part of the C4ControlSyncCheck
    /// digest (C4Control.cpp:445-450).
    pub fn rnd3_ptr(&self) -> i32 {
        self.rnd3_ptr as i32
    }

    /// Read next element from the Rnd3 circular buffer. C4Random.cpp:35-42.
    /// Increments the pointer *before* reading (matches C++ `FRndPtr3++` then
    /// wrap then read).
    pub fn rnd3(&mut self) -> i32 {
        if self.rnd3_buf.len() != RND3_SIZE {
            self.randomize3();
        }
        self.rnd3_ptr += 1;
        if self.rnd3_ptr == RND3_SIZE {
            self.rnd3_ptr = 0;
        }
        self.rnd3_buf[self.rnd3_ptr]
    }
}

impl Default for LcgRng {
    fn default() -> Self {
        Self::new(0)
    }
}

// ── rand_core::RngCore impl ──────────────────────────────────────────────────
// Lets existing `gen_range` / `gen` / `SliceRandom` call-sites in lib.rs
// continue to compile unchanged.  The implementation advances `hold` by one
// LCG step and returns the full 32-bit state — different from `random()` which
// returns only bits 31-16 with modulo.  Non-determinism-critical call-sites
// (landscape spawn, float jitter) keep working; determinism-critical ones have
// been migrated to `rng.random(range)`.

impl rand_core::RngCore for LcgRng {
    #[inline]
    fn next_u32(&mut self) -> u32 {
        self.count = self.count.wrapping_add(1);
        self.hold = self.hold.wrapping_mul(214013).wrapping_add(2531011);
        self.hold
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let lo = self.next_u32() as u64;
        let hi = self.next_u32() as u64;
        lo | (hi << 32)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut chunks = dest.chunks_exact_mut(4);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&self.next_u32().to_le_bytes());
        }
        let tail = chunks.into_remainder();
        if !tail.is_empty() {
            let bytes = self.next_u32().to_le_bytes();
            tail.copy_from_slice(&bytes[..tail.len()]);
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Temp forensics: append draws to LC_RUST_RNG_TRACE (mirrors the C++
/// C4Random.h probe; alignment by count/range/value).
fn trace_file() -> &'static Option<std::sync::Mutex<std::fs::File>> {
    use std::sync::OnceLock;
    static TRACE: OnceLock<Option<std::sync::Mutex<std::fs::File>>> = OnceLock::new();
    TRACE.get_or_init(|| {
        std::env::var("LC_RUST_RNG_TRACE")
            .ok()
            .and_then(|path| std::fs::File::create(path).ok())
            .map(std::sync::Mutex::new)
    })
}

fn rng_trace(count: i32, range: i32, value: i32) {
    use std::io::Write;
    if let Some(file) = trace_file() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{count} {range} {value}");
        }
    }
}

/// Ledger-forensics free-form probe line (env-gated diagnostics).
pub fn rng_trace_line(text: &str) {
    use std::io::Write;
    if let Some(file) = trace_file() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
}

/// Ledger-forensics frame marker: mirrors the C++ probe's `FRAME n`
/// lines so per-frame draw censuses align (env-gated, diagnostic only).
pub fn rng_trace_frame_marker(frame: u64) {
    use std::io::Write;
    if let Some(file) = trace_file() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "FRAME {frame}");
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: one raw LCG step on hold, returns (hold>>16)%range.
    /// Used as an independent reference implementation in tests.
    fn cpp_random_step(hold: &mut u32, range: i32) -> i32 {
        if range == 0 {
            return 0;
        }
        *hold = hold.wrapping_mul(214013).wrapping_add(2531011);
        ((*hold >> 16) % range as u32) as i32
    }

    #[test]
    fn lcg_first_call_seed_zero() {
        // C4Random.h:58-60: hold = hold*214013+2531011; result = (hold>>16)%100
        // hold=0 → 0*214013+2531011 = 2531011; (2531011>>16) = 38; 38%100 = 38
        let mut rng = LcgRng::new(0);
        assert_eq!(rng.random(100), 38);
    }

    #[test]
    fn lcg_sequence_matches_independent_reference() {
        // Verify a sequence of 20 calls against the independently-computed
        // C++ LCG formula (cpp_random_step above). C4Random.h:52-60.
        let mut hold = 42u32;
        let mut rng = LcgRng::new(42);
        for range in [
            1, 2, 3, 5, 7, 10, 13, 37, 100, 256, 1000, 1024, 65535, 65536, 3, 100, 7, 5, 2, 1,
        ] {
            let expected = cpp_random_step(&mut hold, range);
            let got = rng.random(range);
            assert_eq!(got, expected, "range={range}");
        }
    }

    #[test]
    fn random_zero_range_returns_zero() {
        // C4Random.h:46-47: if (iRange == 0) return 0
        let mut rng = LcgRng::new(0);
        assert_eq!(rng.random(0), 0);
    }

    #[test]
    fn random_zero_range_still_increments_count() {
        // C4Random.h:43: RandomCount++ is unconditional
        let mut rng = LcgRng::new(0);
        rng.random(0);
        assert_eq!(rng.count, 1);
    }

    #[test]
    fn random_count_increments_each_call() {
        let mut rng = LcgRng::new(0);
        for i in 1..=10 {
            rng.random(100);
            assert_eq!(rng.count, i);
        }
    }

    #[test]
    fn random_result_in_range() {
        let mut rng = LcgRng::new(12345);
        for range in [1, 2, 3, 5, 10, 100, 1000] {
            let v = rng.random(range);
            assert!(v >= 0 && v < range, "range={range} got {v}");
        }
    }

    #[test]
    fn seeded_random_is_stateless() {
        // Same inputs always produce the same output, no side-effects on state.
        let a = LcgRng::seeded_random(999, 100);
        let b = LcgRng::seeded_random(999, 100);
        assert_eq!(a, b);
    }

    #[test]
    fn seeded_random_zero_range() {
        assert_eq!(LcgRng::seeded_random(12345, 0), 0);
    }

    #[test]
    fn seeded_random_matches_cpp() {
        // C4Random.h:64-69:
        // iSeed = iSeed*214013+2531011; return (iSeed>>16)%iRange
        let seed = 7u32;
        let range = 100u32;
        let expected = {
            let s = seed.wrapping_mul(214013).wrapping_add(2531011);
            (s >> 16) % range
        };
        assert_eq!(LcgRng::seeded_random(seed, range), expected);
    }

    #[test]
    fn randomize3_fills_buffer_with_minus1_0_1() {
        let mut rng = LcgRng::new(0);
        rng.randomize3();
        // All values in the buffer should be -1, 0, or 1
        for v in &rng.rnd3_buf {
            assert!(*v == -1 || *v == 0 || *v == 1, "got {v}");
        }
    }

    #[test]
    fn randomize3_uses_lcg_random3() {
        // The buffer must match calling random(3)-1 independently. C4Random.cpp:31-32.
        let seed = 77u32;
        let mut hold = seed;
        let mut rng = LcgRng::new(seed);
        rng.randomize3();
        for i in 0..500 {
            let expected = cpp_random_step(&mut hold, 3) - 1;
            assert_eq!(rng.rnd3_buf[i], expected, "index={i}");
        }
    }

    #[test]
    fn rnd3_first_call_returns_buf_index_1() {
        // C4Random.cpp:37: FRndPtr3++ THEN read — so first call after
        // Randomize3 (which sets ptr=0) returns buf[1], not buf[0].
        let mut rng = LcgRng::new(0);
        rng.randomize3();
        let first = rng.rnd3();
        assert_eq!(first, rng.rnd3_buf[1]);
    }

    #[test]
    fn rnd3_wraps_at_500() {
        let mut rng = LcgRng::new(0);
        rng.randomize3();
        // Advance to just before wrap
        for _ in 0..(500 - 1) {
            rng.rnd3();
        }
        // ptr is now 499; next call wraps back to 0 then reads buf[0]
        let val = rng.rnd3();
        assert_eq!(val, rng.rnd3_buf[0]);
    }

    #[test]
    fn rnd3_sequence_deterministic_across_clones() {
        let mut a = LcgRng::new(42);
        a.randomize3();
        let mut b = a.clone();
        for _ in 0..100 {
            assert_eq!(a.rnd3(), b.rnd3());
        }
    }
}

