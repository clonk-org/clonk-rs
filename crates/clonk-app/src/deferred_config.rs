//! Runtime config changes held in memory until a clean shutdown.
//!
//! C++ mutates the process-wide `Config` object for ordinary runtime toggles
//! and writes it once during `C4Application::Quit` (`C4Application.cpp:351-367`;
//! `C4Application::Clear` at `:304-331` saves nothing).
//! `C4StartupNetDlg::OnBtnInternet`/`OnBtnRecord` are the clearest examples:
//! they flip `Config.Network.MasterServerSignUp` and `Config.General.Record`
//! and never touch the file (`C4StartupNetDlg.cpp:840-850`).
//!
//! The port wrote each toggle straight through, so a transient change survived
//! a crash that C++ would have discarded, and every toggle rewrote the whole
//! file. This holds them instead. Explicit save surfaces — the ones C++ also
//! writes immediately — keep calling `persist_config_value` directly.

use std::collections::BTreeMap;

/// A pending value together with the native writer its field needs. C++
/// stores a `CFG_MaxString` escaped-string field differently from an unquoted
/// scalar, so the store has to carry the distinction to the flush.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DeferredValue {
    /// An unquoted single-line scalar such as `"0"` or `"1"`.
    RawAscii(String),
    /// Native C4 bytes for an escaped-string field (`C4Config.cpp:379`).
    CppEscaped(Vec<u8>),
}

impl DeferredValue {
    /// The writer this field needs, so both flush sites stay in step.
    pub(crate) fn as_native(&self) -> clonk_app_netplay::NativeConfigValue<'_> {
        match self {
            Self::RawAscii(value) => clonk_app_netplay::NativeConfigValue::RawAscii(value),
            Self::CppEscaped(value) => {
                clonk_app_netplay::NativeConfigValue::CppEscapedString(value)
            }
        }
    }
}

/// Pending `(section, key) -> value` writes, in a deterministic order so a
/// flush produces a stable file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeferredConfig {
    pending: BTreeMap<(String, String), DeferredValue>,
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
        self.pending.insert(
            (section.into(), key.into()),
            DeferredValue::RawAscii(value.into()),
        );
    }

    /// Records a runtime change to an escaped-string field, whose native bytes
    /// the flush must hand to `NativeConfigValue::CppEscapedString` rather than
    /// writing through unquoted.
    pub(crate) fn set_escaped(
        &mut self,
        section: impl Into<String>,
        key: impl Into<String>,
        value: Vec<u8>,
    ) {
        self.pending.insert(
            (section.into(), key.into()),
            DeferredValue::CppEscaped(value),
        );
    }

    /// Drops a pending change because a surface that saves immediately — the
    /// ones C++ also writes straight to the file — has just superseded it.
    /// Without this the stale in-memory value would keep outranking the newer
    /// file value for the rest of the session.
    pub(crate) fn clear(&mut self, section: &str, key: &str) {
        self.pending.remove(&(section.to_owned(), key.to_owned()));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    /// The pending scalar for a key, which is what the running session should
    /// read back rather than the stale file. An escaped-string field holds
    /// native bytes with no presentation form here and reports `None`.
    pub(crate) fn get(&self, section: &str, key: &str) -> Option<&str> {
        match self.pending.get(&(section.to_owned(), key.to_owned()))? {
            DeferredValue::RawAscii(value) => Some(value.as_str()),
            DeferredValue::CppEscaped(_) => None,
        }
    }

    /// Takes the pending writes grouped by section, ready for one
    /// `persist_native_config_values` call each. Leaves the store empty, so a
    /// second flush is a no-op and an aborted run that never flushes discards
    /// everything — which is the point.
    pub(crate) fn take_by_section(&mut self) -> Vec<(String, Vec<(String, DeferredValue)>)> {
        let mut grouped: BTreeMap<String, Vec<(String, DeferredValue)>> = BTreeMap::new();
        for ((section, key), value) in std::mem::take(&mut self.pending) {
            grouped.entry(section).or_default().push((key, value));
        }
        grouped.into_iter().collect()
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
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
                    vec![("Record".to_owned(), DeferredValue::RawAscii("1".to_owned()))]
                ),
                (
                    "Network".to_owned(),
                    vec![(
                        "MasterServerSignUp".to_owned(),
                        DeferredValue::RawAscii("0".to_owned())
                    )]
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

    // A `CFG_MaxString` escaped-string field cannot be flushed as a raw
    // scalar: `NativeConfigValue::CppEscapedString` applies C++'s quoting,
    // NUL termination and length bound where `RawAscii` writes the bytes
    // through unchanged (C4Config.cpp:379).
    #[test]
    fn a_deferred_escaped_string_keeps_its_native_writer() {
        let mut config = DeferredConfig::default();
        config.set("Network", "MasterServerSignUp", "1");
        config.set_escaped("Network", "Comment", b"a \"quoted\" comment".to_vec());

        // Only a raw scalar reads back as text; an escaped field holds native
        // bytes that the session has no presentation form for here.
        assert_eq!(config.get("Network", "MasterServerSignUp"), Some("1"));
        assert_eq!(config.get("Network", "Comment"), None);

        // Either kind replaces the other for the same key, so a flush writes
        // each key exactly once.
        config.set_escaped("Network", "Comment", b"replaced".to_vec());
        assert_eq!(config.len(), 2);

        assert_eq!(
            config.take_by_section(),
            vec![(
                "Network".to_owned(),
                vec![
                    (
                        "Comment".to_owned(),
                        DeferredValue::CppEscaped(b"replaced".to_vec())
                    ),
                    (
                        "MasterServerSignUp".to_owned(),
                        DeferredValue::RawAscii("1".to_owned())
                    ),
                ]
            )]
        );
    }
}
