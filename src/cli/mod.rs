pub mod interactive;
pub mod oneshot;
pub mod output;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::registry::loader::{Registry, RegistryError};
use crate::registry::types::ToolDef;
use crate::scaffold::ScaffoldError;

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

/// Output format. `json` implies non-interactive: it never prompts; if required
/// input is missing it errors instead (TECH_SPEC §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Scaffold a new Cardano protocol project.
#[derive(Parser, Debug)]
#[command(name = "cardano-init", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Output format (global): human-readable text or machine-readable JSON
    #[arg(long, value_enum, global = true, default_value = "human")]
    pub format: Format,

    #[command(flatten)]
    pub init: InitArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Launch the web-based project builder on localhost
    Web {
        /// Port to listen on
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },

    /// Check that the dependencies this project needs are installed, and
    /// advise how to install any that are missing
    Doctor,

    /// List the available roles and tools (use --format json for agents)
    List,
}

/// Arguments for the default init mode (interactive or one-shot).
#[derive(clap::Args, Debug)]
pub struct InitArgs {
    /// Project name (required in one-shot mode)
    #[arg(long)]
    pub name: Option<String>,

    /// On-chain tool (e.g., aiken, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (e.g., meshjs, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Infrastructure tool (repeatable: --infra kupo --infra ogmios)
    #[arg(long, value_name = "TOOL_ID")]
    pub infra: Vec<String>,

    /// Devnet tool (e.g., yaci)
    #[arg(long, value_name = "TOOL_ID")]
    pub devnet: Option<String>,

    /// Formal methods tool (e.g., blaster)
    #[arg(long, value_name = "TOOL_ID")]
    pub formal_methods: Option<String>,

    /// Target network
    #[arg(long, default_value = "preview")]
    pub network: String,

    /// Generate Nix flake for dependency management
    #[arg(long)]
    pub nix: bool,

    /// Show what would be generated without writing to disk
    #[arg(long)]
    pub dry_run: bool,
}

impl InitArgs {
    /// Returns true if any one-shot flags were provided.
    fn has_oneshot_flags(&self) -> bool {
        self.on_chain.is_some()
            || self.off_chain.is_some()
            || !self.infra.is_empty()
            || self.devnet.is_some()
            || self.formal_methods.is_some()
            || self.nix
            || self.dry_run
            || self.network != "preview"
    }
}

// ---------------------------------------------------------------------------
// CLI errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Registry(#[from] RegistryError),

    #[error("{0}")]
    Scaffold(#[from] ScaffoldError),

    #[error("{0}")]
    Web(#[from] crate::web::WebError),

    #[error("{0}")]
    Catalog(#[from] crate::doctor::catalog::CatalogError),

    #[error("directory '{}' already exists — refusing to overwrite", path)]
    DirectoryExists { path: String },

    #[error("unknown tool '{}' for role {}", tool_id, role)]
    UnknownTool {
        tool_id: String,
        role: String,
        valid_tools: Vec<String>,
    },

    #[error("tool '{}' does not support role '{}'", tool_id, role)]
    ToolRoleMismatch {
        tool_id: String,
        role: String,
        valid_roles: Vec<String>,
    },

    #[error("no roles selected — at least one role must be provided")]
    NoRolesSelected,

    #[error("invalid network '{}' — expected preview, preprod, or mainnet", value)]
    InvalidNetwork { value: String },

    #[error("invalid project name '{}' — {}", name, reason)]
    InvalidProjectName { name: String, reason: String },

    #[error(
        "--name is required when using one-shot flags (--on-chain, --off-chain, etc.)\n\n  Run without flags for interactive mode, or provide --name:\n\n    cardano-init --name my-protocol --on-chain aiken"
    )]
    NameRequired,

    #[error("user aborted")]
    Aborted,

    #[error("prompt error: {0}")]
    Prompt(#[from] dialoguer::Error),
}

impl CliError {
    /// Process exit-code category (TECH_SPEC §2.3):
    /// - `2` — usage / validation errors (bad or missing input);
    /// - `1` — runtime errors (I/O, registry/render failure, web bind, …);
    /// - `0` — interactive abort by user choice (not an error).
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::UnknownTool { .. }
            | CliError::ToolRoleMismatch { .. }
            | CliError::NoRolesSelected
            | CliError::InvalidNetwork { .. }
            | CliError::InvalidProjectName { .. }
            | CliError::NameRequired => 2,

            CliError::Aborted => 0,

            CliError::Registry(_)
            | CliError::Scaffold(_)
            | CliError::Web(_)
            | CliError::Catalog(_)
            | CliError::DirectoryExists { .. }
            | CliError::Prompt(_) => 1,
        }
    }

    /// Stable, machine-readable error code (TECH_SPEC §2.5). Part of the JSON
    /// contract; never changes for a given error kind.
    pub fn code(&self) -> &'static str {
        match self {
            CliError::Registry(_) | CliError::Catalog(_) => "registry_load",
            CliError::Scaffold(_) => "scaffold_error",
            CliError::Web(_) => "web_bind",
            CliError::DirectoryExists { .. } => "dir_exists",
            CliError::UnknownTool { .. } => "unknown_tool",
            CliError::ToolRoleMismatch { .. } => "tool_role_mismatch",
            CliError::NoRolesSelected => "no_roles_selected",
            CliError::InvalidNetwork { .. } => "invalid_network",
            CliError::InvalidProjectName { .. } => "invalid_project_name",
            CliError::NameRequired => "name_required",
            CliError::Aborted => "aborted",
            CliError::Prompt(_) => "prompt_error",
        }
    }

    /// Structured, agent-facing context: the offending input plus valid
    /// alternatives where applicable (TECH_SPEC §2.5, PRD FR-15).
    pub fn context(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            CliError::Registry(e) => json!({ "detail": e.to_string() }),
            CliError::Catalog(e) => json!({ "detail": e.to_string() }),
            CliError::Scaffold(e) => json!({ "detail": e.to_string() }),
            CliError::Web(crate::web::WebError::Bind { port, source }) => {
                json!({ "port": port, "detail": source.to_string() })
            }
            CliError::DirectoryExists { path } => json!({ "path": path }),
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
            CliError::InvalidNetwork { value } => {
                json!({ "value": value, "expected": ["preview", "preprod", "mainnet"] })
            }
            CliError::InvalidProjectName { name, reason } => {
                json!({ "name": name, "reason": reason })
            }
            CliError::NoRolesSelected
            | CliError::NameRequired
            | CliError::Aborted
            | CliError::Prompt(_) => json!({}),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool catalog for --help
// ---------------------------------------------------------------------------

/// Build the "Available tools" section appended to --help output.
fn build_tool_catalog(registry: &Registry) -> String {
    use std::fmt::Write;

    let mut out = String::from("Available tools:\n");

    for tool in registry.all_tools() {
        out.push('\n');
        format_tool(&mut out, tool);
    }

    let _ = writeln!(out, "\nExamples:");
    let _ = writeln!(
        out,
        "  cardano-init                                        # interactive mode"
    );
    let _ = writeln!(
        out,
        "  cardano-init --name my-app --on-chain aiken         # one-shot, single role"
    );
    let _ = writeln!(
        out,
        "  cardano-init --name my-app --on-chain aiken --off-chain meshjs --nix"
    );
    let _ = writeln!(
        out,
        "  cardano-init web                                    # web-based builder"
    );

    out
}

pub(super) fn format_tool(out: &mut String, tool: &ToolDef) {
    use std::fmt::Write;

    let mut roles: Vec<&str> = tool.roles.keys().map(|r| r.as_kebab()).collect();
    roles.sort();
    let _ = writeln!(out, "  {} ({})", tool.name, tool.id);
    let _ = writeln!(out, "    Roles:     {}", roles.join(", "));
    let _ = writeln!(out, "    Languages: {}", tool.languages.join(", "));
    let _ = writeln!(out, "    Website:   {}", tool.website);

    // Wrap description to ~72 chars with 4-space indent
    let _ = write!(out, "    ");
    let mut col = 4;
    for word in tool.description.split_whitespace() {
        if col + word.len() + 1 > 76 && col > 4 {
            let _ = write!(out, "\n    ");
            col = 4;
        }
        if col > 4 {
            let _ = write!(out, " ");
            col += 1;
        }
        let _ = write!(out, "{word}");
        col += word.len();
    }
    let _ = writeln!(out);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Main CLI entry point. Parse args, dispatch, and present the result (or a
/// machine-readable error). Returns the process exit code.
pub fn run() -> i32 {
    let registry = match Registry::load() {
        Ok(r) => r,
        Err(e) => {
            // Registry load happens before we know the requested format; this
            // is a packaging bug (embedded data), so default to human output.
            let err = CliError::from(e);
            output::print_error(&err, Format::Human);
            return err.exit_code();
        }
    };

    // Build clap command with dynamic after_help containing tool catalog
    let catalog = build_tool_catalog(&registry);
    let cmd = Cli::command().after_help(catalog);
    let matches = cmd.get_matches();
    let cli = Cli::from_arg_matches(&matches).expect("clap already validated");
    let format = cli.format;

    let result = match cli.command {
        Some(Command::Web { port }) => crate::web::serve(&registry, port).map_err(CliError::from),
        Some(Command::Doctor) => run_doctor(&registry, format),
        Some(Command::List) => {
            output::print_list(&registry, format);
            Ok(())
        }
        None => run_init(cli.init, &registry, format),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            output::print_error(&e, format);
            e.exit_code()
        }
    }
}

/// The required dependencies for a set of tool ids: the base dep `just`, plus
/// the `system_deps` of each tool (deduped/sorted later by the resolver).
fn required_deps<'a>(tool_ids: impl Iterator<Item = &'a str>, registry: &Registry) -> Vec<String> {
    let mut deps = vec![crate::doctor::BASE_DEP.to_string()];
    for id in tool_ids {
        if let Some(tool) = registry.get(id) {
            deps.extend(tool.system_deps.iter().cloned());
        }
    }
    deps
}

/// Run the standalone `doctor`: scan the current directory for generated
/// components, then report dependency status + install plans.
fn run_doctor(registry: &Registry, format: Format) -> Result<(), CliError> {
    use crate::doctor::{self, catalog::DepCatalog, probe};

    let catalog = DepCatalog::load()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let scan = probe::scan_project(&cwd, registry);

    let required = required_deps(scan.components.iter().map(|c| c.tool_id.as_str()), registry);
    let env = probe::detect_environment(&catalog);
    let report = doctor::resolve_all(&required, &catalog, &env);

    output::print_doctor(&scan, &report, &env, registry, format);
    Ok(())
}

/// Run the default init mode (interactive or one-shot).
fn run_init(args: InitArgs, registry: &Registry, format: Format) -> Result<(), CliError> {
    // `--name` is required for one-shot flags, and always in JSON mode (which
    // is non-interactive and must never prompt — TECH_SPEC §2.1).
    if args.name.is_none() && (args.has_oneshot_flags() || format == Format::Json) {
        return Err(CliError::NameRequired);
    }

    // Decide mode: one-shot if --name provided, interactive otherwise
    let selection = if let Some(ref name) = args.name {
        oneshot::build_selection(
            name,
            args.on_chain.as_deref(),
            args.off_chain.as_deref(),
            &args.infra,
            args.devnet.as_deref(),
            args.formal_methods.as_deref(),
            &args.network,
            args.nix,
            registry,
        )?
    } else {
        interactive::run_interactive(registry)?
    };

    let root = PathBuf::from(&selection.project_name);

    // Safety: refuse to write into an existing, non-empty directory (never
    // overwrite user files). A missing or empty target dir is fine (§6.4).
    if root.exists()
        && std::fs::read_dir(&root)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(CliError::DirectoryExists {
            path: selection.project_name.clone(),
        });
    }

    if args.dry_run {
        let plan = crate::scaffold::dry_run(&selection, registry)?;
        output::print_dry_run(&selection, registry, &plan, format);
        return Ok(());
    }

    if format == Format::Human {
        output::print_summary(&selection, registry);
    }
    crate::scaffold::scaffold(&selection, registry, &root)?;

    // Check-and-advise: resolve the deps this selection needs (TECH_SPEC §9).
    let report = resolve_selection_deps(&selection, registry)?;
    output::print_success(&selection, &report, format);

    Ok(())
}

/// Resolve the dependency report for a generated selection (check-and-advise).
fn resolve_selection_deps(
    selection: &crate::registry::types::Selection,
    registry: &Registry,
) -> Result<crate::doctor::Report, CliError> {
    use crate::doctor::{self, catalog::DepCatalog, probe};

    let catalog = DepCatalog::load()?;
    let required = required_deps(
        selection.assignments.iter().map(|a| a.tool_id.as_str()),
        registry,
    );
    let env = probe::detect_environment(&catalog);
    Ok(doctor::resolve_all(&required, &catalog, &env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tool_error_code_and_context() {
        let registry = Registry::load().unwrap();
        // `bogus` is not a tool; one-shot validation should surface it with the
        // stable code + the valid alternatives for the role.
        let err = oneshot::build_selection(
            "demo",
            Some("bogus"),
            None,
            &[],
            None,
            None,
            "preview",
            false,
            &registry,
        )
        .unwrap_err();

        assert_eq!(err.code(), "unknown_tool");
        let ctx = err.context();
        assert_eq!(ctx["tool_id"], "bogus");
        assert_eq!(ctx["role"], "On-chain");
        // on-chain is fillable by aiken + scalus.
        let valid: Vec<&str> = ctx["valid_tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(valid, vec!["aiken", "scalus"]);
    }

    #[test]
    fn tool_role_mismatch_lists_valid_roles() {
        let registry = Registry::load().unwrap();
        // aiken is on-chain only; asking it to fill off-chain is a mismatch.
        let err = oneshot::build_selection(
            "demo",
            None,
            Some("aiken"),
            &[],
            None,
            None,
            "preview",
            false,
            &registry,
        )
        .unwrap_err();
        assert_eq!(err.code(), "tool_role_mismatch");
        let ctx = err.context();
        assert_eq!(ctx["valid_roles"], serde_json::json!(["on-chain"]));
    }

    #[test]
    fn invalid_network_context_lists_expected() {
        let registry = Registry::load().unwrap();
        let err = oneshot::build_selection(
            "demo",
            Some("aiken"),
            None,
            &[],
            None,
            None,
            "badnet",
            false,
            &registry,
        )
        .unwrap_err();
        assert_eq!(err.code(), "invalid_network");
        assert_eq!(
            err.context()["expected"],
            serde_json::json!(["preview", "preprod", "mainnet"])
        );
    }

    #[test]
    fn required_deps_unions_just_with_tool_deps() {
        let registry = Registry::load().unwrap();
        // aiken → ["aiken"], meshjs → ["node"]; plus the base dep "just".
        let deps = required_deps(["aiken", "meshjs"].into_iter(), &registry);
        assert!(deps.contains(&"just".to_string()));
        assert!(deps.contains(&"aiken".to_string()));
        assert!(deps.contains(&"node".to_string()));
    }
}
