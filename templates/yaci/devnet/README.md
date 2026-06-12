# Devnet — Yaci DevKit

[Yaci DevKit](https://devkit.yaci.xyz) runs a private Cardano **devnet** on your
machine: a local node plus the **Yaci Store** indexer, which exposes a
**Blockfrost-compatible API** (`http://localhost:8080/api/v1/`). You deploy and
exercise your contracts against it end-to-end — no public testnet, no real
funds. A built-in faucet tops up test wallets on demand.

It is a development/testing tool, never deployed, which is why it fills the
**devnet** role rather than infrastructure.

## Tasks

| Command | What it does |
|---|---|
| `just test` | **Self-contained.** Spins up a throwaway devnet, runs the integration test against it, then tears it down — one command, no `just dev` first. If `yaci-devkit` isn't installed, it skips and passes (so CI without it stays green). |
| `just dev` | Start a **persistent** devnet (for iterating on transactions with the off-chain `create`/`redeem`) and write its connection details into `../.env`. Long-running — Ctrl-C to stop. |
| `just clean` | Stop the devnet and clear the connection details from `../.env`. |
| `just build` | No-op (a devnet has nothing to build). |

`just test` drives the devnet lifecycle in `scripts/devnet-test.sh` (start →
wait for `:8080` → run `integration.test.mjs` → always tear down). `just dev` is
only for interactive work where you want a devnet to stay up.

## How off-chain connects to it

`just dev` writes the standard connection keys into the shared `../.env`:

```
INDEXER_URL=http://localhost:8080/api/v1/
INDEXER_PORT=8080
YACI_ADMIN_URL=http://localhost:10000
```

Off-chain components read `INDEXER_URL` and, when it is set, talk to this local
devnet (MeshJS uses its `YaciProvider`); when it is absent they fall back to a
public provider such as Blockfrost. Nothing about the off-chain code is
Yaci-specific — it just reacts to the presence of `INDEXER_URL`. This is the
interface contract's connection seam, owned by whichever component provisions a
local endpoint.

## Installing Yaci DevKit

`just dev` uses the npm-distributed CLI:

```bash
npm install -g @bloxbean/yaci-devkit
```

Prefer the Docker distribution? Download it and replace the last line of the
`dev` recipe with the devkit directory's `./bin/devkit.sh start`. See the
[DevKit docs](https://devkit.yaci.xyz/introduction) for both paths.

## Funding test wallets

Inside the running devnet shell you can top up any address:

```
devnet:default> topup addr_test1... 1000
```

or fund programmatically from off-chain via the admin API at `YACI_ADMIN_URL`
(MeshJS: `await provider.addressTopup(address, lovelace)`).

## Extending the test

`integration.test.mjs` is dependency-free (Node's built-in `fetch`). It checks
that the devnet serves protocol parameters and that the on-chain blueprint (if
built) loads. Grow it into real deploy/spend round-trips as your protocol takes
shape — add a `package.json` and a richer test runner if you outgrow plain Node.
