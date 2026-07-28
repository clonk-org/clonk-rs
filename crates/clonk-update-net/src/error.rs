//! The single failure type every transport operation reports.

use std::path::PathBuf;
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
    #[error("update URL {url:?} cannot be parsed")]
    InvalidUrl { url: String },
    #[error("the update HTTP client could not be built: {0}")]
    Client(#[source] reqwest::Error),
    #[error("no async runtime is available for the updater: {0}")]
    RuntimeUnavailable(#[source] std::io::Error),
    #[error("the updater cannot block inside an async runtime that is already running")]
    NestedRuntime,
    #[error("update request to {url} failed: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("update request to {url} answered HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("update fetch was redirected to {url}, which is not served over https")]
    InsecureRedirect { url: String },
    #[error("update fetch was redirected to host {host}, which publishes no releases")]
    RedirectHostNotAllowed { host: String, url: String },
    #[error("{url} answered a redirect with no Location")]
    RedirectWithoutLocation { url: String },
    #[error("update fetch from {url} was redirected more than {limit} times")]
    TooManyRedirects { url: String, limit: usize },
    #[error("update manifest is larger than the {limit}-byte limit this client will read")]
    ManifestTooLarge { declared: Option<u64>, limit: u64 },
    #[error("{url} served a component without declaring its size")]
    UndeclaredSize { url: String },
    #[error("component body is longer than the {declared} bytes it declared")]
    BodyLongerThanDeclared { declared: u64, written: u64 },
    #[error("the update download was cancelled")]
    Cancelled,
    #[error("failed to write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
