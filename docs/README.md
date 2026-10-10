# cardano-init

**Go from zero to a running Cardano protocol in one command.**

Pick a tool for each role you need (on-chain, off-chain, devnet, infrastructure, formal-methods) and `cardano-init` generates a monorepo where every component is **already wired together**, plus a worked end-to-end example that **builds and passes its tests out of the box**.

![cardano-init scaffolds a full stack in one command, then just test passes out of the box](https://raw.githubusercontent.com/input-output-hk/cardano-init/main/assets/demo.gif)

```console
$ cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci

my-protocol/
├── on-chain/     # Validators (smart contracts)
├── off-chain/    # Tx building (protocol transactions)
├── devnet/       # Local chain for integration testing
├── blueprint/    # shared CIP-57 contract interface
├── .env          # shared between components
├── Justfile      # Commands to build, test, and clean
├── AGENTS.md     # Agent brief
└── README.md

$ cd my-protocol && just test
  ✓  All tests passed
```

## Why `cardano-init`?

- **Zero to running in one command.** You get a wired-together monorepo that builds and passes its tests immediately.
- **Mix and match freely.** Components talk to a shared *contract*, so you can combine tools from different roles however you want and they work together.
- **Agent-native.** Every command can emit machine-readable JSON, and every project gets a generated `AGENTS.md`, so coding agents know what the project is and what to do next.
- **Never stuck on setup.** The built-in dependency `doctor` detects your toolchains and tells you the exact installer to run for anything missing.
- **A real example.** Every stack ships the same worked gift-card scenario end to end, so what you generate actually runs.

## Where to go next

- New here? Start with [Installation](installation.md) and the [Quick start](quick-start.md).
- Want to understand how the pieces compose? Read [How it works](how-it-works.md).
- Looking for a flag? See the [Command reference](commands.md).
- Driving `cardano-init` from an LLM? See [For coding agents](agents.md).
- Want to add your tool to the registry? See [Adding a tool](ADDING_A_TOOL.md).

`cardano-init` is open source (Apache-2.0) and developed on [GitHub](https://github.com/input-output-hk/cardano-init).
