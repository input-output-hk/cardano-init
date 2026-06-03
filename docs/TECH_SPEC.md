# cardano-init — Technical Specification

**Status:** Draft · **Last updated:** 2026-06-01 · **Owner:** Robertino Martinez

> This document owns the **exact contracts, schemas, algorithms, and edge cases**. For the *why/for whom* see [PRD.md](./PRD.md); for the *how/structure* see [ARCHITECTURE.md](./ARCHITECTURE.md); for sequencing see [ROADMAP.md](./ROADMAP.md). Where this spec describes behavior not yet in the code, it is marked **(planned)**.

---

## 1. Conventions & versioning

- **Schema version.** All machine-readable (`--format json`) output carries a single integer `schema_version`, starting at **1**, global across every command. Additive fields do **not** bump it; removing/renaming/retyping a field or changing semantics does. Agents should tolerate unknown additive fields.
- **Embedded data is versioned with the binary.** The registry (tools + `registry/deps.toml`), templates, and the code-side installer table are compiled in; there is no on-disk schema-version negotiation. A given binary is the single source of truth for what it generates (PRD A-3).
- **Determinism (§11) is a hard contract**, relied on by snapshot tests and agents.

---

## 2. CLI surface

### 2.1 Commands

```
cardano-init [INIT_FLAGS]            # default: one-shot if --name given, else interactive
cardano-init web [--port <u16>]      # local web builder (default port 3000)
cardano-init list [--format <fmt>]   # (planned) capability discovery
```

`--format human|json` (planned) is a global flag; default `human`. `json` **implies non-interactive** — it never prompts; if required input is missing it errors instead.

### 2.2 Init flags (one-shot)


| Flag | Type | Notes |
|------|------|-------|
| `--name <NAME>` | string | Presence selects one-shot mode. Validated per §3.5. |
| `--on-chain <TOOL_ID>` | string | At most one. |
| `--off-chain <TOOL_ID>` | string | At most one. |
| `--infra <TOOL_ID>` | string, repeatable | Multiple allowed (only multi-tool role). |
| `--testing <TOOL_ID>` | string | At most one. |
| `--formal-methods <TOOL_ID>` | string | At most one. |
| `--network <preview\|preprod\|mainnet>` | enum | Default `preview`. |
| `--nix` | bool | Emit `flake.nix` + `.envrc`. |
| `--dry-run` | bool | Plan only; write nothing; exit 0. |


Mode resolution: if `--name` is present → one-shot; else → interactive. Providing any one-shot flag **without** `--name` is a usage error (`name_required`).

### 2.3 Exit codes


| Code | Meaning | Examples |
|------|---------|----------|
| `0` | Success (incl. `--dry-run`, and interactive abort-by-choice) | generated; planned |
| `2` | **Usage / validation** error | bad flag, `unknown_tool`, `tool_role_mismatch`, `no_roles_selected`, `invalid_network`, `invalid_project_name`, `name_required` |
| `1` | **Runtime** error | `dir_exists` (non-empty), registry load failure, render/IO error, web bind failure |


The fine-grained "what" is the JSON `error.code` (§2.5); exit code is only the category. Interactive **abort** (user declines the confirmation prompt) exits `0` with no error — and never occurs in `json`/non-interactive mode.

### 2.4 JSON envelope (planned)

Every `--format json` response is one of:

```json
{ "schema_version": 1, "ok": true,  "data":  { /* command payload */ } }
{ "schema_version": 1, "ok": false, "error": { "code": "<stable>", "message": "<human>", "context": { /* structured */ } } }
```

Success and error are symmetric (same envelope). `message` is human-readable and may change; `code` and `context` keys are part of the contract.

### 2.5 Error catalog

Stable `code`s, their exit category, and the `context` they carry. The `context` is the agent-facing "how to fix" (PRD FR-15).


| `code` | Exit | `context` fields |
|--------|------|------------------|
| `name_required` | 2 | `{ }` |
| `invalid_project_name` | 2 | `{ name, reason }` |
| `unknown_tool` | 2 | `{ tool_id, role, valid_tools: [..] }` |
| `tool_role_mismatch` | 2 | `{ tool_id, role, valid_roles: [..] }` |
| `no_roles_selected` | 2 | `{ }` |
| `invalid_network` | 2 | `{ value, expected: ["preview","preprod","mainnet"] }` |
| `dir_exists` | 1 | `{ path }` (exists and non-empty) |
| `registry_load` | 1 | `{ file?, detail }` |
| `scaffold_error` | 1 | `{ path?, detail }` (asset-not-found, manifest-parse, render, io) |
| `web_bind` | 1 | `{ port, detail }` |


These map 1:1 to the existing `CliError`/`ScaffoldError`/`RegistryError` variants; the planned work is attaching the `code` + serializable `context` and routing through the presenter (ARCHITECTURE §7.2).

---

## 3. Core data model

Exact types; field-level source of truth is `src/registry/types.rs`.

### 3.1 Role

```rust
pub enum Role { OnChain, OffChain, Infrastructure, Testing, FormalMethods }
```


| Variant | kebab (TOML/flag) | dir (`contract::DIR_*`) | Display | multiple |
|---------|-------------------|--------------------------|---------|----------|
| OnChain | `on-chain` | `on-chain` | On-chain | no |
| OffChain | `off-chain` | `off-chain` | Off-chain | no |
| Infrastructure | `infrastructure` | `infra` | Infrastructure | **yes** |
| Testing | `testing` | `test` | Testing | no |
| FormalMethods | `formal-methods` | `formal-methods` | Formal methods | no |


- `Role::ALL` lists variants in the table's order = the **canonical order** (§11).
- `from_kebab` is the only parse path; unknown → `UnknownRoleError` → `RegistryError::UnknownRole`.
- The enum is the sole role vocabulary; the registry references but cannot add roles (ARCHITECTURE §3.1).

### 3.2 Tool & registry TOML schema

```toml
[tool]
id          = "aiken"          # required, unique across registry, kebab
name        = "Aiken"          # required, human display
description = "…"              # required, newcomer-facing
website      = "https://…"     # required
languages    = ["aiken"]       # required, ≥1
system_deps  = ["aiken"]       # required (may be []); abstract dep ids → registry/deps.toml (§9)
nix_packages = ["aiken"]       # optional (default []); nixpkgs attrs for the dev shell

[roles.on-chain]               # ≥1 [roles.<kebab>] block; key validated against Role
template = "aiken/on-chain"    # required; path under templates/
```

`system_deps` is **per-tool, flat** (§9.1) — it applies whenever the tool is selected for any role.

```rust
RoleConfig { template }
ToolDef { id, name, description, website, languages, nix_packages, roles: HashMap<Role, RoleConfig> }
```

Load-time validation (`registry/loader.rs`), all fatal:
- unparseable TOML → `RegistryError::Parse { file }`.
- unknown role key → `RegistryError::UnknownRole { file, role }`.
- duplicate `tool.id` → `RegistryError::DuplicateId { id }`.
- zero tools discovered → `RegistryError::Empty`.

### 3.3 Selection

```rust
struct Selection { project_name: String, assignments: Vec<RoleAssignment>, network: Network, nix: bool }
struct RoleAssignment { role: Role, tool_id: String }
enum Network { Preview, Preprod, Mainnet }   // Display/from_str = lowercase
```

A `Selection` is **valid by construction** (ARCHITECTURE §3.3); there is no separate validation pass. Edges in §3.5/§12.

### 3.4 Role multiplicity & infra duplicates

- Non-infra roles: at most one tool (interactive allows one; one-shot flags are single).
- Infrastructure: ≥1 tools. **Duplicate `--infra X --infra X` is de-duplicated** (keep first occurrence) so the plan can't emit `infra/X/` twice. (Dedupe, not error — idempotent and harmless.)

### 3.5 Project-name rules

Validated by `oneshot::validate_project_name` (also applied to interactive input): 
- Non-empty. 
- Must not start with `.`.
- Characters limited to `[A-Za-z0-9_-]`. This rejects path separators, spaces, leading-dot/hidden, `.`/`..`. 

Violations → `invalid_project_name { name, reason }`. (No length cap or OS-reserved-name check in v1; revisit if needed.)

---

## 4. Template system

### 4.1 Manifest schema

`templates/<tool>/<role>/manifest.toml`:

```toml
[manifest]
summary = "…"          # shown in interactive mode when this template is highlighted

[[files]]
source = "Justfile.jinja"   # path within the template dir
dest   = "Justfile"         # path within the role dir (see §4.4)
```

- Only `source` + `dest` per file. 
- If file ends with `.jinja`, it's rendered (§4.2). 
- `_base/` and `_nix/` layers are emitted by the planner directly (not via a manifest).

### 4.2 Render derivation (the `.jinja` rule)

A file is rendered through MiniJinja **if its `source` ends with `.jinja`**. The planner records this as `FileEntry.render = source.ends_with(".jinja")`. Authoring contract:

- Name a file `foo.ext.jinja` → it is rendered; set `dest = "foo.ext"` (drop `.jinja`).
- Name it `foo.ext` → copied verbatim (bytes), may be **binary**.
- Rendered templates **must be valid UTF-8** (enforced; non-UTF-8 `.jinja` is a bug). Binary assets must therefore not use the `.jinja` suffix.

### 4.3 Rendering contract

MiniJinja environment (planned config):
- **Undefined = strict**: referencing an undefined variable is a render error (caught at generation, not in the generated project). Authors guard optionals with `{% if has_* %}`.
- **Autoescape off**: output is code/config, not HTML — no entity escaping.
- **Newlines normalized to `\n` (LF)**, UTF-8, for byte-identical cross-platform output (§11).

### 4.4 Path safety & destinations

- `dest` is resolved **relative to the role dir** (`on-chain/`, `off-chain/`, `test/`, `formal-methods/`); for infrastructure, relative to `infra/<tool_id>/`.
- `dest` MUST be relative and MUST NOT contain `..` or a leading `/` (no escaping the project root). Enforced + tested. (Manifests are first-party today, but the check is cheap insurance and required if templates ever become third-party.)
- Base/optional layer dests are fixed (§6).

### 4.5 Executable bit

The writer sets **no executable bits** (and `just` doesn't need them). Templates must invoke helper scripts through an interpreter (`sh scripts/x.sh`, `node …`, `just …`), never `./x.sh`. Rationale: portability (exec bits don't exist on Windows) + trivial writer. Documented as a template-authoring rule.

### 4.6 TemplateContext — the template-authoring API

The entire surface available to templates (`scaffold/context.rs`, `Serialize`):

```rust
struct TemplateContext {
    project_name: String,
    network: String,                 // "preview" | "preprod" | "mainnet"

    has_on_chain: bool, has_off_chain: bool, has_infra: bool,
    has_testing: bool,  has_formal_methods: bool,

    on_chain: Option<RoleContext>,
    off_chain: Option<RoleContext>,
    infra_tools: Vec<RoleContext>,   // 0..n, canonical order (§11)
    testing: Option<RoleContext>,
    formal_methods: Option<RoleContext>,

    blueprint_path: String,          // "blueprint/plutus.json" (contract constant)
    env_vars: <ordered map>,         // see §6.3; iterated in sorted-key order

    nix: bool,
    nix_packages: Vec<String>,       // deduped union across selected tools, first-seen order
}

struct RoleContext { tool_id, tool_name, language, dir }   // language = tool.languages[0]
```

This struct is the contract. Adding a field is additive; renaming/removing is a breaking template-API change.

---

## 5. Registry loading

`Registry::load()` iterates embedded `registry/tools/*.toml`, builds `Vec<ToolDef>` plus indexes `by_id: HashMap<String, usize>` and `by_role: HashMap<Role, Vec<usize>>`. 
Accessors: `get(id)`, `tools_for_role(role)`, `all_tools()`. Immutable after load.
Determinism note: any consumer that emits tools/roles must sort (§11), since `by_role` order follows asset-iteration order.

---

## 6. Scaffolding pipeline contracts

### 6.1 Plan order (canonical)

`planner::plan` emits `FileEntry`s in exactly this order:

1. **Base layer** (always): `Justfile`, `README.md`, `.gitignore`, `.env`.
2. **Blueprint dir**: `blueprint/.gitkeep` — **if  any non-infrastructure role is present** (§6.2). Source is `TemplateSource::Inline(empty)`.
3. **Role layers**: assignments processed in **`Role::ALL` order** (not flag order). For each, read the template manifest and append its files (rendered per §4.2). Infra tools nested under `infra/<tool_id>/`, **tools sorted by `tool_id`** (§11).
4. **Optional layer**: if `nix`, `flake.nix` (rendered) + `.envrc` (`Inline "use flake\n"`).

`--dry-run` returns this `FilePlan` (no rendering, no I/O).

### 6.2 `blueprint/` predicate

```
blueprint_present  ⇔  assignments.iter().any(|a| a.role != Role::Infrastructure)
```

The **directory** (via `.gitkeep`) exists for every project except infrastructure-only; the **`plutus.json` file** is produced by on-chain `build` and may be absent, so consumers must tolerate a missing file (§7).

### 6.3 `.env` seeding

`.env` is always written (base layer), seeded by `context.rs` with:
`CARDANO_NETWORK=<network>`, and empty `INDEXER_URL=`, `INDEXER_PORT=`, `NODE_SOCKET_PATH=`. Infrastructure `dev` fills the connection vars at runtime (§7).
Emitted in **sorted key order** for determinism.

### 6.4 Write semantics & target dir

- Write is the only phase with side effects: `create_dir_all(parent)` then write bytes. No chmod (§4.5).
- **Target directory policy:** generate into `./<project_name>/`. If it does not exist, create it. If it exists and is **empty**, proceed. If it exists and is **non-empty**, fail with `dir_exists` (exit 1). No `--force` in v1. Never overwrites user files.

---

## 7. Interface contract (concrete)

Constants (`contract.rs`): 
- `BLUEPRINT_PATH = "blueprint/plutus.json"`; 
- dirs `on-chain|off-chain|infra|test|formal-methods`;
- env `INDEXER_URL`, `INDEXER_PORT`, `NODE_SOCKET_PATH`, `CARDANO_NETWORK`.

**Every component Justfile** exposes `build`, `test`, `dev`, `clean` and works standalone (its `just build` succeeds with no other roles present). A target that is a no-op for a tool still exists (may print a message).

- **On-chain** `build` writes `../blueprint/plutus.json`.
- **Off-chain / testing / formal-methods** read `../blueprint/plutus.json` and `../.env` if present; degrade gracefully if absent.
- **Infrastructure** `dev` writes the standard connection vars into `../.env`.

### 7.1 Top-level Justfile aggregation

- `build`: Each present component's `build`, on-chain first (so the blueprint exists for consumers), then off-chain/testing/formal in `Role::ALL` order.
- `test`: Each present component's `test`.
- `clean`: Each component's `clean`, then `rm -f blueprint/plutus.json`.

### 7.2 `just dev` semantics (the one orchestrated task)

Top-level `dev` brings up **infrastructure only** — the shared services everything else talks to. App/watch `dev` for on-chain/off-chain/testing/formal are run **per component** by the developer (`just -f off-chain/Justfile dev`, etc.), documented in the README.

This does **not** relax §7: every component still implements its own `dev` target (a no-op that prints guidance where a tool has no watch mode), so `just dev` works in any component directory. The top level simply doesn't *aggregate* the non-infra `dev` targets — only infrastructure's.

Behavior by infra tool count:


| Infra tools | `just dev` does |
|-------------|-----------------|
| 0 | Prints the available per-component `dev` commands; orchestrates nothing (nothing shared/long-running). |
| 1 | Delegates to `just -f infra/<tool>/Justfile dev`. **No orchestrator dependency.** |
| ≥2 | A `process-compose.yaml` is generated (each infra tool's `dev`, with `depends_on`/health-checks for ordering). `just dev` runs `process-compose up` **if it is on `PATH`**; otherwise it prints how to install process-compose, or how to run each infra `dev` manually. **Recommended, not required — degrades gracefully.** |


`process-compose` is a **recommended (optional) dependency**, surfaced by the doctor only in the ≥2-infra case (§9.1) — never a hard requirement, since `just dev` is outside the build/test acceptance bar. It is not owned by any tool's `system_deps`, and is **not** Nix-forced — the catalog offers go/brew/sh/nix install methods (§9). Build/test never touch it.

---

## 8. `list` subcommand schema (planned)

`cardano-init list` (human default) / `cardano-init list --format json`:

```json
{ "schema_version": 1, "ok": true, "data": {
  "roles": [
    { "id": "on-chain",       "dir": "on-chain",       "display": "On-chain",       "multiple": false },
    { "id": "infrastructure", "dir": "infra",          "display": "Infrastructure", "multiple": true  }
    /* … all Role::ALL, in canonical order … */
  ],
  "tools": [
    { "id": "aiken", "name": "Aiken", "description": "…", "website": "https://…",
      "languages": ["aiken"], "roles": ["on-chain"] }
    /* … tools sorted by id; each tool's roles sorted … */
  ]
}}
```

Renders from the same data as `web::build_registry_json`; `roles[].multiple` is `true` only for infrastructure.

---

## 9. Dependency doctor (planned)

### 9.1 Dependency sets (required vs recommended)

Pure functions of the selection. Two tiers:

```
required_deps    = {"just"}                                  // universal task runner
                 ∪ (tool.system_deps for each selected tool) // unioned, deduped
recommended_deps = {"process-compose"}  if infra_tool_count ≥ 2   else {}
```

- **Required** deps gate the build/test acceptance bar (SM-1); their absence is reported prominently. `just` is a base/derived required dep (every project needs the task runner).
- **Recommended** deps improve the experience but are **never required**. `process-compose` is recommended only when ≥2 infra tools are selected — it smooths multi-service `just dev`, which is *outside* the build/test acceptance bar (§7.2). Its absence is a soft note, never a blocking "missing dependency."

`just` and `process-compose` are **base/derived deps owned by no tool**; both have entries in `registry/deps.toml` like any dep. (`cardano-up` is reached as an *installer* and is itself a dep entry, rather than added to either set directly.)

### 9.2 Catalog = installers (code) + recipes (data)

The catalog is a small **graph**. An *installer* is itself a kind of dependency, so the two node types are: code-defined installers, and data-defined dep recipes that reference them.

**Installers (code, `installers.rs`)** — a closed vocabulary. Per installer: the binaries that mean "available", a command template, and a `bootstrap` list of dep ids (**empty ⇒ terminal**, i.e. detect-only/never auto-installed; **non-empty ⇒ bootstrappable** by installing any one of those deps, tried in order):

```rust
enum Installer { Brew, Apt, Dnf, Pacman, Winget, Nix, Go, Cargo, Npm, Aikup, CardanoUp, Curl, PowerShell }

struct InstallerDef {
    detect:    &[&str],                 // ["npm"] — installer available if one is on PATH
    template:  fn(arg: &str) -> String, // Brew → "brew install {arg}"; Curl → "curl -sSfL {arg} | sh"
    bootstrap: &[&str],                 // dep ids that provide this installer; [] ⇒ terminal
}
```


| Installer | template (`{arg}`) | `bootstrap` |
|-----------|--------------------|-------------|
| `Brew` | `brew install {arg}` | `[]` (terminal) |
| `Apt` | `sudo apt install -y {arg}` | `[]` |
| `Dnf`/`Pacman`/`Winget` | native install of `{arg}` | `[]` |
| `Nix` | `nix profile install nixpkgs#{arg}` | `[]` |
| `Curl` | `curl -sSfL {arg} \| sh` | `[]` |
| `PowerShell` | `powershell -c "irm {arg} \| iex"` | `[]` |
| `Npm` | `npm install -g {arg}` | `["node"]` |
| `Cargo` | `cargo install {arg}` | `["rustup", "rust"]` |
| `Go` | `go install {arg}` | `["go"]` |
| `Aikup` | `aikup install {arg}` | `["aikup"]` |
| `CardanoUp` | `cardano-up install {arg}` | `["cardano-up"]` |


The `arg`'s meaning is the installer's: a package name for managers, an installer-script URL for `Curl`/`PowerShell`, a target for `Aikup`/`CardanoUp`. Adding an installer is a deliberate code change, only when a real recipe needs it (same discipline as roles).

**Recipes (data, `registry/deps.toml`)** — keyed by dep id; `install` is an ordered list of single-key `{ installer = arg }` methods (order = preference). Installer keys are validated against the `Installer` enum at load (unknown → load error):

```toml
[node]  
binaries=["node"]
docs="https://nodejs.org/en/download"
install=[ {brew="node"}, {apt="nodejs"}, {winget="OpenJS.NodeJS"}, {nix="nodejs"} ]

[aikup] 
binaries=["aikup"]
docs="https://aiken-lang.org/installation-instructions"
install=[ {npm="@aiken-lang/aikup"}, {curl="https://install.aiken-lang.org"}, {powershell="https://windows.aiken-lang.org"} ]

[aiken] 
binaries=["aiken"]
docs="https://aiken-lang.org/installation-instructions"
install=[ {aikup="latest"}, {nix="aiken"} ]

[just]
binaries=["just"] 
docs="https://just.systems"
install=[ {brew="just"}, {apt="just"}, {cargo="just"}, {nix="just"} ]

[process-compose]
binaries=["process-compose"]
docs="https://f1bonacc1.github.io/process-compose/"
install=[ {brew="process-compose"}, {go="github.com/f1bonacc1/process-compose@latest"}, {nix="process-compose"} ]
```

```rust
struct DepRecipe { binaries: Vec<String>, docs: String, install: Vec<(Installer, String)> }  // ordered
type DepCatalog = HashMap<String, DepRecipe>;   // dep id → recipe (loaded from registry/deps.toml)
```

Installers (logic, closed vocab) are code; recipes (which installer + arg per dep) are data, so a new tool whose deps install via existing installers is **pure data** (ARCHITECTURE §8.1). Shared deps (`node`, `jvm`) are defined once and referenced by many tools' `system_deps`.

### 9.3 Environment (impure probe)

```rust
struct Environment { os: Os, installers: HashSet<Installer> /* detected present */ }
enum Os { Linux, MacOs, Windows, Other }
```

`probe.rs` detects the OS and which installers are present (installer available if one of its `detect` binaries is on `PATH`). A **dep** is present if one of its `binaries` is on `PATH`. No execution, no version (v1).

### 9.4 Resolver (pure, recursive) & Report

```
resolve(dep_id, env, catalog, seen) -> Plan | Unresolved:
    if dep_id ∈ seen:                  return Unresolved          // cycle guard
    rec = catalog[dep_id]
    if any(rec.binaries on PATH):      return Plan([])            // already present
    for (installer, arg) in rec.install:                          // ordered preference
        cmd = installer.template(arg)
        if installer ∈ env.installers:                            // usable right now
            return Plan([ {installer, cmd} ])
        for bdep in installer.bootstrap:                          // [] ⇒ skip (terminal)
            sub = resolve(bdep, env, catalog, seen ∪ {dep_id})
            if sub is Plan:            return Plan(sub.steps + [ {installer, cmd} ])
    return Unresolved                                             // → docs fallback

all_required_present = every required dep resolves to Plan([])   // i.e. already present
```

Picking a single method per dep is exactly why the `nix` path for `aiken` needs no `aikup`. When neither `nix` nor `aikup` is present, the `aikup` installer is bootstrapped (via `node`/`npm`, etc.), producing a multi-step plan.

```json
{ "schema_version": 1, "ok": true, "data": {
  "all_required_present": false,
  "deps": [
    { "id": "node",  "required": true,  "present": true },
    { "id": "aiken", "required": true,  "present": false,
      "plan": [ { "installer": "npm",   "command": "npm install -g @aiken-lang/aikup" },
                { "installer": "aikup", "command": "aikup install latest" } ],
      "docs": "https://aiken-lang.org/installation-instructions" },
    { "id": "process-compose", "required": false, "present": false,
      "reason": "orchestrates multiple infrastructure services for `just dev`",
      "plan": [ { "installer": "brew", "command": "brew install process-compose" } ],
      "docs": "https://f1bonacc1.github.io/process-compose/" }
  ]
}}
```

- `plan` = the ordered, possibly multi-step install sequence the resolver produced **for this host** (empty when present; omitted/empty with only `docs` when unresolved).
- `required` distinguishes tiers; `all_required_present` ignores recommended deps. The presenter shows missing required deps prominently and recommended ones as a soft note with `reason`. `docs` is always available so advice is never empty (FR-20).
- Doctor output is **host-dependent by design** (it reflects detected installers) and is **not** part of the byte-identical generation contract (§11). v1 prints the plan; v2 executes it (same data, same resolver).

### 9.5 Referential integrity (tests)

- Every `system_deps` id (plus base deps `just` and `process-compose`) has a `registry/deps.toml` entry.
- Every installer named in any recipe is an `Installer` enum variant (also enforced at load).
- Every dep id in any installer's `bootstrap` list has a recipe entry.
- The dep graph resolves without infinite recursion (the resolver's cycle guard is exercised by a test).

---

## 10. Version-update check (planned, `cli/update.rs`)

Goal: surface "a newer `cardano-init` is available" **before generation**, so the user can update and regenerate with newer templates rather than discovering it post-write (and deleting/regenerating). Constraints: never block agents/CI, never alter generated output, bounded latency, offline-safe.

- **Gating.** Runs only when stdout is a **TTY and not `--format json`** (interactive, or human one-shot). For json/non-TTY (agents/CI) it is skipped entirely — no network, no spinner, no notice.
- **Cached once/day.** A small file under the OS cache dir (e.g. `~/.cache/cardano-init/update-check`) stores last-checked date + latest-seen version. Already checked today → cached result, **zero network, zero latency**.
- **Surfaced before the write phase; latency hidden where possible:**
  - **Interactive:** the check fires async at process start and completes during tool selection; the notice (if any) shows before generation with **no added latency**.
  - **Human one-shot:** no think-time to mask it, so the async check is joined with a **≤1s deadline** behind a `Checking for updates…` spinner before writing; on hit → notice then generate; on timeout/offline → proceed (worst case **+1s, once/day**).
- **Informational, not a gate.** The notice prints the newer version + suggested update command, then continues with the current version (the user may Ctrl-C to update first). It never blocks beyond the deadline and never alters generated output (determinism, A-3).
- **Fail-silent.** Best-effort GET of the latest release tag (GitHub releases API); offline/timeout/parse error → no-op. Requires a minimal HTTPS client (impl detail; off the generation path).
- `--dry-run` writes nothing, so the delete/regenerate concern doesn't apply; the notice may still show (same gating).

---

## 11. Determinism & reproducibility

Identical `(binary, Selection)` ⇒ byte-identical tree. Rules:

1. **Plan order** is fixed (§6.1): base → blueprint → roles in `Role::ALL` order → optional.
2. **Assignments are reordered into `Role::ALL` order** for emission (user/flag order does not affect output).
3. **Infrastructure tools sorted by `tool_id`**.
4. **Maps emitted in sorted-key order** — `env_vars` and any `HashMap` reaching output use a sorted/canonical view (spec: back `env_vars` with `BTreeMap` or sort at the boundary).
5. **`nix_packages`**: dedup preserving first-seen order across assignments (already so).
6. **Newlines LF, UTF-8, single trailing newline**; no timestamps, no absolute paths, no host-dependent content in generated files.
7. **Snapshot tests** over `--dry-run` and rendered output for a fixed set of selections guard all of the above.

> Implementation note: today `roles` is a `HashMap` and `assignments` keep flag order; realizing rules 2–4 (and `env_vars` ordering) is tracked work.

---

## 12. Edge-case matrix


| Situation | Behavior | Code / exit |
|-----------|----------|-------------|
| One-shot flags without `--name` | error | `name_required` / 2 |
| `--name` invalid (empty, `.`-lead, space, `/`) | error | `invalid_project_name` / 2 |
| Unknown tool id | error, list valid tools for role | `unknown_tool` / 2 |
| Tool doesn't fill the role | error, list tool's valid roles | `tool_role_mismatch` / 2 |
| No roles selected | error | `no_roles_selected` / 2 |
| Bad `--network` | error | `invalid_network` / 2 |
| `--infra X --infra X` | de-duplicated (keep first) | ok |
| Infra-only selection | no `blueprint/` dir | ok |
| Target dir absent | created | ok |
| Target dir empty | proceed | ok |
| Target dir non-empty | refuse | `dir_exists` / 1 |
| `--dry-run` | print plan, write nothing | ok / 0 |
| Interactive: user declines confirm | abort, no write | / 0 |
| Registry empty / dup id / unknown role (build-time data) | fail load | `registry_load` / 1 |
| Manifest missing/malformed, asset missing, render fails | fail | `scaffold_error` / 1 |
| `web` port in use | fail, suggest `--port` flag | `web_bind` / 1 |
| `json` mode but interactive input needed | error, never prompt | usage / 2 |


---

## 13. Web API (local server)

`web/` serves the builder; endpoints are **internal** (consumed by the bundled `ui.html`), so they use bare payloads, not the §2.4 envelope:

- `GET /` → `ui.html`.
- `GET /api/registry` → `{ "tools": [ … ] }` (prebuilt once).
- `GET /api/plan?on_chain=&off_chain=&infra=a,b&testing=&formal_methods=&network=&nix=&name=` → `{ "files": [ … ] }`, computed by the **real `scaffold::planner`** (no duplicated logic). Invalid input → `{ "error": "…" }` with 4xx.

The command-string the UI emits, and the previewed tree, must equal what the CLI produces for the same selection. Hosted-page strategy is **OD-1, open** (ARCHITECTURE §10.2).

---

## 14. Non-functional

- **Language/edition:** Rust 2024 edition (the code uses let-chains and `&[Role]` consts). MSRV pinned in `Cargo.toml`/CI to the stable that supports those (≥1.88).
- **Dependencies (current):** `clap`, `dialoguer`, `minijinja`, `serde`, `serde_json`, `toml`, `rust-embed`, `console`, `thiserror`; `tempfile` (dev). **Planned additions:** a minimal HTTPS client for §10 (e.g. `ureq`), kept off the generation path. The generated *project* may depend on `process-compose` (§7.2) and always on `just`.
- **Distribution:** single statically-linked binary; generation works fully offline.
- **Platforms:** Linux, macOS, Windows. Exec-bit-free output (§4.5) and LF normalization (§11) keep behavior identical across them.

---

## 15. Open technical decisions

- **OD-1 — Hosted web strategy** (WASM core vs. static-JSON+JS preview) — ARCHITECTURE
  §10.2. The only open architectural decision; affects §13 hosted delivery.
