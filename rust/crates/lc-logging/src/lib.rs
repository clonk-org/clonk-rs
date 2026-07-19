use std::sync::OnceLock;

use tracing_subscriber::{fmt, EnvFilter};

static INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialise the global tracing subscriber used across the LegacyClonk binaries.
///
/// Accepts filter directives from the `LC_LOG` environment variable, falling back to `RUST_LOG`
/// and ultimately to `info` level logging when no directives are provided.
pub fn init() {
    INITIALIZED.get_or_init(|| {
        let filter = std::env::var("LC_LOG")
            .or_else(|_| std::env::var("RUST_LOG"))
            .ok()
            .and_then(|directive| EnvFilter::try_new(directive).ok())
            .unwrap_or_else(|| EnvFilter::new("info"));

        let _ = fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_level(true)
            .try_init();
    });
}
