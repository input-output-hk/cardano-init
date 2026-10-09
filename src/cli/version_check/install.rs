//! Detect how this `cardano-init` binary was installed, so the update notice
//! can print the matching update command. Pure over its inputs (the exe path,
//! an env lookup, a file reader) so every channel is unit-testable; only
//! [`detect_current`] touches the real process/filesystem.
//!
//! The shell/PowerShell installers (cargo-dist) and `cargo install` both put
//! the binary in `$CARGO_HOME/bin` (`install-path = "CARGO_HOME"`), so they're
//! told apart by cargo-dist's install receipt, checked first.

use std::path::{Component, Path, PathBuf};

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
    pub fn update_command(self) -> Option<&'static str> {
        Some(match self {
            Self::Npx => "npx cardano-init@latest",
            Self::Npm => "npm install -g cardano-init@latest",
            Self::Nix => "nix profile upgrade cardano-init",
            Self::ShellInstaller => {
                "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.sh | sh"
            }
            Self::PowershellInstaller => {
                "irm https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.ps1 | iex"
            }
            Self::Cargo => {
                "cargo install --git https://github.com/input-output-hk/cardano-init --force"
            }
            Self::Unknown => return None,
        })
    }
}

/// Detect the install method of the running binary. Never fails: anything
/// indeterminate is [`InstallMethod::Unknown`].
pub fn detect_current() -> InstallMethod {
    let Some(exe) = std::env::current_exe()
        .ok()
        .map(|p| p.canonicalize().unwrap_or(p))
    else {
        return InstallMethod::Unknown;
    };
    detect(
        &exe,
        cfg!(windows),
        |k| std::env::var_os(k).map(PathBuf::from),
        |p| std::fs::read_to_string(p).ok(),
    )
}

fn detect(
    exe: &Path,
    windows: bool,
    var: impl Fn(&str) -> Option<PathBuf>,
    read: impl Fn(&Path) -> Option<String>,
) -> InstallMethod {
    let has_component = |name: &str| {
        exe.components()
            .any(|c| matches!(c, Component::Normal(s) if s == name))
    };
    if has_component("_npx") {
        return InstallMethod::Npx;
    }
    if has_component("node_modules") {
        return InstallMethod::Npm;
    }
    if exe.starts_with("/nix/store") {
        return InstallMethod::Nix;
    }
    if installed_by_receipt(exe, windows, &var, &read) {
        return if windows {
            InstallMethod::PowershellInstaller
        } else {
            InstallMethod::ShellInstaller
        };
    }
    if installed_by_cargo(exe, &var, &read) {
        return InstallMethod::Cargo;
    }
    InstallMethod::Unknown
}

/// A cargo-dist install receipt exists and its `install_prefix` contains `exe`.
/// The receipt lives in `%LOCALAPPDATA%\cardano-init` on Windows and in
/// `$XDG_CONFIG_HOME|~/.config` + `cardano-init` elsewhere.
fn installed_by_receipt(
    exe: &Path,
    windows: bool,
    var: &impl Fn(&str) -> Option<PathBuf>,
    read: &impl Fn(&Path) -> Option<String>,
) -> bool {
    let config = if windows {
        var("LOCALAPPDATA")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    };
    let Some(receipt) = config
        .map(|c| c.join("cardano-init").join("cardano-init-receipt.json"))
        .and_then(|p| read(&p))
    else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&receipt)
        .ok()
        .and_then(|v| v.get("install_prefix")?.as_str().map(PathBuf::from))
        .is_some_and(|prefix| exe.starts_with(prefix))
}

/// `cargo install` recorded `cardano-init` in `$CARGO_HOME/.crates2.json` and
/// `exe` lives in `$CARGO_HOME/bin`.
fn installed_by_cargo(
    exe: &Path,
    var: &impl Fn(&str) -> Option<PathBuf>,
    read: &impl Fn(&Path) -> Option<String>,
) -> bool {
    let Some(cargo_home) = var("CARGO_HOME").or_else(|| var("HOME").map(|h| h.join(".cargo")))
    else {
        return false;
    };
    if !exe.starts_with(cargo_home.join("bin")) {
        return false;
    }
    read(&cargo_home.join(".crates2.json"))
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| {
            v.get("installs")?
                .as_object()
                .map(|m| m.keys().any(|k| k.starts_with("cardano-init ")))
        })
        .unwrap_or(false)
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const HOME: &str = "/home/u";
    const CARGO_EXE: &str = "/home/u/.cargo/bin/cardano-init";
    const RECEIPT: &str = "/home/u/.config/cardano-init/cardano-init-receipt.json";
    const CRATES2: &str = "/home/u/.cargo/.crates2.json";

    fn run(exe: &str, files: &[(&str, &str)]) -> InstallMethod {
        let files: HashMap<PathBuf, String> = files
            .iter()
            .map(|(p, c)| (PathBuf::from(p), c.to_string()))
            .collect();
        detect(
            Path::new(exe),
            false,
            |k| (k == "HOME").then(|| PathBuf::from(HOME)),
            |p| files.get(p).cloned(),
        )
    }

    fn receipt() -> (&'static str, &'static str) {
        (
            RECEIPT,
            r#"{"install_prefix":"/home/u/.cargo/bin","binaries":["cardano-init"]}"#,
        )
    }

    fn crates2() -> (&'static str, &'static str) {
        (
            CRATES2,
            r#"{"installs":{"cardano-init 0.2.1 (git+https://github.com/input-output-hk/cardano-init#abc)":{}}}"#,
        )
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
            run(CARGO_EXE, &[receipt(), crates2()]),
            InstallMethod::ShellInstaller
        );
    }

    #[test]
    fn receipt_for_another_prefix_is_ignored() {
        let other = (RECEIPT, r#"{"install_prefix":"/opt/elsewhere/bin"}"#);
        assert_eq!(run(CARGO_EXE, &[other, crates2()]), InstallMethod::Cargo);
    }

    #[test]
    fn cargo_install() {
        assert_eq!(run(CARGO_EXE, &[crates2()]), InstallMethod::Cargo);
    }

    #[test]
    fn cargo_bin_without_record_is_unknown() {
        assert_eq!(run(CARGO_EXE, &[]), InstallMethod::Unknown);
    }

    #[test]
    fn manual_download_is_unknown() {
        assert_eq!(
            run("/usr/local/bin/cardano-init", &[crates2()]),
            InstallMethod::Unknown
        );
        assert_eq!(InstallMethod::Unknown.update_command(), None);
    }

    #[test]
    fn windows_receipt_is_powershell() {
        let files: HashMap<PathBuf, String> = [(
            PathBuf::from("/appdata/cardano-init/cardano-init-receipt.json"),
            r#"{"install_prefix":"/cargo/bin"}"#.to_string(),
        )]
        .into();
        let method = detect(
            Path::new("/cargo/bin/cardano-init"),
            true,
            |k| (k == "LOCALAPPDATA").then(|| PathBuf::from("/appdata")),
            |p| files.get(p).cloned(),
        );
        assert_eq!(method, InstallMethod::PowershellInstaller);
    }
}
