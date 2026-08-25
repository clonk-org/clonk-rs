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
    /// Temp forensics: nonzero routes draws to this engine's own trace file
    /// under LC_RUST_RNG_TRACE. Zero means untraced.
    #[serde(skip, default)]
    pub trace_index: u32,
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
            trace_index: 0,
            rnd3_buf: vec![0i32; RND3_SIZE],
            rnd3_ptr: 0,
        }
    }

    /// Engine-start seeding: `FixedRandom(seed); Randomize3();`.
    pub fn seed_from_u64(seed: u64) -> Self {
        Self::seed_from_u64_traced(seed, false)
    }

    /// Seed with tracing armed *before* the Rnd3 fill.
    ///
    /// `randomize3` spends 500 `Random(3)` draws, and native traces them:
    /// `Randomize3` runs after `LcRngTraceFile` is available, so an oracle
    /// trace opens with 500 `range=3` lines. Arming the flag afterwards
    /// silently drops those 500 from this side, which offsets any head-aligned
    /// diff against the oracle by exactly 500 draws
    /// (clonk-org/clonk-rs#1050).
    pub fn seed_from_u64_traced(seed: u64, trace: bool) -> Self {
        let mut rng = Self::new(seed as u32);
        rng.trace_index = if trace { next_trace_index() } else { 0 };
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
        if self.trace_index != 0 {
            rng_trace(self.trace_index, self.count, range, value);
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

// ── rand_core::TryRng impl ──────────────────────────────────────────────────
// This adapter advances `hold` by one LCG step and returns the full 32-bit
// state, preserving the raw-word contract from rand_core 0.6. That is
// deliberately different from C++ `Random()`, which projects bits 31-16 and
// applies modulo. Lockstep call-sites must use `random(range)`, never `RngExt`
// distributions whose rejection sampling may consume a different draw count.

impl rand_core::TryRng for LcgRng {
    type Error = rand_core::Infallible;

    #[inline]
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.count = self.count.wrapping_add(1);
        self.hold = self.hold.wrapping_mul(214013).wrapping_add(2531011);
        Ok(self.hold)
    }

    #[inline]
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let lo = u64::from(self.try_next_u32()?);
        let hi = u64::from(self.try_next_u32()?);
        Ok(lo | (hi << 32))
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
        let mut chunks = dest.chunks_exact_mut(4);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&self.try_next_u32()?.to_le_bytes());
        }
        let tail = chunks.into_remainder();
        if !tail.is_empty() {
            let bytes = self.try_next_u32()?.to_le_bytes();
            tail.copy_from_slice(&bytes[..tail.len()]);
        }
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// The file one engine's draws are written to.
///
/// The sink used to be a single process-global file while `RandomCount` is
/// per-engine, so a process that builds several engines interleaved their
/// streams with colliding counters — which reads as duplicated counts and
/// breaks any diff against the oracle. Giving each engine its own file keeps
/// every line in the oracle's exact `count range value` format while making
/// each file a single coherent stream.
///
/// The first engine keeps the base path, so an existing single-engine workflow
/// is unchanged; later engines get `.2`, `.3` and so on. After a run, the file
/// carrying `FRAME` markers is the engine that played the round — which is a
/// property of the output rather than a guess at construction order
/// (clonk-org/clonk-rs#1050).
pub(crate) fn trace_path_for_engine(base: &str, index: u32) -> String {
    if index <= 1 {
        base.to_string()
    } else {
        format!("{base}.{index}")
    }
}

/// Temp forensics: append draws to this engine's LC_RUST_RNG_TRACE file
/// (mirrors the C++ C4Random.h probe; alignment by count/range/value).
fn trace_base_path() -> Option<&'static str> {
    static BASE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| std::env::var("LC_RUST_RNG_TRACE").ok())
        .as_deref()
}

/// Ordinal of the next traced engine in this process, starting at 1.
fn next_trace_index() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

type TraceSinks = std::sync::Mutex<std::collections::HashMap<u32, Option<std::fs::File>>>;

fn trace_sinks() -> &'static TraceSinks {
    static SINKS: std::sync::OnceLock<TraceSinks> = std::sync::OnceLock::new();
    SINKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn write_trace(index: u32, line: &str) {
    use std::io::Write;
    let Some(base) = trace_base_path() else {
        return;
    };
    let Ok(mut sinks) = trace_sinks().lock() else {
        return;
    };
    let sink = sinks
        .entry(index)
        .or_insert_with(|| std::fs::File::create(trace_path_for_engine(base, index)).ok());
    if let Some(file) = sink.as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

fn rng_trace(index: u32, count: i32, range: i32, value: i32) {
    write_trace(index, &format!("{count} {range} {value}"));
}

/// Ledger-forensics free-form probe line (env-gated diagnostics).
pub fn rng_trace_line(index: u32, text: &str) {
    write_trace(index, text);
}

/// Ledger-forensics frame marker: mirrors the C++ probe's `FRAME n`
/// lines so per-frame draw censuses align (env-gated, diagnostic only).
pub fn rng_trace_frame_marker(index: u32, frame: u64) {
    write_trace(index, &format!("FRAME {frame}"));
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

    /// One state transition from src/C4Random.h:70,77, without C++ Random's
    /// high-word/modulo projection.
    fn raw_lcg_step(hold: &mut u32) -> u32 {
        *hold = hold.wrapping_mul(214013).wrapping_add(2531011);
        *hold
    }

    #[test]
    fn rng_core_next_u64_preserves_word_order_and_draw_count() {
        use rand_core::Rng as _;

        // The Rust adapter must not hide, repeat, or reorder either underlying
        // src/C4Random.h:70,77 state transition when rand_core changes traits.
        let seed = 0x1234_5678;
        let mut expected_hold = seed;
        let low = raw_lcg_step(&mut expected_hold);
        let high = raw_lcg_step(&mut expected_hold);

        let mut rng = LcgRng::new(seed);
        assert_eq!(rng.next_u64(), u64::from(low) | (u64::from(high) << 32));
        assert_eq!(rng.count, 2);
        assert_eq!(rng.hold, expected_hold);
    }

    #[test]
    fn rng_core_fill_bytes_preserves_little_endian_tail_and_draw_count() {
        use rand_core::Rng as _;

        // Each complete or partial four-byte word consumes exactly one
        // src/C4Random.h:70,77 transition; the partial word keeps its low bytes.
        let seed = 0x89ab_cdef;
        let mut expected_hold = seed;
        let first = raw_lcg_step(&mut expected_hold).to_le_bytes();
        let second = raw_lcg_step(&mut expected_hold).to_le_bytes();
        let expected = [
            first[0], first[1], first[2], first[3], second[0], second[1], second[2],
        ];

        let mut rng = LcgRng::new(seed);
        let mut actual = [0; 7];
        rng.fill_bytes(&mut actual);

        assert_eq!(actual, expected);
        assert_eq!(rng.count, 2);
        assert_eq!(rng.hold, expected_hold);
    }

    #[test]
    fn rng_core_empty_fill_consumes_no_draw() {
        use rand_core::Rng as _;

        let mut rng = LcgRng::new(42);
        rng.fill_bytes(&mut []);
        assert_eq!(rng.count, 0);
        assert_eq!(rng.hold, 42);
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
    fn each_engine_gets_its_own_trace_file() {
        // One process builds many engines; RandomCount is per-engine, so a
        // shared sink interleaves streams with colliding counters and breaks
        // any diff against the oracle. Per-engine files keep each stream
        // coherent and each line in the oracle's exact format
        // (clonk-org/clonk-rs#1050).
        //
        // The first engine keeps the base path, so an existing single-engine
        // workflow is unchanged.
        assert_eq!(trace_path_for_engine("/tmp/rng.txt", 1), "/tmp/rng.txt");
        assert_eq!(trace_path_for_engine("/tmp/rng.txt", 0), "/tmp/rng.txt");
        assert_eq!(trace_path_for_engine("/tmp/rng.txt", 2), "/tmp/rng.txt.2");
        assert_eq!(trace_path_for_engine("/tmp/rng.txt", 31), "/tmp/rng.txt.31");
    }

    #[test]
    fn seeding_arms_the_trace_before_the_rnd3_fill() {
        // Native's Randomize3 runs with LcRngTraceFile already available, so an
        // oracle trace opens with 500 `range=3` lines (C4Random.h:40-47,76-83).
        // Arming this side's flag after the fill drops those 500 draws from the
        // trace and offsets every head-aligned diff by exactly that much --
        // which is a silent wrong answer, not a missing file
        // (clonk-org/clonk-rs#1050).
        let traced = LcgRng::seed_from_u64_traced(7, true);
        assert_ne!(traced.trace_index, 0, "the sink must survive seeding");
        assert_eq!(traced.count, RND3_SIZE as i32, "the Rnd3 fill still runs");

        // Arming it changes nothing about the stream itself: same hold, same
        // count, same buffer as the untraced constructor.
        let plain = LcgRng::seed_from_u64(7);
        assert_eq!(plain.trace_index, 0);
        assert_eq!(traced.count, plain.count);
        assert_eq!(traced.hold, plain.hold);
        assert_eq!(traced.rnd3_buf, plain.rnd3_buf);
        assert_eq!(traced.rnd3_ptr, plain.rnd3_ptr);
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
