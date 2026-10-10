# Command reference

```text
cardano-init [OPTIONS]            # create a project (interactive, or one-shot with --name)
cardano-init list [--table]       # list roles and tools
cardano-init doctor               # check this project's dependencies
cardano-init add [ROLE FLAGS]     # add or swap a tool in the current project
cardano-init remove [ROLE FLAGS]  # remove a role or infrastructure provider
cardano-init help [COMMAND]       # print help
```

Every command accepts the global `--format <human|json>` flag (default `human`). See [For coding agents](agents.md) for the JSON output.

## Create a project

```bash
cardano-init                                       # interactive mode
cardano-init --name my-app --on-chain aiken        # one-shot, single role
cardano-init --name my-app --on-chain aiken --off-chain meshjs --nix
cardano-init -n my-app -p scalus -d yaci           # short flags
```

**Mode selection:** if `--name` is given, the command runs in **one-shot** mode and never prompts. Otherwise it runs the **interactive** guided setup. Passing any other creation flag without `--name` is an error (`name_required`).

| Flag | Short / aliases | Description |
|------|-----------------|-------------|
| `--name <NAME>` | `-n` | Project name (required in one-shot mode). Letters, digits, `-` and `_` only; must not start with `.`. |
| `--on-chain <TOOL_ID>` | `--on`, `--onchain` | On-chain tool (e.g. `aiken`, `scalus`). |
| `--off-chain <TOOL_ID>` | `--off`, `--offchain` | Off-chain tool (e.g. `meshjs`, `scalus`). |
| `--fullstack <TOOL_ID>` | `-f`, `-p`, `--protocol` | One tool for both on-chain and off-chain, generated as a single `protocol/` component. Can't be combined with `--on-chain` / `--off-chain`. |
| `--infra <TOOL_ID>` | `-i`, `--infrastructure` | Infrastructure provider. Repeatable: `--infra kupo --infra ogmios`. |
| `--devnet <TOOL_ID>` | `-d` | Devnet tool (e.g. `yaci`). |
| `--formal-methods <TOOL_ID>` | `--formal`, `--formalmethods` | Formal-methods tool (e.g. `blaster`). |
| `--nix` | | Also generate a Nix flake (`flake.nix` + `.envrc`) for the project's dependencies. |
| `--allow-experimental` | `-e`, `--experimental` | Opt in to [experimental tools](how-it-works.md#experimental-tools). |
| `--dry-run` | `--dryrun` | Show what would be generated without writing to disk. |
| `--ignore-warning` | `--ignore-warnings` | Scaffold a combination the [compatibility check](how-it-works.md#compatibility-checks) flags as incompatible. |
| `--format <FORMAT>` | | `human` (default) or `json`. |
| `--help` / `--version` | `-h` / `-V` | Print help / version. |

Short flags and aliases are conveniences only. Errors and JSON output always use the canonical long name. The target directory must not already exist with content (`dir_exists`).

## `list`

List the available roles and tools, including each tool's language, status, documentation, supporting tooling, and community links.

```bash
cardano-init list                 # detailed, grouped by role
cardano-init list --table         # compact matrix of tools by role
cardano-init list --format json   # machine-readable catalog
```

| Flag | Description |
|------|-------------|
| `--table` | Show a compact matrix of tools by role (human output only). |

## `doctor`

Run inside a generated project. It detects which components the project contains, checks that the toolchains they need are installed, and prints the exact installer command for anything missing. It also lists the supporting tools (editor extensions, language servers) and community channels for the detected stack.

```bash
cd my-protocol
cardano-init doctor
cardano-init doctor --format json
```

## `add`

Add a tool for a role, or swap the current one, in the project in the current directory. It takes the same role flags as project creation. Giving a role flag assigns or replaces that role.

```bash
cardano-init add --on-chain plinth            # swap the on-chain tool
cardano-init add --devnet yaci                # add a devnet
cardano-init add --infra kupo --infra ogmios  # add infrastructure providers
cardano-init add --fullstack scalus           # switch to a fullstack protocol/ component
```

| Flag | Short / aliases | Description |
|------|-----------------|-------------|
| `--on-chain <TOOL_ID>` | `--on`, `--onchain` | On-chain tool (replaces the current one, if any). |
| `--off-chain <TOOL_ID>` | `--off`, `--offchain` | Off-chain tool (replaces the current one, if any). |
| `--fullstack <TOOL_ID>` | `-f`, `-p`, `--protocol` | Fullstack tool for both roles as one `protocol/` component. |
| `--infra <TOOL_ID>` | `-i`, `--infrastructure` | Infrastructure provider to add (repeatable). |
| `--devnet <TOOL_ID>` | `-d` | Devnet tool (replaces the current one, if any). |
| `--formal-methods <TOOL_ID>` | `--formal`, `--formalmethods` | Formal-methods tool (replaces the current one, if any). |

Plus the [shared update flags](#shared-update-flags).

## `remove`

Remove a role, or a single infrastructure provider, from the project in the current directory. Role flags take no value. `--infra` takes the provider id.

```bash
cardano-init remove --devnet
cardano-init remove --infra ogmios
cardano-init remove --off-chain --formal-methods
```

| Flag | Short / aliases | Description |
|------|-----------------|-------------|
| `--on-chain` | `--on`, `--onchain` | Remove the on-chain component. |
| `--off-chain` | `--off`, `--offchain` | Remove the off-chain component. |
| `--infra <TOOL_ID>` | `-i`, `--infrastructure` | Remove an infrastructure provider by id (repeatable). |
| `--devnet` | `-d` | Remove the devnet component. |
| `--formal-methods` | `--formal`, `--formalmethods` | Remove the formal-methods component. |

Plus the [shared update flags](#shared-update-flags).

## Shared update flags

`add` and `remove` work out the project's current selection from its component directories, apply the change, and re-wire the shared top-level files (`Justfile`, `README.md`, `AGENTS.md`, …). By default they require a **clean git working tree**, so the whole change can be reviewed with `git diff` and reverted if needed.

| Flag | Short / aliases | Description |
|------|-----------------|-------------|
| `--dry-run` | `--dryrun` | Show the change set without writing anything. |
| `--force` | | Update even when the git working tree has uncommitted changes (or isn't a git repository). |
| `--ignore-warning` | `--ignore-warnings` | Proceed even if the resulting off-chain ↔ provider combination is flagged incompatible. |
| `--allow-experimental` | `-e`, `--experimental` | Allow experimental tools in the resulting selection. |

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success, including `--dry-run` and declining the interactive confirmation. |
| `1` | Runtime error: the target directory exists, a dirty git tree, I/O or rendering failures. |
| `2` | Usage or validation error: unknown tool, tool in the wrong role, invalid project name, missing `--name`, incompatible tools, … |

For the exact reason, use `--format json` and read `error.code` (see [For coding agents](agents.md#error-codes)).
