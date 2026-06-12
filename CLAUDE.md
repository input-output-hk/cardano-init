# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`cardano-init` is a Rust CLI tool that scaffolds Cardano protocol projects. Users select tools for each functional role (on-chain, off-chain, infrastructure, testing, formal-methods) and the CLI generates a working monorepo. The authoritative docs live in `docs/`: read `docs/PRD.md` (product), `docs/ARCHITECTURE.md` (system design), and `docs/TECH_SPEC.md` (contracts, schemas, edge cases) before making any significant changes; `docs/ADDING_A_TOOL.md` is the contributor guide and `docs/ROADMAP.md` is the milestone plan.

It's an early prototype: several capabilities (the dependency `doctor`, `list` / `--format json`, a hosted web builder) are **planned, not yet implemented**. `docs/ROADMAP.md` tracks what's real vs. upcoming.

## Commands

```bash
# Build
cargo build

# Run
cargo run -- [args]

# Tests
cargo test

# Single test
cargo test <test_name>

# Lint/format
cargo fmt
cargo clippy
```

## Architecture

The codebase is a single Rust crate. The module structure is as follows (see `docs/ARCHITECTURE.md` §2):

- `src/cli/`: impure edge: argument parsing (clap), interactive prompts (dialoguer), and the output presenter. Orchestrates the core; holds no generation logic.
- `src/registry/`: deserializes embedded TOML tool definitions into typed structs. The `Role` enum (5 roles) is the sole source of truth for the role vocabulary; the registry only *references* roles.
- `src/scaffold/`: four-phase pipeline: context building → planning → rendering → writing.
- `src/web/`: impure edge: hand-rolled local web builder server.
- `src/doctor/`: **(planned)** dependency detection + install advice.
- `src/contract.rs`: constants for the interface contract (canonical paths, env var names, role dirs).

**Key invariant:** `registry/`, `scaffold/`, `contract`, and the pure part of `doctor/` have zero dependency on `cli/` or `web/` (the impure edges). They are pure logic over data.

### Data model

- **`Selection`**: fully resolved user choices (project name, role assignments, network, nix flag).
- **`ToolDef`**: loaded from `registry/tools/<tool>.toml`. Each tool declares which roles it fills and which template path to use.
- **`TemplateContext`**: built from `Selection` + `Registry`; passed to MiniJinja for rendering.
- **Infrastructure role** is the only role that allows multiple tools simultaneously.

### Registry and templates

Tool definitions live in `registry/tools/<tool>.toml`. Templates live in `templates/<tool>/<role>/` with a `manifest.toml` listing files. Both are embedded into the binary at compile time via **rust-embed** (`#[folder = "…"]`). There is **no `build.rs`**. (Dependency install recipes will live in `registry/deps.toml`, consumed by the planned `doctor`, see `docs/TECH_SPEC.md` §9.)

The **interface contract** (`contract.rs`) is what enables any on-chain tool to compose with any off-chain tool without per-pair logic:
- On-chain templates must produce `blueprint/plutus.json` during `build`.
- Whichever component provisions a local chain endpoint writes the standard env vars (e.g., `INDEXER_URL`) to `.env` during `dev` — an infrastructure service, or a local devnet such as Yaci DevKit in the *testing* role. Consumers read them and degrade gracefully when blank. (The `.env` connection seam is role-agnostic: role = a tool's purpose; writing `.env` = the capability of exposing a local endpoint. These are orthogonal.)
- All templates must expose `build`, `test`, `clean` Justfile targets and work standalone. `dev` is **optional** — a component provides it only when it has a real watch/daemon/devnet mode (no no-op `dev`s). The **top level** aggregates only `build`/`test`/`clean` (terminating, composable tasks); `test` builds on-chain first (blueprint), then runs each component's `test` in `Role::ALL` order (incl. formal `verify`). Long-running/interactive `dev`, where present, is **per-component** (run directly, e.g. `just -f test/Justfile dev`), never aggregated at the top level.

### Scaffolding pipeline

1. **Context** (`scaffold/context.rs`): builds `TemplateContext` from selection.
2. **Plan** (`scaffold/planner.rs`): collects all `FileEntry` items to emit; `--dry-run` exits here.
3. **Render** (`scaffold/renderer.rs`): runs MiniJinja on each renderable file.
4. **Write**: only phase with disk side effects.

## Dependencies

- `clap`: argument parsing
- `dialoguer`: interactive prompts
- `minijinja`: template rendering
- `serde` / `serde_json` / `toml`: (de)serialization
- `rust-embed`: embed registry + templates into the binary
- `thiserror`: error types
- `console`: terminal styling
- `tempfile` (dev): test scaffolding
