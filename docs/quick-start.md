# Quick start

## 1. Create a project

The easiest way to start is the interactive guided setup. Run the command with no arguments and it asks for a project name and a tool for each role:

```bash
cardano-init
```

If you already know what you want, use **one-shot mode** instead. Passing `--name` makes the command fully non-interactive:

```bash
cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --devnet yaci
```

You only need to select the roles you want. A single role is fine:

```bash
cardano-init --name my-validators --on-chain aiken
```

To see which tools you can pick for each role, run:

```bash
cardano-init list          # detailed list
cardano-init list --table  # compact matrix
```

## 2. Check your dependencies

Each tool needs its own toolchain. From inside the generated project, run the dependency doctor:

```bash
cd my-protocol
cardano-init doctor
```

It detects the components in the project, checks which toolchains are installed, and prints the exact install command for anything missing.

## 3. Build and test

Every generated project is driven by [`just`](https://just.systems):

```bash
just build   # build every component (on-chain first, so the blueprint exists)
just test    # build, then run every component's tests
just clean   # remove build artifacts
```

`just test` passes out of the box: every stack ships a worked gift-card example. From there, modify the protocol to your liking and run `just test` often.

## 4. Run a local chain (optional)

If you selected a devnet or infrastructure provider, its long-running `dev` task is run per component:

```bash
just -f devnet/Justfile dev   # e.g. start Yaci DevKit
just -f infra/Justfile dev    # e.g. bring up Kupo + Ogmios via cardano-up
```

These write their connection details into the project's `.env`, which the off-chain component picks up automatically.

## More examples

```bash
# Preview what would be generated, without writing anything
cardano-init --name my-protocol --on-chain aiken --dry-run

# Fullstack: one tool for both on-chain and off-chain, as a single protocol/ component
cardano-init --name my-protocol --fullstack scalus

# Short flags
cardano-init -n my-protocol -p scalus -d yaci

# Also generate a Nix flake for the project's dependencies
cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --nix

# Later, from inside the project: add or swap a tool, or remove one
cardano-init add --on-chain plinth
cardano-init remove --devnet
```

See the [Command reference](commands.md) for every flag.
