use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderPathStatus {
    Ready,
    Missing,
    NotDirectory,
    Inaccessible(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAutomationState {
    Idle,
    Submitted { detail: String },
    Stale { reason: String },
    Skipped { reason: String },
    Failed { error: String },
}

impl Default for ProviderAutomationState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderPathProvenance {
    default_path: PathBuf,
    overrides: Vec<ProviderPathOverride>,
}

impl ProviderPathProvenance {
    pub fn new(default_path: PathBuf) -> Self {
        Self {
            default_path,
            overrides: Vec::new(),
        }
    }

    pub fn default_path(&self) -> &Path {
        &self.default_path
    }

    pub fn overrides(&self) -> &[ProviderPathOverride] {
        &self.overrides
    }

    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    pub fn has_preference_override(&self) -> bool {
        self.overrides.iter().any(|override_entry| {
            matches!(override_entry.source, ProviderOverrideSource::Preference)
        })
    }

    pub fn apply_override(&mut self, path: PathBuf, source: ProviderOverrideSource) {
        if let Some(latest) = self.overrides.last() {
            if latest.path == path && latest.source == source {
                return;
            }
        }
        self.overrides.push(ProviderPathOverride { path, source });
    }

    pub fn remove_preference_overrides(&mut self) {
        self.overrides.retain(|override_entry| {
            !matches!(override_entry.source, ProviderOverrideSource::Preference)
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderPathOverride {
    path: PathBuf,
    source: ProviderOverrideSource,
}

impl ProviderPathOverride {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> &ProviderOverrideSource {
        &self.source
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderOverrideSource {
    Preference,
    Retargeted { applied_at: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderStatus {
    pub name: String,
    pub path: PathBuf,
    pub path_status: ProviderPathStatus,
    pub automation: ProviderAutomationState,
    pub path_provenance: ProviderPathProvenance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderDiagnostics {
    pub share: Vec<ProviderStatus>,
    pub upload: Vec<ProviderStatus>,
}
