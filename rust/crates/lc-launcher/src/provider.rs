use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderPathStatus {
    Ready,
    Missing,
    NotDirectory,
    Inaccessible(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAutomationState {
    Idle,
    Submitted { detail: String },
    Skipped { reason: String },
    Failed { error: String },
}

impl Default for ProviderAutomationState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderStatus {
    pub name: String,
    pub path: PathBuf,
    pub path_status: ProviderPathStatus,
    pub automation: ProviderAutomationState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderDiagnostics {
    pub share: Vec<ProviderStatus>,
    pub upload: Vec<ProviderStatus>,
}
