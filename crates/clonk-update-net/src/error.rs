//! The single failure type every transport operation reports.

use thiserror::Error;

/// Why a fetch did not produce the bytes that were asked for.
///
/// Everything here is reachable from network input, so no path in this crate
/// unwraps or panics its way out of one of these.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TransportError {
    #[error("update release version {0:?} is not a plain release tag")]
    UnsafeReleaseVersion(String),
    #[error("component archive {0:?} is not a plain asset file name")]
    UnsafeArchiveName(String),
}
