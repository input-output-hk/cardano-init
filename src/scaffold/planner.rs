//! Planning phase: decide every file the generated project contains. Reads only
//! embedded assets; nothing is rendered or written here.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::{ScaffoldError, TemplateAssets};
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, RoleAssignment, Selection, ToolDef};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where a file's content comes from in the embedded templates.
#[derive(Debug, Clone)]
pub enum TemplateSource {
    /// `templates/_base/<path>`.
    Base(String),
    /// `templates/<path>`, inside a tool's component template.
    Role(String),
    /// `templates/<path>`, inside an optional layer (`_spec/`, `_nix/`).
    Optional(String),
    /// Literal content (e.g. an empty `.gitkeep`).
    Inline(Vec<u8>),
}

impl TemplateSource {
    /// The `TemplateAssets` key for this source; `None` for `Inline`.
    pub fn asset_key(&self) -> Option<String> {
        match self {
            TemplateSource::Base(path) => Some(format!("_base/{path}")),
            TemplateSource::Role(path) | TemplateSource::Optional(path) => Some(path.clone()),
            TemplateSource::Inline(_) => None,
        }
    }
}

/// One file to emit in the generated project.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Destination path relative to the project root.
    pub dest: PathBuf,
    pub source: TemplateSource,
    /// Whether to render through `MiniJinja` (otherwise copied verbatim).
    pub render: bool,
}

impl FileEntry {
    /// An entry backed by an embedded asset: `.jinja` sources are rendered,
    /// anything else is copied verbatim.
    fn asset(dest: impl Into<PathBuf>, source: TemplateSource) -> Self {
        let render = matches!(
            &source,
            TemplateSource::Base(path) | TemplateSource::Role(path) | TemplateSource::Optional(path)
                if path.ends_with(".jinja")
        );
        Self {
            dest: dest.into(),
            source,
            render,
        }
    }
}

/// The complete, ordered list of files to generate.
#[derive(Debug)]
pub struct FilePlan {
    pub entries: Vec<FileEntry>,
}

/// Top-level files every project gets, as `(dest, source under _base/)`.
const BASE_FILES: &[(&str, &str)] = &[
    ("Justfile", "Justfile.jinja"),
    ("README.md", "README.md.jinja"),
    // `CLAUDE.md` only imports `AGENTS.md`: Claude Code does not read AGENTS.md
    // natively.
    ("AGENTS.md", "AGENTS.md.jinja"),
    ("CLAUDE.md", "CLAUDE.md"),
    (".gitignore", "gitignore"),
    (".env", "env.jinja"),
];

/// Files emitted under `--nix`, as `(dest, asset key)`.
pub(crate) const NIX_FILES: &[(&str, &str)] = &[
    ("flake.nix", "_nix/flake.nix.jinja"),
    (".envrc", "_nix/envrc.jinja"),
];

/// Empty marker that keeps `blueprint/` in the tree until a build writes the
/// blueprint into it.
pub(crate) const BLUEPRINT_MARKER: &str = "blueprint/.gitkeep";

/// Destinations of the shared top-level layer: files derived from the whole
/// selection rather than owned by one component. `spec/` is excluded: once
/// generated it belongs to the user.
pub(crate) fn shared_layer_dests() -> impl Iterator<Item = &'static str> {
    BASE_FILES
        .iter()
        .chain(NIX_FILES)
        .map(|(dest, _)| *dest)
        .chain([BLUEPRINT_MARKER])
}

// ---------------------------------------------------------------------------
// Manifest TOML (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ManifestToml {
    #[expect(dead_code, reason = "parsed only so a manifest lacking it is rejected")]
    manifest: ManifestMeta,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestMeta {
    #[expect(dead_code, reason = "parsed only so a manifest lacking it is rejected")]
    summary: String,
}

#[derive(Deserialize)]
struct ManifestFile {
    source: String,
    dest: String,
    /// Emission guard; absent means always emitted.
    #[serde(default)]
    when: Option<FileCondition>,
}

/// A condition gating whether a manifest file is emitted (kebab-case `when`).
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FileCondition {
    /// Emit only under `--nix`.
    Nix,
}

impl FileCondition {
    fn holds(&self, selection: &Selection) -> bool {
        match self {
            FileCondition::Nix => selection.nix,
        }
    }
}

/// Load and parse `<template>/manifest.toml` from the embedded assets.
///
/// # Errors
/// [`ScaffoldError::AssetNotFound`] if the manifest is missing,
/// [`ScaffoldError::ManifestParse`] if it is malformed.
fn read_manifest(template: &str) -> Result<ManifestToml, ScaffoldError> {
    let key = format!("{template}/manifest.toml");
    let asset = TemplateAssets::require(&key)?;
    // Embedded first-party asset: invalid UTF-8 is an authoring bug.
    let text = std::str::from_utf8(&asset.data).expect("manifest.toml must be valid UTF-8");
    toml::from_str(text).map_err(|source| ScaffoldError::ManifestParse { path: key, source })
}

/// Reject a manifest `dest` that could escape the project root (§4.4): it must
/// be non-empty and relative, with no `..` component.
fn validate_dest(dest: &str) -> Result<(), ScaffoldError> {
    let is_safe = !dest.is_empty()
        && Path::new(dest)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if is_safe {
        Ok(())
    } else {
        Err(ScaffoldError::UnsafePath {
            path: dest.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Selection queries shared with the context builder and `update`
// ---------------------------------------------------------------------------

/// Whether `spec/giftcard.md` ships: it describes the gift-card protocol, which
/// only on-chain/off-chain (incl. fullstack) components implement.
pub(crate) fn spec_example_present(selection: &Selection) -> bool {
    selection
        .assignments
        .iter()
        .any(|a| matches!(a.role, Role::OnChain | Role::OffChain))
}

/// Whether `blueprint/` is scaffolded: any non-infrastructure role is present
/// (tech spec §6.2).
pub(crate) fn blueprint_dir_present(selection: &Selection) -> bool {
    selection
        .assignments
        .iter()
        .any(|a| a.role != Role::Infrastructure)
}

/// The tool that fills both on-chain and off-chain as one fused `protocol/`
/// component, if any (tech spec §3.4, §6.1). Requires the same tool on both
/// roles **and** a `[fullstack]` template; otherwise the roles stay separate.
pub(crate) fn fullstack_tool_id(selection: &Selection, registry: &Registry) -> Option<String> {
    let tool_for_role = |role| {
        selection
            .assignments
            .iter()
            .find(|a| a.role == role)
            .map(|a| a.tool_id.as_str())
    };
    let tool_id = tool_for_role(Role::OnChain)?;
    if tool_for_role(Role::OffChain)? != tool_id {
        return None;
    }
    registry
        .get(tool_id)?
        .fullstack
        .as_ref()
        .map(|_| tool_id.to_string())
}

/// Whether `assignment` is one half of the fused fullstack pair.
pub(crate) fn is_fullstack_member(assignment: &RoleAssignment, fullstack_id: Option<&str>) -> bool {
    fullstack_id == Some(assignment.tool_id.as_str())
        && matches!(assignment.role, Role::OnChain | Role::OffChain)
}

/// The component directory `assignment` emits into: `protocol/` for the
/// fullstack pair, otherwise the role's own directory.
pub(crate) fn component_dir(
    assignment: &RoleAssignment,
    fullstack_id: Option<&str>,
) -> &'static str {
    if is_fullstack_member(assignment, fullstack_id) {
        contract::DIR_PROTOCOL
    } else {
        assignment.role.dir()
    }
}

/// Position of `role` in `Role::ALL`, the canonical role order.
fn role_rank(role: Role) -> usize {
    Role::ALL
        .iter()
        .position(|r| *r == role)
        .expect("Role::ALL lists every role")
}

/// The selection's assignments in canonical order (`Role::ALL`, then tool id),
/// so every order-sensitive output is independent of how the selection was
/// built (determinism, §11).
pub(crate) fn canonical_assignments(selection: &Selection) -> Vec<&RoleAssignment> {
    let mut ordered: Vec<&RoleAssignment> = selection.assignments.iter().collect();
    ordered.sort_by(|a, b| (role_rank(a.role), &a.tool_id).cmp(&(role_rank(b.role), &b.tool_id)));
    ordered
}

/// Look up the registry tool for `assignment`.
///
/// # Errors
/// [`ScaffoldError::ToolNotFound`] if the registry has no such tool.
pub(crate) fn tool_for<'r>(
    registry: &'r Registry,
    assignment: &RoleAssignment,
) -> Result<&'r ToolDef, ScaffoldError> {
    registry
        .get(&assignment.tool_id)
        .ok_or_else(|| ScaffoldError::ToolNotFound {
            tool_id: assignment.tool_id.clone(),
        })
}

/// The error for a tool assigned to a role it does not declare.
pub(crate) fn role_mismatch(assignment: &RoleAssignment) -> ScaffoldError {
    ScaffoldError::RoleMismatch {
        tool_id: assignment.tool_id.clone(),
        role: assignment.role.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Build the [`FilePlan`]: base layer, blueprint marker, one layer per
/// component (canonical order), then the optional `spec/` and Nix layers.
///
/// # Errors
/// Fails on an unknown tool or role, mismatched infrastructure templates, or a
/// missing/malformed/unsafe template manifest.
pub fn plan(selection: &Selection, registry: &Registry) -> Result<FilePlan, ScaffoldError> {
    let mut entries: Vec<FileEntry> = BASE_FILES
        .iter()
        .map(|(dest, source)| FileEntry::asset(*dest, TemplateSource::Base((*source).to_string())))
        .collect();

    if blueprint_dir_present(selection) {
        entries.push(FileEntry {
            dest: PathBuf::from(BLUEPRINT_MARKER),
            source: TemplateSource::Inline(Vec::new()),
            render: false,
        });
    }

    for (template, dir) in components(selection, registry)? {
        entries.extend(component_entries(template, dir, selection)?);
    }

    entries.extend(optional_entries(selection));

    Ok(FilePlan { entries })
}

/// Resolve the selection into its components as `(template path, dest dir)`, in
/// canonical order. The fullstack pair fuses into one `protocol/` component, and
/// all infrastructure tools share a single `infra/` driver template.
fn components<'r>(
    selection: &Selection,
    registry: &'r Registry,
) -> Result<Vec<(&'r str, &'static str)>, ScaffoldError> {
    let fullstack_id = fullstack_tool_id(selection, registry);
    let mut infra_template: Option<&str> = None;
    let mut components = Vec::new();

    for assignment in canonical_assignments(selection) {
        let tool = tool_for(registry, assignment)?;

        if is_fullstack_member(assignment, fullstack_id.as_deref()) {
            // Emit the pair once, on the on-chain half (which sorts first).
            if assignment.role == Role::OnChain {
                let fullstack = tool
                    .fullstack
                    .as_ref()
                    .expect("fullstack_tool_id guarantees a [fullstack] template");
                components.push((fullstack.template.as_str(), contract::DIR_PROTOCOL));
            }
            continue;
        }

        let template = tool
            .roles
            .get(&assignment.role)
            .ok_or_else(|| role_mismatch(assignment))?
            .template
            .as_str();

        if assignment.role == Role::Infrastructure {
            match infra_template {
                Some(expected) if expected != template => {
                    return Err(ScaffoldError::InfraTemplateMismatch {
                        tool_id: assignment.tool_id.clone(),
                        template: template.to_string(),
                        expected: expected.to_string(),
                    });
                }
                // The aggregated component is already planned.
                Some(_) => continue,
                None => infra_template = Some(template),
            }
        }

        components.push((template, assignment.role.dir()));
    }

    Ok(components)
}

/// The files `template`'s manifest emits under `dir`, honoring `when` guards.
fn component_entries(
    template: &str,
    dir: &str,
    selection: &Selection,
) -> Result<Vec<FileEntry>, ScaffoldError> {
    read_manifest(template)?
        .files
        .iter()
        .filter(|file| file.when.as_ref().is_none_or(|cond| cond.holds(selection)))
        .map(|file| {
            validate_dest(&file.dest)?;
            Ok(FileEntry::asset(
                Path::new(dir).join(&file.dest),
                TemplateSource::Role(format!("{template}/{}", file.source)),
            ))
        })
        .collect()
}

/// The optional layers: the `spec/` folder and the Nix flake.
fn optional_entries(selection: &Selection) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    if selection.spec {
        entries.push(FileEntry::asset(
            "spec/README.md",
            TemplateSource::Optional("_spec/README.md.jinja".into()),
        ));
        if spec_example_present(selection) {
            entries.push(FileEntry::asset(
                "spec/giftcard.md",
                TemplateSource::Optional("_spec/giftcard.md.jinja".into()),
            ));
        }
    }
    if selection.nix {
        entries.extend(NIX_FILES.iter().map(|(dest, source)| {
            FileEntry::asset(*dest, TemplateSource::Optional((*source).to_string()))
        }));
    }
    entries
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::test_support::{a, registry};
    use super::*;
    use crate::registry::types::Network;

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "test-project".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    fn plan_dests(sel: &Selection) -> Vec<String> {
        plan(sel, &registry())
            .unwrap()
            .entries
            .iter()
            .map(|e| e.dest.to_string_lossy().into_owned())
            .collect()
    }

    fn assert_plan_contains(sel: &Selection, expected: &[&str]) {
        let dests = plan_dests(sel);
        for want in expected {
            assert!(dests.iter().any(|d| d == want), "plan is missing '{want}'");
        }
    }

    #[test]
    fn base_files_always_present() {
        assert_plan_contains(
            &selection(vec![a(Role::OnChain, "aiken")]),
            &[
                "Justfile",
                "README.md",
                "AGENTS.md",
                "CLAUDE.md",
                ".gitignore",
                ".env",
            ],
        );
    }

    #[test]
    fn blueprint_gitkeep_when_on_chain() {
        assert_plan_contains(
            &selection(vec![a(Role::OnChain, "aiken")]),
            &["blueprint/.gitkeep"],
        );
    }

    #[test]
    fn blueprint_present_for_non_onchain_role() {
        // Off-chain-only still gets the blueprint dir: it's a consuming role
        // and a user may drop in an externally-built plutus.json.
        assert_plan_contains(
            &selection(vec![a(Role::OffChain, "meshjs")]),
            &["blueprint/.gitkeep"],
        );
    }

    #[test]
    fn no_blueprint_for_infra_only() {
        let infra_only = selection(vec![a(Role::Infrastructure, "some-infra")]);
        assert!(!blueprint_dir_present(&infra_only));

        let devnet_only = selection(vec![a(Role::Devnet, "yaci")]);
        assert!(blueprint_dir_present(&devnet_only));
    }

    #[test]
    fn yaci_devnet_entries() {
        assert_plan_contains(
            &selection(vec![a(Role::Devnet, "yaci")]),
            &[
                "blueprint/.gitkeep",
                "devnet/Justfile",
                "devnet/integration.test.mjs",
                "devnet/scripts/devnet-test.sh",
                "devnet/scripts/set-env.mjs",
            ],
        );
    }

    #[test]
    fn aiken_on_chain_entries() {
        assert_plan_contains(
            &selection(vec![a(Role::OnChain, "aiken")]),
            &[
                "on-chain/aiken.toml",
                "on-chain/Justfile",
                "on-chain/lib/helpers.ak",
                "on-chain/validators/giftcard.ak",
            ],
        );
    }

    #[test]
    fn meshjs_off_chain_entries() {
        assert_plan_contains(
            &selection(vec![a(Role::OffChain, "meshjs")]),
            &[
                "off-chain/package.json",
                "off-chain/Justfile",
                "off-chain/src/index.ts",
            ],
        );
    }

    #[test]
    fn plan_order_is_canonical_regardless_of_input_order() {
        let forward = selection(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let reversed = selection(vec![a(Role::OffChain, "meshjs"), a(Role::OnChain, "aiken")]);

        let fwd = plan_dests(&forward);
        assert_eq!(
            fwd,
            plan_dests(&reversed),
            "flag order must not affect plan"
        );

        let oc = fwd.iter().position(|d| d.starts_with("on-chain/")).unwrap();
        let off = fwd
            .iter()
            .position(|d| d.starts_with("off-chain/"))
            .unwrap();
        assert!(
            oc < off,
            "on-chain layer must precede off-chain (Role::ALL order)"
        );
    }

    #[test]
    fn combined_selection_entry_count() {
        let sel = selection(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let plan = plan(&sel, &registry()).unwrap();

        // base 6 + blueprint marker 1 + aiken 4 + meshjs 11 + spec 2
        assert_eq!(plan.entries.len(), 24);
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![a(Role::OnChain, "nonexistent")]);
        assert!(matches!(
            plan(&sel, &registry()),
            Err(ScaffoldError::ToolNotFound { .. })
        ));
    }

    #[test]
    fn fullstack_collapses_into_single_protocol_component() {
        let sel = selection(vec![
            a(Role::OnChain, "scalus"),
            a(Role::OffChain, "scalus"),
        ]);
        let dests = plan_dests(&sel);

        assert_plan_contains(
            &sel,
            &[
                "protocol/Justfile",
                "protocol/src/main/scala/app/GiftCard.scala",
                "protocol/src/main/scala/app/Main.scala",
                "blueprint/.gitkeep",
            ],
        );
        assert_eq!(
            dests.iter().filter(|d| *d == "protocol/Justfile").count(),
            1,
            "protocol component must be emitted once, not once per role"
        );
        assert!(!dests.iter().any(|d| d.starts_with("on-chain/")));
        assert!(!dests.iter().any(|d| d.starts_with("off-chain/")));
    }

    #[test]
    fn fullstack_tool_id_detects_the_pair() {
        let reg = registry();
        let both = selection(vec![
            a(Role::OnChain, "scalus"),
            a(Role::OffChain, "scalus"),
        ]);
        assert_eq!(fullstack_tool_id(&both, &reg).as_deref(), Some("scalus"));

        let mixed = selection(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        assert_eq!(fullstack_tool_id(&mixed, &reg), None);

        let single = selection(vec![a(Role::OnChain, "scalus")]);
        assert_eq!(fullstack_tool_id(&single, &reg), None);
    }

    #[test]
    fn infra_aggregates_into_single_component() {
        let sel = selection(vec![
            a(Role::Infrastructure, "kupo"),
            a(Role::Infrastructure, "ogmios"),
        ]);
        let dests = plan_dests(&sel);

        assert_plan_contains(
            &sel,
            &[
                "infra/Justfile",
                "infra/README.md",
                "infra/scripts/write-env.sh",
            ],
        );
        assert_eq!(
            dests.iter().filter(|d| *d == "infra/Justfile").count(),
            1,
            "infra driver must be emitted exactly once"
        );
        assert!(!dests.iter().any(|d| d.starts_with("infra/kupo/")));
        assert!(!dests.iter().any(|d| d.starts_with("infra/ogmios/")));
        assert!(!dests.iter().any(|d| d.starts_with("blueprint/")));
    }

    #[test]
    fn validate_dest_accepts_relative_paths() {
        assert!(validate_dest("Justfile").is_ok());
        assert!(validate_dest("src/index.ts").is_ok());
        assert!(validate_dest(".gitkeep").is_ok());
        assert!(validate_dest("a/b/c.txt").is_ok());
    }

    #[test]
    fn validate_dest_rejects_escapes() {
        assert!(validate_dest("").is_err());
        assert!(validate_dest("/etc/passwd").is_err());
        assert!(validate_dest("../escape").is_err());
        assert!(validate_dest("a/../../b").is_err());
        assert!(validate_dest("sub/../../../etc").is_err());
    }

    #[test]
    fn spec_true_includes_spec_files() {
        assert_plan_contains(
            &selection(vec![a(Role::OnChain, "aiken")]),
            &["spec/README.md", "spec/giftcard.md"],
        );
    }

    #[test]
    fn spec_false_excludes_spec_files() {
        let mut sel = selection(vec![a(Role::OnChain, "aiken")]);
        sel.spec = false;
        assert!(!plan_dests(&sel).iter().any(|d| d.starts_with("spec/")));
    }

    #[test]
    fn spec_example_only_with_protocol_roles() {
        // Devnet-only projects have no gift-card code to describe.
        let sel = selection(vec![a(Role::Devnet, "yaci")]);
        assert_plan_contains(&sel, &["spec/README.md"]);
        assert!(!plan_dests(&sel).iter().any(|d| d == "spec/giftcard.md"));
    }

    #[test]
    fn nix_true_includes_flake() {
        let mut sel = selection(vec![a(Role::OnChain, "aiken")]);
        sel.nix = true;
        assert_plan_contains(&sel, &["flake.nix", ".envrc"]);
    }

    #[test]
    fn nix_false_excludes_flake() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        assert!(!plan_dests(&sel).iter().any(|d| d == "flake.nix"));
    }

    /// Every tool's role and fullstack templates resolve to a manifest whose
    /// listed sources all exist as embedded assets.
    #[test]
    fn every_registered_template_resolves() {
        let reg = registry();
        for tool in reg.all_tools() {
            let templates = tool
                .roles
                .values()
                .chain(&tool.fullstack)
                .map(|cfg| &cfg.template);
            for template in templates {
                let manifest =
                    read_manifest(template).unwrap_or_else(|e| panic!("tool '{}': {e}", tool.id));
                for file in &manifest.files {
                    let source_key = format!("{template}/{}", file.source);
                    assert!(
                        TemplateAssets::get(&source_key).is_some(),
                        "tool '{}' template '{template}' references missing source '{source_key}'",
                        tool.id,
                    );
                }
            }
        }
    }

    /// The recipe name if `line` is a column-0 Justfile target header, else
    /// `None`. A target header is `name [params]: [deps]` at column 0 —
    /// distinguished from a `name := value` assignment and from `#` / `{% … %}`
    /// (comment / Jinja) lines.
    fn justfile_target_name(line: &str) -> Option<String> {
        if line.is_empty() || line.starts_with(char::is_whitespace) {
            return None; // recipe body or blank, not a header
        }
        let trimmed = line.trim_end();
        if trimmed.starts_with('#') || trimmed.starts_with("{%") || trimmed.starts_with("{#") {
            return None; // comment or Jinja control line
        }
        let colon = trimmed.find(':')?;
        if trimmed[colon..].starts_with(":=") {
            return None; // variable assignment, not a recipe
        }
        // The name is the first token before the ':' (params follow it).
        trimmed[..colon]
            .split_whitespace()
            .next()
            .map(str::to_string)
    }

    /// Parse a Justfile template's text into `target name -> recipe body lines`
    /// (indentation stripped). Jinja control lines interleaved in a recipe
    /// (e.g. the infra driver's `{%- for … %}`) are skipped; a blank line or
    /// the next column-0 line ends a recipe.
    fn justfile_targets(text: &str) -> BTreeMap<String, Vec<String>> {
        let lines: Vec<&str> = text.lines().collect();
        let mut targets = BTreeMap::new();
        for (i, line) in lines.iter().enumerate() {
            let Some(name) = justfile_target_name(line) else {
                continue;
            };
            let mut body = Vec::new();
            for l in &lines[i + 1..] {
                if l.trim().is_empty() {
                    break; // blank line terminates the recipe
                }
                if l.starts_with(char::is_whitespace) {
                    body.push(l.trim().to_string());
                } else if !(l.starts_with("{%") || l.starts_with("{#")) {
                    break; // next column-0 line; Jinja control lines are skipped
                }
            }
            targets.insert(name, body);
        }
        targets
    }

    /// Whether a recipe body carries no real work — every line is empty, a
    /// comment, or merely prints a message (`echo` / `true`) after stripping
    /// just's line prefixes (`@` silent, `-` ignore-error). A no-op `build`,
    /// `test`, or `clean` is allowed (a tool with nothing to do still exposes
    /// the target); a no-op `dev` is not (§7).
    fn is_noop_recipe(body: &[String]) -> bool {
        body.iter().all(|line| {
            let cmd = line.trim_start_matches(['@', '-']).trim_start();
            cmd.is_empty()
                || cmd.starts_with('#')
                || cmd == "echo"
                || cmd.starts_with("echo ")
                || cmd == "true"
                || cmd == ":"
        })
    }

    /// Interface contract (§7), statically checked across every distinct
    /// component template: the Justfile exposes `build`/`test`/`clean`, and
    /// `dev` only for a genuine watch/daemon/devnet mode. Whether components
    /// actually build is left to the smoke matrix.
    #[test]
    fn every_template_satisfies_interface_contract() {
        let reg = registry();

        // Distinct templates (the infra driver is shared), each mapped to an
        // owning tool id for failure messages.
        let mut templates: BTreeMap<&str, &str> = BTreeMap::new();
        for tool in reg.all_tools() {
            for cfg in tool.roles.values().chain(&tool.fullstack) {
                templates.entry(&cfg.template).or_insert(&tool.id);
            }
        }

        for (template, owner) in &templates {
            let manifest = read_manifest(template)
                .unwrap_or_else(|e| panic!("template '{template}' ({owner}): {e}"));

            let justfile = manifest
                .files
                .iter()
                .find(|f| f.dest == "Justfile")
                .unwrap_or_else(|| {
                    panic!(
                        "template '{template}' ({owner}) emits no Justfile; the interface \
                         contract requires build/test/clean targets (§7)"
                    )
                });

            let jf_key = format!("{template}/{}", justfile.source);
            let jf_data = TemplateAssets::get(&jf_key).unwrap_or_else(|| {
                panic!("template '{template}' Justfile source '{jf_key}' missing")
            });
            let jf_text = std::str::from_utf8(&jf_data.data).expect("Justfile must be UTF-8");

            let targets = justfile_targets(jf_text);

            // Required, terminating, composable targets (§7).
            for required in ["build", "test", "clean"] {
                assert!(
                    targets.contains_key(required),
                    "template '{template}' ({owner}) Justfile is missing the required \
                     '{required}' target (interface contract §7)"
                );
            }

            // `dev` is optional, but present only for a real mode — no no-op dev.
            if let Some(body) = targets.get("dev") {
                assert!(
                    !is_noop_recipe(body),
                    "template '{template}' ({owner}) has a no-op 'dev' target; per §7 a \
                     component exposes 'dev' only for a genuine watch/daemon/devnet mode"
                );
            }
        }

        // Guard the guard: make sure we actually iterated the templates (a
        // silently-empty registry would vacuously pass everything above).
        assert!(
            templates.len() >= 8,
            "expected to check every component template; only found {}",
            templates.len()
        );
    }
}
