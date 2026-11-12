#!/usr/bin/env python3
"""Select smoke-matrix rows: those a diff affects, or all with no arguments.

Usage: select_checks.py [BASE HEAD]
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MATRIX_FILE = ROOT / ".github" / "smoke-matrix.json"

# Blaster has no binary to run yet. Infrastructure tools are tested in their own repos.
NO_CHECK = {
    "blaster",
    "cardano-node",
    "cardano-node-api",
    "dingo",
    "dolos",
    "kupo",
    "ogmios",
    "tx-submit-api",
}

# Generated from the templates, so template PRs regenerating them stay scoped.
NOT_SHARED = ("src/scaffold/snapshots/",)
SHARED = (
    "src/scaffold/",
    "src/registry/",
    "src/contract.rs",
    "templates/_base/",
    "templates/_nix/",
    "templates/_spec/",
    "Cargo.toml",
    "Cargo.lock",
    ".github/workflows/tool-smoke.yml",
    ".github/workflows/devnet-smoke.yml",
    ".github/scripts/blueprint_shape.py",
    ".github/blueprint-shape.json",
)
TOOL_PATH = re.compile(r"templates/([^/]+)/|registry/tools/([^/]+)\.toml$")


def select(paths, matrix):
    tools = set()
    for path in paths:
        if path.startswith(NOT_SHARED):
            continue
        if path.startswith(SHARED):
            return matrix
        match = TOOL_PATH.match(path)
        if match:
            tools.add(match[1] or match[2])
    return {
        section: [row for row in rows if tools & set(row["tools"])]
        for section, rows in matrix.items()
    }


def coverage_errors(matrix):
    registered = {path.stem for path in (ROOT / "registry" / "tools").glob("*.toml")}
    covered = {tool for rows in matrix.values() for row in rows for tool in row["tools"]}
    errors = [f"unknown tool in smoke-matrix.json: {t}" for t in sorted(covered - registered)]
    errors += [f"no check in smoke-matrix.json for: {t}" for t in sorted(registered - covered - NO_CHECK)]
    return errors


def changed_paths(base, head):
    # --no-renames: moving a file out of a tool's directory still affects it.
    diff = ["git", "diff", "--no-renames", "--name-only", f"{base}...{head}"]
    return subprocess.check_output(diff, cwd=ROOT, text=True).splitlines()


def main():
    matrix = json.loads(MATRIX_FILE.read_text())
    errors = coverage_errors(matrix)
    if errors:
        sys.exit("\n".join(errors))
    selected = select(changed_paths(*sys.argv[1:3]), matrix) if sys.argv[1:] else matrix

    lines = []
    for section, rows in selected.items():
        lines.append(f"{section}={json.dumps({'include': rows})}")
        lines.append(f"has_{section}={'true' if rows else 'false'}")
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as out:
        out.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
