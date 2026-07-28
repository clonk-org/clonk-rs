//! Client-side core of the in-app updater.
//!
//! This crate answers four questions and nothing else:
//!
//! * *What did the server publish?* — [`manifest`] parses the document a client
//!   fetches from `releases/latest/download/manifest.json`.
//! * *What is already here?* — [`state`] persists which component digests are
//!   currently installed, beside the install root.
//! * *Should we act, and on what?* — [`decide`] ports the gating C++ performs in
//!   `C4UpdateDlg::IsValidUpdate` (`C4UpdateDlg.cpp:246-260`).
//! * *Is the payload the one that was promised, and is it safe to unpack?* —
//!   [`digest`] and [`extract`].
//!
//! It is deliberately **UI-free and network-free**: no `reqwest`, no `tokio`, no
//! dialog types. Fetching lives in `clonk-update-net`; presenting lives in
//! `clonk-app`. `clonk-game` owns *applying* an update and must never link an
//! HTTP client, which is only possible because this crate stays transport-free.
//!
//! # Trust model
//!
//! There is no manifest signature, by decision rather than omission. Releases
//! are published daily by CI, so a signing key would have to live in CI, where
//! anyone able to publish a release could equally invoke the workflow that
//! signs. Integrity therefore rests on TLS for the manifest fetch plus the
//! SHA-256 this crate verifies for every component archive
//! ([`digest::verify_reader`]), and on the extraction guards in [`extract`],
//! which assume the archive is hostile even after its digest matches.

pub mod decide;
pub mod digest;
pub mod extract;
pub mod journal;
pub mod manifest;
pub mod state;

pub use decide::{
    decide, decide_for_this_build, should_check_for_updates, Decision, PlannedComponent,
    RefusalReason,
};
pub use digest::{sha256_file, sha256_reader, verify_file, verify_reader, DigestError};
pub use extract::{extract_archive, EntryFault, ExtractError, ExtractSummary};
pub use journal::{
    Journal, JournalError, JournalStep, StepState, JOURNAL_FILE_NAME, JOURNAL_SCHEMA,
};
pub use manifest::{ComponentEntry, Manifest, ManifestError, TargetArchive, SUPPORTED_SCHEMA};
pub use state::{InstalledComponent, InstalledState, StateError};
