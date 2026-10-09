//! Detect how this `cardano-init` binary was installed, so the update notice
//! can print the matching update command. Detection is pure over the exe path
//! and a [`Host`] (folder lookups + file reads), so every channel is
//! unit-testable; only [`detect_current`] touches the real system.
//!
//! The shell/PowerShell installers (cargo-dist) and `cargo install` both put
//! the binary in `$CARGO_HOME/bin` (`install-path = "CARGO_HOME"`), so they're
//! told apart by cargo-dist's install receipt, checked first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::IgnoredAny;

const INSTALLER_BASE: &str = "https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// A one-off `npx cardano-init` run.
    Npx,
    /// `npm install -g cardano-init`.
    Npm,
    /// `nix profile add github:input-output-hk/cardano-init` (or `nix run`).
    Nix,
    /// The cargo-dist shell installer (`curl … | sh`).
    ShellInstaller,
    /// The cargo-dist PowerShell installer (`irm … | iex`).
    PowershellInstaller,
    /// `cargo install --git …`.
    Cargo,
    /// None of the above (manual download, custom build, …).
    Unknown,
}

impl InstallMethod {
    /// The command that updates an install of this kind; `None` → show the
    /// release link instead.
    pub fn update_command(self) -> Option<String> {
        Some(match self {
            Self::Npx => "npx cardano-init@latest".into(),
            Self::Npm => "npm install -g cardano-init@latest".into(),
            Self::Nix => "nix profile upgrade cardano-init".into(),
            Self::ShellInstaller => {
                format!("curl --proto '=https' --tlsv1.2 -LsSf {INSTALLER_BASE}.sh | sh")
            }
            Self::PowershellInstaller => format!("irm {INSTALLER_BASE}.ps1 | iex"),
            Self::Cargo => {
                "cargo install --git https://github.com/input-output-hk/cardano-init --force".into()
            }
            Self::Unknown => return None,
        })
    }
}

/// The system facts detection needs, abstracted so tests can fake them.
trait Host {
    fn is_windows(&self) -> bool;
    /// Where cargo-dist keeps its install receipt: `%LOCALAPPDATA%` on Windows,
    /// `$XDG_CONFIG_HOME|~/.config` elsewhere (macOS included).
    fn receipt_base(&self) -> Option<PathBuf>;
    /// `$CARGO_HOME`, or `~/.cargo`.
    fn cargo_home(&self) -> Option<PathBuf>;
    fn read(&self, path: &Path) -> Option<String>;
}

struct System;

impl Host for System {
    fn is_windows(&self) -> bool {
        cfg!(windows)
    }

    fn receipt_base(&self) -> Option<PathBuf> {
        if cfg!(windows) {
            dirs::data_local_dir()
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        }
    }

    fn cargo_home(&self) -> Option<PathBuf> {
        std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// Detect the install method of the running binary. Never fails: anything
/// indeterminate is [`InstallMethod::Unknown`].
pub fn detect_current() -> InstallMethod {
    std::env::current_exe()
        .map(|p| p.canonicalize().unwrap_or(p))
        .map_or(InstallMethod::Unknown, |exe| detect(&exe, &System))
}

/// Try each detector in priority order; the first hit wins.
fn detect(exe: &Path, host: &impl Host) -> InstallMethod {
    from_path(exe)
        .or_else(|| from_receipt(exe, host))
        .or_else(|| from_cargo(exe, host))
        .unwrap_or(InstallMethod::Unknown)
}

/// npx / npm / nix, recognizable from the exe path alone. An npx cache path
/// also contains `node_modules`, but `_npx` comes first, so it wins.
fn from_path(exe: &Path) -> Option<InstallMethod> {
    if exe.starts_with("/nix/store") {
        return Some(InstallMethod::Nix);
    }
    exe.components()
        .find_map(|c| match c.as_os_str().to_str()? {
            "_npx" => Some(InstallMethod::Npx),
            "node_modules" => Some(InstallMethod::Npm),
            _ => None,
        })
}

/// cargo-dist's `cardano-init-receipt.json` (only the field we need).
#[derive(Deserialize)]
struct Receipt {
    install_prefix: PathBuf,
}

/// cargo's `.crates2.json`: installed packages keyed by `"<name> <version> (<source>)"`.
#[derive(Deserialize)]
struct CratesRegistry {
    installs: HashMap<String, IgnoredAny>,
}

/// A cargo-dist install receipt exists and its `install_prefix` contains `exe`.
fn from_receipt(exe: &Path, host: &impl Host) -> Option<InstallMethod> {
    let path = host
        .receipt_base()?
        .join("cardano-init")
        .join("cardano-init-receipt.json");
    let receipt: Receipt = serde_json::from_str(&host.read(&path)?).ok()?;
    exe.starts_with(receipt.install_prefix)
        .then_some(match host.is_windows() {
            true => InstallMethod::PowershellInstaller,
            false => InstallMethod::ShellInstaller,
        })
}

/// `exe` lives in `$CARGO_HOME/bin` and `cargo install` recorded `cardano-init`
/// in `$CARGO_HOME/.crates2.json`.
fn from_cargo(exe: &Path, host: &impl Host) -> Option<InstallMethod> {
    let cargo_home = host
        .cargo_home()
        .filter(|home| exe.starts_with(home.join("bin")))?;
    let registry: CratesRegistry =
        serde_json::from_str(&host.read(&cargo_home.join(".crates2.json"))?).ok()?;
    registry
        .installs
        .keys()
        .any(|k| k.starts_with("cardano-init "))
        .then_some(InstallMethod::Cargo)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const CARGO_EXE: &str = "/home/u/.cargo/bin/cardano-init";
    const RECEIPT: &str = "/home/u/.config/cardano-init/cardano-init-receipt.json";
    const CRATES2: &str = "/home/u/.cargo/.crates2.json";
    const RECEIPT_JSON: &str =
        r#"{"install_prefix":"/home/u/.cargo/bin","binaries":["cardano-init"]}"#;
    const CRATES2_JSON: &str = r#"{"installs":{"cardano-init 0.2.1 (git+https://github.com/input-output-hk/cardano-init#abc)":{}}}"#;

    struct Fake {
        windows: bool,
        files: HashMap<PathBuf, String>,
    }

    impl Fake {
        fn unix(files: &[(&str, &str)]) -> Self {
            Self {
                windows: false,
                files: files
                    .iter()
                    .map(|(p, c)| (PathBuf::from(p), c.to_string()))
                    .collect(),
            }
        }
    }

    impl Host for Fake {
        fn is_windows(&self) -> bool {
            self.windows
        }
        fn receipt_base(&self) -> Option<PathBuf> {
            Some(PathBuf::from(if self.windows {
                "/appdata"
            } else {
                "/home/u/.config"
            }))
        }
        fn cargo_home(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/u/.cargo"))
        }
        fn read(&self, path: &Path) -> Option<String> {
            self.files.get(path).cloned()
        }
    }

    fn run(exe: &str, files: &[(&str, &str)]) -> InstallMethod {
        detect(Path::new(exe), &Fake::unix(files))
    }

    #[test]
    fn npx_cache_dir() {
        let exe = "/home/u/.npm/_npx/1a2b/node_modules/cardano-init/node_modules/.bin_cardano-init/cardano-init";
        assert_eq!(run(exe, &[]), InstallMethod::Npx);
    }

    #[test]
    fn global_npm_install() {
        let exe =
            "/usr/local/lib/node_modules/cardano-init/node_modules/.bin_cardano-init/cardano-init";
        assert_eq!(run(exe, &[]), InstallMethod::Npm);
    }

    #[test]
    fn nix_store() {
        let exe = "/nix/store/abc-cardano-init-0.2.1/bin/cardano-init";
        assert_eq!(run(exe, &[]), InstallMethod::Nix);
    }

    #[test]
    fn shell_installer_receipt_wins_over_cargo() {
        assert_eq!(
            run(
                CARGO_EXE,
                &[(RECEIPT, RECEIPT_JSON), (CRATES2, CRATES2_JSON)]
            ),
            InstallMethod::ShellInstaller
        );
    }

    #[test]
    fn receipt_for_another_prefix_is_ignored() {
        let other = (RECEIPT, r#"{"install_prefix":"/opt/elsewhere/bin"}"#);
        assert_eq!(
            run(CARGO_EXE, &[other, (CRATES2, CRATES2_JSON)]),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn cargo_install() {
        assert_eq!(
            run(CARGO_EXE, &[(CRATES2, CRATES2_JSON)]),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn cargo_bin_without_record_is_unknown() {
        assert_eq!(run(CARGO_EXE, &[]), InstallMethod::Unknown);
    }

    #[test]
    fn malformed_files_are_ignored() {
        assert_eq!(
            run(CARGO_EXE, &[(RECEIPT, "{"), (CRATES2, "not json")]),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn manual_download_is_unknown() {
        assert_eq!(
            run("/usr/local/bin/cardano-init", &[(CRATES2, CRATES2_JSON)]),
            InstallMethod::Unknown
        );
        assert_eq!(InstallMethod::Unknown.update_command(), None);
    }

    #[test]
    fn windows_receipt_is_powershell() {
        let host = Fake {
            windows: true,
            files: [(
                PathBuf::from("/appdata/cardano-init/cardano-init-receipt.json"),
                r#"{"install_prefix":"/cargo/bin"}"#.to_string(),
            )]
            .into(),
        };
        assert_eq!(
            detect(Path::new("/cargo/bin/cardano-init"), &host),
            InstallMethod::PowershellInstaller
        );
    }

    #[test]
    fn installer_commands_match_readme() {
        assert_eq!(
            InstallMethod::ShellInstaller.update_command().unwrap(),
            "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.sh | sh"
        );
        assert_eq!(
            InstallMethod::PowershellInstaller.update_command().unwrap(),
            "irm https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.ps1 | iex"
        );
    }
}
