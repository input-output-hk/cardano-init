//! Impure system probes for the doctor (tech-spec §9.3).
//!
//! Detects the OS, the installers and dependency binaries on `PATH`, and scans
//! a generated project to identify the tool in each component directory.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use super::INFRA_DRIVER_ID;
use super::catalog::DepCatalog;
use super::installers::Installer;
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{DetectSignature, Network, Role, RoleAssignment, Selection, ToolDef};

/// Detected operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::enum_variant_names)] // renaming `MacOs` would change the serialized value
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

/// The system-derived inputs to the pure resolver.
#[derive(Debug, Clone)]
pub struct Environment {
    pub os: Os,
    /// Installers available on this host.
    pub installers: HashSet<Installer>,
    /// Dependency and installer binaries found on `PATH`.
    pub present_binaries: HashSet<String>,
}

/// Detect the OS, available installers, and which catalog/installer binaries
/// are on `PATH`.
pub fn detect_environment(catalog: &DepCatalog) -> Environment {
    let dep_binaries = catalog
        .dep_ids()
        .filter_map(|id| catalog.get(id))
        .flat_map(|recipe| recipe.binaries.iter().map(String::as_str));
    let installer_binaries = Installer::ALL
        .iter()
        .flat_map(|i| i.detect().iter().copied());
    let candidates: HashSet<&str> = dep_binaries.chain(installer_binaries).collect();

    let present_binaries: HashSet<String> = candidates
        .into_iter()
        .filter(|b| is_on_path(b))
        .map(str::to_string)
        .collect();

    let installers = Installer::ALL
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

/// True if `bin` is found in any `PATH` entry, trying common executable
/// extensions on Windows.
fn is_on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        dir.join(bin).is_file()
            || (cfg!(windows)
                && ["exe", "cmd", "bat"]
                    .iter()
                    .any(|ext| dir.join(format!("{bin}.{ext}")).is_file()))
    })
}

/// What a scanned component directory is: one of the roles, or the fused
/// fullstack `protocol/` component (which is not a role).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Role(Role),
    Protocol,
}

impl ComponentKind {
    /// The kebab identifier: the role's kebab, or `"protocol"`.
    pub fn as_kebab(self) -> &'static str {
        match self {
            ComponentKind::Role(role) => role.as_kebab(),
            ComponentKind::Protocol => contract::DIR_PROTOCOL,
        }
    }

    /// The component's directory relative to the project root.
    fn dir(self) -> &'static str {
        match self {
            ComponentKind::Role(role) => role.dir(),
            ComponentKind::Protocol => contract::DIR_PROTOCOL,
        }
    }
}

impl std::fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComponentKind::Role(role) => write!(f, "{role}"),
            ComponentKind::Protocol => write!(f, "Protocol"),
        }
    }
}

/// A component directory whose contents were recognized as a specific tool.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedComponent {
    #[serde(rename = "role", serialize_with = "ser_kind")]
    pub kind: ComponentKind,
    pub tool_id: String,
}

/// A component directory whose contents matched no known tool.
#[derive(Debug, Clone, Serialize)]
pub struct UnrecognizedDir {
    #[serde(rename = "role", serialize_with = "ser_kind")]
    pub kind: ComponentKind,
    pub dir: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // signature required by `serialize_with`
fn ser_kind<S: serde::Serializer>(kind: &ComponentKind, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(kind.as_kebab())
}

/// The result of scanning a project tree.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub components: Vec<DetectedComponent>,
    pub unrecognized: Vec<UnrecognizedDir>,
}

impl ScanResult {
    fn record(&mut self, kind: ComponentKind, tool_id: Option<&str>) {
        match tool_id {
            Some(tool_id) => self.components.push(DetectedComponent {
                kind,
                tool_id: tool_id.to_string(),
            }),
            None => self.unrecognized.push(UnrecognizedDir {
                kind,
                dir: kind.dir().to_string(),
            }),
        }
    }
}

/// True if `dir` holds the aggregated infra driver: a `Justfile` that
/// references `cardano-up`.
fn infra_driver_present(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Justfile")).is_ok_and(|text| text.contains("cardano-up"))
}

/// True if the signature's file exists under `dir` and, when a `contains`
/// needle is given, its text includes it.
fn signature_matches(dir: &Path, sig: &DetectSignature) -> bool {
    let path = dir.join(&sig.file);
    match &sig.contains {
        None => path.exists(),
        Some(needle) => {
            std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle.as_str()))
        }
    }
}

/// True if `tool` matches `dir` via an existence-only signature, i.e. a
/// manifest unique to the tool (`trix.toml`, `aiken.toml`). Stronger evidence
/// than a `contains` needle on a shared file like `package.json`.
fn matches_definitively(dir: &Path, tool: &ToolDef) -> bool {
    tool.detect
        .iter()
        .any(|sig| sig.contains.is_none() && signature_matches(dir, sig))
}

/// Identify the single tool from `candidates` occupying `dir`. A tool matches
/// if any of its `detect` signatures matches. When several match, a definitive
/// manifest match wins (a `trix.toml` is a Tx3 project even if its
/// `package.json` pulls in `@meshsdk`); otherwise the dir is unrecognized.
fn identify_tool<'a>(dir: &Path, candidates: &[&'a ToolDef]) -> Option<&'a str> {
    let matched: Vec<&'a ToolDef> = candidates
        .iter()
        .copied()
        .filter(|tool| tool.detect.iter().any(|sig| signature_matches(dir, sig)))
        .collect();

    match matched.as_slice() {
        [tool] => Some(tool.id.as_str()),
        [] => None,
        _ => {
            let definitive: Vec<&&ToolDef> = matched
                .iter()
                .filter(|tool| matches_definitively(dir, tool))
                .collect();
            match definitive.as_slice() {
                [tool] => Some(tool.id.as_str()),
                _ => None,
            }
        }
    }
}

/// Scan a project root and identify the tool in each component directory
/// present: every role directory, then the fullstack `protocol/`.
pub fn scan_project(root: &Path, registry: &Registry) -> ScanResult {
    let mut scan = ScanResult {
        components: Vec::new(),
        unrecognized: Vec::new(),
    };
    let kinds = Role::ALL
        .iter()
        .copied()
        .map(ComponentKind::Role)
        .chain(std::iter::once(ComponentKind::Protocol));

    for kind in kinds {
        let dir = root.join(kind.dir());
        if !dir.is_dir() {
            continue;
        }
        let tool_id = match kind {
            // Infra aggregates into one driver component with no per-tool
            // subdirs, so it is recognized by the driver marker instead.
            ComponentKind::Role(Role::Infrastructure) => {
                infra_driver_present(&dir).then_some(INFRA_DRIVER_ID)
            }
            ComponentKind::Role(role) => identify_tool(&dir, &registry.tools_for_role(role)),
            ComponentKind::Protocol => {
                let fullstack: Vec<&ToolDef> = registry
                    .all_tools()
                    .iter()
                    .filter(|tool| tool.fullstack.is_some())
                    .collect();
                identify_tool(&dir, &fullstack)
            }
        };
        scan.record(kind, tool_id);
    }
    scan
}

/// A `Selection` reconstructed from an existing project tree, plus the caveats
/// hit along the way, for the caller to confirm with the user.
#[derive(Debug, Clone)]
pub struct Reconstructed {
    /// Best-effort recovered selection.
    pub selection: Selection,
    /// Component directories that matched no known tool.
    pub unrecognized: Vec<UnrecognizedDir>,
    /// Fields that had to be guessed (e.g. `"network"` when `.env` is missing).
    pub low_confidence: Vec<&'static str>,
    /// `cardano-up` packages in `infra/Justfile` that map to no registry tool.
    pub unknown_infra: Vec<String>,
}

/// Reconstruct the [`Selection`] that would (re)generate the project at `root`.
///
/// Roles and tools come from [`scan_project`], infra providers from
/// `infra/Justfile`, and `network`/`nix`/`spec`/`project_name` from `.env`,
/// `flake.nix`, `spec/` and the directory name.
pub fn reconstruct(root: &Path, registry: &Registry) -> Reconstructed {
    let scan = scan_project(root, registry);

    let mut assignments = Vec::new();
    let mut low_confidence = Vec::new();
    let mut unknown_infra = Vec::new();

    for component in &scan.components {
        match component.kind {
            ComponentKind::Role(Role::Infrastructure) => {
                let (tool_ids, unknown) = parse_infra_providers(root, registry);
                if tool_ids.is_empty() || !unknown.is_empty() {
                    low_confidence.push("infra");
                }
                assignments.extend(tool_ids.into_iter().map(|tool_id| RoleAssignment {
                    role: Role::Infrastructure,
                    tool_id,
                }));
                unknown_infra = unknown;
            }
            ComponentKind::Role(role) => assignments.push(RoleAssignment {
                role,
                tool_id: component.tool_id.clone(),
            }),
            // A fused `protocol/` is one tool filling both roles; the planner
            // re-collapses the two assignments.
            ComponentKind::Protocol => {
                assignments.extend([Role::OnChain, Role::OffChain].map(|role| RoleAssignment {
                    role,
                    tool_id: component.tool_id.clone(),
                }));
            }
        }
    }

    let network = read_network(root).unwrap_or_else(|| {
        low_confidence.push("network");
        Network::Preview
    });
    let project_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    Reconstructed {
        selection: Selection {
            project_name,
            assignments,
            network,
            nix: root.join("flake.nix").is_file(),
            spec: root.join("spec").is_dir(),
        },
        unrecognized: scan.unrecognized,
        low_confidence,
        unknown_infra,
    }
}

/// Recover the infra providers from the `cardano-up install <package>` lines in
/// `infra/Justfile`, mapping each package back to a registry tool. Returns
/// `(tool ids, unknown packages)`, both empty if the Justfile is unreadable.
fn parse_infra_providers(root: &Path, registry: &Registry) -> (Vec<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(root.join(contract::DIR_INFRA).join("Justfile")) else {
        return (Vec::new(), Vec::new());
    };

    let mut tool_ids = Vec::new();
    let mut unknown = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for window in tokens.windows(3) {
            let &["cardano-up", "install", pkg] = window else {
                continue;
            };
            match tool_for_package(pkg, registry) {
                Some(id) => {
                    if !tool_ids.contains(&id) {
                        tool_ids.push(id);
                    }
                }
                None => {
                    if !unknown.iter().any(|u| u == pkg) {
                        unknown.push(pkg.to_string());
                    }
                }
            }
        }
    }
    (tool_ids, unknown)
}

/// The infra tool whose `cardano_up_package` is `pkg`, if any.
fn tool_for_package(pkg: &str, registry: &Registry) -> Option<String> {
    registry
        .tools_for_role(Role::Infrastructure)
        .into_iter()
        .find(|t| {
            t.infra
                .as_ref()
                .is_some_and(|i| i.cardano_up_package == pkg)
        })
        .map(|t| t.id.clone())
}

/// Read `CARDANO_NETWORK` from the project's `.env`, if present and valid.
fn read_network(root: &Path) -> Option<Network> {
    let text = std::fs::read_to_string(root.join(".env")).ok()?;
    let prefix = format!("{}=", contract::ENV_NETWORK);
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))?
        .trim()
        .parse()
        .ok()
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
            .find(|c| c.kind == ComponentKind::Role(Role::OnChain))
            .unwrap();
        assert_eq!(onchain.tool_id, "aiken");
        let offchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OffChain))
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
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::OffChain)
        );
    }

    #[test]
    fn scan_identifies_tx3_despite_meshsdk_dependency() {
        // The Tx3 off-chain template legitimately depends on @meshsdk/core (it
        // imports it in blueprint.ts), so its package.json trips MeshJS's
        // `contains = "@meshsdk"` needle. Both tools match the dir — but a
        // trix.toml is a definitive, Tx3-unique manifest, so it must win over
        // the shared-file MeshJS match rather than reading as ambiguous.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(root.join("off-chain/trix.toml"), "[protocol]\n").unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "@meshsdk/core": "^1.9.0", "tx3-sdk": "^0.15.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());

        assert!(result.unrecognized.is_empty());
        let offchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OffChain))
            .expect("off-chain component identified");
        assert_eq!(offchain.tool_id, "tx3");
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
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::OnChain)
        );
    }

    #[test]
    fn scan_distinguishes_scalus_on_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(
            root.join("on-chain/build.sbt"),
            "\"org.scalus\" %% \"scalus\" % scalusVersion\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let onchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OnChain))
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
            .find(|c| c.kind == ComponentKind::Role(Role::Infrastructure))
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
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::Infrastructure)
        );
    }

    #[test]
    fn scan_detects_fullstack_protocol_component() {
        // A protocol/ dir with scalus's signatures is identified as the scalus
        // fullstack component (its real system_deps then feed the required set).
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("protocol")).unwrap();
        fs::write(
            root.join("protocol/build.sbt"),
            "\"org.scalus\" %% \"scalus\" % scalusVersion\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let protocol = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Protocol)
            .expect("protocol component should be detected");
        assert_eq!(protocol.tool_id, "scalus");
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn scan_protocol_without_signatures_is_unrecognized() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("protocol")).unwrap();
        fs::write(root.join("protocol/whatever.txt"), "").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(result.unrecognized[0].kind, ComponentKind::Protocol);
    }

    #[test]
    fn scan_empty_project_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_project(tmp.path(), &registry());
        assert!(result.components.is_empty());
        assert!(result.unrecognized.is_empty());
    }

    // Reconstruction round-trips: scaffold a real project, reconstruct its
    // Selection, and compare with what generated it.

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    /// Scaffold `selection` into `<tmp>/<name>` and reconstruct it back. The
    /// project is generated into a dir named after the project so the recovered
    /// `project_name` (derived from the dir) round-trips too.
    fn roundtrip(assignments: Vec<RoleAssignment>, network: Network, nix: bool) -> Reconstructed {
        let reg = registry();
        let sel = Selection {
            project_name: "my-protocol".to_string(),
            assignments,
            network,
            nix,
            spec: true,
        };
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(&sel.project_name);
        crate::scaffold::scaffold(&sel, &reg, &root).unwrap();
        // reconstruct reads eagerly, so it's safe to let `tmp` drop afterwards.
        reconstruct(&root, &reg)
    }

    /// Assignments compared as a set (planner order is not canonical in a
    /// `Selection`; the plan sorts at generation time).
    fn assert_same_assignments(mut got: Vec<RoleAssignment>, mut want: Vec<RoleAssignment>) {
        let key = |r: &RoleAssignment| (r.role.as_kebab(), r.tool_id.clone());
        got.sort_by_key(&key);
        want.sort_by_key(&key);
        assert_eq!(got, want);
    }

    #[test]
    fn reconstruct_aiken_and_meshjs() {
        let r = roundtrip(
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
            Network::Preview,
            false,
        );
        assert!(r.unrecognized.is_empty());
        assert!(r.low_confidence.is_empty(), "{:?}", r.low_confidence);
        assert_eq!(r.selection.project_name, "my-protocol");
        assert_eq!(r.selection.network, Network::Preview);
        assert!(!r.selection.nix);
        assert!(r.selection.spec);
        assert_same_assignments(
            r.selection.assignments,
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
        );
    }

    #[test]
    fn reconstruct_recovers_infra_provider_set() {
        // The scan collapses infra to one driver marker; reconstruction must
        // recover the individual providers from infra/Justfile.
        let r = roundtrip(
            vec![
                a(Role::OnChain, "aiken"),
                a(Role::Infrastructure, "kupo"),
                a(Role::Infrastructure, "ogmios"),
            ],
            Network::Preview,
            false,
        );
        assert!(r.unknown_infra.is_empty());
        assert_same_assignments(
            r.selection.assignments,
            vec![
                a(Role::OnChain, "aiken"),
                a(Role::Infrastructure, "kupo"),
                a(Role::Infrastructure, "ogmios"),
            ],
        );
    }

    #[test]
    fn reconstruct_expands_fullstack_protocol() {
        // scalus fills both roles via [fullstack]; the fused protocol/ must
        // round-trip to the two assignments the planner re-collapses.
        let r = roundtrip(
            vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
            Network::Preview,
            false,
        );
        assert!(r.unrecognized.is_empty());
        assert_same_assignments(
            r.selection.assignments,
            vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
        );
    }

    #[test]
    fn reconstruct_infers_spec_from_dir() {
        let reg = registry();
        let sel = Selection {
            project_name: "my-protocol".to_string(),
            assignments: vec![a(Role::OnChain, "aiken")],
            network: Network::Preview,
            nix: false,
            spec: false,
        };
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(&sel.project_name);
        crate::scaffold::scaffold(&sel, &reg, &root).unwrap();
        assert!(!reconstruct(&root, &reg).selection.spec);
    }

    #[test]
    fn reconstruct_recovers_network_and_nix() {
        let r = roundtrip(vec![a(Role::OnChain, "aiken")], Network::Preprod, true);
        assert_eq!(r.selection.network, Network::Preprod);
        assert!(r.selection.nix);
        assert!(r.low_confidence.is_empty(), "{:?}", r.low_confidence);
    }

    #[test]
    fn reconstruct_missing_env_flags_network_low_confidence() {
        // A project with a role dir but no .env: network can't be recovered.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/aiken.toml"), "").unwrap();

        let r = reconstruct(root, &registry());
        assert_eq!(r.selection.network, Network::Preview);
        assert!(r.low_confidence.contains(&"network"));
    }
}
