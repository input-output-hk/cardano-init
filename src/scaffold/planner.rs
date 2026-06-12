use std::path::PathBuf;

use serde::Deserialize;

use super::{ScaffoldError, TemplateAssets};
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

/// Whether the `blueprint/` directory is scaffolded: true when any
/// non-infrastructure role is present (equivalently, unless the project is
/// infrastructure-only). See TECH_SPEC §6.2. Mirrors `TemplateContext::has_blueprint`.
pub(crate) fn blueprint_dir_present(selection: &Selection) -> bool {
    selection
        .assignments
        .iter()
        .any(|a| a.role != Role::Infrastructure)
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

    for assignment in ordered {
        let tool =
            registry
                .get(&assignment.tool_id)
                .ok_or_else(|| ScaffoldError::ToolNotFound {
                    tool_id: assignment.tool_id.clone(),
                })?;

        let role_config =
            tool.roles
                .get(&assignment.role)
                .ok_or_else(|| ScaffoldError::RoleMismatch {
                    tool_id: assignment.tool_id.clone(),
                    role: assignment.role.to_string(),
                })?;

        let template_path = &role_config.template; // e.g., "aiken/on-chain"
        let role_dir = assignment.role.dir(); // e.g., "on-chain"

        // For infrastructure, each tool gets its own subdirectory
        let dest_prefix = if assignment.role == Role::Infrastructure {
            PathBuf::from(role_dir).join(&assignment.tool_id)
        } else {
            PathBuf::from(role_dir)
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
            validate_dest(&file.dest)?;
            entries.push(FileEntry {
                dest: dest_prefix.join(&file.dest),
                source: TemplateSource::Role(format!("{}/{}", template_path, file.source)),
                render: file.source.ends_with(".jinja"),
            });
        }
    }

    // --- Optional layers ---
    if selection.nix {
        entries.push(FileEntry {
            dest: PathBuf::from("flake.nix"),
            source: TemplateSource::Optional("_nix/flake.nix.jinja".into()),
            render: true,
        });
        entries.push(FileEntry {
            dest: PathBuf::from(".envrc"),
            source: TemplateSource::Inline(b"use flake\n".to_vec()),
            render: false,
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
        // end-to-end (Yaci moved to the testing role).
        let infra_only = selection(vec![RoleAssignment {
            role: Role::Infrastructure,
            tool_id: "some-infra".into(),
        }]);
        assert!(!blueprint_dir_present(&infra_only));

        // Any non-infra role flips it on.
        let testing_only = selection(vec![RoleAssignment {
            role: Role::Testing,
            tool_id: "yaci".into(),
        }]);
        assert!(blueprint_dir_present(&testing_only));
    }

    #[test]
    fn yaci_testing_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::Testing,
            tool_id: "yaci".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        // Testing role lives under test/, and still gets the blueprint dir.
        assert!(dests.contains(&"blueprint/.gitkeep"));
        assert!(dests.contains(&"test/Justfile"));
        assert!(dests.contains(&"test/integration.test.mjs"));
        assert!(dests.contains(&"test/scripts/devnet-test.sh"));
        assert!(dests.contains(&"test/scripts/set-env.mjs"));
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

        // base: 4 (Justfile, README, .gitignore, .env)
        // blueprint/.gitkeep: 1
        // aiken on-chain: 4 (aiken.toml, Justfile, lib/helpers.ak, validators/giftcard.ak)
        // meshjs off-chain: 10 (package.json, tsconfig.json, Justfile, .env.example,
        //                       scripts/bundle-blueprint.mjs,
        //                       src/{contract,node,index,cli,contract.test}.ts)
        // total: 19
        assert_eq!(plan.entries.len(), 19);
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
            for (role, cfg) in &tool.roles {
                let manifest_key = format!("{}/manifest.toml", cfg.template);
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
                    let source_key = format!("{}/{}", cfg.template, file.source);
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
