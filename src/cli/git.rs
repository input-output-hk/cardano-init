//! The only place the tool shells out to `git`: the update commands'
//! clean-tree gate, and the initial repo + commit for a new project.

use std::path::Path;
use std::process::{Command, Output};

/// Run `git -C <root> <args>`, returning `None` if `git` couldn't be executed.
fn git(root: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
}

/// Run `git -C <root> <args>`, reporting whether it ran and succeeded.
fn git_succeeds(root: &Path, args: &[&str]) -> bool {
    git(root, args).is_some_and(|out| out.status.success())
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether `root` is already inside a git work tree (so we must not nest a repo).
fn inside_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|out| {
        out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
    })
}

/// Whether the git working tree at `root` has no changes and no untracked
/// files. A missing `git`, a non-repo, or any error counts as **not** clean, so
/// the update commands require `--force` rather than proceeding blindly.
pub fn is_clean(root: &Path) -> bool {
    git(root, &["status", "--porcelain"])
        .is_some_and(|out| out.status.success() && out.stdout.is_empty())
}

/// Result of trying to set up a repo for a freshly scaffolded project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitOutcome {
    /// `git init` + `add -A` + initial commit all succeeded.
    Committed,
    /// Initialized and staged, but the commit failed (typically no
    /// `user.name`/`user.email` configured).
    InitializedNoCommit,
    /// Skipped — `git` is not installed.
    GitMissing,
    /// Skipped — already inside an existing repository (don't nest one).
    AlreadyRepo,
}

impl InitOutcome {
    /// Stable slug for the JSON envelope.
    pub fn as_str(self) -> &'static str {
        match self {
            InitOutcome::Committed => "committed",
            InitOutcome::InitializedNoCommit => "initialized",
            InitOutcome::GitMissing => "git_missing",
            InitOutcome::AlreadyRepo => "already_repo",
        }
    }
}

/// Best-effort: initialize a repo with an initial commit for the new project at
/// `root`. Never fails; every problem degrades to a lesser [`InitOutcome`].
pub fn init_project_repo(root: &Path) -> InitOutcome {
    if !git_available() {
        return InitOutcome::GitMissing;
    }
    if inside_repo(root) {
        return InitOutcome::AlreadyRepo;
    }
    if !git_succeeds(root, &["init", "--quiet"]) {
        return InitOutcome::GitMissing;
    }
    let _ = git(root, &["add", "-A"]);
    let commit = [
        "commit",
        "--quiet",
        "-m",
        "chore: scaffold project with cardano-init",
    ];
    if git_succeeds(root, &commit) {
        InitOutcome::Committed
    } else {
        InitOutcome::InitializedNoCommit
    }
}
