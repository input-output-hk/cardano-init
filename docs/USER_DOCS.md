# cardano-init — User Documentation

How `cardano-init` works, how it fits into an agent workflow, how it relates to the rest of the Cardano tooling ecosystem, and the infrastructure providers it supports, and the supporting tools (editor extensions, CBOR inspectors) around it. For installation and quick start, see the [README](../README.md).

## How it works

You choose tools for **roles**. Only the directories for selected roles are created, and a base layer (top-level `Justfile`, README, `.env`, `blueprint/`) wires them together.

| Role | What it does | Multiple tools? |
|------|--------------|-----------------|
| `on-chain` | Validators / smart-contract logic; produces the CIP-57 blueprint | no |
| `off-chain` | Transaction building & submission | no |
| `devnet` | Local throwaway chain to develop & integration-test against | no |
| `infrastructure` | Indexers, node providers, chain followers | **yes** |
| `formal-methods` | Specification & verification | no |

The magic is the **interface contract**: on-chain components always emit `blueprint/plutus.json`, and whatever provisions a local endpoint writes standard vars (like `INDEXER_URL`) into `.env`. Consumers read those and degrade gracefully when blank. Because components talk to the *contract* rather than to each other, mixing and matching tools Just Works.

```mermaid
flowchart LR
    OC["on-chain<br/>(validators)"] -->|"blueprint/plutus.json"| BP[["blueprint/"]]
    BP --> OFF["off-chain<br/>(tx building)"]
    INFRA["devnet / infrastructure<br/>(local endpoint)"] -->|"INDEXER_URL, …"| ENV[[".env"]]
    ENV --> OFF
```

Every tool writes to and reads from those two seams (`blueprint/` and `.env`) — never from each other — so a swap on one side never breaks the other.

Every on-chain and off-chain template ships the **same worked example — a gift card**: a one-shot minting policy that mints a unique token gated by a specific UTxO, plus a `redeem` validator that releases a locked gift when the token is burned. Because all tools demonstrate the same scenario with a shared parameter ABI, a generated project builds and tests end-to-end, and any on-chain tool composes with any off-chain one (e.g. an Aiken contract driven by the Scalus off-chain, or a Scalus contract driven by the MeshJS off-chain).

**Fullstack tools.** Some tools (e.g. Scalus) implement both on-chain and off-chain in one language. Pick such a tool for both roles (e.g., `--fullstack scalus`, or `--on-chain scalus --off-chain scalus`) and instead of two folders you get a single unified **`protocol/`** component. It still writes the standard `blueprint/plutus.json` and reads `.env`, so it composes with devnet, formal-methods, and infrastructure.

**Compatibility checks.** Not every off-chain tool can talk to every provider and each devnet/infra provider serves some set of them. `cardano-init` knows these and **stops before generating** a project whose off-chain tool can't reach a chain from its selected providers. The error lists the providers that *would* work; pass `--ignore-warning` to scaffold the combination anyway. Interactive mode simply hides the incompatible options.

## For coding agents

`cardano-init` is built to be driven by LLMs, end to end:

- **Machine-readable interface.** `cardano-init list --format json` enumerates every role and tool; any command accepts `--format json` and emits a stable envelope with machine-readable error `code`s and a `context` that says how to fix each error. One-shot mode (`--name …`) is fully non-interactive, so an agent can scaffold in a single call. See [TECH_SPEC](TECH_SPEC.md) §2.
- **Generated `AGENTS.md`.** Every project ships an `AGENTS.md` (plus a `CLAUDE.md` that imports it) tailored to the chosen stack: the layout, the interface contract and its invariants, the exact `just` workflow, per-tool official doc links, and the [cardano-dev-skills](https://github.com/cardano-foundation/cardano-dev-skills) most relevant to that stack. An agent dropped into a fresh project knows what it is and what to do next.
- **Works in tandem with [cardano-dev-skills](https://github.com/cardano-foundation/cardano-dev-skills).** That Cardano Foundation skill set is the *knowledge* layer (writing validators, building transactions, debugging on-chain failures); `cardano-init` is the *scaffolding* layer. The generated `AGENTS.md` points agents at the plugin and the right skills for the stack they're in.

## How it relates to `aikup`, `cardano-up`, and friends

`cardano-init` is a **project scaffolder**, not a version manager or an environment manager. It runs once, generates a wired-together monorepo, and steps out. That makes it complementary to (not a replacement for) the per-tool installers in the ecosystem.

These sit at different layers: `cardano-init` decides *what tools your project uses and how they compose*, while `aikup` / `cardano-up` install and manage *the toolchains and infrastructure those tools need*. The two meet at the dependency [`doctor`](ROADMAP.md): when toolchains are missing, `cardano-init` advises the right installer (`aikup` for Aiken, `cardano-up` for the infrastructure role) rather than reinventing them.

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

- **Dolos and Dingo are self-contained nodes**: No separate `cardano-node`. Each provides its own `NODE_SOCKET_PATH`, and Dingo also serves a Blockfrost-compatible API as `INDEXER_URL`.
- **One chain-index per project**: `INDEXER_URL` has a single slot, so Kupo and Dingo are alternatives, not additive.

## Supporting tools

Beyond the toolchains themselves, there's a layer of supporting tools: editor extensions, language servers, and inspectors for the CBOR that Cardano runs on.

**Per-language editor tooling.** `cardano-init doctor` lists the editor tooling for the stack it detects in your project (text output under *Supporting tools*, and `deps[].support` in `--format json`). The links are data in [`registry/deps.toml`](../registry/deps.toml) (the `support` field), so they stay in one place:

| Language (tool) | Supporting tools |
|-----------------|------------------|
| Aiken | [VS Code extension](https://marketplace.visualstudio.com/items?itemName=TxPipe.aiken) · [Neovim plugin](https://github.com/aiken-lang/editor-integration-nvim) · [Zed extension](https://github.com/aiken-lang/zed-aiken) · [Emacs mode](https://github.com/aiken-lang/aiken-mode) · [JetBrains plugin](https://github.com/MedusaLabs-cardano/intellij_aiken) · [Online playground](https://play.aiken-lang.org). Any other LSP-capable editor (e.g. Helix): run the built-in `aiken lsp` |
| Tx3 | [VS Code extension](https://marketplace.visualstudio.com/items?itemName=TxPipe.tx3) · [Language server](https://github.com/tx3-lang/tx3-lsp) |
| Haskell (Plinth) | [Haskell Language Server](https://haskell-language-server.readthedocs.io) · [VS Code extension](https://marketplace.visualstudio.com/items?itemName=haskell.haskell) |
| Scala (Scalus) | [Metals](https://scalameta.org/metals/) · [VS Code extension](https://marketplace.visualstudio.com/items?itemName=scalameta.metals) · [Neovim plugin](https://github.com/scalameta/nvim-metals) · [IntelliJ IDEA Scala plugin](https://plugins.jetbrains.com/plugin/1347-scala) |

**Chain-level tools.** These work with any stack:

| Tool | What it's for |
|------|---------------|
| [CQuisitor](https://cardananium.github.io/cquisitor/) | Decode, inspect and validate Cardano transaction CBOR (inputs, outputs, witnesses, redeemers, Phase-1/2 checks) |
| [cbor.me](https://cbor.me) · [cbor.nemo157.com](https://cbor.nemo157.com) | Generic CBOR ↔ diagnostic-notation decoders (datums, redeemers, `plutus.json` script bytes) |
| Cardanoscan: [mainnet](https://cardanoscan.io) · [preprod](https://preprod.cardanoscan.io) · [preview](https://preview.cardanoscan.io) | Block explorer: look up transactions, addresses, and scripts |
| Cexplorer: [mainnet](https://cexplorer.io) · [preprod](https://preprod.cexplorer.io) · [preview](https://preview.cexplorer.io) | Block explorer: look up transactions, addresses, and scripts |
| [Testnet faucet](https://docs.cardano.org/cardano-testnets/tools/faucet) | Free test ada for `preview` / `preprod` |

## Getting help

Each tool's official community channels are recorded in its registry entry (`community` in `registry/tools/<tool>.toml`). `cardano-init doctor` lists them for the components it detects (under *Get help*), and `cardano-init list --format json` exposes them as `tools[].community`.

| Tool | Where to ask |
|------|--------------|
| Aiken | [Discord (PRAGMA)](https://discord.gg/JnWjkrErJr) · [GitHub Discussions](https://github.com/aiken-lang/aiken/discussions) |
| Scalus | [Discord (Lantr)](https://discord.gg/B6tXmBzhTn) |
| Plinth | [Discord (Intersect)](https://discord.gg/RJWdVsMkvR), channel `#wg-plutus` · [Cardano Stack Exchange, `plutus` tag](https://cardano.stackexchange.com/questions/tagged/plutus) |
| MeshJS | [Discord](https://discord.gg/dH48jH3BKa) |
| Evolution SDK | [Discord (No Witness)](https://discord.gg/39xMk9DwQv) · [GitHub Discussions](https://github.com/IntersectMBO/evolution-sdk/discussions) |
| Tx3, Dolos | [Discord (TxPipe)](https://discord.gg/eVc6HJrYmP) |
| Yaci DevKit | [Discord (Bloxbean)](https://discord.gg/JtQ54MSw6p) · [GitHub Discussions](https://github.com/bloxbean/yaci-devkit/discussions) |
| Kupo | [Discord (IOG Technical Community)](https://discord.gg/ZeyDn65t5v), channel `#ogmios` · [GitHub Discussions](https://github.com/CardanoSolutions/kupo/discussions) |
| Ogmios | [Discord (IOG Technical Community)](https://discord.gg/ZeyDn65t5v), channel `#ogmios` · [GitHub Discussions](https://github.com/CardanoSolutions/ogmios/discussions) |
| Cardano Node | [GitHub Discussions](https://github.com/IntersectMBO/cardano-node/discussions) · [Discord (Intersect)](https://discord.gg/RJWdVsMkvR) |
| Dingo, Cardano Node API, Tx Submit API, cardano-up | [Discord (Blink Labs)](https://discord.gg/5fPRZnX4qW) |

**General Cardano developer channels.** For questions that aren't specific to one tool: the [Cardano Foundation developer Discord](https://discord.gg/MmeqpAzKbp), the [Cardano Forum developers category](https://forum.cardano.org/c/developers/29), and [Cardano Stack Exchange](https://cardano.stackexchange.com) (low traffic today, but a useful searchable archive).
