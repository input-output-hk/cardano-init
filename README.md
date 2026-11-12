# cardano-init

[![CI](https://github.com/input-output-hk/cardano-init/actions/workflows/ci.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/badge/docs-github%20pages-blue)](https://input-output-hk.github.io/cardano-init/)
[![Code Quality](https://github.com/input-output-hk/cardano-init/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/github-code-scanning/codeql)

### Go from zero to a running Cardano protocol in one command.

Pick a tool for each role you need (on-chain, off-chain, devnet, infrastructure, formal-methods) and `cardano-init` generates a monorepo where every component is **already wired together**, plus a worked end-to-end example that **builds and passes its tests out of the box**.

📖 **Documentation:** [input-output-hk.github.io/cardano-init](https://input-output-hk.github.io/cardano-init/)

<p align="center">
  <img src="assets/demo.gif" alt="cardano-init lists its tools by role, scaffolds an Aiken + MeshJS + Yaci DevKit stack in one command, then just test passes against a local devnet (test run sped up)" width="800">
</p>

```console
$ cardano-init --name my-protocol --on-chain <tool> --off-chain <tool> --devnet <tool>

my-protocol/
├── on-chain/     # Validators (smart contracts)
├── off-chain/    # Tx building (protocol transactions)
├── devnet/       # Local chain for integration testing
├── blueprint/    # shared CIP-57 contract interface
├── spec/         # Protocol specification (plain Markdown; --no-spec to skip)
├── .env          # shared between components
├── Justfile      # Commands to build, test, and clean
├── AGENTS.md     # Agent brief
└── README.md

$ cd my-protocol && just test
  ✓  All tests passed
```

## Why `cardano-init`?

- ⚡ **Zero to running in one command:** A wired-together monorepo that builds and passes its tests immediately.
- 🧩 **Mix and match, freely:** Components talk to a shared *contract*. So, you can combine tools of different roles however you want and it'll just work.
- 🤖 **Agent-native:** Machine-readable JSON on every command and a generated `AGENTS.md` in every project, so coding agents know what the project is and what to do next.
- 🩺 **Never stuck on setup:** A built-in dependency `doctor` detects your toolchains and tells you the exact command to install anything missing or upgrade anything too old.
- 🔁 **Change your mind later:** `add` and `remove` swap a tool or a whole role in an existing project, under a clean-git safety net so every change is a reviewable `git diff`.
- 🧪 **Real example:** Every stack ships the same worked gift-card scenario end-to-end, so what you generate actually runs.

## Quick start

### Run without installing

Using npx:
```bash
npx cardano-init help
```
Using nix:
```bash
 nix run github:input-output-hk/cardano-init -- help
```

### Install

Linux and macOS (x86_64 and arm64):
```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.sh | sh
```

Windows:
```powershell
# Windows (PowerShell)
irm https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.ps1 | iex
```

Prefer a specific version or a manual download? Grab it from the [Releases page](https://github.com/input-output-hk/cardano-init/releases).

<details>
<summary><b>With Nix (flake)</b></summary>

```bash
# Install the CLI into your profile
nix profile add github:input-output-hk/cardano-init

# Or run it once, without installing
nix run github:input-output-hk/cardano-init -- help
```
</details>

<details>
<summary><b>With Cargo</b> (requires a recent Rust toolchain, 2024 edition)</summary>

```bash
# From the published repo
cargo install --git https://github.com/input-output-hk/cardano-init

# Or from a clone
cargo install --path .
```
</details>

### Usage

<details>
<summary><b>Normal workflow</b></summary>

```bash
# 1. Create your project
cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci

# 2. Enter it and change the protocol to your liking
cd my-protocol

# 3. Run tests often
just test
```

Every generated project is driven by [`just`](https://just.systems): `just build`, `just test`, `just clean`.
Missing a dependency, or running one that is too old? Run the built-in dependency doctor, `cardano-init doctor`, and it tells you exactly how to fix it.
New projects start as a git repository with an initial commit, so you can review every later change with `git diff`.

When a newer `cardano-init` is out, you get a notice with the update command for your install method before anything is generated (checked at most once a day; set `CARDANO_INIT_NO_UPDATE_CHECK=1` to turn it off).

</details>

<details>
<summary><b>Other useful commands</b></summary>

```bash
# Check how to use it
cardano-init help

# Check available tooling: compact matrix with the id for each flag, or the detailed list
cardano-init table
cardano-init list

# Interactive guided setup (the easiest way to start)
cardano-init

# Fullstack: one tool for both on-chain and off-chain, as a single `protocol/` component
cardano-init --name my-protocol --fullstack scalus

# Short flags and aliases: -n (--name), -p/-f/--protocol (--fullstack), --on/--off, -i, -d, -e
cardano-init -n my-protocol -p scalus -d yaci

# Preview what would be generated, without writing
cardano-init --name my-protocol --on-chain aiken --dry-run

# Check if you have all dependencies needed (inside generated project);
# --links also lists editor tooling and community channels for your stack
cardano-init doctor
cardano-init doctor --links

# Add/replace tool
cardano-init add --on-chain plinth

# Remove a role (or one provider: --infra kupo)
cardano-init remove --devnet
```

</details>


## Tools

The registry has 21 tools (✅ supported · 🔌 supported, provisioned through cardano-up · 🧪 experimental · 🚧 placeholder). The [tools catalog](docs/user-guide/tools.md) explains each status and lists the [combinations CI tests](docs/user-guide/tools.md#tested-combinations).

| On-chain | Off-chain | Devnet | Infrastructure | Formal methods |
|----------|-----------|--------|----------------|----------------|
| ✅ Aiken | ✅ MeshJS | ✅ Yaci DevKit | 🔌 Kupo | 🚧 Blaster |
| ✅ Scalus | ✅ Scalus | | 🔌 Ogmios | |
| ✅ Plinth | ✅ Evolution SDK | | 🔌 Dolos | |
| ✅ Plutarch | 🧪 Tx3 | | 🔌 Tx Submit API | |
| 🧪 Pebble | ✅ Lucid Evolution | | 🔌 Cardano Node | |
| | ✅ Blaze | | 🔌 Cardano Node API | |
| | ✅ Apollo | | 🔌 Dingo | |
| | 🧪 PyCardano | | | |

You can also check locally with `cardano-init table`, which shows the id to pass to each role flag. Experimental tools need `--allow-experimental` (`-e`).
Infrastructure provisioned via [cardano-up](https://github.com/blinklabs-io/cardano-up).

## Development Documentation

Internal CI (smoke tests, installer recipes, devnet):

[![Tool Smoke](https://github.com/input-output-hk/cardano-init/actions/workflows/tool-smoke.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/tool-smoke.yml)
[![Installer Recipes](https://github.com/input-output-hk/cardano-init/actions/workflows/installer-recipes.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/installer-recipes.yml)
[![Devnet Smoke](https://github.com/input-output-hk/cardano-init/actions/workflows/devnet-smoke.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/devnet-smoke.yml)

Installer Recipes runs each `doctor` install command on one platform per installer. See [Tested install commands](docs/user-guide/commands.md#tested-install-commands) for which platform covers each installer.

| Doc | Purpose |
|-----|---------|
| [docs/SUMMARY.md](docs/SUMMARY.md) | Docs site table of contents (mdBook; preview with `mdbook serve docs`) |
| [docs/design/prd.md](docs/design/prd.md) | Product requirements: who it's for, problem, scope, success metrics |
| [docs/contributing/architecture.md](docs/contributing/architecture.md) | System design, module structure, data model, pipeline |
| [docs/design/tech-spec.md](docs/design/tech-spec.md) | Exact contracts, schemas, algorithms, edge cases |
| [docs/design/roadmap.md](docs/design/roadmap.md) | Phases & milestones (DX.02, DX.05) |
| [docs/contributing/adding-a-tool.md](docs/contributing/adding-a-tool.md) | Contributor guide for integrating a new tool |
| [RELEASING.md](RELEASING.md) | How to cut a release and publish prebuilt binaries (cargo-dist) |


```bash
cargo build       # build
cargo test        # run tests
cargo fmt         # format
cargo clippy      # lint
```

A pre-commit hook blocks commits that would fail CI's `cargo fmt --check`. Enable it once per clone:

```bash
git config core.hooksPath .githooks
```

A Nix flake is provided. Use `nix develop` for a dev shell with the Rust toolchain, or `nix build .#cardano-init` to build the package.
