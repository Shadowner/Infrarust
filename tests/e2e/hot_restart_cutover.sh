#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BINARY="${INFRARUST_BINARY:-$REPO/target/release/infrarust}"

usage() {
  cat <<'USAGE'
Usage: ./hot_restart_cutover.sh [options]

Measures how long forwarded sessions stop moving during a hot restart (SIGUSR2):
for every stream, the gap between the last byte the old image relayed and the
first byte the new image relayed. Linux only. Never run on CI.

  --build               Build target/release/infrarust first.
  --sessions <n>        Sessions to hold across the restart (default 1000).
  --zero-copy <ratio>   Share of zero_copy sessions, the rest are passthrough (default 0.2).
  --tick <ms>           Each side sends one u64 per session every <ms> (default 10).
  --settle <ms>         Steady traffic before and after the restart (default 2000).
  --json                Print the result as JSON.
  --keep                Keep the temporary proxy directory.

INFRARUST_BINARY overrides the proxy binary (default target/release/infrarust).
Exit codes: 0 every stream survived · 1 a stream broke, closed or timed out.
USAGE
}

BUILD=0
ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build) BUILD=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: hot restart only exists on Linux" >&2
  exit 1
fi

if [[ $BUILD -eq 1 ]]; then
  echo "==> building infrarust"
  cargo build --release -p infrarust --manifest-path "$REPO/Cargo.toml"
fi

if [[ ! -x "$BINARY" ]]; then
  echo "error: proxy binary not found at $BINARY (use --build or INFRARUST_BINARY)" >&2
  exit 1
fi

ulimit -n "$(ulimit -Hn)" 2>/dev/null || true
exec node "$HERE/src/hot-restart-cutover.js" --binary "$BINARY" "${ARGS[@]}"
