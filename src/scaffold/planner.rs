use std::path::PathBuf;

use serde::Deserialize;

use super::{ScaffoldError, TemplateAssets};
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, RoleAssignment, Selection};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where a file's content comes from in the embedded templates.
#[derive(Debug, Clone)]
pub enum TemplateSource {
    /// From `templates/_base/<path>`
    Base(String),
    /// From `templates/<tool>/<role>/<path>`
    Role(String),
    /// From `templates/_nix/<path>`
    Optional(String),
    /// Inline content (e.g., empty `.gitkeep` files)
    Inline(Vec<u8>),
}

impl TemplateSource {
    /// The asset key used to look up this source in `TemplateAssets`.
    /// Returns `None` for `Inline` sources.
    pub fn asset_key(&self) -> Option<String> {
        match self {
            TemplateSource::Base(path) => Some(format!("_base/{path}")),
            TemplateSource::Role(path) => Some(path.clone()),
            TemplateSource::Optional(path) => Some(path.clone()),
            TemplateSource::Inline(_) => None,
        }
    }
}

/// One file to emit in the generated project.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Destination path relative to the project root.
    pub dest: PathBuf,
    /// Where the content comes from.
    pub source: TemplateSource,
    /// Whether to render through MiniJinja.
    pub render: bool,
}

/// The complete list of files to generate.
#[derive(Debug)]
pub struct FilePlan {
    pub entries: Vec<FileEntry>,
}

// ---------------------------------------------------------------------------
// Manifest TOML (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ManifestToml {
    #[allow(dead_code)]
    manifest: ManifestMeta,
    #[serde(rename = "files")]
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestMeta {
    #[allow(dead_code)]
    summary: String,
}

#[derive(Deserialize)]
struct ManifestFile {
    source: String,
    dest: String,
    /// Optional emission guard. When present, the file is emitted only if the
    /// named condition holds for the current selection (e.g. `when = "nix"`
    /// emits only under `--nix`). Absent ⇒ always emitted.
    #[serde(default)]
    when: Option<FileCondition>,
}

/// A condition gating whether a manifest file is emitted. Deserialized from the
/// kebab-case `when` field in `manifest.toml`.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FileCondition {
    /// Emit only when the project is generated with Nix support (`--nix`).
    Nix,
}

impl FileCondition {
    /// Whether this condition holds for the given selection.
    fn holds(&self, selection: &Selection) -> bool {
        match self {
            FileCondition::Nix => selection.nix,
        }
    }
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Reject a manifest `dest` that could escape the project root (§4.4).
///
/// A `dest` must be relative, non-empty, and contain no `..` component and no
/// leading `/`. Manifests are first-party today, but the check is cheap
/// insurance and required if templates ever become third-party.
fn validate_dest(dest: &str) -> Result<(), ScaffoldError> {
    use std::path::Component;

    let unsafe_path = || ScaffoldError::UnsafePath {
        path: dest.to_string(),
    };

    if dest.is_empty() {
        return Err(unsafe_path());
    }
    let path = std::path::Path::new(dest);
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            // RootDir / Prefix (absolute or `C:\`), ParentDir (`..`) → reject.
            _ => return Err(unsafe_path()),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Whether `spec/giftcard.md` ships: the example describes the gift-card
/// protocol, which only on-chain/off-chain (incl. fullstack) components implement.
pub(crate) fn spec_example_present(selection: &Selection) -> bool {
    selection
        .assignments
        .iter()
        .any(|a| matches!(a.role, Role::OnChain | Role::OffChain))
}

/// Whether the `blueprint/` directory is scaffolded: true when any
/// non-infrastructure role is present (equivalently, unless the project is
/// infrastructure-only). See TECH_SPEC §6.2. Mirrors `TemplateContext::has_blueprint`.
pub(crate) fn blueprint_dir_present(selection: &Selection) -> bool {
    selection
        .assignments
        .iter()
        .any(|a| a.role != Role::Infrastructure)
}

/// The tool id that fills **both** on-chain and off-chain as a single fullstack
/// component, if any. Returns `Some(tool_id)` only when one tool is assigned to
/// both `Role::OnChain` and `Role::OffChain` **and** declares a `[fullstack]`
/// template; in that case the two assignments collapse into one `protocol/`
/// component (TECH_SPEC §3.4, §6.1). Same tool for both roles with no
/// `[fullstack]` template returns `None` (falls back to two folders).
///
/// Shared by the context builder and the planner so both agree on the collapse.
pub(crate) fn fullstack_tool_id(selection: &Selection, registry: &Registry) -> Option<String> {
    let on_chain = selection
        .assignments
        .iter()
        .find(|a| a.role == Role::OnChain)?;
    let off_chain = selection
        .assignments
        .iter()
        .find(|a| a.role == Role::OffChain)?;
    if on_chain.tool_id != off_chain.tool_id {
        return None;
    }
    let tool = registry.get(&on_chain.tool_id)?;
    tool.fullstack.as_ref().map(|_| on_chain.tool_id.clone())
}

/// Build a `FilePlan` from a `Selection` and the tool `Registry`.
///
/// This determines every file that will be written during scaffolding.
/// No I/O is performed — only embedded assets are read.
pub fn plan(selection: &Selection, registry: &Registry) -> Result<FilePlan, ScaffoldError> {
    let mut entries = vec![
        // --- Base layer ---
        FileEntry {
            dest: PathBuf::from("Justfile"),
            source: TemplateSource::Base("Justfile.jinja".into()),
            render: true,
        },
        FileEntry {
            dest: PathBuf::from("README.md"),
            source: TemplateSource::Base("README.md.jinja".into()),
            render: true,
        },
        // Agent-context files. `AGENTS.md` is the canonical, cross-agent
        // (Codex/Cursor/…) brief tailored to the selected stack; `CLAUDE.md`
        // is a one-line `@AGENTS.md` import so Claude Code — which does not read
        // AGENTS.md natively — picks it up too. See TECH_SPEC §6.1.
        FileEntry {
            dest: PathBuf::from("AGENTS.md"),
            source: TemplateSource::Base("AGENTS.md.jinja".into()),
            render: true,
        },
        FileEntry {
            dest: PathBuf::from("CLAUDE.md"),
            source: TemplateSource::Base("CLAUDE.md".into()),
            render: false,
        },
        FileEntry {
            dest: PathBuf::from(".gitignore"),
            source: TemplateSource::Base("gitignore".into()),
            render: false,
        },
        FileEntry {
            dest: PathBuf::from(".env"),
            source: TemplateSource::Base("env.jinja".into()),
            render: true,
        },
    ];

    // Blueprint directory: present for every project that has any
    // blueprint-producing-or-consuming role — i.e. any role except
    // infrastructure (equivalently: present unless the project is
    // infrastructure-only). See TECH_SPEC §6.2.
    if blueprint_dir_present(selection) {
        entries.push(FileEntry {
            dest: PathBuf::from("blueprint/.gitkeep"),
            source: TemplateSource::Inline(Vec::new()),
            render: false,
        });
    }

    // --- Role layers ---
    // Emit in canonical order regardless of flag/selection order (determinism, §11):
    // roles in `Role::ALL` order, and within Infrastructure (the only multi-tool
    // role) tools sorted by `tool_id`.
    let mut ordered: Vec<&RoleAssignment> = selection.assignments.iter().collect();
    ordered.sort_by(|a, b| {
        let ai = Role::ALL.iter().position(|r| *r == a.role).unwrap();
        let bi = Role::ALL.iter().position(|r| *r == b.role).unwrap();
        ai.cmp(&bi).then_with(|| a.tool_id.cmp(&b.tool_id))
    });

    // Fullstack collapse: when one tool fills both on-chain and off-chain and
    // declares a `[fullstack]` template, the two assignments emit a single
    // `protocol/` component (TECH_SPEC §6.1), emitted once on the on-chain
    // assignment (which sorts first) with the off-chain assignment skipped.
    let fullstack_id = fullstack_tool_id(selection, registry);

    // Infrastructure aggregates into a single component: all selected infra tools
    // share one driver template (`_infra/cardano-up`) emitted once at `infra/`,
    // rendered over the full set via `TemplateContext.infra_tools`. The shared
    // template's manifest is read on the first infra assignment; the rest are
    // skipped (they are contiguous after the canonical sort). See TECH_SPEC §6.1.
    let mut infra_template: Option<String> = None;

    for assignment in ordered {
        let tool =
            registry
                .get(&assignment.tool_id)
                .ok_or_else(|| ScaffoldError::ToolNotFound {
                    tool_id: assignment.tool_id.clone(),
                })?;

        let is_fullstack_member = fullstack_id.as_deref() == Some(assignment.tool_id.as_str())
            && matches!(assignment.role, Role::OnChain | Role::OffChain);

        // Resolve (template path, destination dir) for this assignment. Fullstack
        // and Infrastructure aggregate; every other role is one-tool-one-dir.
        let (template_path, dest_prefix): (String, PathBuf) =
            if is_fullstack_member {
                // Emit the aggregated protocol/ component once (on the on-chain
                // assignment); skip the off-chain half.
                if assignment.role == Role::OffChain {
                    continue;
                }
                let fs = tool
                    .fullstack
                    .as_ref()
                    .expect("fullstack_tool_id guarantees a [fullstack] template");
                (fs.template.clone(), PathBuf::from(contract::DIR_PROTOCOL))
            } else if assignment.role == Role::Infrastructure {
                let role_config = tool.roles.get(&assignment.role).ok_or_else(|| {
                    ScaffoldError::RoleMismatch {
                        tool_id: assignment.tool_id.clone(),
                        role: assignment.role.to_string(),
                    }
                })?;
                let template_path = role_config.template.clone();
                // All infra tools must resolve to the same shared driver template.
                match &infra_template {
                    Some(first) => {
                        if *first != template_path {
                            return Err(ScaffoldError::InfraTemplateMismatch {
                                tool_id: assignment.tool_id.clone(),
                                template: template_path,
                                expected: first.clone(),
                            });
                        }
                        // Already emitted the aggregated component; skip duplicates.
                        continue;
                    }
                    None => {
                        infra_template = Some(template_path.clone());
                        (template_path, PathBuf::from(assignment.role.dir()))
                    }
                }
            } else {
                let role_config = tool.roles.get(&assignment.role).ok_or_else(|| {
                    ScaffoldError::RoleMismatch {
                        tool_id: assignment.tool_id.clone(),
                        role: assignment.role.to_string(),
                    }
                })?;
                (
                    role_config.template.clone(),
                    PathBuf::from(assignment.role.dir()),
                )
            };

        // Read the manifest
        let manifest_key = format!("{template_path}/manifest.toml");
        let manifest_data =
            TemplateAssets::get(&manifest_key).ok_or_else(|| ScaffoldError::AssetNotFound {
                path: manifest_key.clone(),
            })?;
        let manifest_text =
            std::str::from_utf8(&manifest_data.data).expect("manifest.toml must be valid UTF-8");
        let manifest: ManifestToml =
            toml::from_str(manifest_text).map_err(|e| ScaffoldError::ManifestParse {
                path: manifest_key,
                source: e,
            })?;

        for file in &manifest.files {
            // Honor an optional emission guard (e.g. `when = "nix"`).
            if let Some(cond) = &file.when
                && !cond.holds(selection)
            {
                continue;
            }
            validate_dest(&file.dest)?;
            entries.push(FileEntry {
                dest: dest_prefix.join(&file.dest),
                source: TemplateSource::Role(format!("{}/{}", template_path, file.source)),
                render: file.source.ends_with(".jinja"),
            });
        }
    }

    // --- Optional layers ---
    // Protocol specification (plain text, non-executable). The worked example
    // describes the gift-card protocol every on-/off-chain template implements,
    // so it ships only when one of those roles is present.
    if selection.spec {
        entries.push(FileEntry {
            dest: PathBuf::from("spec/README.md"),
            source: TemplateSource::Optional("_spec/README.md.jinja".into()),
            render: true,
        });
        if spec_example_present(selection) {
            entries.push(FileEntry {
                dest: PathBuf::from("spec/giftcard.md"),
                source: TemplateSource::Optional("_spec/giftcard.md.jinja".into()),
                render: true,
            });
        }
    }
    if selection.nix {
        entries.push(FileEntry {
            dest: PathBuf::from("flake.nix"),
            source: TemplateSource::Optional("_nix/flake.nix.jinja".into()),
            render: true,
        });
        entries.push(FileEntry {
            dest: PathBuf::from(".envrc"),
            source: TemplateSource::Optional("_nix/envrc.jinja".into()),
            render: true,
        });
    }

    Ok(FilePlan { entries })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, RoleAssignment};

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "test-project".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    #[test]
    fn base_files_always_present() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"Justfile"));
        assert!(dests.contains(&"README.md"));
        assert!(dests.contains(&"AGENTS.md"));
        assert!(dests.contains(&"CLAUDE.md"));
        assert!(dests.contains(&".gitignore"));
        assert!(dests.contains(&".env"));
    }

    #[test]
    fn blueprint_gitkeep_when_on_chain() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"blueprint/.gitkeep"));
    }

    #[test]
    fn blueprint_present_for_non_onchain_role() {
        // Off-chain-only still gets the blueprint dir: it's a consuming role
        // and a user may drop in an externally-built plutus.json.
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"blueprint/.gitkeep"));
    }

    #[test]
    fn no_blueprint_for_infra_only() {
        // Infrastructure-only → no blueprint dir. Exercised through the predicate
        // because the registry currently ships no infrastructure tool to plan
        // end-to-end (Yaci fills the devnet role, not infra).
        let infra_only = selection(vec![RoleAssignment {
            role: Role::Infrastructure,
            tool_id: "some-infra".into(),
        }]);
        assert!(!blueprint_dir_present(&infra_only));

        // Any non-infra role flips it on.
        let devnet_only = selection(vec![RoleAssignment {
            role: Role::Devnet,
            tool_id: "yaci".into(),
        }]);
        assert!(blueprint_dir_present(&devnet_only));
    }

    #[test]
    fn yaci_devnet_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::Devnet,
            tool_id: "yaci".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        // Devnet role lives under devnet/, and still gets the blueprint dir.
        assert!(dests.contains(&"blueprint/.gitkeep"));
        assert!(dests.contains(&"devnet/Justfile"));
        assert!(dests.contains(&"devnet/integration.test.mjs"));
        assert!(dests.contains(&"devnet/scripts/devnet-test.sh"));
        assert!(dests.contains(&"devnet/scripts/set-env.mjs"));
    }

    #[test]
    fn aiken_on_chain_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"on-chain/aiken.toml"));
        assert!(dests.contains(&"on-chain/Justfile"));
        assert!(dests.contains(&"on-chain/lib/helpers.ak"));
        assert!(dests.contains(&"on-chain/validators/giftcard.ak"));
    }

    #[test]
    fn meshjs_off_chain_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"off-chain/package.json"));
        assert!(dests.contains(&"off-chain/Justfile"));
        assert!(dests.contains(&"off-chain/src/index.ts"));
    }

    #[test]
    fn plan_order_is_canonical_regardless_of_input_order() {
        let forward = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
        ]);
        let reversed = selection(vec![
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
        ]);

        let dests = |s| {
            plan(s, &registry())
                .unwrap()
                .entries
                .iter()
                .map(|e| e.dest.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let fwd = dests(&forward);
        assert_eq!(fwd, dests(&reversed), "flag order must not affect plan");

        // On-chain layer precedes off-chain layer (Role::ALL order).
        let oc = fwd.iter().position(|d| d.starts_with("on-chain/")).unwrap();
        let off = fwd
            .iter()
            .position(|d| d.starts_with("off-chain/"))
            .unwrap();
        assert!(oc < off);
    }

    #[test]
    fn combined_selection_entry_count() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
        ]);
        let plan = plan(&sel, &registry()).unwrap();

        // base: 6 (Justfile, README, AGENTS.md, CLAUDE.md, .gitignore, .env)
        // blueprint/.gitkeep: 1
        // aiken on-chain: 4 (aiken.toml, Justfile, lib/helpers.ak, validators/giftcard.ak)
        // meshjs off-chain: 11 (package.json, tsconfig.json, Justfile, .env.example,
        //                       scripts/bundle-blueprint.mjs,
        //                       src/{contract,node,index,cli,contract.test,contract.integration.test}.ts)
        // spec: 2 (spec/README.md, spec/giftcard.md)
        // total: 24
        assert_eq!(plan.entries.len(), 24);
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "nonexistent".into(),
        }]);
        assert!(matches!(
            plan(&sel, &registry()),
            Err(ScaffoldError::ToolNotFound { .. })
        ));
    }

    #[test]
    fn fullstack_collapses_into_single_protocol_component() {
        // Scalus fills both on-chain and off-chain and declares a [fullstack]
        // template, so the two assignments emit ONE protocol/ component — no
        // on-chain/ or off-chain/ directories.
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "scalus".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "scalus".into(),
            },
        ]);
        let plan = plan(&sel, &registry()).unwrap();
        let dests: Vec<String> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_string_lossy().into_owned())
            .collect();

        assert!(dests.contains(&"protocol/Justfile".to_string()));
        assert!(dests.contains(&"protocol/src/main/scala/app/GiftCard.scala".to_string()));
        assert!(dests.contains(&"protocol/src/main/scala/app/Main.scala".to_string()));
        // The protocol Justfile is emitted exactly once (not once per role).
        assert_eq!(
            dests.iter().filter(|d| *d == "protocol/Justfile").count(),
            1
        );
        // No separate role directories.
        assert!(!dests.iter().any(|d| d.starts_with("on-chain/")));
        assert!(!dests.iter().any(|d| d.starts_with("off-chain/")));
        // Blueprint dir is still present (protocol is a producer/consumer).
        assert!(dests.contains(&"blueprint/.gitkeep".to_string()));
    }

    #[test]
    fn fullstack_tool_id_detects_the_pair() {
        let reg = registry();
        // Same tool on both roles + [fullstack] template → collapse.
        let both = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "scalus".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "scalus".into(),
            },
        ]);
        assert_eq!(fullstack_tool_id(&both, &reg).as_deref(), Some("scalus"));

        // Different tools per role → no collapse.
        let mixed = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
        ]);
        assert_eq!(fullstack_tool_id(&mixed, &reg), None);

        // Only one role → no collapse.
        let single = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "scalus".into(),
        }]);
        assert_eq!(fullstack_tool_id(&single, &reg), None);
    }

    #[test]
    fn infra_aggregates_into_single_component() {
        // Multiple --infra providers emit ONE aggregated infra/ component (the
        // shared cardano-up driver), not per-tool infra/<tool>/ subdirs.
        let sel = selection(vec![
            RoleAssignment {
                role: Role::Infrastructure,
                tool_id: "kupo".into(),
            },
            RoleAssignment {
                role: Role::Infrastructure,
                tool_id: "ogmios".into(),
            },
        ]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<String> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_string_lossy().into_owned())
            .collect();

        // Single aggregated component at infra/, emitted once.
        assert!(dests.contains(&"infra/Justfile".to_string()));
        assert!(dests.contains(&"infra/README.md".to_string()));
        assert!(dests.contains(&"infra/scripts/write-env.sh".to_string()));
        assert_eq!(
            dests.iter().filter(|d| *d == "infra/Justfile").count(),
            1,
            "infra driver must be emitted exactly once"
        );
        // No per-tool subdirs.
        assert!(!dests.iter().any(|d| d.starts_with("infra/kupo/")));
        assert!(!dests.iter().any(|d| d.starts_with("infra/ogmios/")));
        // Infra-only project: no blueprint dir.
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

    fn plan_dests(sel: &Selection) -> Vec<String> {
        plan(sel, &registry())
            .unwrap()
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn spec_true_includes_spec_files() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let dests = plan_dests(&sel);
        assert!(dests.contains(&"spec/README.md".to_string()));
        assert!(dests.contains(&"spec/giftcard.md".to_string()));
    }

    #[test]
    fn spec_false_excludes_spec_files() {
        let mut sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        sel.spec = false;
        assert!(!plan_dests(&sel).iter().any(|d| d.starts_with("spec/")));
    }

    #[test]
    fn spec_example_only_with_protocol_roles() {
        // Infra/devnet-only projects have no gift-card code to describe: the
        // folder (README) ships, the worked example doesn't.
        let sel = selection(vec![RoleAssignment {
            role: Role::Devnet,
            tool_id: "yaci".into(),
        }]);
        let dests = plan_dests(&sel);
        assert!(dests.contains(&"spec/README.md".to_string()));
        assert!(!dests.contains(&"spec/giftcard.md".to_string()));
    }

    #[test]
    fn nix_true_includes_flake() {
        let mut sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        sel.nix = true;
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"flake.nix"));
    }

    /// Guard against the "registered but missing template" class of bug:
    /// every tool's every role template must resolve to a real manifest, and
    /// every file the manifest lists must exist as an embedded asset.
    #[test]
    fn every_registered_template_resolves() {
        let reg = registry();
        for tool in reg.all_tools() {
            // Check each role template plus the optional fullstack template.
            let templates = tool
                .roles
                .iter()
                .map(|(role, cfg)| (role.to_string(), &cfg.template))
                .chain(
                    tool.fullstack
                        .iter()
                        .map(|cfg| ("fullstack".to_string(), &cfg.template)),
                );
            for (role, template) in templates {
                let manifest_key = format!("{template}/manifest.toml");
                let data = TemplateAssets::get(&manifest_key).unwrap_or_else(|| {
                    panic!(
                        "tool '{}' role '{}' points at missing manifest '{}'",
                        tool.id, role, manifest_key
                    )
                });
                let text = std::str::from_utf8(&data.data).expect("manifest must be UTF-8");
                let manifest: ManifestToml = toml::from_str(text)
                    .unwrap_or_else(|e| panic!("manifest '{manifest_key}' failed to parse: {e}"));

                for file in &manifest.files {
                    let source_key = format!("{}/{}", template, file.source);
                    assert!(
                        TemplateAssets::get(&source_key).is_some(),
                        "tool '{}' manifest '{}' references missing source '{}'",
                        tool.id,
                        manifest_key,
                        source_key
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
    fn justfile_targets(text: &str) -> std::collections::BTreeMap<String, Vec<String>> {
        let lines: Vec<&str> = text.lines().collect();
        let mut targets = std::collections::BTreeMap::new();
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
                    body.push(l.trim().to_string()); // a command line
                } else if l.trim_start().starts_with("{%") || l.trim_start().starts_with("{#") {
                    continue; // Jinja control line inside the recipe — not a command
                } else {
                    break; // next column-0 line
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

    /// Contract-compliance test (issue #10): every component template must
    /// honor the interface contract (§7) — expose `build`/`test`/`clean`
    /// Justfile targets, and provide `dev` only for a genuine watch/daemon/
    /// devnet mode (no no-op `dev`). Runs across every distinct template
    /// referenced by the registry.
    ///
    /// This checks the statically-verifiable contract. That each component
    /// *actually builds* standalone is the job of `just build`/`just test`
    /// (the smoke matrix), deliberately not duplicated in a heuristic here
    /// (cf. the doctor scope note, TECH_SPEC §9).
    #[test]
    fn every_template_satisfies_interface_contract() {
        use std::collections::BTreeMap;

        let reg = registry();

        // Every distinct component template (role + fullstack), deduped by
        // path — the infra driver template is shared across all infra tools.
        // Value = an owning tool id, for clearer failure messages.
        let mut templates: BTreeMap<String, String> = BTreeMap::new();
        for tool in reg.all_tools() {
            for cfg in tool.roles.values() {
                templates
                    .entry(cfg.template.clone())
                    .or_insert_with(|| tool.id.clone());
            }
            if let Some(cfg) = &tool.fullstack {
                templates
                    .entry(cfg.template.clone())
                    .or_insert_with(|| tool.id.clone());
            }
        }

        for (template, owner) in &templates {
            // Locate the Justfile the manifest emits (dest == "Justfile").
            let manifest_key = format!("{template}/manifest.toml");
            let data = TemplateAssets::get(&manifest_key)
                .unwrap_or_else(|| panic!("template '{template}' ({owner}) missing manifest"));
            let text = std::str::from_utf8(&data.data).expect("manifest must be UTF-8");
            let manifest: ManifestToml = toml::from_str(text)
                .unwrap_or_else(|e| panic!("manifest '{manifest_key}' failed to parse: {e}"));

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

    #[test]
    fn nix_false_excludes_flake() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(!dests.contains(&"flake.nix"));
    }
}
