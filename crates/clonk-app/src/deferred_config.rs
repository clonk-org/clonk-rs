//! Runtime config changes held in memory until a clean shutdown.
//!
//! C++ mutates the process-wide `Config` object for ordinary runtime toggles
//! and writes it once during `C4Application::Clear` (`C4Application.cpp:351-367`).
//! `C4StartupNetDlg::OnBtnInternet`/`OnBtnRecord` are the clearest examples:
//! they flip `Config.Network.MasterServerSignUp` and `Config.General.Record`
//! and never touch the file (`C4StartupNetDlg.cpp:840-850`).
//!
//! The port wrote each toggle straight through, so a transient change survived
//! a crash that C++ would have discarded, and every toggle rewrote the whole
//! file. This holds them instead. Explicit save surfaces — the ones C++ also
//! writes immediately — keep calling `persist_config_value` directly.

use std::collections::BTreeMap;

/// Pending `(section, key) -> value` writes, in a deterministic order so a
/// flush produces a stable file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeferredConfig {
    pending: BTreeMap<(String, String), String>,
}

impl DeferredConfig {
    /// Records a runtime change. A later change to the same key replaces the
    /// earlier one, so a toggle flipped twice writes once — or not at all, if
    /// it is flipped back to a value already on disk and the caller drops it.
    pub(crate) fn set(
        &mut self,
        section: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.pending
            .insert((section.into(), key.into()), value.into());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    /// The pending value for a key, which is what the running session should
    /// read back rather than the stale file.
    pub(crate) fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.pending
            .get(&(section.to_owned(), key.to_owned()))
            .map(String::as_str)
    }

    /// Takes the pending writes grouped by section, ready for one
    /// `persist_native_config_values` call each. Leaves the store empty, so a
    /// second flush is a no-op and an aborted run that never flushes discards
    /// everything — which is the point.
    pub(crate) fn take_by_section(&mut self) -> Vec<(String, Vec<(String, String)>)> {
        let mut grouped: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for ((section, key), value) in std::mem::take(&mut self.pending) {
            grouped.entry(section).or_default().push((key, value));
        }
        grouped.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Application.cpp:351-367; C4StartupNetDlg.cpp:840-850 — runtime toggles
    // accumulate in memory and reach the file only on a clean shutdown.
    #[test]
    fn runtime_config_mutations_remain_process_local_until_shutdown_save() {
        let mut config = DeferredConfig::default();
        assert!(config.is_empty());

        config.set("Network", "MasterServerSignUp", "1");
        config.set("General", "Record", "1");
        assert_eq!(config.len(), 2);
        // The running session reads its own pending value, not the stale file.
        assert_eq!(config.get("Network", "MasterServerSignUp"), Some("1"));
        assert_eq!(config.get("General", "Missing"), None);

        // Flipping the same toggle again replaces rather than queueing twice,
        // so a shutdown writes each key once.
        config.set("Network", "MasterServerSignUp", "0");
        assert_eq!(config.len(), 2);
        assert_eq!(config.get("Network", "MasterServerSignUp"), Some("0"));

        // A flush groups by section, in a deterministic order.
        let flushed = config.take_by_section();
        assert_eq!(
            flushed,
            vec![
                (
                    "General".to_owned(),
                    vec![("Record".to_owned(), "1".to_owned())]
                ),
                (
                    "Network".to_owned(),
                    vec![("MasterServerSignUp".to_owned(), "0".to_owned())]
                ),
            ]
        );

        // Flushing empties the store: a second flush writes nothing, and an
        // aborted run that never flushes discards every pending change.
        assert!(config.is_empty());
        assert!(config.take_by_section().is_empty());
        assert_eq!(config.get("General", "Record"), None);

        let mut aborted = DeferredConfig::default();
        aborted.set("General", "Record", "1");
        drop(aborted);
    }
}
