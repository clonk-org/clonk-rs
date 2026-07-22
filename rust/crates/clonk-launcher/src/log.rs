use anyhow::Result;

/// Trait for logging launcher progress without tying to a specific backend.
pub trait LauncherLog: Send + Sync {
    fn log_line(&self, message: &str) -> Result<()>;
}
