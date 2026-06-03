# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`cardano-init` is a Rust CLI tool that scaffolds Cardano protocol projects. Users select tools for each functional role (on-chain, off-chain, infrastructure, testing) and the CLI generates a working monorepo. The authoritative docs live in `docs/`: read `docs/PRD.md` (product), `docs/ARCHITECTURE.md` (system design), and `docs/TECH_SPEC.md` (contracts, schemas, edge cases) before making any significant changes; `docs/ADDING_A_TOOL.md` is the contributor guide.

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

- `src/cli/` — user interaction only (dialoguer, output formatting). No logic.
- `src/registry/` — deserializes embedded TOML tool definitions into typed structs.
- `src/scaffold/` — four-phase pipeline: context building → planning → rendering → writing.
- `src/contract.rs` — constants for the interface contract (canonical paths, env var names, Justfile task names).

**Key invariant:** `registry/`, `contract`, and `scaffold/` must have zero dependency on `cli/`. They are pure logic over data.

### Data model

- **`Selection`** — fully resolved user choices (project name, role assignments, network, nix flag).
- **`ToolDef`** — loaded from `registry/tools/<tool>.toml`. Each tool declares which roles it fills and which template path to use.
- **`TemplateContext`** — built from `Selection` + `Registry`; passed to MiniJinja for rendering.
- **Infrastructure role** is the only role that allows multiple tools simultaneously.

### Registry and templates

Tool definitions live in `registry/tools/<tool>.toml`. Templates live in `templates/<tool>/<role>/` with a `manifest.toml` listing files. Both are embedded into the binary at compile time via `build.rs`.

The **interface contract** (`contract.rs`) is what enables any on-chain tool to compose with any off-chain tool without per-pair logic:
- On-chain templates must produce `blueprint/plutus.json` during `build`.
- Infra templates must write standard env vars (e.g., `INDEXER_URL`) to `.env` during `dev`.
- All templates must expose `build`, `test`, `dev`, `clean` Justfile targets.

### Scaffolding pipeline

1. **Context** (`scaffold/context.rs`) — builds `TemplateContext` from selection.
2. **Plan** (`scaffold/planner.rs`) — collects all `FileEntry` items to emit; `--dry-run` exits here.
3. **Render** (`scaffold/renderer.rs`) — runs MiniJinja on each renderable file.
4. **Write** — only phase with disk side effects.

## Dependencies (planned)

- `clap` — argument parsing
- `dialoguer` — interactive prompts
- `minijinja` — template rendering
- `serde` / `toml` — registry deserialization
