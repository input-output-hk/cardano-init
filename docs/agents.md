# For coding agents

`cardano-init` is built to be driven by LLMs from start to finish.

- **Machine-readable interface.** `cardano-init list --format json` enumerates every role and tool. Every command accepts `--format json` and emits a stable envelope with machine-readable error codes and a `context` that says how to fix each error.
- **Non-interactive by design.** One-shot mode (`--name …`) never prompts, so an agent can scaffold in a single call. `--format json` also implies non-interactive: if required input is missing, it errors instead of prompting.
- **Generated `AGENTS.md`.** Every project ships an `AGENTS.md` (plus a `CLAUDE.md` that imports it) tailored to the chosen stack: the layout, the interface contract and its invariants, the exact `just` workflow, official documentation links for each tool, and the [cardano-dev-skills](https://github.com/cardano-foundation/cardano-dev-skills) most relevant to that stack. An agent dropped into a fresh project knows what it is and what to do next.
- **Works with [cardano-dev-skills](https://github.com/cardano-foundation/cardano-dev-skills).** That Cardano Foundation skill set is the *knowledge* layer (writing validators, building transactions, debugging on-chain failures). `cardano-init` is the *scaffolding* layer. The generated `AGENTS.md` points agents at the plugin and the right skills for their stack.

## A typical agent flow

```bash
# 1. Discover what's available
cardano-init list --format json

# 2. Scaffold in one call
cardano-init --format json --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci

# 3. Check toolchains from inside the project
cd my-protocol && cardano-init doctor --format json

# 4. Build and test
just test
```

## JSON envelope

Every `--format json` response is a single JSON object in one of two shapes:

```json
{ "schema_version": 1, "ok": true,  "data":  { } }
{ "schema_version": 1, "ok": false, "error": { "code": "<stable>", "message": "<human>", "context": { } } }
```

`message` is meant for humans and may change. `code` and the `context` keys are part of the contract. The process exit code gives the category (`0` success, `1` runtime, `2` usage). See [Exit codes](commands.md#exit-codes).

## Error codes

| `code` | Exit | `context` | Meaning |
|--------|------|-----------|---------|
| `name_required` | 2 | `{}` | A creation flag was given without `--name`. |
| `invalid_project_name` | 2 | `{ name, reason }` | The name has characters outside `[A-Za-z0-9_-]` or starts with `.`. |
| `unknown_tool` | 2 | `{ tool_id, role, valid_tools }` | No tool with that id for the role. |
| `tool_role_mismatch` | 2 | `{ tool_id, role, valid_roles }` | The tool exists but doesn't fill that role. |
| `fullstack_conflict` | 2 | `{}` | `--fullstack` combined with `--on-chain` / `--off-chain`. |
| `fullstack_unsupported` | 2 | `{ tool_id, valid_tools }` | The tool has no fullstack template. |
| `no_roles_selected` | 2 | `{}` | No role flag was given. |
| `experimental_not_allowed` | 2 | `{ tools, remedy }` | An experimental tool was selected without `--allow-experimental`. |
| `incompatible_tools` | 2 | `{ tools, reason, compatible_providers, compatible_off_chain, remedy }` | The off-chain tool can't reach any selected provider. |
| `project_unrecognized` | 2 | `{ dirs }` | `add` / `remove` couldn't identify a component directory. |
| `slot_occupied` | 2 | `{ role, dir }` | A target directory exists and isn't the recognized component. |
| `nothing_to_change` | 2 | `{}` | The requested `add` / `remove` is a no-op. |
| `dir_exists` | 1 | `{ path }` | The project directory already exists and isn't empty. |
| `worktree_dirty` | 1 | `{ path }` | Uncommitted changes (or not a git repository) and no `--force`. |
| `registry_load` | 1 | `{ file?, detail }` | The embedded registry failed to load. |
| `scaffold_error` | 1 | `{ path?, detail }` | A template asset, render, or I/O failure. |
