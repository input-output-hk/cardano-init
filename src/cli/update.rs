//! The `add` / `remove` commands: edit an existing project's role/tool
//! set in place. Impure edge — reconstructs the current selection from the tree
//! (via `doctor::probe`), mutates it, then diffs and writes with `scaffold`.
//!
//! Reuses the init gates verbatim (compat + experimental) so an update is held
//! to exactly the same rules as a fresh scaffold.

use std::path::{Path, PathBuf};

use super::oneshot::{validate_fullstack_tool, validate_tool_for_role};
use super::{AddArgs, CliError, Format, RemoveArgs, UpdateFlags, output, theme};
use crate::doctor::probe;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, RoleAssignment, Selection};
use crate::scaffold::update::{self, Mutation, SlotOp};

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

/// `cardano-init add …` — assign/replace tools in the current project.
pub fn run_add(args: AddArgs, registry: &Registry, format: Format) -> Result<(), CliError> {
    // `--fullstack X` is sugar for on-chain + off-chain X; not combinable.
    if args.fullstack.is_some() && (args.on_chain.is_some() || args.off_chain.is_some()) {
        return Err(CliError::FullstackConflict);
    }

    let mut mutations = Vec::new();
    if let Some(id) = &args.fullstack {
        validate_fullstack_tool(id, registry)?;
        mutations.push(add(Role::OnChain, id));
        mutations.push(add(Role::OffChain, id));
    }
    if let Some(id) = &args.on_chain {
        validate_tool_for_role(id, Role::OnChain, registry)?;
        mutations.push(add(Role::OnChain, id));
    }
    if let Some(id) = &args.off_chain {
        validate_tool_for_role(id, Role::OffChain, registry)?;
        mutations.push(add(Role::OffChain, id));
    }
    for id in &args.infra {
        validate_tool_for_role(id, Role::Infrastructure, registry)?;
        mutations.push(add(Role::Infrastructure, id));
    }
    if let Some(id) = &args.devnet {
        validate_tool_for_role(id, Role::Devnet, registry)?;
        mutations.push(add(Role::Devnet, id));
    }
    if let Some(id) = &args.formal_methods {
        validate_tool_for_role(id, Role::FormalMethods, registry)?;
        mutations.push(add(Role::FormalMethods, id));
    }

    let cwd = current_dir();
    let old = detect_selection(&cwd, registry, format)?;
    let new = update::apply_all(&old, &mutations);
    finish_update(&cwd, old, new, &args.flags, registry, format)
}

/// `cardano-init remove …` — drop roles / infra providers from the project.
pub fn run_remove(args: RemoveArgs, registry: &Registry, format: Format) -> Result<(), CliError> {
    let mut mutations = Vec::new();
    if args.on_chain {
        mutations.push(Mutation::Remove(Role::OnChain));
    }
    if args.off_chain {
        mutations.push(Mutation::Remove(Role::OffChain));
    }
    if args.devnet {
        mutations.push(Mutation::Remove(Role::Devnet));
    }
    if args.formal_methods {
        mutations.push(Mutation::Remove(Role::FormalMethods));
    }
    for id in &args.infra {
        mutations.push(Mutation::RemoveInfra(id.clone()));
    }

    let cwd = current_dir();
    let old = detect_selection(&cwd, registry, format)?;
    let new = update::apply_all(&old, &mutations);
    finish_update(&cwd, old, new, &args.flags, registry, format)
}

// ---------------------------------------------------------------------------
// Shared flow
// ---------------------------------------------------------------------------

fn add(role: Role, tool_id: &str) -> Mutation {
    Mutation::Add(RoleAssignment {
        role,
        tool_id: tool_id.to_string(),
    })
}

fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Reconstruct the current selection, refusing to guess when the tree has
/// unrecognized components (fatal in non-interactive/JSON; a confirm otherwise).
fn detect_selection(
    cwd: &Path,
    registry: &Registry,
    format: Format,
) -> Result<Selection, CliError> {
    let recon = probe::reconstruct(cwd, registry);

    let mut problems: Vec<String> = recon.unrecognized.iter().map(|u| u.dir.clone()).collect();
    problems.extend(recon.unknown_infra.iter().cloned());

    if !problems.is_empty() {
        // Automation must not act on a guess.
        if format == Format::Json {
            return Err(CliError::ProjectUnrecognized { dirs: problems });
        }
        println!(
            "  {}  these look off and will be ignored: {}",
            theme::badge_warn("HEADS UP"),
            problems.join(", ")
        );
        if !confirm("Continue with the detected stack anyway?", false)? {
            return Err(CliError::Aborted);
        }
    }

    if format == Format::Human && !recon.low_confidence.is_empty() {
        println!(
            "  {}  guessed (couldn't read from the project): {}",
            theme::dim("note:"),
            recon.low_confidence.join(", ")
        );
    }

    Ok(recon.selection)
}

/// Validate the mutated selection, then plan → (dry-run | git-gate + confirm +
/// apply) → report. Shared by add/remove.
fn finish_update(
    cwd: &Path,
    old: Selection,
    new: Selection,
    flags: &UpdateFlags,
    registry: &Registry,
    format: Format,
) -> Result<(), CliError> {
    if new == old {
        return Err(CliError::NothingToChange);
    }
    if new.assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    // Same gates a fresh scaffold runs, on the resulting selection.
    if !flags.allow_experimental {
        super::experimental_gate(&new, registry)?;
    }
    if let Some(inc) = crate::registry::compat::check(&new.assignments, registry) {
        if flags.ignore_warning {
            if format == Format::Human {
                output::print_incompatibility_warning(&inc);
            }
        } else {
            return Err(super::incompatible_tools_error(inc));
        }
    }

    let plan = update::plan_update(&old, &new, registry)?;

    // A create target must not already exist (a foreign/renamed dir would have
    // been flagged unrecognized; this is the last-line guard).
    for op in &plan.slot_ops {
        if let SlotOp::Create(dir) = op
            && cwd.join(dir).exists()
        {
            let name = dir.to_string_lossy().into_owned();
            return Err(CliError::SlotOccupied {
                role: name.clone(),
                dir: name,
            });
        }
    }

    if flags.dry_run {
        output::print_update_plan(&old, &new, &plan, registry, format);
        return Ok(());
    }

    // Git safety net: the change must be reviewable/revertible.
    let clean = super::git::is_clean(cwd);
    if !flags.force && !clean {
        return Err(CliError::WorktreeDirty {
            path: cwd.display().to_string(),
        });
    }

    // A clean tree makes the whole change reviewable and revertible with git, so
    // apply it straight away — no prompt (good agent DevX). Only pause to confirm
    // when `--force` is overriding a dirty tree, where the update can't be cleanly
    // undone; there we show the plan first, then ask.
    if format == Format::Human && !clean {
        output::print_update_plan(&old, &new, &plan, registry, format);
        if !confirm("Apply this update to a dirty working tree?", true)? {
            return Err(CliError::Aborted);
        }
    }

    crate::scaffold::writer::apply_update(&plan, cwd)?;

    let report = super::resolve_selection_deps(&new, registry)?;
    output::print_update_success(&old, &new, registry, &plan, &report, format);
    Ok(())
}

fn confirm(prompt: &str, default: bool) -> Result<bool, CliError> {
    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(CliError::from)
}

// ---------------------------------------------------------------------------
// Tests — the CLI decision logic (`finish_update` / `detect_selection`): the
// git-clean gate, the trust boundary, dry-run branching, and the slot/no-op
// guards. Exercised on the internal seams with an explicit `cwd` so nothing
// touches the process-global working directory (keeps them parallel-safe).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::registry::types::Network;

    fn reg() -> Registry {
        Registry::load().expect("registry loads")
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    fn sel(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "proj".into(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    /// Scaffold `s` into a fresh temp dir; return (guard, root).
    fn scaffolded(s: &Selection) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        crate::scaffold::scaffold(s, &reg(), &root).unwrap();
        (tmp, root)
    }

    fn git(root: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Turn `root` into a git repo with everything committed → a clean tree.
    /// Neutralizes host git config (signing, hooks, identity) so the commit is
    /// deterministic regardless of the developer/CI environment.
    fn commit_all(root: &Path) {
        assert!(git(root, &["init", "--quiet"]));
        assert!(git(root, &["config", "user.email", "test@example.com"]));
        assert!(git(root, &["config", "user.name", "Test"]));
        assert!(git(root, &["config", "commit.gpgsign", "false"]));
        assert!(git(root, &["config", "core.hooksPath", "/dev/null"]));
        assert!(git(root, &["add", "-A"]));
        assert!(git(
            root,
            &["commit", "--quiet", "--no-verify", "-m", "init"]
        ));
        // Sanity: the tree must actually be clean for the gate tests to mean
        // anything.
        assert!(super::super::git::is_clean(root));
    }

    fn add_offchain(old: &Selection) -> Selection {
        update::apply(old, &Mutation::Add(a(Role::OffChain, "meshjs")))
    }

    // ----- trust boundary: clean tree applies with no prompt -----

    #[test]
    fn clean_tree_applies_without_prompting() {
        if !git_available() {
            return;
        }
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);
        commit_all(&root); // clean

        // Human format + clean tree: the confirm is skipped, so this returns
        // instead of blocking on stdin. A regression that re-adds the prompt
        // would hang this test.
        let new = add_offchain(&old);
        finish_update(
            &root,
            old,
            new,
            &UpdateFlags::default(),
            &reg(),
            Format::Human,
        )
        .expect("clean-tree update applies");

        assert!(root.join("off-chain/package.json").is_file());
    }

    // ----- git safety net: dirty tree is refused without --force -----

    #[test]
    fn dirty_tree_refused_without_force() {
        if !git_available() {
            return;
        }
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);
        commit_all(&root);
        fs::write(root.join("on-chain/DIRTY.md"), b"uncommitted").unwrap();

        let new = add_offchain(&old);
        let err = finish_update(
            &root,
            old,
            new,
            &UpdateFlags::default(),
            &reg(),
            Format::Human,
        )
        .unwrap_err();

        assert!(matches!(err, CliError::WorktreeDirty { .. }));
        // Nothing was written past the gate.
        assert!(!root.join("off-chain").exists());
    }

    #[test]
    fn non_git_tree_refused_without_force() {
        // A non-repo counts as "not clean": refuse unless --force.
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);

        let new = add_offchain(&old);
        let err = finish_update(
            &root,
            old,
            new,
            &UpdateFlags::default(),
            &reg(),
            Format::Human,
        )
        .unwrap_err();

        assert!(matches!(err, CliError::WorktreeDirty { .. }));
    }

    // ----- --force overrides a dirty tree (json → no confirm) -----

    #[test]
    fn force_applies_on_dirty_tree() {
        if !git_available() {
            return;
        }
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);
        commit_all(&root);
        fs::write(root.join("on-chain/DIRTY.md"), b"uncommitted").unwrap();

        let flags = UpdateFlags {
            force: true,
            ..UpdateFlags::default()
        };
        // Json format never prompts, so --force applies straight through.
        let new = add_offchain(&old);
        finish_update(&root, old, new, &flags, &reg(), Format::Json).expect("--force applies");

        assert!(root.join("off-chain/package.json").is_file());
    }

    // ----- dry-run branch: prints the plan, writes nothing, before the git gate

    #[test]
    fn dry_run_writes_nothing() {
        // No git repo at all: dry-run returns before the git gate is reached.
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);

        let flags = UpdateFlags {
            dry_run: true,
            ..UpdateFlags::default()
        };
        let new = add_offchain(&old);
        finish_update(&root, old, new, &flags, &reg(), Format::Human).expect("dry-run is Ok");

        assert!(!root.join("off-chain").exists());
    }

    // ----- no-op guard: identical selection -----

    #[test]
    fn identical_selection_is_nothing_to_change() {
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);

        let err = finish_update(
            &root,
            old.clone(),
            old,
            &UpdateFlags::default(),
            &reg(),
            Format::Human,
        )
        .unwrap_err();

        assert!(matches!(err, CliError::NothingToChange));
    }

    // ----- slot_occupied: a create target already exists on disk -----

    #[test]
    fn create_target_dir_already_occupied() {
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);
        // A foreign directory sits where the new off-chain component would land.
        fs::create_dir(root.join("off-chain")).unwrap();
        fs::write(root.join("off-chain/foreign.txt"), b"not ours").unwrap();

        let new = add_offchain(&old);
        let err = finish_update(
            &root,
            old,
            new,
            &UpdateFlags::default(),
            &reg(),
            Format::Human,
        )
        .unwrap_err();

        assert!(matches!(err, CliError::SlotOccupied { .. }));
    }

    // ----- detect_selection: unrecognized dir is fatal in json/non-interactive

    #[test]
    fn unrecognized_dir_is_fatal_in_json() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        // An `on-chain/` dir whose contents match no on-chain tool's signatures.
        fs::create_dir(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/mystery.txt"), b"not a known tool").unwrap();

        let err = detect_selection(&root, &reg(), Format::Json).unwrap_err();
        assert!(matches!(err, CliError::ProjectUnrecognized { .. }));
    }

    #[test]
    fn recognized_project_reconstructs_in_json() {
        // The clean path through detect_selection: a real scaffolded tree is
        // recovered without error in non-interactive mode.
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let (_tmp, root) = scaffolded(&old);

        let recovered = detect_selection(&root, &reg(), Format::Json).expect("recognized");
        assert!(
            recovered
                .assignments
                .iter()
                .any(|x| x.role == Role::OnChain && x.tool_id == "aiken")
        );
        assert!(
            recovered
                .assignments
                .iter()
                .any(|x| x.role == Role::OffChain && x.tool_id == "meshjs")
        );
    }
}
