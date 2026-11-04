#!/usr/bin/env python3
"""Verify the `cardano-init doctor` install recipes actually work.

Given ONE installer key (e.g. "apt"), this reads `registry/deps.toml`, finds
every dependency whose recipe offers a method for that installer, runs the
rendered install command, and asserts the dep's declared `binaries` land on
PATH afterwards. Exits non-zero if any recipe fails to install or leaves its
binary unreachable.

This is the real-world half of the two-tier installer gate: the PR gate
(`cargo test`, via `src/doctor/catalog.rs`) already proves every recipe parses
and names a known installer. What it CANNOT prove is that the recipe still
works against the live world — a renamed apt package, a moved aikup npm path, a
brew formula rename. That is external drift, so it belongs on a schedule, not a
PR gate (same rationale as tool-smoke.yml).

Usage:
    python3 verify_installers.py <installer-key> [--deps <path>]

Requires Python 3.11+ (uses the stdlib `tomllib`), present on all runners and
container images the workflow targets.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

# ---------------------------------------------------------------------------
# Command templates.
#
# MUST stay in lockstep with `Installer::command()` in
# src/doctor/installers.rs — that function is the single source of truth for
# what `cardano-init doctor` tells a user to run; this table mirrors it so the
# workflow exercises the exact same commands. `{arg}` is the recipe's arg (a
# package name, an installer-script URL, or a target, per installer).
# ---------------------------------------------------------------------------
COMMANDS: dict[str, str] = {
    "brew": "brew install {arg}",
    "apt": "sudo apt install -y {arg}",
    "dnf": "sudo dnf install -y {arg}",
    "pacman": "sudo pacman -S --noconfirm {arg}",
    "winget": "winget install {arg}",
    "nix": "nix profile install nixpkgs#{arg}",
    "go": "go install {arg}",
    "cargo": "cargo install {arg}",
    "npm": "npm install -g {arg}",
    "aikup": "aikup install {arg}",
    "cardano-up": "cardano-up install {arg}",
    "tx3up": "tx3up install {arg}",
    "curl": "curl -sSfL {arg} | sh",
    "powershell": 'powershell -c "irm {arg} | iex"',
}

# Tool-managed installers drop binaries into their own dir, which their
# interactive installer would add to a shell profile — but CI shells are
# non-interactive, so we augment the lookup PATH with the standard locations.
# (The install itself is unchanged; only our presence check looks wider.)
EXTRA_BIN_DIRS = [
    Path.home() / ".cargo" / "bin",
    Path.home() / ".aiken" / "bin",
    Path.home() / "go" / "bin",
    Path.home() / ".local" / "bin",
    # tx3up installs the toolchain (trix/dolos/…) under ~/.tx3/<channel>/bin,
    # with `default` symlinked to the active channel.
    Path.home() / ".tx3" / "default" / "bin",
    Path("/root/.cargo/bin"),
    Path("/root/.aiken/bin"),
    Path("/root/.tx3/default/bin"),
]


def load_methods(deps_path: Path, installer: str) -> list[tuple[str, str, list[str]]]:
    """Return (dep_id, arg, binaries) for every recipe offering `installer`."""
    with deps_path.open("rb") as f:
        recipes = tomllib.load(f)

    found: list[tuple[str, str, list[str]]] = []
    for dep_id, recipe in recipes.items():
        binaries = recipe.get("binaries", [])
        for entry in recipe.get("install", []):
            # Each entry is a single-key table: { <installer> = "<arg>" }.
            for key, arg in entry.items():
                if key == installer:
                    found.append((dep_id, arg, binaries))
    return found


def _is_dir(d: Path) -> bool:
    """Like `Path.is_dir()` but treats an unstattable path as absent.

    A candidate we can't reach (e.g. `/root/.cargo/bin` from a non-root runner)
    raises `PermissionError`, which `Path.is_dir()` propagates rather than
    swallowing. For a best-effort PATH-augmentation probe, unreachable == not a
    usable bin dir, so fold every stat error into False.
    """
    try:
        return d.is_dir()
    except OSError:
        return False


def lookup_path(extra: list[Path] | None = None) -> str:
    """PATH augmented with the tool-managed bin dirs (for the presence check)."""
    parts = os.environ.get("PATH", "").split(os.pathsep)
    parts += [str(d) for d in EXTRA_BIN_DIRS if _is_dir(d)]
    parts += [str(d) for d in (extra or []) if _is_dir(d)]
    return os.pathsep.join(parts)


def build_command(installer: str, template: str, arg: str) -> tuple[str, list[Path]]:
    """Render a recipe's install command and any extra bin dirs to look in.

    Mirrors `Installer::command()`; `.rstrip()` matches its bare `aikup install`
    (empty arg ⇒ latest, no trailing space).

    For `nix`, each recipe is redirected into its own throwaway profile. A real
    user installs ONE tool, but this gate installs every nix recipe into a
    single environment, where two packages that ship the same file collide
    (e.g. rustup and cargo both provide `cargo.bash`). Per-recipe profiles
    remove that false conflict; the nixpkgs attr installed is exactly what
    `doctor` prints — only the profile location differs, in the same spirit as
    the EXTRA_BIN_DIRS PATH augmentation above.
    """
    command = template.format(arg=arg).rstrip()
    extra: list[Path] = []
    if installer == "nix":
        profile = Path(tempfile.mkdtemp(prefix="nixprof-")) / "profile"
        command = command.replace(
            "nix profile install ",
            f"nix profile install --profile {profile} ",
            1,
        )
        extra.append(profile / "bin")
    return command, extra


def verify_one(
    dep_id: str, binaries: list[str], command: str, extra_bin_dirs: list[Path]
) -> bool:
    """Run one install command; report whether its binaries are reachable.

    Success is defined by the binary being reachable afterwards, NOT by the
    install command's exit code. Some recipes exit non-zero for reasons that
    don't reflect a broken recipe — e.g. brew's rustup post-install step fails
    on a runner that already ships a Rust toolchain, yet `rustup` is installed
    and on PATH. We surface a non-zero exit as a warning but only fail when a
    declared binary is actually missing.
    """
    print(f"::group::{dep_id} via {command}", flush=True)
    install_rc = subprocess.run(command, shell=True).returncode

    path = lookup_path(extra_bin_dirs)
    missing = False
    for binary in binaries:
        if shutil.which(binary, path=path):
            print(f"  ✓ {binary} on PATH", flush=True)
        else:
            print(f"  ✗ {binary} NOT found after install", flush=True)
            missing = True

    if install_rc != 0:
        if missing:
            print(f"  ✗ install command exited {install_rc}", flush=True)
        else:
            print(
                f"  ! install command exited {install_rc}, but binaries are "
                f"present — treating as OK",
                flush=True,
            )
    print("::endgroup::", flush=True)
    return not missing


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("installer", help="installer key, e.g. apt, brew, nix")
    parser.add_argument(
        "--deps",
        type=Path,
        default=Path(__file__).resolve().parents[2] / "registry" / "deps.toml",
        help="path to registry/deps.toml",
    )
    args = parser.parse_args()

    if args.installer not in COMMANDS:
        print(f"unknown installer '{args.installer}'", file=sys.stderr)
        return 2

    methods = load_methods(args.deps, args.installer)
    if not methods:
        # An installer with no recipes today (e.g. dnf/pacman currently) is not
        # a failure — it is simply nothing to verify. Logged, never silent.
        print(f"no recipes use '{args.installer}' — nothing to verify.")
        return 0

    template = COMMANDS[args.installer]
    print(f"verifying {len(methods)} recipe(s) for '{args.installer}':\n")

    failures: list[str] = []
    for dep_id, arg, binaries in methods:
        command, extra_bin_dirs = build_command(args.installer, template, arg)
        if not verify_one(dep_id, binaries, command, extra_bin_dirs):
            failures.append(dep_id)

    print()
    if failures:
        print(f"FAILED for '{args.installer}': {', '.join(failures)}")
        return 1
    print(f"OK: all '{args.installer}' recipes install and expose their binaries.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
