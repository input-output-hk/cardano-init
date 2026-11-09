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
# Written by the npm `yaci-devkit` wrapper; a later `yaci-devkit` run kills the
# PID it names.
YACI_PIDFILE="$HOME/.yaci-cli/yaci-cli.pid"

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

# Is any of these PIDs (or `-PGID` process groups) still alive?
any_alive() {
  for p in "$@"; do kill -0 "$p" 2>/dev/null && return 0; done
  return 1
}

# The process tree under a PID, as it is right now.
process_tree() {
  ps -A -o pid= -o ppid= | awk -v root="$1" '
    { parent[$1] = $2 }
    END {
      keep[root] = 1
      do { n = 0; for (p in parent) if (!(p in keep) && (parent[p] in keep)) { keep[p] = 1; n++ } } while (n)
      for (p in keep) print p
    }'
}

# Stop the whole devnet. The npm CLI has no `down` command, and killing just
# the `yaci-devkit` wrapper orphans the `yaci-cli` it spawned, which keeps
# running (and restarts the node it supervises). The devnet normally runs in
# its own process group (see below): the wrapper, yaci-cli and every component
# (cardano-node, submit-api, ogmios, yaci-store). Signal the group, give it
# 15s to shut down, then force it. Without a group of its own, fall back to
# the process trees under the wrapper and under yaci-cli (found via its PID
# file, as a Ctrl-C may already have killed the wrapper and orphaned it).
# Upstream: https://github.com/bloxbean/yaci-devkit/issues/220
stop_devnet() {
  [ -n "$DEVNET_PID" ] || return 0
  if kill -0 "-$DEVNET_PID" 2>/dev/null; then
    set -- "-$DEVNET_PID"
  else
    cli_pid=""
    if [ -f "$YACI_PIDFILE" ]; then
      cli_pid="$(cat "$YACI_PIDFILE")"
      ps -p "$cli_pid" -o comm= 2>/dev/null | grep -q yaci-cli || cli_pid=""
    fi
    # shellcheck disable=SC2046 # one PID per word
    set -- $(process_tree "$DEVNET_PID") ${cli_pid:+$(process_tree "$cli_pid")}
  fi
  [ "$#" -gt 0 ] || return 0
  kill -TERM "$@" 2>/dev/null || true
  waited=0
  while any_alive "$@" && [ "$waited" -lt 15 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  kill -KILL "$@" 2>/dev/null || true
  # The wrapper removes its PID file only when yaci-cli exits cleanly. A stale
  # one would make the next `yaci-devkit` run kill whatever process reuses
  # that PID.
  if [ -f "$YACI_PIDFILE" ] && ! kill -0 "$(cat "$YACI_PIDFILE")" 2>/dev/null; then
    rm -f "$YACI_PIDFILE"
  fi
}

# The exit status: the script's own, or the signal's 128+n (a trap's `$?` is
# the interrupted command's, so Ctrl-C would otherwise exit 0).
teardown() {
  status=${1:-$?}
  trap - EXIT INT TERM # run once, even when a signal's exit re-fires EXIT
  if [ "$status" -ne 0 ]; then
    echo "Recent devnet log:"
    tail -n 80 "$LOG" 2>/dev/null || true
  fi
  echo "Tearing down the ephemeral devnet ..."
  stop_devnet
  exit "$status"
}
trap teardown EXIT
trap 'teardown 130' INT
trap 'teardown 143' TERM

echo "Starting ephemeral Yaci devnet (logs: $LOG) ..."
# Start the devnet in its own process group (its PGID is its PID), so
# stop_devnet can stop all of it at once. `setsid` does this on Linux; job
# control does it elsewhere (macOS), but dash turns job control off when
# there is no terminal, as in CI.
if command -v setsid >/dev/null 2>&1; then
  nohup setsid yaci-devkit up --enable-yaci-store >"$LOG" 2>&1 &
else
  set -m 2>/dev/null
  nohup yaci-devkit up --enable-yaci-store >"$LOG" 2>&1 &
  set +m
fi
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

# Run any off-chain or fullstack component's suite against this devnet too. Its
# integration tests pick up INDEXER_URL and self-skip when no devnet is set.
for component in off-chain protocol; do
  if [ -f "../$component/Justfile" ]; then
    echo "Running $component integration tests against the devnet ..."
    INDEXER_URL="$API_URL" YACI_ADMIN_URL="http://localhost:10000" \
      just -f "../$component/Justfile" test
  fi
done
