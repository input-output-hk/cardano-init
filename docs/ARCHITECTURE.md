# cardano-init — Architecture

**Status:** Draft · **Last updated:** 2026-06-01 · **Owner:** Robertino Martinez

> This is the **canonical** architecture document. It supersedes the legacy root-level `REQUIREMENTS.md` and `ARCHITECTURE.md` (retained only in git history / `docs/legacy/`). Read [PRD.md](./PRD.md) for the *why* and *for whom*; this document owns the *how*. Detailed contracts, data shapes, and edge cases live in [TECH_SPEC.md](./TECH_SPEC.md); sequencing lives in [ROADMAP.md](./ROADMAP.md).

---

## 1. Design principles

Five principles drive every structural decision in the codebase. When a tradeoff arises, these are the tie-breakers.

1. **The interface contract is the core abstraction.** Every tool template conforms to a shared set of conventions (canonical blueprint path, standard Justfile tasks, standard `.env` variables). Because each template *independently* conforms, any producer composes with any consumer **without per-pair integration code**. Composition is generic over the *set of roles present*, never over *which tools fill them*. This is what makes the system scale as O(tools) rather than O(tools²).

2. **Tools are data-driven; roles are a fixed code vocabulary.** Tools and templates are declarative data embedded at compile time — adding a *tool* is a data change (a TOML file + a template directory + a recompile), never a change to CLI logic. **Roles**, by contrast, are a small fixed vocabulary defined in code (the `Role` enum, §3.1): the registry *references* roles but cannot introduce them. The set is not frozen at a particular number — it can grow — but growing it is a deliberate, rare code change, not a data change.

3. **Pure core, impure edges.** `registry/`, `scaffold/`, `contract`, and the pure part of `doctor/` are pure logic over data with **zero dependency on `cli/`**. All user interaction, terminal formatting, network, and system probing live at the edges (`cli/`, `web/`, the impure half of `doctor/`). This keeps the core testable and makes future extraction (e.g. WASM) straightforward.

4. **Deterministic generation.** Identical inputs produce byte-identical output. This is a hard requirement for coding-agent trust, reproducibility, and snapshot tests. Determinism is guaranteed at the **planning** phase (§6.4).

5. **Offline and self-contained.** The registry and all templates are embedded in the binary; generation makes no network calls. Network is used only for installing toolchains (the doctor) and a best-effort version-update notice (§9). The binary, for a given version, is the single source of truth for what it generates.

---

## 2. Crate & module structure

A single Rust crate. The boundary between "library logic" and "CLI concerns" is enforced by the module dependency graph (§2.2), not by separate crates. Items marked **(planned)** are introduced by the PRD and not yet implemented.

```
cardano-init/
├── Cargo.toml
├── src/
│   ├── main.rs                 # Entry point: delegates to cli::run()
│   │
│   ├── cli/                    # Impure edge: user interaction, formatting, process control
│   │   ├── mod.rs              # Arg parsing (clap), dispatch, top-level error type
│   │   ├── interactive.rs      # Guided interactive flow (dialoguer)
│   │   ├── oneshot.rs          # Flag → Selection, validation, machine-readable errors
│   │   ├── output.rs           # Presenter: renders results/errors as human text or JSON
│   │   └── update.rs           # (planned) async, cached, fail-silent version check (§9)
│   │
│   ├── registry/               # Pure: tool + role definitions from embedded TOML
│   │   ├── mod.rs
│   │   ├── types.rs            # Role, ToolDef, RoleConfig, Selection, Network, …
│   │   └── loader.rs           # rust-embed → Registry (indexed by id and by role)
│   │
│   ├── scaffold/               # Pure: project generation pipeline
│   │   ├── mod.rs              # Orchestrator (scaffold / dry_run) + embedded templates
│   │   ├── context.rs          # Phase 1: Selection + Registry → TemplateContext
│   │   ├── planner.rs          # Phase 2: → FilePlan (canonical order; dry-run stops here)
│   │   ├── renderer.rs         # Phase 3: MiniJinja render / pass-through
│   │   └── writer.rs           # Phase 4: the only phase with disk side effects
│   │
│   ├── doctor/                 # (planned) dependency detection + install advice (§8)
│   │   ├── mod.rs              # Pure: (deps, environment) → missing + advice
│   │   ├── catalog.rs          # Pure: dep id → per-platform check/install knowledge
│   │   └── probe.rs            # Impure: detect OS, package managers, PATH
│   │
│   ├── web/                    # Impure edge: local web builder server
│   │   ├── mod.rs              # Hand-rolled HTTP server; /, /api/registry, /api/plan
│   │   └── ui.html             # Embedded single-page UI
│   │
│   └── contract.rs             # Interface-contract constants (paths, env vars, dirs)
│
├── registry/tools/             # Embedded data: one TOML per tool
│   ├── aiken.toml  meshjs.toml  scalus.toml  blaster.toml
│
└── templates/                  # Embedded data: tool/role template trees
    ├── _base/    (Justfile.jinja, README.md.jinja, gitignore, env.jinja)
    ├── _nix/     (flake.nix.jinja)
    └── <tool>/<role>/  (manifest.toml + template files)
```

Assets are embedded with **rust-embed** via `#[folder = "registry/"]` and `#[folder = "templates/"]`. There is **no `build.rs`** — embedding is handled by the derive macro directly. (The legacy architecture doc referenced a `build.rs` asset manifest; that is obsolete.)

### 2.2 Module dependency graph

The graph flows strictly downward; there are no cycles. The key invariant: **`registry`, `scaffold`, `contract`, and the pure part of `doctor` never depend on `cli` or `web`.**

```
main.rs
  │
  ├── cli/ ──────┬─▶ scaffold/ ─▶ registry/        web/ ─┬─▶ scaffold::planner
  │              ├─▶ doctor/   ─▶ registry/              └─▶ registry/
  │              ├─▶ registry/                           (web is an edge, like cli)
  │              └─▶ contract
  │
  scaffold/, doctor/(pure), registry/  ──▶  contract
```

`cli/` and `web/` are sibling **edges**: both orchestrate the pure core and present results. Neither is depended upon by the core.

---

## 3. Data model

All core types live in `registry/types.rs` (and `scaffold/` for pipeline-internal types). Exact field-level definitions and invariants are in TECH_SPEC; this is the shape and intent.

### 3.1 Roles

```rust
pub enum Role { OnChain, OffChain, Infrastructure, Testing, FormalMethods }
```

- `Role::ALL` defines the **canonical order** used for deterministic output.
- Each role maps to a kebab string (`on-chain`, `formal-methods`, …) for TOML/flags, a `Display` name for humans, and a contract directory (`dir()` → §4).
- **The enum is the sole source of truth for the role vocabulary — roles are *not* defined by the repository data.** A tool's `[roles.<kebab>]` blocks merely *reference* existing roles; the registry cannot introduce a new one. Role strings are validated against the enum at load time via `Role::from_kebab` (an unknown role → `RegistryError::UnknownRole`). What the registry data determines is which *tools* exist and which of these fixed roles each can fill — not the set of roles itself.
- Adding a role is therefore a deliberate code change touching every site that names roles: a new `Role` variant + `Role::ALL` + `from_kebab`/`as_kebab`/`dir()`/`Display`, a `contract::DIR_*` constant, `TemplateContext` handling, a CLI flag, and the web query params. Adding a *tool*, by contrast, is pure data. The role set is small and grows rarely.

### 3.2 Tools

```rust
pub struct ToolDef {
    pub id, name, description, website: String,
    pub languages: Vec<String>,
    pub nix_packages: Vec<String>,         // toolchains for the Nix dev shell
    pub roles: HashMap<Role, RoleConfig>,  // which roles this tool can fill
}
pub struct RoleConfig { pub template: String }  // path under templates/
```

`system_deps` is declared in the tool TOML and consumed by the **doctor** (§8). One tool can fill multiple roles (e.g. Scalus: on-chain + off-chain + testing), each with its own template path.

### 3.3 Selection (the resolved user choice)

```rust
pub struct Selection {
    pub project_name: String,
    pub assignments: Vec<RoleAssignment>,  // Infrastructure may appear multiple times
    pub network: Network,                  // Preview | Preprod | Mainnet (default Preview)
    pub nix: bool,
}
pub struct RoleAssignment { pub role: Role, pub tool_id: String }
```

**Constraint enforcement is by construction.** Role uniqueness (one tool per role, except Infrastructure) is enforced at the edge: interactive mode only allows one tool per non-infra role; one-shot uses single-value flags per role (`--infra` is repeatable). A `Selection` that exists is valid — there is no separate validation module.

### 3.4 Pipeline types (`scaffold/`)

```rust
pub struct TemplateContext { … }   // per-role flags + RoleContexts + contract constants
pub struct RoleContext { tool_id, tool_name, language, dir }
pub struct FilePlan { entries: Vec<FileEntry> }
pub struct FileEntry { dest: PathBuf, source: TemplateSource, render: bool }
pub enum TemplateSource { Base(String), Role(String), Optional(String), Inline(Vec<u8>) }
```

`TemplateContext` is `Serialize` and is the entire surface templates can see. It carries `has_*` booleans per role, an `Option<RoleContext>` per single-tool role, `infra_tools: Vec<RoleContext>`, the contract constants (`blueprint_path`, `env_vars`), and Nix info. `render` is derived from the `.jinja` extension.

---

## 4. Interface contract (`contract.rs`)

The contract is a set of constants every template conforms to. It is the seam that makes composition generic.

```rust
pub const BLUEPRINT_PATH: &str = "blueprint/plutus.json";
pub const DIR_ON_CHAIN = "on-chain"; DIR_OFF_CHAIN = "off-chain";
pub const DIR_INFRA = "infra"; DIR_TESTING = "test"; DIR_FORMAL_METHODS = "formal-methods";
pub const ENV_INDEXER_URL = "INDEXER_URL"; ENV_INDEXER_PORT = "INDEXER_PORT";
pub const ENV_NODE_SOCKET_PATH = "NODE_SOCKET_PATH"; ENV_NETWORK = "CARDANO_NETWORK";
```

**Compliance checklist (enforced mechanically by contract-compliance tests):**

- **Every template** ships a `Justfile` exposing `build`, `test`, `dev`, `clean`, and works **independently** (its `just build` succeeds with no other roles present). A target that is a no-op for that tool must still exist (printing a message is fine).
- **On-chain** produces the CIP-57 blueprint at `../blueprint/plutus.json` during `build`. Other roles read it from that path if present.
- **Infrastructure** writes standard connection vars to `../.env` during `dev`.
- **Off-chain / testing / formal-methods** read the blueprint and `.env` if present, and degrade gracefully when absent.

The `blueprint/` **directory** is scaffolded whenever any blueprint-producing-or-consuming
role is present — every project except infrastructure-only (§6.2) — so the canonical
path exists wherever it's meaningful; the `plutus.json` **file** within it may still be
absent (no on-chain role, or no build yet), which is why consumers must tolerate its
absence. The CLI never tracks which tools produce/consume blueprints — it is a
template-level convention verified by tests, not registry metadata.

---

## 5. Registry

Each tool is one TOML file under `registry/tools/`:

```toml
[tool]
id = "aiken"
name = "Aiken"
description = "…newcomer-friendly explanation…"
website = "https://aiken-lang.org"
languages = ["aiken"]
system_deps = ["aiken"]    # abstract dep ids → resolved by the doctor catalog (§8)
nix_packages = ["aiken"]       # packages for the generated Nix dev shell

[roles.on-chain]
template = "aiken/on-chain"    # path under templates/
```

`loader.rs` iterates embedded assets, parses each TOML into a `ToolDef`, and builds a `Registry` with two indexes: `by_id` (lookup) and `by_role` (list tools for a role). Loading rejects duplicate ids and an empty registry. The registry is immutable after load.

---

## 6. Scaffolding pipeline

Four independent, individually testable phases. `--dry-run` stops after phase 2.

```
Selection + Registry
        │
        ▼
┌────────────────┐   ┌──────────────┐   ┌──────────────┐   ┌───────────────┐
│ 1. Context     │──▶│ 2. Plan      │──▶│ 3. Render    │──▶│ 4. Write      │
│ build_context()│   │ plan()       │   │ render()     │   │ write()       │
│ (pure)         │   │ (pure)       │   │ (pure)       │   │ (side effects)│
└────────────────┘   └──────────────┘   └──────────────┘   └───────────────┘
                         │
                    --dry-run exits here (returns FilePlan)
```

### 6.1 Context (`context.rs`)

Walks `selection.assignments`, resolves each tool against the registry, and builds the `TemplateContext`: per-role `has_*` flags and `RoleContext`s, deduplicated `nix_packages`, contract constants, and `.env` variable seeds. Errors on unknown tool or role mismatch.

### 6.2 Plan (`planner.rs`)

Produces the ordered `FilePlan`:
1. **Base layer** (always): `Justfile`, `README.md`, `.gitignore`, `.env`.
2. **Blueprint dir**: `blueprint/.gitkeep`, emitted whenever the selection includes any blueprint-producing-or-consuming role — i.e., any role **except** infrastructure (equivalently: present unless the project is infrastructure-only).
3. **Role layers**: for each assignment, read the template's `manifest.toml` and add its files. Infrastructure tools each nest under `infra/<tool_id>/`.
4. **Optional layer**: `flake.nix` + `.envrc` when `nix` is set.

No I/O — only embedded assets are read. `render` is set from the `.jinja` extension.

The `blueprint/` directory gives every blueprint-consuming role — off-chain, testing, formal-methods — a stable, predictable path to read from, and lets a user drop a hand-supplied or externally-built `plutus.json` into the same place even when on-chain isn't scaffolded in this project. It is omitted only for infrastructure-only projects, where no role produces or consumes a blueprint. Only the directory (via `.gitkeep`) is created; the `plutus.json` *file* is produced by on-chain `build`, so consumers must still handle its absence gracefully (§4).

> **Code note:** the current `planner.rs` creates `blueprint/.gitkeep` only when on-chain is present (guarded by a `has_on_chain` check). The rule above broadens that guard to "any non-infrastructure role present".

### 6.3 Render (`renderer.rs`) & Write (`writer.rs`)

Render processes each entry whose source is a `.jinja` template through MiniJinja with the `TemplateContext`. Render-ness is **derived from the file extension** at plan time — the planner sets `FileEntry.render = source.ends_with(".jinja")` (§6.2); it is not an authored manifest field (manifests list only `source`/`dest`). Non-`.jinja` files, and `Inline` sources, pass through verbatim. Write is the **only** phase that touches disk: it creates parent directories and writes each file's content.

### 6.4 Determinism rule

**Determinism is a guarantee of the planning phase.** The planner emits entries in a fixed order: base layer → blueprint dir (when any non-infrastructure role is present) → role layers in **`Role::ALL` order** → optional layer. Within Infrastructure (the only multi-tool role), tools are ordered by **sorted tool id**. Any `HashMap` (e.g. `env_vars`) is iterated through a sorted/canonical view before it reaches output. Snapshot tests over `--dry-run` and rendered output enforce byte-stability. No other phase may introduce nondeterministic ordering.

> Current code stores `roles` in a `HashMap` and preserves `assignments` in flag order; formalizing the canonical ordering above (esp. sorting infra by id and iterating `assignments` in `Role::ALL` order) is the concrete work item this rule mandates.

---

## 7. CLI surfaces & output model

### 7.1 Modes

`cli/mod.rs` parses args with `clap`. There is an optional subcommand and a flattened set of init flags:

- **One-shot** (`--name` + role flags): flags → `Selection` in `oneshot.rs`, non-interactive, deterministic. Primary path for agents and CI.
- **Interactive** (no `--name`): guided `dialoguer` flow in `interactive.rs`.
- **`web` subcommand**: launches the local builder server (§10).
- **`list` subcommand** (planned): capability discovery — lists roles/tools, human by default, `--format json` for agents (see §7.3).

A safety check refuses to overwrite an existing target directory.

### 7.2 Output model — `--format` + presenter (planned)

Today output is human-styled text printed directly. To serve both humans and agents without scattering format branches, the architecture introduces:

- A global **`--format human|json`** flag (default `human`; `json` implies non-interactive — JSON mode never prompts).
- **`output.rs` as a presenter**: the pure core returns *structured results* and *typed errors*; only the presenter knows about colors, tables, or JSON. Adding a new output is a presenter change, nothing else.

### 7.3 Machine-readable errors & discovery (planned, PRD FR-13/FR-15)

- **Errors** carry a **stable string code** (e.g. `unknown_tool`, `tool_role_mismatch`, `name_required`, `dir_exists`) plus context (offending input + valid alternatives) and map to **meaningful exit codes**. In `--format json`, errors serialize to a stable shape on stderr; the core never falls back to interactive prompting in non-interactive mode. `CliError` already enumerates these cases; this work adds the code + serializable representation.
- **Discovery** is a dedicated **`list` subcommand** (`cardano-init list`) that emits the registry (roles, tools, the roles each fills, languages, deps). It defaults to human-readable output and accepts **`--format json`** for structured/agent consumption — both forms render from the same data. `web::build_registry_json` and `cli::build_tool_catalog` are the existing JSON/human renderers to converge here.

---

## 8. Dependency doctor (`doctor/`, planned)

v1 scope is **check + advise** (auto-install and a standalone subcommand are deferred — see ROADMAP). The module is split to preserve the purity invariant:

```
doctor/
├── mod.rs      Pure: fn diagnose(selected_deps, env: &Environment) -> Report
├── catalog.rs  Pure: in-code declarative table — dep id → check + install methods
└── probe.rs    Impure: detect OS, available package managers, PATH → Environment
```

- **Inputs:** the `system_deps` of the selected tools (from the registry) + a detected `Environment` (OS, package managers, what's on `PATH`).
- **The catalog is an in-code declarative table** (`catalog.rs`), not an embedded data file — see §8.1 for the rationale. Each entry maps a dep id to: presence-check binaries, a universal `docs` fallback, and an ordered list of install methods. The registry keeps declaring abstract dep ids (`aiken-cli`, `node`, `sbt`); the catalog centralizes the platform/manager quirks in one tested place rather than duplicating them across tool TOMLs.
- **Package managers are a code-defined vocabulary** (`PackageManager` enum: `Brew`, `Apt`, `Dnf`, `Pacman`, `Winget`, `Nix`, plus tool-specific installers like `Aikup` and `CardanoUp`, and a `Curl` script escape hatch). The catalog references managers as enum values (compile-time checked); `probe.rs` owns detecting which are present. Adding a manager is a deliberate code change (a variant + its detection + command formatting).
- **Selection logic (`diagnose`, pure):** a dep is present iff any of its `binaries` is on `PATH`. For each missing dep, recommend the **first install method whose manager is detected** in the `Environment`, list the rest as alternatives, and always show `docs` as the fallback so advice is never empty (FR-20). Version constraints are out of scope for v1 (presence only).
- **Infrastructure deps** are advised via `cardano-up` (an ordinary `PackageManager::CardanoUp` method); `cardano-up` itself is a catalog entry. v1 instructs the user to install it if absent; auto-installing `cardano-up` is deferred to post-v1.
- **Output:** a structured `Report` (missing deps + exact install commands, or "all present") handed to the presenter (§7.2). If a dep can't be satisfied, the doctor states what to install manually and affirms the template is otherwise ready.
- **Boundary:** `mod.rs`/`catalog.rs` are pure and unit-tested by feeding synthetic `Environment`s; only `probe.rs` touches the system. `doctor` depends on `registry`/`contract`, never on `cli`.

### 8.1 Why an in-code table, not an embedded data file

The catalog looks like registry data but differs in the property that matters: it is
bound to the closed `PackageManager` vocabulary and is maintained by core maintainers,
not contributed by the ecosystem.

- **Edits rarely stay data-only.** Adding a *tool* is pure data (reference an existing
  role + a template). Adding catalog knowledge for a genuinely new dep usually arrives
  with a new installer/manager (`aikup`, `ghcup`, `cardano-up`, a `curl|sh` script),
  which needs probe detection + a `PackageManager` variant + command formatting anyway.
  When the data can't change without code changing in lockstep, a separate TOML adds
  parse/validate/keep-in-sync indirection without the registry's payoff.
- **Compile-time safety.** Manager references are enum values — un-typo-able, and a
  renamed/removed manager fails to compile. A TOML catalog would push all of that to
  runtime validation code.
- **Locality & audience.** The entry, the manager it references, the probe that detects
  it, and the formatter that consumes it all live in `doctor/` and co-evolve. Unlike the
  tool registry (the ecosystem contribution surface), the catalog is internal
  maintainer knowledge, so a low-Rust-barrier data file serves no real author.

The table is kept **declarative** (a `static &[DepEntry]`) so it reads like data while
retaining type safety:

```rust
static CATALOG: &[DepEntry] = &[
    DepEntry {
        id: "node", name: "Node.js", binaries: &["node"],
        docs: "https://nodejs.org/en/download",
        install: &[
            Method::Pkg { manager: Brew,   package: "node" },
            Method::Pkg { manager: Apt,    package: "nodejs" },
            Method::Pkg { manager: Winget, package: "OpenJS.NodeJS" },
        ],
    },
    DepEntry {
        id: "aiken-cli", name: "Aiken CLI", binaries: &["aiken"],
        docs: "https://aiken-lang.org/installation-instructions",
        install: &[
            Method::Cmd { manager: Aikup, command: "aikup install" },
            Method::Pkg { manager: Nix,   package: "aiken" },
        ],
    },
];
```

The one remaining runtime check is **referential integrity**: a test asserts every
`system_deps` id used by any tool has a `CATALOG` entry (this would be required with a
data file too).

---

## 9. Version-update check (`cli/update.rs`, planned)

The chosen mechanism for template freshness without runtime template fetching (PRD A-3/FR-24). It is a **thin `cli/` concern** (UX, network — never core):

- Best-effort check against the GitHub releases API on a **background thread** with a short timeout; the notice (if any) prints at the **end**, never gating generation.
- **Cached** (e.g. once/day) to avoid repeated network calls.
- **Fail-silent**: offline, timeout, or error → no-op. It never affects generated output and never blocks. This preserves both offline operation and determinism.

---

## 10. Web UI architecture

The CLI is the **single source of truth**; the web UI never generates a project — it configures, previews structure, and emits a copyable `cardano-init …` command.

### 10.1 Local server (`web/`, exists)

A hand-rolled, zero-dependency HTTP/1.1 server (`TcpListener` + threads) chosen to keep the "single static binary, zero runtime deps" goal. Routes:
- `GET /` → embedded `ui.html`.
- `GET /api/registry` → registry as JSON (prebuilt once).
- `GET /api/plan?…` → runs the **actual Rust planner** and returns the file tree.

Because `/api/plan` calls `scaffold::planner`, the local server's preview is guaranteed to match real generation — no duplicated logic.

### 10.2 Hosted page — **open decision**

A hosted page has no binary behind it, so it needs another way to derive the preview + command string while honoring "no duplicated generation logic." Two candidates, to be decided in TECH_SPEC:

- **(A) WASM core.** Compile the pure registry+planner to WASM; the same Rust logic runs in-browser and natively. Zero duplication; realizes the "future extraction" goal. Cost: a WASM build target + JS bindings.
- **(B) Static registry JSON + JS preview.** Ship the registry as static JSON and reimplement the lightweight tree preview + command assembly in JS. Trivial to host (pure static site) but duplicates preview logic in a second language (drift risk).

The local `serve` path (10.1) ships regardless; the hosted page is additive. If (B) is chosen, the JS preview must be covered by tests that compare it against the planner's output to bound drift.

---

## 11. Testing strategy

- **Unit (pure core):** registry loading (every TOML parses, fields present); context building; planning (exact file set + order); rendering (context + template → expected output); doctor `diagnose` over synthetic environments.
- **Contract compliance (mechanical):** for each template, assert the Justfile exposes `build`/`test`/`dev`/`clean`; for on-chain, assert `just build` produces `blueprint/plutus.json`. This is what lets us avoid testing tool combinations.
- **Per-tool build smoke tests:** scaffold each tool in isolation and, where CI has the toolchain (or via Nix), run `just build && just test`. New tools must add these (PRD SM-1).
- **Determinism / snapshot tests:** `--dry-run` and rendered output compared against committed snapshots for a set of selections; guards §6.4.
- **No combinatorial testing:** composition is guaranteed by the contract, so we verify each tool individually rather than every pair.

---

## 12. Extensibility — adding a tool

1. Add `registry/tools/<tool>.toml` with metadata, `system_deps`, `nix_packages`, and a `[roles.<role>]` block per supported role.
2. Add `templates/<tool>/<role>/` with a `manifest.toml` and template files (conforming to the contract, §4).
3. If the tool introduces a new `system_deps` id, add a `doctor` catalog entry (§8).
4. Add the per-tool tests (§11).
5. Recompile (assets are embedded at compile time).

No CLI/core code changes are required for a new tool. Contract conformance guarantees it composes with every existing tool in other roles.

---

## 13. Open architectural decisions

- **OD-1 — Hosted web strategy:** WASM core (A) vs. static JSON + JS preview (B) (§10.2). Local `serve` is unaffected.
