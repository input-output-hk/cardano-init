//! The impure edge: argument parsing, command dispatch, prompts and output.
//! Orchestrates the pure core (`registry`, `scaffold`, `doctor`); holds no
//! generation logic of its own.

mod args;
mod error;
pub mod git;
pub mod interactive;
pub mod oneshot;
pub mod output;
pub mod theme;
pub mod update;
pub mod version_check;

use std::path::{Path, PathBuf};

use clap::{CommandFactory, FromArgMatches};

pub use args::{AddArgs, Cli, Command, InitArgs, RemoveArgs, UpdateFlags};
pub use error::CliError;

use crate::doctor::{self, catalog::DepCatalog, probe};
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection};

/// Appended to `--help`: the tool catalog lives in `list`, not here.
const HELP_FOOTER: &str = "\
Run 'cardano-init list' to see the tools available for each role.

Examples:
  cardano-init                                        # interactive mode
  cardano-init --name my-app --on-chain aiken         # one-shot, single role
  cardano-init --name my-app --on-chain aiken --off-chain meshjs --nix
  cardano-init -n my-app -p scalus -d yaci            # short flags
";

/// Main CLI entry point: parse args, dispatch, and present the result (or the
/// error). Returns the process exit code.
pub fn run() -> i32 {
    let registry = match Registry::load() {
        Ok(r) => r,
        // Loaded before `--json` is known; a failure here is a packaging bug
        // (embedded data), so report it as human output.
        Err(e) => return fail(&CliError::from(e), false),
    };

    let matches = Cli::command().after_help(HELP_FOOTER).get_matches();
    let cli = Cli::from_arg_matches(&matches).expect("clap already validated");
    let json = cli.json;

    if matches!(
        cli.command,
        None | Some(Command::Add(_) | Command::Remove(_))
    ) {
        version_check::start(json);
    }

    let result = match cli.command {
        Some(Command::Doctor) => run_doctor(&registry, json),
        Some(Command::List(args)) => DepCatalog::load()
            .map_err(CliError::from)
            .map(|catalog| output::print_list(&registry, &catalog, json, args.table)),
        Some(Command::Add(args)) => update::run_add(&args, &registry, json),
        Some(Command::Remove(args)) => update::run_remove(&args, &registry, json),
        None => run_init(&cli.init, &registry, json),
    };

    match result {
        Ok(()) => 0,
        Err(e) => fail(&e, json),
    }
}

/// Present `err` and return its exit code.
fn fail(err: &CliError, json: bool) -> i32 {
    output::print_error(err, json);
    err.exit_code()
}

/// The required dependencies for a set of tool ids: the base dep `just` plus
/// each tool's `system_deps` (the resolver dedups/sorts).
///
/// The scan reports the aggregated infra component under
/// [`doctor::INFRA_DRIVER_ID`]; it needs the union of every infra tool's deps.
fn required_deps<'a>(tool_ids: impl Iterator<Item = &'a str>, registry: &Registry) -> Vec<String> {
    let mut deps = vec![doctor::BASE_DEP.to_string()];
    for id in tool_ids {
        if id == doctor::INFRA_DRIVER_ID {
            for tool in registry.tools_for_role(Role::Infrastructure) {
                deps.extend(tool.system_deps.iter().cloned());
            }
        } else if let Some(tool) = registry.get(id) {
            deps.extend(tool.system_deps.iter().cloned());
        }
    }
    deps
}

/// Run `work` behind a transient spinner on stderr. Only for human, attended
/// output (a spinner would corrupt `--json` or piped streams), and only around
/// genuinely slow work.
fn with_spinner<T>(message: &str, json: bool, work: impl FnOnce() -> T) -> T {
    use indicatif::{ProgressBar, ProgressStyle};

    if json || !console::user_attended() {
        return work();
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .expect("static spinner template is valid"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    let result = work();
    pb.finish_and_clear();
    result
}

/// `doctor`: scan the current directory for generated components, then report
/// dependency status and install plans.
fn run_doctor(registry: &Registry, json: bool) -> Result<(), CliError> {
    let catalog = DepCatalog::load()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let scan = probe::scan_project(&cwd, registry);

    let required = required_deps(scan.components.iter().map(|c| c.tool_id.as_str()), registry);
    let (env, report, tooling) = with_spinner("Probing environment…", json, || {
        let env = probe::detect_environment(&catalog);
        let report = doctor::resolve_all(&required, &catalog, &env);
        let tooling = if scan.components.is_empty() && scan.unrecognized.is_empty() {
            doctor::present_tooling(&catalog, registry, &env)
        } else {
            report
                .deps
                .iter()
                .filter(|d| d.present)
                .map(|d| d.id.clone())
                .collect()
        };
        (env, report, tooling)
    });

    output::print_doctor(&scan, &report, &env, &tooling, registry, json);
    Ok(())
}

/// Fail if `selection` includes an experimental tool. Callers skip it when
/// `--allow-experimental` was given.
fn experimental_gate(selection: &Selection, registry: &Registry) -> Result<(), CliError> {
    let experimental = output::selected_experimental_tools(selection, registry);
    if experimental.is_empty() {
        return Ok(());
    }
    let tools: Vec<String> = experimental.iter().map(|t| t.id.clone()).collect();
    let labels = experimental
        .iter()
        .map(|t| format!("{} ({})", t.name, t.id))
        .collect();
    let example_flags = selection
        .assignments
        .iter()
        .filter(|a| tools.contains(&a.tool_id))
        .flat_map(|a| [role_flag(a.role).to_string(), a.tool_id.clone()])
        .collect();
    Err(CliError::ExperimentalNotAllowed {
        tools,
        labels,
        example_flags,
    })
}

/// Off-chain ↔ provider compatibility gate. With `--ignore-warning` an
/// incompatibility is only a warning (printed for human output).
fn compat_gate(
    selection: &Selection,
    registry: &Registry,
    ignore_warning: bool,
    json: bool,
) -> Result<(), CliError> {
    let Some(inc) = crate::registry::compat::check(&selection.assignments, registry) else {
        return Ok(());
    };
    if !ignore_warning {
        return Err(inc.into());
    }
    if !json {
        output::print_incompatibility_warning(&inc);
    }
    Ok(())
}

/// The one-shot flag that assigns a tool to `role`, e.g. `--on-chain`.
fn role_flag(role: Role) -> &'static str {
    match role {
        Role::OnChain => "--on-chain",
        Role::OffChain => "--off-chain",
        Role::Infrastructure => "--infra",
        Role::Devnet => "--devnet",
        Role::FormalMethods => "--formal-methods",
    }
}

/// The default command: build a selection (one-shot when `--name` is given,
/// interactive otherwise), gate it, then scaffold (or preview) the project.
fn run_init(args: &InitArgs, registry: &Registry, json: bool) -> Result<(), CliError> {
    // JSON mode is non-interactive and must never prompt (tech spec §2.1).
    if args.name.is_none() && (args.has_oneshot_flags() || json) {
        return Err(CliError::NameRequired);
    }
    let roles = args.role_flags();
    if roles.has_fullstack_conflict() {
        return Err(CliError::FullstackConflict);
    }

    let selection = if let Some(name) = &args.name {
        let selection = oneshot::build_selection(name, &roles, args.nix, !args.no_spec, registry)?;
        // Interactive mode asks instead; non-interactive can only refuse.
        if !args.allow_experimental {
            experimental_gate(&selection, registry)?;
        }
        selection
    } else {
        interactive::run_interactive(registry, args.allow_experimental, args.ignore_warning)?
    };

    compat_gate(&selection, registry, args.ignore_warning, json)?;

    let root = PathBuf::from(&selection.project_name);
    // Never write into a non-empty directory (an empty one is fine, §6.4).
    if root.exists() && !is_empty_dir(&root) {
        return Err(CliError::DirectoryExists {
            path: selection.project_name.clone(),
        });
    }

    version_check::announce(json);

    if args.dry_run {
        let plan = crate::scaffold::dry_run(&selection, registry)?;
        output::print_dry_run(&selection, registry, &plan, json);
        return Ok(());
    }

    with_spinner("Scaffolding project…", json, || {
        crate::scaffold::scaffold(&selection, registry, &root)
    })?;

    // Best-effort: gives `add`/`remove` a clean tree to diff against.
    let git = git::init_project_repo(&root);

    let report = with_spinner("Checking dependencies…", json, || {
        resolve_selection_deps(&selection, registry)
    })?;
    output::print_success(&selection, registry, &report, git, json);
    Ok(())
}

/// Whether `dir` is a readable directory with no entries.
fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

/// Resolve the dependency report for a generated selection (check-and-advise,
/// tech spec §9).
fn resolve_selection_deps(
    selection: &Selection,
    registry: &Registry,
) -> Result<doctor::Report, CliError> {
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
    use crate::cli::oneshot::RoleFlags;

    fn string_array(value: &serde_json::Value) -> Vec<&str> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect()
    }

    #[test]
    fn unknown_tool_error_code_and_context() {
        let registry = Registry::load().unwrap();
        let roles = RoleFlags {
            on_chain: Some("bogus"),
            ..RoleFlags::default()
        };
        let err = oneshot::build_selection("demo", &roles, false, true, &registry).unwrap_err();

        assert_eq!(err.code(), "unknown_tool");
        let ctx = err.context();
        assert_eq!(ctx["tool_id"], "bogus");
        assert_eq!(ctx["role"], "On-chain");
        assert_eq!(
            string_array(&ctx["valid_tools"]),
            vec!["aiken", "plinth", "scalus"]
        );
    }

    #[test]
    fn tool_role_mismatch_lists_valid_roles() {
        let registry = Registry::load().unwrap();
        // aiken is on-chain only.
        let roles = RoleFlags {
            off_chain: Some("aiken"),
            ..RoleFlags::default()
        };
        let err = oneshot::build_selection("demo", &roles, false, true, &registry).unwrap_err();
        assert_eq!(err.code(), "tool_role_mismatch");
        assert_eq!(
            err.context()["valid_roles"],
            serde_json::json!(["on-chain"])
        );
    }

    #[test]
    fn fullstack_conflict_with_explicit_role_flag() {
        let registry = Registry::load().unwrap();
        let args = InitArgs {
            name: Some("demo".to_string()),
            on_chain: Some("aiken".to_string()),
            fullstack: Some("scalus".to_string()),
            dry_run: true,
            ..InitArgs::default()
        };
        let err = run_init(&args, &registry, true).unwrap_err();
        assert_eq!(err.code(), "fullstack_conflict");
        assert_eq!(err.exit_code(), 2);
    }

    /// One-shot dry-run `InitArgs` for the experimental-gate tests.
    fn init_args(formal_methods: Option<&str>, allow_experimental: bool) -> InitArgs {
        InitArgs {
            name: Some("exp-gate-demo".to_string()),
            on_chain: Some("aiken".to_string()),
            formal_methods: formal_methods.map(str::to_string),
            allow_experimental,
            dry_run: true,
            ..InitArgs::default()
        }
    }

    #[test]
    fn experimental_tool_gated_without_flag() {
        let registry = Registry::load().unwrap();
        let err = run_init(&init_args(Some("blaster"), false), &registry, true).unwrap_err();
        assert_eq!(err.code(), "experimental_not_allowed");
        assert_eq!(err.exit_code(), 2);
        let ctx = err.context();
        assert_eq!(string_array(&ctx["tools"]), vec!["blaster"]);
        assert_eq!(ctx["remedy"], "--allow-experimental");

        // The message names the tool; the `help:` line is a copy-pasteable
        // command with the exact role flag.
        let msg = err.to_string();
        assert!(msg.contains("Blaster (blaster)"), "message: {msg}");
        let help = miette::Diagnostic::help(&err)
            .map(|h| h.to_string())
            .unwrap_or_default();
        assert!(
            help.contains("--formal-methods blaster --allow-experimental"),
            "help: {help}"
        );
    }

    #[test]
    fn experimental_tool_allowed_with_flag() {
        let registry = Registry::load().unwrap();
        let res = run_init(&init_args(Some("blaster"), true), &registry, true);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    #[test]
    fn non_experimental_selection_needs_no_flag() {
        let registry = Registry::load().unwrap();
        let res = run_init(&init_args(None, false), &registry, true);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    /// One-shot dry-run `InitArgs` for tx3 (experimental, so allowed) + `devnet`.
    fn tx3_devnet_args(devnet: &str, ignore_warning: bool) -> InitArgs {
        InitArgs {
            name: Some("compat-demo".to_string()),
            off_chain: Some("tx3".to_string()),
            devnet: Some(devnet.to_string()),
            allow_experimental: true,
            dry_run: true,
            ignore_warning,
            ..InitArgs::default()
        }
    }

    #[test]
    fn incompatible_offchain_devnet_is_gated() {
        // tx3 (TRP, self-hosted devnet) + yaci (Blockfrost) don't share a seam.
        let registry = Registry::load().unwrap();
        let err = run_init(&tx3_devnet_args("yaci", false), &registry, true).unwrap_err();
        assert_eq!(err.code(), "incompatible_tools");
        assert_eq!(err.exit_code(), 2);
        let ctx = err.context();
        assert_eq!(ctx["remedy"], "--ignore-warning");
        assert_eq!(string_array(&ctx["tools"]), vec!["tx3", "yaci"]);
    }

    #[test]
    fn incompatible_offchain_devnet_allowed_with_ignore_warning() {
        let registry = Registry::load().unwrap();
        let res = run_init(&tx3_devnet_args("yaci", true), &registry, true);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
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

    #[test]
    fn required_deps_resolves_infra_driver_to_union() {
        let registry = Registry::load().unwrap();
        let deps = required_deps([doctor::INFRA_DRIVER_ID].into_iter(), &registry);
        assert!(deps.contains(&"just".to_string()));
        assert!(deps.contains(&"docker".to_string()));
        assert!(deps.contains(&"cardano-up".to_string()));
    }
}
