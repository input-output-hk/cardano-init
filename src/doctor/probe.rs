//! Impure system probes for the doctor (TECH_SPEC §9.3).
//!
//! This is the only doctor module that touches the system: it detects the OS,
//! which installers are on `PATH`, which dependency binaries are present, and
//! scans a generated project to identify the tool filling each role directory.
//! No execution, no version detection (v1).

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use super::catalog::DepCatalog;
use super::installers::Installer;
use crate::registry::loader::Registry;
use crate::registry::types::{DetectSignature, Role};

/// Detected operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::enum_variant_names)] // `MacOs` ends in "Os"; renaming would change the serialized value
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Os {
    /// The OS of the running host.
    pub fn detect() -> Self {
        match std::env::consts::OS {
            "linux" => Os::Linux,
            "macos" => Os::MacOs,
            "windows" => Os::Windows,
            _ => Os::Other,
        }
    }
}

/// The probed environment the pure resolver runs against. Holds everything
/// system-derived so the resolver itself stays pure and unit-testable with a
/// synthetic value.
#[derive(Debug, Clone)]
pub struct Environment {
    pub os: Os,
    /// Installers detected as available on this host.
    pub installers: HashSet<Installer>,
    /// Dependency/installer binaries found on `PATH`.
    pub present_binaries: HashSet<String>,
}

/// Detect the environment: OS, available installers, and which catalog/installer
/// binaries are on `PATH`.
pub fn detect_environment(catalog: &DepCatalog) -> Environment {
    // All binaries worth probing: every dep's presence binaries plus every
    // installer's detection binaries.
    let mut binaries: HashSet<String> = HashSet::new();
    for id in catalog.dep_ids() {
        if let Some(recipe) = catalog.get(id) {
            binaries.extend(recipe.binaries.iter().cloned());
        }
    }
    for installer in Installer::ALL {
        binaries.extend(installer.detect().iter().map(|s| s.to_string()));
    }

    let present_binaries: HashSet<String> =
        binaries.into_iter().filter(|b| is_on_path(b)).collect();

    let installers: HashSet<Installer> = Installer::ALL
        .iter()
        .copied()
        .filter(|inst| inst.detect().iter().any(|b| present_binaries.contains(*b)))
        .collect();

    Environment {
        os: Os::detect(),
        installers,
        present_binaries,
    }
}

/// True if `bin` is found in any `PATH` entry. On Windows, also tries common
/// executable extensions.
fn is_on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return true;
        }
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                if dir.join(format!("{bin}.{ext}")).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Project scan
// ---------------------------------------------------------------------------

/// A role directory whose contents were recognized as a specific tool.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedComponent {
    #[serde(serialize_with = "ser_role")]
    pub role: Role,
    pub tool_id: String,
}

/// A role directory that exists but whose contents matched no known tool
/// (renamed, modified, or a tool not in the registry).
#[derive(Debug, Clone, Serialize)]
pub struct UnrecognizedDir {
    #[serde(serialize_with = "ser_role")]
    pub role: Role,
    pub dir: String,
}

fn ser_role<S: serde::Serializer>(role: &Role, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(role.as_kebab())
}

/// The result of scanning a project tree.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub components: Vec<DetectedComponent>,
    pub unrecognized: Vec<UnrecognizedDir>,
}

/// True if `dir` holds the aggregated cardano-up infra driver: a `Justfile`
/// that references `cardano-up`. The infra component has no per-tool subdirs to
/// match against `detect` signatures, so it is recognized by this driver marker
/// (infra-via-cardano-up proposal §9.3).
fn infra_driver_present(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Justfile"))
        .map(|text| text.contains("cardano-up"))
        .unwrap_or(false)
}

/// True if a detect signature matches under `dir`: the file exists and, when a
/// `contains` substring is given, the file's text includes it.
fn signature_matches(dir: &Path, sig: &DetectSignature) -> bool {
    let path = dir.join(&sig.file);
    match &sig.contains {
        None => path.exists(),
        Some(needle) => std::fs::read_to_string(&path)
            .map(|text| text.contains(needle.as_str()))
            .unwrap_or(false),
    }
}

/// Scan a project root for role directories and identify the tool in each.
///
/// For each contract role directory present, the candidate tools are exactly
/// those that declare that role; a tool matches if any of its `detect`
/// signature files exists under the directory. Exactly one match ⇒ detected;
/// zero (or an ambiguous multiple) ⇒ unrecognized.
pub fn scan_project(root: &Path, registry: &Registry) -> ScanResult {
    let mut components = Vec::new();
    let mut unrecognized = Vec::new();

    for &role in Role::ALL {
        let dir = root.join(role.dir());
        if !dir.is_dir() {
            continue;
        }

        // Infrastructure aggregates into a single cardano-up-driven component
        // (no per-tool subdirs), so it is recognized by the driver marker rather
        // than per-tool `detect` signatures.
        if role == Role::Infrastructure {
            if infra_driver_present(&dir) {
                components.push(DetectedComponent {
                    role,
                    tool_id: super::INFRA_DRIVER_ID.to_string(),
                });
            } else {
                unrecognized.push(UnrecognizedDir {
                    role,
                    dir: role.dir().to_string(),
                });
            }
            continue;
        }

        let matched: Vec<&str> = registry
            .tools_for_role(role)
            .into_iter()
            .filter(|tool| tool.detect.iter().any(|sig| signature_matches(&dir, sig)))
            .map(|tool| tool.id.as_str())
            .collect();

        if matched.len() == 1 {
            components.push(DetectedComponent {
                role,
                tool_id: matched[0].to_string(),
            });
        } else {
            unrecognized.push(UnrecognizedDir {
                role,
                dir: role.dir().to_string(),
            });
        }
    }

    ScanResult {
        components,
        unrecognized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn registry() -> Registry {
        Registry::load().expect("registry loads")
    }

    #[test]
    fn os_detect_is_known_on_ci() {
        // Just exercise the path; the value depends on the host.
        let _ = Os::detect();
    }

    #[test]
    fn scan_identifies_aiken_and_meshjs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // on-chain/ with aiken signature
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/aiken.toml"), "").unwrap();

        // off-chain/ with meshjs signature (package.json referencing @meshsdk)
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "@meshsdk/core": "^1.9.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());

        assert_eq!(result.components.len(), 2);
        let onchain = result
            .components
            .iter()
            .find(|c| c.role == Role::OnChain)
            .unwrap();
        assert_eq!(onchain.tool_id, "aiken");
        let offchain = result
            .components
            .iter()
            .find(|c| c.role == Role::OffChain)
            .unwrap();
        assert_eq!(offchain.tool_id, "meshjs");
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn foreign_package_json_is_unrecognized_not_meshjs() {
        // A from-scratch JS project (e.g. Next.js) has a package.json but no
        // @meshsdk dependency — content-aware detection must NOT call it MeshJS.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "next": "^14.0.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(result.unrecognized[0].role, Role::OffChain);
    }

    #[test]
    fn scan_flags_unrecognized_role_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // on-chain/ exists but contains no recognizable signature (renamed files).
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/renamed.txt"), "").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(result.unrecognized[0].role, Role::OnChain);
    }

    #[test]
    fn scan_distinguishes_scalus_on_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("on-chain/src")).unwrap();
        fs::write(root.join("on-chain/src/Validator.scala"), "").unwrap();

        let result = scan_project(root, &registry());
        let onchain = result
            .components
            .iter()
            .find(|c| c.role == Role::OnChain)
            .unwrap();
        assert_eq!(onchain.tool_id, "scalus");
    }

    #[test]
    fn scan_recognizes_aggregated_infra_by_driver_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Aggregated infra/: a single Justfile referencing cardano-up, no subdirs.
        fs::create_dir_all(root.join("infra")).unwrap();
        fs::write(
            root.join("infra/Justfile"),
            "dev:\n    cardano-up up --context demo\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let infra = result
            .components
            .iter()
            .find(|c| c.role == Role::Infrastructure)
            .expect("infra should be detected via the driver marker");
        assert_eq!(infra.tool_id, crate::doctor::INFRA_DRIVER_ID);
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn scan_infra_without_marker_is_unrecognized() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("infra")).unwrap();
        fs::write(root.join("infra/Justfile"), "dev:\n    echo nope\n").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(result.unrecognized[0].role, Role::Infrastructure);
    }

    #[test]
    fn scan_empty_project_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_project(tmp.path(), &registry());
        assert!(result.components.is_empty());
        assert!(result.unrecognized.is_empty());
    }
}
