//! The closed `Installer` vocabulary (tech-spec §9.2).
//!
//! An installer is itself a kind of dependency: its `bootstrap` list names the
//! dep ids that provide it. Empty means terminal (detected, never installed).
//! The commands an installer runs are recipe data (`registry/deps.toml`).

use std::fmt;

use serde::{Serialize, Serializer};

/// A closed vocabulary of installers. Adding one is a deliberate code change
/// (same discipline as `Role`), done only when a real recipe needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Installer {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Winget,
    Nix,
    Go,
    Cargo,
    Npm,
    Aikup,
    CardanoUp,
    Tx3up,
    Curl,
    PowerShell,
}

impl Installer {
    /// Every installer variant.
    pub const ALL: &[Installer] = &[
        Installer::Brew,
        Installer::Apt,
        Installer::Dnf,
        Installer::Pacman,
        Installer::Winget,
        Installer::Nix,
        Installer::Go,
        Installer::Cargo,
        Installer::Npm,
        Installer::Aikup,
        Installer::CardanoUp,
        Installer::Tx3up,
        Installer::Curl,
        Installer::PowerShell,
    ];

    /// Parse an installer key as used in `registry/deps.toml`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|i| i.key() == key)
    }

    /// The stable key: the `deps.toml` key and the JSON `installer` value.
    pub fn key(self) -> &'static str {
        match self {
            Installer::Brew => "brew",
            Installer::Apt => "apt",
            Installer::Dnf => "dnf",
            Installer::Pacman => "pacman",
            Installer::Winget => "winget",
            Installer::Nix => "nix",
            Installer::Go => "go",
            Installer::Cargo => "cargo",
            Installer::Npm => "npm",
            Installer::Aikup => "aikup",
            Installer::CardanoUp => "cardano-up",
            Installer::Tx3up => "tx3up",
            Installer::Curl => "curl",
            Installer::PowerShell => "powershell",
        }
    }

    /// Binaries whose presence on `PATH` means this installer is available.
    pub fn detect(self) -> &'static [&'static str] {
        match self {
            Installer::Brew => &["brew"],
            Installer::Apt => &["apt"],
            Installer::Dnf => &["dnf"],
            Installer::Pacman => &["pacman"],
            Installer::Winget => &["winget"],
            Installer::Nix => &["nix"],
            Installer::Go => &["go"],
            Installer::Cargo => &["cargo"],
            Installer::Npm => &["npm"],
            Installer::Aikup => &["aikup"],
            Installer::CardanoUp => &["cardano-up"],
            Installer::Tx3up => &["tx3up"],
            Installer::Curl => &["curl"],
            Installer::PowerShell => &["powershell", "pwsh"],
        }
    }

    /// Dep ids that provide this installer. Empty ⇒ terminal (detect-only).
    pub fn bootstrap(self) -> &'static [&'static str] {
        match self {
            Installer::Npm => &["node"],
            Installer::Cargo => &["rustup", "rust"],
            Installer::Go => &["go"],
            Installer::Aikup => &["aikup"],
            Installer::CardanoUp => &["cardano-up"],
            Installer::Tx3up => &["tx3up"],
            Installer::Brew
            | Installer::Apt
            | Installer::Dnf
            | Installer::Pacman
            | Installer::Winget
            | Installer::Nix
            | Installer::Curl
            | Installer::PowerShell => &[],
        }
    }
}

impl fmt::Display for Installer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Serializes as the stable key, e.g. `"npm"`.
impl Serialize for Installer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trip() {
        for inst in Installer::ALL {
            assert_eq!(Installer::from_key(inst.key()), Some(*inst));
        }
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(Installer::from_key("snap"), None);
        assert_eq!(Installer::from_key(""), None);
    }

    #[test]
    fn terminals_have_no_bootstrap() {
        assert!(Installer::Brew.bootstrap().is_empty());
        assert!(Installer::Nix.bootstrap().is_empty());
        assert!(Installer::Curl.bootstrap().is_empty());
    }

    #[test]
    fn bootstrappable_have_deps() {
        assert_eq!(Installer::Npm.bootstrap(), &["node"]);
        assert_eq!(Installer::Aikup.bootstrap(), &["aikup"]);
    }
}
