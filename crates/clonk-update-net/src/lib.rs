//! HTTP transport for the in-app updater, and nothing else.
//!
//! [`clonk_update`] answers what the server published and whether to act;
//! `clonk-game` applies the result. This crate only moves bytes, and exists as
//! a separate crate purely so that `clonk-game` — which owns *apply* and runs
//! inside the shipped engine — never links `reqwest` or `tokio`.
//!
//! It is deliberately not routed through `clonk-network`, whose HTTP policy is
//! built for the league server: cookies, gzip (pure waste on already-deflated
//! zips) and a 20-second whole-request timeout that a 300 MB component download
//! could never meet. Reusing it would also drag `clonk-engine` and
//! `clonk-resources` into the updater.
//!
//! # Trust model
//!
//! There is no manifest signature. Integrity rests on TLS for the fetch —
//! `rustls` with its bundled roots, so a tampered system trust store does not
//! silently apply — plus the SHA-256 `clonk-update` verifies per component
//! archive afterwards. This crate therefore refuses anything that would move
//! the transfer off that footing: a redirect to an unexpected host, a plaintext
//! hop, an endless redirect chain, or a body larger than was declared.

pub mod error;
pub mod transport;
pub mod urls;

pub use error::TransportError;
pub use transport::{HttpTransport, UpdateTransport, MANIFEST_MAX_BYTES, MAX_REDIRECTS};
pub use urls::{
    archive_url_for, component_archive_url, default_manifest_url, manifest_url,
    ALLOWED_REDIRECT_HOSTS, DEFAULT_UPDATE_BASE_URL, MANIFEST_FILE_NAME,
};
