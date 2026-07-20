use std::sync::OnceLock;

use tracing_subscriber::{fmt, EnvFilter};

static INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialise the global tracing subscriber used across the LegacyClonk binaries.
///
/// Accepts filter directives from the `LC_LOG` environment variable, falling back to `RUST_LOG`
/// and ultimately to `info` level logging when no directives are provided.
pub fn init() {
    init_with_default_level("info");
}

/// Initialise tracing, defaulting to `debug` when `verbose` is true.
///
/// Explicit `LC_LOG` and `RUST_LOG` directives keep the same precedence as [`init`].
pub fn init_verbose(verbose: bool) {
    init_with_default_level(if verbose { "debug" } else { "info" });
}

fn init_with_default_level(default_level: &'static str) {
    INITIALIZED.get_or_init(|| {
        let lc_log = std::env::var("LC_LOG").ok();
        let rust_log = std::env::var("RUST_LOG").ok();
        let directive =
            select_filter_directive(lc_log.as_deref(), rust_log.as_deref(), default_level);
        let filter = EnvFilter::new(directive);

        let _ = fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_level(true)
            .try_init();
    });
}

#[doc(hidden)]
pub fn select_filter_directive<'a>(
    lc_log: Option<&'a str>,
    rust_log: Option<&'a str>,
    default_level: &'a str,
) -> &'a str {
    lc_log
        .or(rust_log)
        .filter(|directive| EnvFilter::try_new(directive).is_ok())
        .unwrap_or(default_level)
}
