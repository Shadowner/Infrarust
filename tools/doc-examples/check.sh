#!/usr/bin/env bash
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="${1:?usage: check.sh OUT_DIR}"

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/doc-examples}"

mkdir -p "$out"
find "$out" -mindepth 1 -maxdepth 1 -type d -name 'doc-*' -exec rm -rf {} +
python3 "$here/extract.py" gen "$out" || exit 2

(cd "$out" && cargo check --target wasm32-wasip2 --workspace --keep-going --message-format short) >"$out/check.log" 2>&1
status=$?

python3 "$here/extract.py" report "$out" "$out/check.log"
echo
echo "cargo exit status: $status (full log: $out/check.log)"
exit "$status"
