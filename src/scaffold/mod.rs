//! The scaffolding pipeline: context -> plan -> render -> write.
//!
//! Everything except [`writer`] is pure: it reads only embedded assets.

pub mod context;
pub mod planner;
pub mod renderer;
pub mod update;
pub mod writer;

use std::path::Path;

use rust_embed::RustEmbed;

use crate::registry::loader::Registry;
use crate::registry::types::Selection;

/// The `templates/` tree, embedded at compile time.
#[derive(RustEmbed)]
#[folder = "templates/"]
pub(crate) struct TemplateAssets;

impl TemplateAssets {
    /// The embedded asset at `key`.
    ///
    /// # Errors
    /// [`ScaffoldError::AssetNotFound`] if no such asset is embedded.
    pub(crate) fn require(key: &str) -> Result<rust_embed::EmbeddedFile, ScaffoldError> {
        Self::get(key).ok_or_else(|| ScaffoldError::AssetNotFound {
            path: key.to_string(),
        })
    }
}

/// Errors from any phase of the scaffolding pipeline.
#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("template asset not found: '{path}'")]
    AssetNotFound { path: String },

    #[error("failed to parse manifest '{path}': {source}")]
    ManifestParse {
        path: String,
        source: toml::de::Error,
    },

    #[error("unsafe destination path '{path}' (must be relative, no '..' or leading '/')")]
    UnsafePath { path: String },

    #[error("tool '{tool_id}' not found in registry")]
    ToolNotFound { tool_id: String },

    #[error("tool '{tool_id}' does not support role '{role}'")]
    RoleMismatch { tool_id: String, role: String },

    #[error(
        "infrastructure tools must share one driver template, but '{tool_id}' uses '{template}' \
         (expected '{expected}'); the infra role emits a single aggregated component"
    )]
    InfraTemplateMismatch {
        tool_id: String,
        template: String,
        expected: String,
    },

    #[error("template rendering failed for '{path}': {source}")]
    Render {
        path: String,
        source: minijinja::Error,
    },

    #[error("I/O error writing '{path}': {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Generate the project for `selection` under `root`.
///
/// # Errors
/// Any [`ScaffoldError`] from the context, plan, render, or write phase.
pub fn scaffold(
    selection: &Selection,
    registry: &Registry,
    root: &Path,
) -> Result<(), ScaffoldError> {
    let ctx = context::build_context(selection, registry)?;
    let plan = planner::plan(selection, registry)?;
    let files = renderer::render(&plan, &ctx)?;
    writer::write(&files, root)
}

/// Plan the project for `selection` without rendering or writing anything.
///
/// # Errors
/// Any [`ScaffoldError`] from [`planner::plan`].
pub fn dry_run(
    selection: &Selection,
    registry: &Registry,
) -> Result<planner::FilePlan, ScaffoldError> {
    planner::plan(selection, registry)
}

/// Fixtures shared by the scaffold test modules.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::registry::loader::Registry;
    use crate::registry::types::{Role, RoleAssignment};

    pub(crate) fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    pub(crate) fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{a, registry};
    use super::*;
    use crate::registry::types::{Network, Role, RoleAssignment};

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "my-protocol".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    #[test]
    fn scaffold_aiken_only() {
        let dir = tempfile::tempdir().unwrap();
        let sel = selection(vec![a(Role::OnChain, "aiken")]);

        scaffold(&sel, &registry(), dir.path()).unwrap();

        // Base files
        assert!(dir.path().join("Justfile").is_file());
        assert!(dir.path().join("README.md").is_file());
        assert!(dir.path().join("AGENTS.md").is_file());
        assert!(dir.path().join("CLAUDE.md").is_file());
        assert!(dir.path().join(".gitignore").is_file());
        assert!(dir.path().join(".env").is_file());

        // Blueprint dir
        assert!(dir.path().join("blueprint/.gitkeep").is_file());

        // On-chain files
        assert!(dir.path().join("on-chain/aiken.toml").is_file());
        assert!(dir.path().join("on-chain/Justfile").is_file());
        assert!(dir.path().join("on-chain/validators/giftcard.ak").is_file());

        // Content checks
        let justfile = std::fs::read_to_string(dir.path().join("Justfile")).unwrap();
        assert!(justfile.contains("my-protocol"));
        assert!(justfile.contains("build-on-chain"));
        assert!(!justfile.contains("build-off-chain"));

        let readme = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert!(readme.contains("my-protocol"));
        assert!(readme.contains("Aiken"));

        // AGENTS.md is rendered (no leftover Jinja) and tailored to the stack;
        // CLAUDE.md imports it so Claude Code picks it up.
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("my-protocol"));
        assert!(agents.contains("Aiken"));
        assert!(agents.contains("cardano-dev-skills"));
        assert!(!agents.contains("{{"));
        assert!(!agents.contains("{%"));
        let claude = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(claude.contains("@AGENTS.md"));
    }

    #[test]
    fn scaffold_aiken_and_meshjs() {
        let dir = tempfile::tempdir().unwrap();
        let sel = selection(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);

        scaffold(&sel, &registry(), dir.path()).unwrap();

        // Both role directories present
        assert!(dir.path().join("on-chain/aiken.toml").is_file());
        assert!(dir.path().join("off-chain/package.json").is_file());
        assert!(dir.path().join("off-chain/src/index.ts").is_file());

        // Justfile references both roles
        let justfile = std::fs::read_to_string(dir.path().join("Justfile")).unwrap();
        assert!(justfile.contains("build-on-chain"));
        assert!(justfile.contains("build-off-chain"));

        // Package.json has project name
        let pkg = std::fs::read_to_string(dir.path().join("off-chain/package.json")).unwrap();
        assert!(pkg.contains("my-protocol"));
    }

    #[test]
    fn dry_run_returns_plan_without_writing() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);

        let plan = dry_run(&sel, &registry()).unwrap();

        assert!(!plan.entries.is_empty());
        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"Justfile"));
        assert!(dests.contains(&"on-chain/aiken.toml"));
    }
}
