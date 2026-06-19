//! The closed `Installer` vocabulary (TECH_SPEC §9.2, ARCHITECTURE §8.1).
//!
//! Installers are **code**: detection binaries, the command template, and the
//! `bootstrap` edges are logic over a closed set, so they earn compile-time
//! safety. An installer is itself a kind of dependency — its `bootstrap` list
//! names the dep ids that *provide* it. An **empty** `bootstrap` ⇒ terminal
//! (we detect it, never auto-install it); a **non-empty** list ⇒ bootstrappable
//! by installing any one of those deps in order.

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
        Installer::Curl,
        Installer::PowerShell,
    ];

    /// Parse the installer key used in `registry/deps.toml`. Unknown → `None`
    /// (the caller turns this into a load error).
    pub fn from_key(s: &str) -> Option<Self> {
        Some(match s {
            "brew" => Installer::Brew,
            "apt" => Installer::Apt,
            "dnf" => Installer::Dnf,
            "pacman" => Installer::Pacman,
            "winget" => Installer::Winget,
            "nix" => Installer::Nix,
            "go" => Installer::Go,
            "cargo" => Installer::Cargo,
            "npm" => Installer::Npm,
            "aikup" => Installer::Aikup,
            "cardano-up" => Installer::CardanoUp,
            "curl" => Installer::Curl,
            "powershell" => Installer::PowerShell,
            _ => return None,
        })
    }

    /// The stable key for this installer (matches the `deps.toml` key and the
    /// JSON `installer` field).
    pub fn key(&self) -> &'static str {
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
            Installer::Curl => "curl",
            Installer::PowerShell => "powershell",
        }
    }

    /// Binaries whose presence on `PATH` means this installer is available.
    pub fn detect(&self) -> &'static [&'static str] {
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
            Installer::Curl => &["curl"],
            Installer::PowerShell => &["powershell", "pwsh"],
        }
    }

    /// Dep ids that provide this installer. Empty ⇒ terminal (detect-only).
    pub fn bootstrap(&self) -> &'static [&'static str] {
        match self {
            Installer::Npm => &["node"],
            Installer::Cargo => &["rustup", "rust"],
            Installer::Go => &["go"],
            Installer::Aikup => &["aikup"],
            Installer::CardanoUp => &["cardano-up"],
            // System package managers, nix, and the download-and-run shells are
            // terminal: we detect them, never install them.
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

    /// Render the install command for `arg` (a package name, installer-script
    /// URL, or target, per the installer).
    pub fn command(&self, arg: &str) -> String {
        match self {
            Installer::Brew => format!("brew install {arg}"),
            Installer::Apt => format!("sudo apt install -y {arg}"),
            Installer::Dnf => format!("sudo dnf install -y {arg}"),
            Installer::Pacman => format!("sudo pacman -S --noconfirm {arg}"),
            Installer::Winget => format!("winget install {arg}"),
            Installer::Nix => format!("nix profile install nixpkgs#{arg}"),
            Installer::Go => format!("go install {arg}"),
            Installer::Cargo => format!("cargo install {arg}"),
            Installer::Npm => format!("npm install -g {arg}"),
            Installer::Aikup => format!("aikup install {arg}"),
            Installer::CardanoUp => format!("cardano-up install {arg}"),
            Installer::Curl => format!("curl -sSfL {arg} | sh"),
            Installer::PowerShell => format!("powershell -c \"irm {arg} | iex\""),
        }
    }
}

impl fmt::Display for Installer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Serializes as the stable key string (e.g. `"npm"`), matching the JSON
/// `installer` field in the doctor Report (§9.4).
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

    #[test]
    fn command_templates() {
        assert_eq!(Installer::Brew.command("just"), "brew install just");
        assert_eq!(
            Installer::Npm.command("@aiken-lang/aikup"),
            "npm install -g @aiken-lang/aikup"
        );
        assert_eq!(
            Installer::Nix.command("aiken"),
            "nix profile install nixpkgs#aiken"
        );
        assert_eq!(
            Installer::Curl.command("https://sh.rustup.rs"),
            "curl -sSfL https://sh.rustup.rs | sh"
        );
    }
}
