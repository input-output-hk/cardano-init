# Ephemeral Yaci devnet lifecycle for `just test`.
#
# Starts a throwaway devnet, waits until its Blockfrost-compatible API is
# serving, runs the integration test against it, then ALWAYS tears it down —
# the whole round-trip from a single `just test`, with no `just dev` first.
#
# Degrades gracefully when yaci-devkit is not installed, so CI without it (and
# the per-tool build/test smoke gate) stays green: the integration test then
# runs with no devnet and skips itself.
#
# Set CARDANO_INIT_REQUIRE_DEVNET=1 to turn every graceful skip into a hard
# failure. The end-to-end CI smoke test uses this so a green run proves the
# devnet actually came up and the seam ran — never a vacuous skip.
#
# Invoked via `sh scripts/devnet-test.sh` (no exec bit; see the interface
# contract). Yaci DevKit supports Linux x64 and macOS arm64 (not Windows).
set -eu

API_URL="http://localhost:8080/api/v1/"
LOG=".yaci-devnet.log"

if ! command -v yaci-devkit >/dev/null 2>&1; then
  if [ -n "${CARDANO_INIT_REQUIRE_DEVNET:-}" ]; then
    echo "✗ yaci-devkit is not installed, but CARDANO_INIT_REQUIRE_DEVNET is set."
    echo "  Install it: npm install -g @bloxbean/yaci-devkit"
    exit 1
  fi
  echo "• yaci-devkit not installed — running the test without a devnet (it will skip)."
  echo "  Install it for the full integration test:"
  echo "  npm install -g @bloxbean/yaci-devkit"
  exec node integration.test.mjs
fi

DEVNET_PID=""
teardown() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "Recent devnet log:"
    tail -n 80 "$LOG" 2>/dev/null || true
  fi
  echo "Tearing down the ephemeral devnet ..."
  # The npm yaci-devkit CLI has no `down`/`stop` command. killing the process tree.
  if [ -n "$DEVNET_PID" ]; then kill "$DEVNET_PID" >/dev/null 2>&1 || true; fi
  pkill -f "$HOME/.yaci-cli/" >/dev/null 2>&1 || true
  exit "$status"
}
trap teardown EXIT INT TERM

echo "Starting ephemeral Yaci devnet (logs: $LOG) ..."
nohup yaci-devkit up --enable-yaci-store >"$LOG" 2>&1 &
DEVNET_PID=$!

echo "Waiting for the devnet node and indexer to advance (up to 150s) ..."
node -e "
const url = '${API_URL}blocks/latest';
const adminUrl = 'http://localhost:10000/local-cluster/api/admin/devnet/tip';
const deadline = Date.now() + 150000;
let firstHeight;
const probe = async () => {
  try {
    const [res, admin] = await Promise.all([
      fetch(url, { signal: AbortSignal.timeout(5000) }),
      fetch(adminUrl, { signal: AbortSignal.timeout(5000) }),
    ]);
    if (res.ok && admin.ok) {
      const block = await res.json();
      const { tips } = await admin.json();
      const tip = tips?.find((value) => value.nodeName === 'node-1');
      if (Number.isInteger(tip?.blockNumber) && tip.blockNumber >= 0 &&
          Number.isInteger(block.height) && block.height >= 0) {
        firstHeight ??= tip.blockNumber;
        // HTTP 200 (including a genesis/bootstrapped tip) is not readiness:
        // wait for the live node to forge and the indexer to catch up before
        // using the faucet's local-node query/submission connection.
        if (tip.blockNumber > firstHeight && block.height >= firstHeight) process.exit(0);
      }
    }
  } catch {}
  if (Date.now() > deadline) { console.error('timed out waiting for the devnet node and indexer'); process.exit(1); }
  setTimeout(probe, 3000);
};
probe();
" || { echo "✗ Devnet did not become ready."; exit 1; }

# Devnet is up. Point the test at it explicitly (overrides ../.env) and run the
# standalone smoke test.
INDEXER_URL="$API_URL" node integration.test.mjs

# If an off-chain component is present, run its suite against this devnet too:
# its integration tests pick up INDEXER_URL and exercise real transactions
# (they self-skip when no devnet is set, e.g. during the top-level test-off-chain
# phase). Generic — keyed on the off-chain role's dir, not on any specific tool.
if [ -f ../off-chain/Justfile ]; then
  echo "Running off-chain integration tests against the devnet ..."
  INDEXER_URL="$API_URL" YACI_ADMIN_URL="http://localhost:10000" \
    just -f ../off-chain/Justfile test
fi
