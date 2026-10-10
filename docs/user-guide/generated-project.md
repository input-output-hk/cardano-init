# The generated project

A typical project (on-chain + off-chain + devnet) looks like this:

```text
my-protocol/
├── Justfile        # top-level build / test / clean
├── README.md       # how to work with this specific stack
├── AGENTS.md       # brief for coding agents
├── CLAUDE.md       # imports AGENTS.md
├── .gitignore
├── .env            # shared connection details
├── blueprint/      # CIP-57 blueprint, written by on-chain `build`
├── on-chain/       # validators + their own Justfile
├── off-chain/      # transaction building + its own Justfile
└── devnet/         # local chain + its own Justfile
```

Only the roles you select get a directory. Add `--nix` to also get a `flake.nix` and `.envrc` that pin the project's toolchains. Add `--dry-run` to see the full file list without writing anything.

## The Justfile workflow

Every component has its own `Justfile` and works on its own. The top-level `Justfile` runs the tasks that finish and can be combined:

| Target | What it does |
|--------|--------------|
| `just build` | Builds every component. On-chain builds first, so `blueprint/plutus.json` exists for the other components. |
| `just test` | Builds the on-chain blueprint, then runs each component's tests (including formal verification, if selected). |
| `just clean` | Removes build artifacts from every component. |

There are also targets for each role, such as `just build-on-chain` and `just test-off-chain`. Run `just` with no arguments to list them.

### Long-running tasks: `dev`

Tasks that keep running (watch modes, local devnets, service stacks) belong to **one component**, and the top-level `Justfile` does not run them. A component has a `dev` target only when it has such a task. Run it from the component's `Justfile`:

```bash
just -f devnet/Justfile dev
just -f infra/Justfile dev
just -f <dir>/Justfile --list   # see what a component offers
```

## `blueprint/`

The on-chain component's `build` writes the compiled validators to `blueprint/plutus.json`, a [CIP-57](https://cips.cardano.org/cip/CIP-0057) blueprint. The off-chain code reads the compiled scripts from this file and applies its parameters. See [The interface contract](how-it-works.md#the-interface-contract).

## `.env`

`.env` holds the chain connection details for all components:

```bash
CARDANO_NETWORK=preview

INDEXER_URL=
INDEXER_PORT=
NODE_SOCKET_PATH=
OGMIOS_URL=
TX_SUBMIT_URL=
DOLOS_GRPC_URL=
CARDANO_NODE_API_URL=
```

The values start empty. A component that starts a local endpoint (a devnet such as Yaci DevKit, or an [infrastructure provider](infrastructure.md)) fills them in during its `dev` task. When a value is empty, the components that read it keep working, for example by connecting to a public provider.

## `AGENTS.md`

Every project includes an `AGENTS.md`, and a `CLAUDE.md` that imports it, written for the tools you selected. See [For coding agents](agents.md#generated-agentsmd) for what it contains.

## Changing the stack later

From inside the project, use [`add`](commands.md#add) and [`remove`](commands.md#remove) to add, replace, or remove components. `cardano-init` reads the current selection from the component directories (the project has no metadata file) and updates the shared top-level files.
