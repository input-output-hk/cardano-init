//! The CLI's top-level error type and its machine-readable contract (tech spec
//! §2.3–2.5): exit code, stable `code()` slug, and structured `context()`.

use crate::doctor::catalog::CatalogError;
use crate::registry::compat::Incompatibility;
use crate::registry::loader::RegistryError;
use crate::scaffold::ScaffoldError;

// `miette::Diagnostic` drives the human rendering (title + `help:` remedy); the
// inherent `code()`/`context()`/`exit_code()` drive JSON and the exit status.
// Each `#[diagnostic(code(...))]` slug must mirror `code()` under the
// `cardano_init::` namespace.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum CliError {
    #[error("{0}")]
    #[diagnostic(code(cardano_init::registry_load))]
    Registry(#[from] RegistryError),

    #[error("{0}")]
    #[diagnostic(code(cardano_init::scaffold_error))]
    Scaffold(#[from] ScaffoldError),

    #[error("{0}")]
    #[diagnostic(code(cardano_init::registry_load))]
    Catalog(#[from] CatalogError),

    #[error("directory '{}' already exists. Refusing to overwrite", path)]
    #[diagnostic(
        code(cardano_init::dir_exists),
        help("choose a different --name, or remove/empty the existing directory.")
    )]
    DirectoryExists { path: String },

    #[error("no tool named '{tool_id}' fills the {role} role")]
    #[diagnostic(
        code(cardano_init::unknown_tool),
        help("valid {role} tools: {}", valid_tools.join(", ")),
        url("https://github.com/input-output-hk/cardano-init#tools")
    )]
    UnknownTool {
        tool_id: String,
        role: String,
        valid_tools: Vec<String>,
    },

    #[error("tool '{tool_id}' does not support the '{role}' role")]
    #[diagnostic(
        code(cardano_init::tool_role_mismatch),
        help("'{tool_id}' fills: {}", valid_roles.join(", ")),
        url("https://github.com/input-output-hk/cardano-init#tools")
    )]
    ToolRoleMismatch {
        tool_id: String,
        role: String,
        valid_roles: Vec<String>,
    },

    #[error("no roles selected")]
    #[diagnostic(
        code(cardano_init::no_roles_selected),
        help("select at least one tool, or pass a one-shot flag like --on-chain aiken.")
    )]
    NoRolesSelected,

    #[error(
        "{} is experimental — it may be unstable or incomplete, so it's opt-in (expect rough edges and breaking changes)",
        labels.join(", ")
    )]
    #[diagnostic(
        code(cardano_init::experimental_not_allowed),
        help(
            "to scaffold it anyway, re-run with --allow-experimental:\n\n    cardano-init --name <project> {} --allow-experimental",
            example_flags.join(" ")
        )
    )]
    ExperimentalNotAllowed {
        /// Tool ids (machine-facing `context`).
        tools: Vec<String>,
        /// Human labels, `Name (id)`, for the message.
        labels: Vec<String>,
        /// The role flags that selected the experimental tool(s), e.g.
        /// `["--formal-methods", "blaster"]`, so the example is copy-pasteable.
        example_flags: Vec<String>,
    },

    #[error("tool '{tool_id}' does not support fullstack (no [fullstack] template)")]
    #[diagnostic(
        code(cardano_init::fullstack_unsupported),
        help("fullstack-capable tools: {}", valid_tools.join(", "))
    )]
    FullstackUnsupported {
        tool_id: String,
        valid_tools: Vec<String>,
    },

    #[error("--fullstack cannot be combined with --on-chain or --off-chain")]
    #[diagnostic(
        code(cardano_init::fullstack_conflict),
        help("--fullstack X already fills both roles; use it alone.")
    )]
    FullstackConflict,

    // Boxed to keep `CliError` small.
    #[error(transparent)]
    #[diagnostic(transparent)]
    IncompatibleTools(Box<IncompatibleToolsError>),

    #[error("invalid project name '{name}' — {reason}")]
    #[diagnostic(code(cardano_init::invalid_project_name))]
    InvalidProjectName { name: String, reason: String },

    #[error("--name is required when using one-shot flags (--on-chain, --off-chain, etc.)")]
    #[diagnostic(
        code(cardano_init::name_required),
        help(
            "run without flags for interactive mode, or provide --name:\n\n    cardano-init --name my-protocol --on-chain aiken"
        )
    )]
    NameRequired,

    #[error(
        "this directory doesn't look like a cardano-init project — unrecognized: {}",
        .dirs.join(", ")
    )]
    #[diagnostic(
        code(cardano_init::project_unrecognized),
        help(
            "run add/remove inside a generated project; fix or rename any renamed/foreign component directories first."
        )
    )]
    ProjectUnrecognized { dirs: Vec<String> },

    #[error("can't add the {role} component: '{dir}/' already exists and isn't recognized")]
    #[diagnostic(
        code(cardano_init::slot_occupied),
        help("remove or rename '{dir}/' first, then re-run.")
    )]
    SlotOccupied { role: String, dir: String },

    #[error("the working tree at '{path}' has uncommitted changes")]
    #[diagnostic(
        code(cardano_init::worktree_dirty),
        help(
            "commit or stash first so the update is reviewable with `git diff`, or re-run with --force to update anyway."
        )
    )]
    WorktreeDirty { path: String },

    #[error("nothing to change — the project already matches this selection")]
    #[diagnostic(
        code(cardano_init::nothing_to_change),
        help(
            "pass a different tool/role, or use `cardano-init add`/`remove` to change the stack."
        )
    )]
    NothingToChange,

    #[error("user aborted")]
    #[diagnostic(code(cardano_init::aborted))]
    Aborted,

    #[error("prompt error: {0}")]
    #[diagnostic(code(cardano_init::prompt_error))]
    Prompt(#[from] dialoguer::Error),
}

/// Payload for [`CliError::IncompatibleTools`]: an off-chain ↔ provider pairing
/// that can't talk, with both the human message parts and the JSON context.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{off_chain} can't be paired with {providers} — {reason}")]
#[diagnostic(code(cardano_init::incompatible_tools))]
pub struct IncompatibleToolsError {
    /// Human label `Name (id)` for the off-chain tool.
    pub off_chain: String,
    /// Human labels `Name (id)` for the offending provider(s), comma-joined.
    pub providers: String,
    /// Why the pair can't talk (seam mismatch / self-hosting).
    pub reason: String,
    /// Multi-line remedy, surfaced as the `help:` line.
    #[help]
    pub help: String,
    /// Machine-facing ids `[off_chain_id, provider_ids…]` (JSON context).
    pub ids: Vec<String>,
    /// Provider ids compatible with the chosen off-chain tool (JSON context).
    pub compatible_providers: Vec<String>,
    /// Off-chain ids compatible with the chosen providers (JSON context).
    pub compatible_off_chain: Vec<String>,
}

impl From<Incompatibility> for CliError {
    /// Pre-formats the "which tools do fit" remedy; a self-hosting off-chain
    /// tool with no compatible provider is told to omit the provider instead.
    fn from(inc: Incompatibility) -> Self {
        let providers = inc
            .providers
            .iter()
            .map(|p| format!("{} ({})", p.name, p.id))
            .collect::<Vec<_>>()
            .join(", ");

        // miette owns the `help:` indentation, so these lines carry none.
        let providers_line = if !inc.compatible_providers.is_empty() {
            format!(
                "Providers that work with {}: {}",
                inc.off_chain_name,
                inc.compatible_providers.join(", ")
            )
        } else if inc.self_hosted {
            format!(
                "{} provides its own devnet — omit the provider selection.",
                inc.off_chain_name
            )
        } else {
            format!(
                "No bundled provider serves {} — point it at a public provider (see its README).",
                inc.off_chain_name
            )
        };
        let off_chain_line = if inc.compatible_off_chain.is_empty() {
            String::new()
        } else {
            format!(
                "\nOff-chain tools that work with {providers}: {}",
                inc.compatible_off_chain.join(", ")
            )
        };

        let ids = std::iter::once(inc.off_chain_id.clone())
            .chain(inc.providers.iter().map(|p| p.id.clone()))
            .collect();

        CliError::IncompatibleTools(Box::new(IncompatibleToolsError {
            off_chain: format!("{} ({})", inc.off_chain_name, inc.off_chain_id),
            providers,
            reason: inc.reason,
            help: format!(
                "{providers_line}{off_chain_line}\n\nTo scaffold this combination anyway, re-run with --ignore-warning."
            ),
            ids,
            compatible_providers: inc.compatible_providers,
            compatible_off_chain: inc.compatible_off_chain,
        }))
    }
}

impl CliError {
    /// Process exit code (tech spec §2.3): `2` for usage/validation errors,
    /// `1` for runtime errors, `0` for an interactive abort (a user choice).
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::UnknownTool { .. }
            | CliError::ToolRoleMismatch { .. }
            | CliError::NoRolesSelected
            | CliError::ExperimentalNotAllowed { .. }
            | CliError::FullstackUnsupported { .. }
            | CliError::FullstackConflict
            | CliError::IncompatibleTools(_)
            | CliError::InvalidProjectName { .. }
            | CliError::NameRequired
            | CliError::ProjectUnrecognized { .. }
            | CliError::SlotOccupied { .. }
            | CliError::NothingToChange => 2,

            CliError::Aborted => 0,

            CliError::Registry(_)
            | CliError::Scaffold(_)
            | CliError::Catalog(_)
            | CliError::DirectoryExists { .. }
            | CliError::WorktreeDirty { .. }
            | CliError::Prompt(_) => 1,
        }
    }

    /// Stable, machine-readable error code (tech spec §2.5). Part of the JSON
    /// contract; never changes for a given error kind.
    pub fn code(&self) -> &'static str {
        match self {
            CliError::Registry(_) | CliError::Catalog(_) => "registry_load",
            CliError::Scaffold(_) => "scaffold_error",
            CliError::DirectoryExists { .. } => "dir_exists",
            CliError::UnknownTool { .. } => "unknown_tool",
            CliError::ToolRoleMismatch { .. } => "tool_role_mismatch",
            CliError::NoRolesSelected => "no_roles_selected",
            CliError::ExperimentalNotAllowed { .. } => "experimental_not_allowed",
            CliError::FullstackUnsupported { .. } => "fullstack_unsupported",
            CliError::FullstackConflict => "fullstack_conflict",
            CliError::IncompatibleTools(_) => "incompatible_tools",
            CliError::InvalidProjectName { .. } => "invalid_project_name",
            CliError::NameRequired => "name_required",
            CliError::ProjectUnrecognized { .. } => "project_unrecognized",
            CliError::SlotOccupied { .. } => "slot_occupied",
            CliError::WorktreeDirty { .. } => "worktree_dirty",
            CliError::NothingToChange => "nothing_to_change",
            CliError::Aborted => "aborted",
            CliError::Prompt(_) => "prompt_error",
        }
    }

    /// Structured, agent-facing context: the offending input plus valid
    /// alternatives where applicable (tech spec §2.5, PRD FR-15).
    pub fn context(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            // These variants display their inner error verbatim.
            CliError::Registry(_) | CliError::Catalog(_) | CliError::Scaffold(_) => {
                json!({ "detail": self.to_string() })
            }
            CliError::DirectoryExists { path } | CliError::WorktreeDirty { path } => {
                json!({ "path": path })
            }
            CliError::UnknownTool {
                tool_id,
                role,
                valid_tools,
            } => json!({ "tool_id": tool_id, "role": role, "valid_tools": valid_tools }),
            CliError::ToolRoleMismatch {
                tool_id,
                role,
                valid_roles,
            } => json!({ "tool_id": tool_id, "role": role, "valid_roles": valid_roles }),
            CliError::FullstackUnsupported {
                tool_id,
                valid_tools,
            } => json!({ "tool_id": tool_id, "valid_tools": valid_tools }),
            CliError::InvalidProjectName { name, reason } => {
                json!({ "name": name, "reason": reason })
            }
            CliError::ExperimentalNotAllowed { tools, .. } => {
                json!({ "tools": tools, "remedy": "--allow-experimental" })
            }
            CliError::IncompatibleTools(e) => json!({
                "tools": e.ids,
                "reason": e.reason,
                "compatible_providers": e.compatible_providers,
                "compatible_off_chain": e.compatible_off_chain,
                "remedy": "--ignore-warning",
            }),
            CliError::ProjectUnrecognized { dirs } => json!({ "dirs": dirs }),
            CliError::SlotOccupied { role, dir } => json!({ "role": role, "dir": dir }),
            CliError::NoRolesSelected
            | CliError::FullstackConflict
            | CliError::NameRequired
            | CliError::NothingToChange
            | CliError::Aborted
            | CliError::Prompt(_) => json!({}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_code_mirrors_inherent_code() {
        let err = CliError::NameRequired;
        let diag = miette::Diagnostic::code(&err)
            .map(|c| c.to_string())
            .expect("variant declares a diagnostic code");
        assert_eq!(diag, format!("cardano_init::{}", err.code()));
    }
}
