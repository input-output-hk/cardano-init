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

Only the directories for the roles you selected are created. Add `--nix` to also get a `flake.nix` and `.envrc` that pin the project's toolchains. Run `--dry-run` to see the exact file list for a given selection before writing anything.

## The Justfile workflow

Every component has its own `Justfile` and works standalone. The top-level `Justfile` aggregates the tasks that terminate and compose:

| Target | What it does |
|--------|--------------|
| `just build` | Builds every component. On-chain builds first so `blueprint/plutus.json` exists for everyone else. |
| `just test` | Builds the on-chain blueprint, then runs each component's tests (including formal verification, if selected). |
| `just clean` | Removes build artifacts from every component. |

Per-role targets such as `just build-on-chain` or `just test-off-chain` are also available. Run `just` on its own to list them.

### Long-running tasks: `dev`

Watch modes, local devnets, and service stacks are **per component** and never aggregated at the top level. A component provides a `dev` target only when it has such a mode. Run it directly:

```bash
just -f devnet/Justfile dev
just -f infra/Justfile dev
just -f <dir>/Justfile --list   # see what a component offers
```

## `blueprint/`

The on-chain component's `build` writes the compiled validators to `blueprint/plutus.json` (a [CIP-57](https://cips.cardano.org/cip/CIP-0057) blueprint). Off-chain code reads the compiled scripts from there and applies its parameters, which is why any off-chain tool can drive any on-chain tool.

## `.env`

`.env` is the shared, stable place for chain connection details:

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

The values start blank. Whichever component provisions a local endpoint (a devnet such as Yaci DevKit, or an [infrastructure provider](infrastructure.md)) fills them in during its `dev` task. Consumers read them and fall back gracefully when they're empty, for example by using a public provider instead of a local one.

## `AGENTS.md`

Every project ships an `AGENTS.md` (and a `CLAUDE.md` that imports it) tailored to the chosen stack: the layout, the interface contract, the exact `just` workflow, official documentation links for each tool, and the most relevant Cardano developer skills. See [For coding agents](agents.md).

## Changing the stack later

From inside the project, use [`add`](commands.md#add) and [`remove`](commands.md#remove) to add, swap, or drop components. `cardano-init` detects the current selection from the directories on disk (there's no metadata file) and re-wires the shared top-level files.
