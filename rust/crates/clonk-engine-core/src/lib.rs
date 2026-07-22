//! Determinism-critical primitives shared by the engine: C4Fixed 16.16
//! fixed-point math (with the bit-exact sine table) and the C++ LCG RNG.
//!
//! These change rarely; keeping them out of clonk-engine means an engine edit no
//! longer re-parses the 9k-line sine table or recompiles the math/rng units.

pub mod math;
pub mod rng;
