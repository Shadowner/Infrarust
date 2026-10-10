#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$REPO/target}"

export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

echo "==> proxy (default features plus the native hello plugin)"
cargo build --release -p infrarust --features plugin-hello --manifest-path "$REPO/Cargo.toml"

echo "==> mc-bench"
cargo build --release -p infrarust-mc-bench --manifest-path "$REPO/Cargo.toml"

echo "==> stats plugin, built the way docs/v2/plugins/wasm/building.md says"
cargo build --release --target wasm32-wasip2 \
  --manifest-path "$REPO/plugins/infrarust-plugin-stats/wasm/Cargo.toml" \
  --target-dir "$REPO/target/soak-stats"

echo "==> soak plugins"
(cd "$HERE/plugins" && cargo build --release --target wasm32-wasip2 --target-dir "$REPO/target/soak-wasm")

if [[ ! -d "$REPO/tests/e2e/node_modules" ]]; then
  echo "==> node harness dependencies"
  (cd "$REPO/tests/e2e" && npm install --no-audit --no-fund)
fi

echo "binary: $TARGET/release/infrarust"
