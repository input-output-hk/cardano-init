# Installation

`cardano-init` is a single self-contained binary. Pick whichever install method suits you.

## Run without installing

Using `npx`:

```bash
npx cardano-init help
```

Using Nix:

```bash
nix run github:input-output-hk/cardano-init -- help
```

## Install script

Linux and macOS (x86_64 and arm64):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/input-output-hk/cardano-init/releases/latest/download/cardano-init-installer.ps1 | iex
```

If you want a specific version or a manual download, grab it from the [Releases page](https://github.com/input-output-hk/cardano-init/releases).

## Nix (flake)

```bash
# Install the CLI into your profile
nix profile add github:input-output-hk/cardano-init

# Or run it once, without installing
nix run github:input-output-hk/cardano-init -- help
```

## Cargo

You need a recent Rust toolchain that supports the 2024 edition.

```bash
# From the published repository
cargo install --git https://github.com/input-output-hk/cardano-init

# Or from a local clone
cargo install --path .
```

## Verify

```bash
cardano-init --version
```

`cardano-init` itself has no runtime dependencies. The projects it generates do: they all use [`just`](https://just.systems), plus each tool's own toolchain (Aiken, Node.js, a JVM, Docker, …). You don't need to install them up front. Generate a project, then run [`cardano-init doctor`](commands.md#doctor) inside it to see exactly what's missing and how to install it.
