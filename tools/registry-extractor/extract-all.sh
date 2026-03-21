#!/usr/bin/env bash
# =============================================================================
# extract-all.sh — Automated registry .bin extraction for all protocol versions
#
# Starts a vanilla Minecraft server in Docker for each protocol version,
# runs the registry-extractor tool, then tears down the container.
#
# Usage:
#   ./extract-all.sh              # Extract all missing versions
#   ./extract-all.sh --force      # Re-extract even if .bin already exists
#   ./extract-all.sh --only 765   # Extract only protocol 765
#   ./extract-all.sh --skip-build # Don't rebuild registry-extractor
# =============================================================================
set -euo pipefail

# ─── Configuration ───────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUTPUT_DIR="$PROJECT_ROOT/data/registry"
EXTRACTOR_BIN="$PROJECT_ROOT/target/release/registry-extractor"

CONTAINER_NAME="infrarust-registry-extractor"
DOCKER_IMAGE="itzg/minecraft-server:java21"
HOST_PORT=25599
STARTUP_TIMEOUT=300  # 5 minutes

# MC_VERSION:PROTOCOL_VERSION
# One entry per unique protocol version. Versions sharing a protocol are skipped.
VERSIONS=(
  "1.20.2:764"
  "1.20.3:765"
  "1.20.5:766"
  "1.21:767"
  "1.21.2:768"
  "1.21.4:769"
  "1.21.5:770"
  "1.21.6:771"
  "1.21.7:772"
  "1.21.9:773"
  "1.21.11:774"
)

# ─── CLI flags ───────────────────────────────────────────────────────────────

FORCE=false
ONLY=""
SKIP_BUILD=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)     FORCE=true; shift ;;
    --only)      ONLY="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    -h|--help)
      echo "Usage: $0 [--force] [--only <protocol>] [--skip-build]"
      echo ""
      echo "  --force       Re-extract even if .bin already exists"
      echo "  --only <N>    Extract only protocol version N (e.g. 765)"
      echo "  --skip-build  Don't rebuild registry-extractor before running"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ─── Helpers ─────────────────────────────────────────────────────────────────

log()  { echo "[$(date '+%H:%M:%S')] $*"; }
ok()   { echo "[$(date '+%H:%M:%S')] ✓ $*"; }
fail() { echo "[$(date '+%H:%M:%S')] ✗ $*" >&2; }

cleanup_container() {
  if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    log "Cleaning up container $CONTAINER_NAME..."
    docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
  fi
}

trap cleanup_container EXIT INT TERM

wait_for_server() {
  local elapsed=0
  local interval=5

  log "Waiting for server to start (timeout: ${STARTUP_TIMEOUT}s)..."

  while [ $elapsed -lt $STARTUP_TIMEOUT ]; do
    # Check if container is still running
    if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
      fail "Container stopped unexpectedly"
      docker logs --tail 30 "$CONTAINER_NAME" 2>&1 || true
      return 1
    fi

    # Check server health via mc-health
    if docker exec "$CONTAINER_NAME" mc-health >/dev/null 2>&1; then
      ok "Server is ready (took ${elapsed}s)"
      return 0
    fi

    sleep $interval
    elapsed=$((elapsed + interval))
  done

  fail "Server did not start within ${STARTUP_TIMEOUT}s"
  docker logs --tail 50 "$CONTAINER_NAME" 2>&1 || true
  return 1
}

start_server() {
  local mc_version="$1"

  cleanup_container

  log "Starting vanilla MC $mc_version on port $HOST_PORT..."

  docker run -d \
    --name "$CONTAINER_NAME" \
    -p "${HOST_PORT}:25565" \
    -e EULA=TRUE \
    -e VERSION="$mc_version" \
    -e TYPE=VANILLA \
    -e ONLINE_MODE=false \
    -e MEMORY=512M \
    -e SPAWN_PROTECTION=0 \
    -e VIEW_DISTANCE=4 \
    -e MAX_TICK_TIME=-1 \
    "$DOCKER_IMAGE" >/dev/null

  wait_for_server
}

extract_version() {
  local mc_version="$1"
  local protocol="$2"

  log "Extracting registry for MC $mc_version (protocol $protocol)..."

  "$EXTRACTOR_BIN" \
    --server "localhost:${HOST_PORT}" \
    --protocol-version "$protocol" \
    --output "$OUTPUT_DIR"

  if [ -f "$OUTPUT_DIR/v${protocol}.bin" ]; then
    local size
    size=$(stat --printf="%s" "$OUTPUT_DIR/v${protocol}.bin" 2>/dev/null || stat -f%z "$OUTPUT_DIR/v${protocol}.bin" 2>/dev/null)
    ok "v${protocol}.bin created (${size} bytes)"
    return 0
  else
    fail "v${protocol}.bin was not created"
    return 1
  fi
}

# ─── Main ────────────────────────────────────────────────────────────────────

log "=== Infrarust Registry Extractor — Batch Mode ==="
log "Output: $OUTPUT_DIR"

# Build extractor
if [ "$SKIP_BUILD" = false ]; then
  log "Building registry-extractor (release)..."
  cargo build --release -p registry-extractor --manifest-path "$PROJECT_ROOT/Cargo.toml"
  ok "Build complete"
else
  if [ ! -f "$EXTRACTOR_BIN" ]; then
    fail "Extractor binary not found at $EXTRACTOR_BIN — run without --skip-build"
    exit 1
  fi
  log "Skipping build (using existing binary)"
fi

mkdir -p "$OUTPUT_DIR"

# Track results
succeeded=0
skipped=0
failed=0

for entry in "${VERSIONS[@]}"; do
  mc_version="${entry%%:*}"
  protocol="${entry##*:}"

  # Filter by --only
  if [ -n "$ONLY" ] && [ "$protocol" != "$ONLY" ]; then
    continue
  fi

  # Skip existing
  if [ "$FORCE" = false ] && [ -f "$OUTPUT_DIR/v${protocol}.bin" ]; then
    log "v${protocol}.bin already exists — skipping MC $mc_version (use --force to re-extract)"
    skipped=$((skipped + 1))
    continue
  fi

  log "────────────────────────────────────────────────"
  log "Processing MC $mc_version → protocol $protocol"
  log "────────────────────────────────────────────────"

  if start_server "$mc_version"; then
    if extract_version "$mc_version" "$protocol"; then
      succeeded=$((succeeded + 1))
    else
      fail "Extraction failed for MC $mc_version (protocol $protocol)"
      failed=$((failed + 1))
    fi
  else
    fail "Server startup failed for MC $mc_version"
    failed=$((failed + 1))
  fi

  cleanup_container
done

# Summary
echo ""
log "=== Summary ==="
log "  Succeeded: $succeeded"
log "  Skipped:   $skipped"
log "  Failed:    $failed"

if [ "$failed" -gt 0 ]; then
  exit 1
fi

ok "All done!"
