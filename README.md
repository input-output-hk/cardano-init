# cardano-init

[![CI](https://github.com/input-output-hk/cardano-init/actions/workflows/ci.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/ci.yml)
[![Code Quality](https://github.com/input-output-hk/cardano-init/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/github-code-scanning/codeql)
[![Scheduled Smoke](https://github.com/input-output-hk/cardano-init/actions/workflows/scheduled-smoke.yml/badge.svg)](https://github.com/input-output-hk/cardano-init/actions/workflows/scheduled-smoke.yml)

Scaffold a complete, runnable **Cardano protocol project** in seconds. Pick a tool for each role you need (e.g., on-chain, off-chain, infrastructure, devnet, formal-methods) and `cardano-init` generates a monorepo where every component is already wired together and a small end-to-end example that builds and passes its tests out of the box.

Built for newcomers and coding agents alike.

> [!WARNING]
> **Prototype: do not use yet.** This is an early POC under active design; scope, CLI flags, templates, and generated output **will change** without notice. Targeting a working showcase build (DX.02) and a public Release Candidate (DX.05). See the [Roadmap](docs/ROADMAP.md).

## Quick start (pre-release)

### Install

With Nix (flake):

```bash
# Install the CLI into your profile
nix profile add github:input-output-hk/cardano-init

# Or run it without installing
nix run github:input-output-hk/cardano-init -- --help
```

With Cargo (requires a recent Rust toolchain, 2024 edition):

```bash
# From the published repo
cargo install --git https://github.com/input-output-hk/cardano-init

# Or from a clone
cargo install --path .
```

### Usage

```bash
# Interactive guided setup
cardano-init

# One-shot (non-interactive)
cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci

# Preview what would be generated, without writing
cardano-init --name my-protocol --on-chain aiken --dry-run

# Local web builder (visual configurator → copyable command)
cardano-init web
```

A generated project is driven by [`just`](https://just.systems): `just build`, `just test`, `just dev`, `just clean`.

## How it works

You choose tools for **roles**. Only the directories for selected roles are created, and a base layer (top-level `Justfile`, README, `.env`, `blueprint/`) wires them together.


| Role | What it does | Multiple tools? |
|------|--------------|-----------------|
| `on-chain` | Validators / smart-contract logic; produces the CIP-57 blueprint | no |
| `off-chain` | Transaction building & submission | no |
| `devnet` | Local throwaway chain to develop & integration-test against | no |
| `infrastructure` | Indexers, node providers, chain followers | **yes** |
| `formal-methods` | Specification & verification | no |


## Status

Early prototype. Tools currently in the registry (✅ available · ⬜ planned). Infrastructure is multi-tool and provisioned via `cardano-up`; every other role takes one tool.

| On-chain | Off-chain | Devnet | Infrastructure | Formal methods |
|----------|-----------|--------|----------------|----------------|
| ✅ Aiken | ✅ MeshJS | ✅ Yaci DevKit | ✅ Kupo | ⬜ Blaster |
| ⬜ Scalus | ⬜ Tx3 | | ✅ Ogmios | |
| ⬜ Plinth | ⬜ Scalus | | ✅ Dolos | |
| ⬜ Pebble | ⬜ Lucid Evolution | | ✅ Tx Submit API | |
| ⬜ Plutarch | ⬜ Evolution SDK | | ✅ Cardano Node | |
| ⬜ Opshin | ⬜ Blaze | | ✅ Cardano Node API | |
| | ⬜ Elm Cardano | | ✅ Dingo | |
| | ⬜ PyCardano | | | |


## How it relates to `aikup`, `cardano-up`, and friends

`cardano-init` is a **project scaffolder**, not a version manager or an environment manager. It runs once, generates a wired-together monorepo, and steps out. That makes it complementary to (not a replacement for) the per-tool installers in the ecosystem:


| Tool | Concern | Lifetime |
|------|---------|----------|
| **`cardano-init`** | Generates a multi-tool protocol project, with every role wired together | One-shot, at project creation |
| **`aikup`** | Installs & pins the Aiken toolchain (like `rustup` for Aiken) | Ongoing, per developer machine |
| **`cardano-up`** | Provisions & runs Cardano infrastructure (node, indexers, devnets) | Ongoing, per environment |


These sit at different layers: `cardano-init` decides *what tools your project uses and how they compose*, while `aikup` / `cardano-up` install and manage *the toolchains and infrastructure those tools need*. The two meet at the (planned) dependency [`doctor`](docs/ROADMAP.md): when toolchains are missing, `cardano-init` advises the right installer (`aikup` for Aiken, `cardano-up` for the infrastructure role) rather than reinventing them.

By design, `cardano-init` is **not** a package or version manager: it does not pin or upgrade tool versions, manage dependencies after generation, or migrate existing projects. There is no `cardano-init update`.

## Infrastructure providers

The **infrastructure** role is backed by [`cardano-up`](https://github.com/blinklabs-io/cardano-up) (requires Docker). Unlike the other roles, infrastructure is **multi-tool**: select any combination with repeated `--infra` flags and they are provisioned together as a single project-scoped `cardano-up` context, aggregated into one `infra/` component. Each provider publishes its connection details to the project `.env`, which off-chain components read automatically.

| Provider | Flag | Publishes to `.env` | Upstream |
|----------|------|---------------------|----------|
| Kupo | `--infra kupo` | `INDEXER_URL` | https://github.com/CardanoSolutions/kupo |
| Ogmios | `--infra ogmios` | `OGMIOS_URL` | https://ogmios.dev |
| Dolos | `--infra dolos` | `DOLOS_GRPC_URL`, `NODE_SOCKET_PATH` | https://github.com/txpipe/dolos |
| Tx Submit API | `--infra tx-submit-api` | `TX_SUBMIT_URL` | https://github.com/blinklabs-io/tx-submit-api |
| Cardano Node | `--infra cardano-node` | `NODE_SOCKET_PATH` | https://github.com/IntersectMBO/cardano-node |
| Cardano Node API | `--infra cardano-node-api` | `CARDANO_NODE_API_URL` | https://github.com/blinklabs-io/cardano-node-api |
| Dingo | `--infra dingo` | `INDEXER_URL`, `NODE_SOCKET_PATH` | https://github.com/blinklabs-io/dingo |

```bash
# An indexer + query bridge over a shared node (cardano-up pulls in cardano-node):
cardano-init --name my-protocol --off-chain meshjs --infra kupo --infra ogmios

# Bring the stack up (provisions the services and writes connection details into .env. Long-running):
just -f infra/Justfile dev
```

- **They compose**: `cardano-up` resolves shared dependencies (e.g. `cardano-node`) automatically, so combinations work without per-pair wiring.
- **Dolos and Dingo are self-contained nodes**: No separate `cardano-node`. Each provides its own `NODE_SOCKET_PATH`, and Dingo also serves a Blockfrost-compatible API as `INDEXER_URL`.
- **One chain-index per project**: `INDEXER_URL` has a single slot, so Kupo and Dingo are alternatives, not additive.

## Documentation


| Doc | Purpose |
|-----|---------|
| [docs/PRD.md](docs/PRD.md) | Product requirements: who it's for, problem, scope, success metrics |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design, module structure, data model, pipeline |
| [docs/TECH_SPEC.md](docs/TECH_SPEC.md) | Exact contracts, schemas, algorithms, edge cases |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phases & milestones (DX.02, DX.05) |
| [docs/ADDING_A_TOOL.md](docs/ADDING_A_TOOL.md) | Contributor guide for integrating a new tool |


## Development

```bash
cargo build       # build
cargo test        # run tests
cargo fmt         # format
cargo clippy      # lint
```

A Nix flake is provided. Use `nix develop` for a dev shell with the Rust toolchain, or `nix build .#cardano-init` to build the package.
