//! Shared leaf types and pure helpers peeled out of the clonk-app
//! monolith so the extracted area crates (menus, netplay, render) and the
//! app depend on this crate instead of on `main.rs`.

pub mod menu_images;
pub mod native_config;
pub mod pictures;

use std::fmt;

use clonk_frontend::game_lobby::{LobbyRosterId, LobbySheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    Menu,
    Loading,
    Running,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassicGameLobbyChild {
    Start,
    AbortCountdown,
    Ready,
    Sheet(LobbySheet),
    RosterContext(LobbyRosterId),
    Chat,
    GameOptionSideEffect(&'static str),
    NetworkEvent(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassicGameLobbyBoundary {
    Resources { detail: String },
    Model { detail: String },
    Child(ClassicGameLobbyChild),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassicGuiBootstrapDefect {
    Missing,
    Malformed {
        expected: &'static str,
        actual: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicGuiBootstrapIssue {
    pub resource: &'static str,
    pub defect: ClassicGuiBootstrapDefect,
}

impl ClassicGuiBootstrapIssue {
    pub const fn missing(resource: &'static str) -> Self {
        Self {
            resource,
            defect: ClassicGuiBootstrapDefect::Missing,
        }
    }

    pub fn malformed(
        resource: &'static str,
        expected: &'static str,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            resource,
            defect: ClassicGuiBootstrapDefect::Malformed {
                expected,
                actual: actual.into(),
            },
        }
    }
}

impl fmt::Display for ClassicGuiBootstrapIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.defect {
            ClassicGuiBootstrapDefect::Missing => write!(f, "{}: missing", self.resource),
            ClassicGuiBootstrapDefect::Malformed { expected, actual } => write!(
                f,
                "{}: malformed (expected {expected}, got {actual})",
                self.resource
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassicStartupBootstrapDefect {
    Missing,
    Malformed {
        expected: &'static str,
        actual: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicStartupBootstrapIssue {
    pub resource: &'static str,
    pub defect: ClassicStartupBootstrapDefect,
}

impl ClassicStartupBootstrapIssue {
    pub const fn missing(resource: &'static str) -> Self {
        Self {
            resource,
            defect: ClassicStartupBootstrapDefect::Missing,
        }
    }

    pub fn malformed(
        resource: &'static str,
        expected: &'static str,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            resource,
            defect: ClassicStartupBootstrapDefect::Malformed {
                expected,
                actual: actual.into(),
            },
        }
    }
}

impl fmt::Display for ClassicStartupBootstrapIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.defect {
            ClassicStartupBootstrapDefect::Missing => {
                write!(f, "{}: missing", self.resource)
            }
            ClassicStartupBootstrapDefect::Malformed { expected, actual } => {
                write!(
                    f,
                    "{}: malformed (expected {expected}, got {actual})",
                    self.resource
                )
            }
        }
    }
}
