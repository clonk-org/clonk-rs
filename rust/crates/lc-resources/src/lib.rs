#[cfg(feature = "ffi")]
pub mod ffi;
pub mod group;
pub mod scenario;

pub use group::{Group, GroupEntry, GroupError};
pub use scenario::{
    discover, discover_many, ScenarioDiscoveryError, ScenarioEntry, ScenarioEntryKind,
};
