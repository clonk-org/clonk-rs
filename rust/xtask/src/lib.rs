//! Lightweight developer-feedback tooling.
//!
//! Engine-backed packaging and parity commands live in the optional
//! `xtask-engine-tools` binary so invoking `dev-check` does not build the game
//! engine or resource stack before it can select focused checks.

pub mod dev_check;
