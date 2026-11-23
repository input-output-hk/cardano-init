//! Installers, declared as data in `registry/installers.toml` (tech-spec §9.2).
//!
//! An installer is itself a kind of dependency: its `bootstrap` list names the
//! dep ids that provide it. Empty means terminal (detected, never installed).
//! The commands an installer runs are recipe data (`registry/deps.toml`).

use std::fmt;

use serde::{Deserialize, Serialize, Serializer};

/// An installer id, e.g. `npm`: the `deps.toml` method key and the JSON
/// `installer` value. Only ids declared in `installers.toml` survive load.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Installer(String);

impl Installer {
    pub fn new(key: impl Into<String>) -> Self {
        Installer(key.into())
    }

    pub fn key(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Installer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<&str> for Installer {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Serializes as the id, e.g. `"npm"`.
impl Serialize for Installer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// How to detect an installer, and which deps provide it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallerDef {
    /// Binaries whose presence on `PATH` means this installer is available.
    pub detect: Vec<String>,
    /// Dep ids that provide this installer. Empty ⇒ terminal (detect-only).
    #[serde(default)]
    pub bootstrap: Vec<String>,
}
