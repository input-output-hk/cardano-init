//! The closed `Installer` vocabulary (tech-spec §9.2).
//!
//! An installer is itself a kind of dependency: its `bootstrap` list names the
//! dep ids that provide it. Empty means terminal (detected, never installed).

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

    /// Render the install command for `arg`: a package name, installer-script
    /// URL, or target, depending on the installer.
    pub fn command(self, arg: &str) -> String {
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
            // An empty arg installs the latest release / active channel; avoid a
            // trailing space.
            Installer::Aikup | Installer::Tx3up if arg.is_empty() => {
                format!("{} install", self.key())
            }
            Installer::Aikup => format!("aikup install {arg}"),
            Installer::CardanoUp => format!("cardano-up install {arg}"),
            Installer::Tx3up => format!("tx3up install {arg}"),
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
        // Empty aikup arg renders bare `aikup install` (latest), no trailing space.
        assert_eq!(Installer::Aikup.command(""), "aikup install");
        assert_eq!(Installer::Aikup.command("v1.1.0"), "aikup install v1.1.0");
    }
}
