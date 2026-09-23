#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BINARY="${INFRARUST_BINARY:-$REPO/target/release/infrarust}"

usage() {
  cat <<'USAGE'
Usage: ./realclient.sh [options]

  --set anchors         One release per era where the login path changed (default).
  --set all             Every release from 1.7.10 to the latest. Hours, not minutes.
  --versions <list>     Comma-separated releases, e.g. 1.7.10,1.20.1
  --scenario <list>     Comma-separated scenario ids (see src/real/scenarios.js)
  --session-server <url>  Yggdrasil base URL. Without it the client_only
                        scenarios are skipped rather than failed.
  --donor-mc <dir>      An existing launcher directory (MultiMC, Prism, .minecraft)
                        whose assets/ and libraries/ are hard-linked instead of
                        re-downloaded.
  --cache <dir>         Where to keep downloads. Defaults to ./.mccache
  --timeout <seconds>   Per-client budget. Default 210.
  --keep-worlds         Do not delete each version's world after it is done.
                        Worlds are superflat and regenerate in seconds, so they
                        are dropped by default; keep them to inspect a failure.
  --skip-build          Do not rebuild the proxy first.

Exit codes: 0 all good · 1 a case failed.

Requires: Xvfb, unzip. No container runtime, no root, no /etc/hosts entry.
USAGE
}

SKIP_BUILD=0
ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

for tool in Xvfb unzip; do
  command -v "$tool" >/dev/null || { echo "error: $tool is required but not installed" >&2; exit 1; }
done

if [[ $SKIP_BUILD -eq 0 ]]; then
  echo "==> building infrarust (use --skip-build to skip)"
  cargo build --release -p infrarust --manifest-path "$REPO/Cargo.toml"
fi

[[ -x "$BINARY" ]] || { echo "error: proxy binary not found at $BINARY" >&2; exit 1; }

if [[ ! -d "$HERE/node_modules" ]]; then
  echo "==> installing harness dependencies"
  (cd "$HERE" && npm install --no-audit --no-fund)
fi

cd "$HERE"
exec node src/real/run.js --binary "$BINARY" "${ARGS[@]}"
