# cardano-init

Scaffold a complete, runnable **Cardano protocol project** in seconds. Pick a tool for each role you need (e.g., on-chain, off-chain, infrastructure, testing, formal-methods) and `cardano-init` generates a monorepo where every component is already wired together and a small end-to-end example that builds and passes its tests out of the box.

Built for newcomers and coding agents alike.

> [!WARNING]
> **Prototype: do not use yet.** This is an early POC under active design; scope, CLI flags, templates, and generated output **will change** without notice. Targeting a working showcase build (DX.02) and a public Release Candidate (DX.05). See the [Roadmap](docs/ROADMAP.md).

## Quick start (pre-release)

Requires a recent Rust toolchain (2024 edition). From a clone:

```bash
# Interactive guided setup
cargo run

# One-shot (non-interactive)
cargo run -- --name my-protocol --on-chain aiken --off-chain meshjs --nix

# Preview what would be generated, without writing
cargo run -- --name my-protocol --on-chain aiken --dry-run

# Local web builder (visual configurator → copyable command)
cargo run -- web
```

A generated project is driven by [`just`](https://just.systems): `just build`, `just test`, `just dev`, `just clean`.

## How it works

You choose tools for **roles**. Only the directories for selected roles are created, and a base layer (top-level `Justfile`, README, `.env`, `blueprint/`) wires them together.


| Role | What it does | Multiple tools? |
|------|--------------|-----------------|
| `on-chain` | Validators / smart-contract logic; produces the CIP-57 blueprint | no |
| `off-chain` | Transaction building & submission | no |
| `infrastructure` | Indexers, node providers, chain followers | **yes** |
| `testing` | Contract & integration testing | no |
| `formal-methods` | Specification & verification | no |


## How it relates to `aikup`, `cardano-up`, and friends

`cardano-init` is a **project scaffolder**, not a version manager or an environment manager. It runs once, generates a wired-together monorepo, and steps out. That makes it complementary to (not a replacement for) the per-tool installers in the ecosystem:


| Tool | Concern | Lifetime |
|------|---------|----------|
| **`cardano-init`** | Generates a multi-tool protocol project, with every role wired together | One-shot, at project creation |
| **`aikup`** | Installs & pins the Aiken toolchain (like `rustup` for Aiken) | Ongoing, per developer machine |
| **`cardano-up`** | Provisions & runs Cardano infrastructure (node, indexers, devnets) | Ongoing, per environment |


These sit at different layers: `cardano-init` decides *what tools your project uses and how they compose*, while `aikup` / `cardano-up` install and manage *the toolchains and infrastructure those tools need*. The two meet at the (planned) dependency [`doctor`](docs/ROADMAP.md): when toolchains are missing, `cardano-init` advises the right installer (`aikup` for Aiken, `cardano-up` for the infrastructure role) rather than reinventing them.

By design, `cardano-init` is **not** a package or version manager: it does not pin or upgrade tool versions, manage dependencies after generation, or migrate existing projects. There is no `cardano-init update`.

## Documentation


| Doc | Purpose |
|-----|---------|
| [docs/PRD.md](docs/PRD.md) | Product requirements: who it's for, problem, scope, success metrics |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design, module structure, data model, pipeline |
| [docs/TECH_SPEC.md](docs/TECH_SPEC.md) | Exact contracts, schemas, algorithms, edge cases |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phases & milestones (DX.02, DX.05) |
| [docs/ADDING_A_TOOL.md](docs/ADDING_A_TOOL.md) | Contributor guide for integrating a new tool |


## Status

Early prototype. Currently in the registry: 

**On-chain:**
- [ ] Aiken
- [ ] Scalus
- [ ] Plinth
- [ ] Pebble
- [ ] Plutarch
- [ ] Opshin

**Off-chain:**
- [ ] MeshJS
- [ ] Tx3
- [ ] Scalus
- [ ] Lucid Evolution
- [ ] Evolution SDK
- [ ] Blaze
- [ ] Elm Cardano
- [ ] PyCardano

**Testing:**
- [ ] Scalus

**Infrastructure:**
- [ ] Cardano Node
- [ ] Cardano Node CLI
- [ ] Cardano Node API
- [ ] Cardano Node Tx Submit API
- [ ] Dingo
- [ ] Dolos
- [ ] Mithril Client
- [ ] Kupo
- [ ] Ogmios
- [ ] Yaci DevKit

**Formal Methods:**
- [ ] Blaster

## Development

```bash
cargo build       # build
cargo test        # run tests
cargo fmt         # format
cargo clippy      # lint
```
