#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BINARY="${INFRARUST_BINARY:-$REPO/target/release/infrarust}"

usage() {
  cat <<'USAGE'
Usage: ./run.sh [options]

  --tier a              Mock backends, every client version (default, ~5 min).
  --tier b              Real Java servers, anchor versions only (slow).
  --version <list>      Comma-separated client versions, e.g. 1.7,1.20.1
  --scenario <list>     Comma-separated scenario ids, e.g. offline-velocity
  --session-server <url>  Base URL of a Yggdrasil session server. Without it the
                        client_only scenarios are skipped rather than failed.
  --skip-build          Do not rebuild the proxy first.
  --skip-selftest       Do not verify the harness before running the matrix.

Everything after -- is passed straight to src/run.js.

Exit codes: 0 all good · 1 a case failed · 2 a known finding no longer reproduces.
USAGE
}

SKIP_BUILD=0
SKIP_SELFTEST=0
ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --skip-selftest) SKIP_SELFTEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

if [[ $SKIP_BUILD -eq 0 ]]; then
  echo "==> building infrarust (use --skip-build to skip)"
  cargo build --release -p infrarust --manifest-path "$REPO/Cargo.toml"
fi

if [[ ! -x "$BINARY" ]]; then
  echo "error: proxy binary not found at $BINARY" >&2
  exit 1
fi

if [[ ! -d "$HERE/node_modules" ]]; then
  echo "==> installing harness dependencies"
  (cd "$HERE" && npm install --no-audit --no-fund)
fi

cd "$HERE"

if [[ $SKIP_SELFTEST -eq 0 ]]; then
  echo "==> verifying the harness itself"
  node src/selftest.js
fi

echo "==> running matrix"
exec node src/run.js --binary "$BINARY" "${ARGS[@]}"
