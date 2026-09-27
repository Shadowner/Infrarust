#!/usr/bin/env bash
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
OUT="${1:-${TMPDIR:-/tmp}/codec-exec-soak/$(date +%Y%m%d-%H%M%S)}"
RUNS="${RUNS:-1}"
DURATION="${DURATION:-150}"
PORT_BASE="${PORT_BASE:-42100}"
CONFIGS="${CONFIGS:-noattack default}"
mkdir -p "$OUT"

config_args() {
  case "$1" in
    noattack) echo "--codec-budget default --attackers 0" ;;
    default|before) echo "--codec-budget default" ;;
    old-budget) echo "--codec-budget 800ms --epoch-tick 50ms" ;;
    *) echo "unknown config $1" >&2; exit 2 ;;
  esac
}

summarize() {
  python3 - "$1" <<'PY'
import re, sys, statistics
text = open(sys.argv[1] + "/report.txt").read()
bots = text.split("## Bot totals", 1)[1].split("##", 1)[0]
def grab(pattern, source=bots):
    m = re.search(pattern, source)
    return m.group(1) if m else "n/a"
bench = text.split("## mc-bench", 1)[1].split("##", 1)[0] if "## mc-bench" in text else ""
p50s, p99s = [], []
for line in bench.splitlines():
    parts = line.split()
    if len(parts) == 9 and parts[0].isdigit():
        p50s.append(float(parts[5]) / 1000)
        p99s.append(float(parts[6]) / 1000)
log = open(sys.argv[1] + "/proxy.log", errors="replace").read()
cpu = re.findall(r"^\s*\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+(\d+)", text.split("## Proxy process", 1)[1].split("##", 1)[0], re.M)
print("chat_p50_ms=" + grab(r"rtt p50 ([\d.]+)"),
      "chat_p99_ms=" + grab(r"rtt p50 [\d.]+ p99 ([\d.]+)"),
      "chat_max_ms=" + grab(r"max ([\d.]+)\n/"),
      "hello_p99_ms=" + grab(r"/hello: .*? p99 ([\d.]+)"),
      "wping_p99_ms=" + grab(r"/wping: .*? p99 ([\d.]+)"),
      "login_p50_ms=" + grab(r"login p50 ([\d.]+)"),
      "login_p99_ms=" + grab(r"login p50 [\d.]+ ms p95 [\d.]+ ms p99 ([\d.]+)"),
      "bench_p50_ms=" + (f"{statistics.median(p50s):.2f}" if p50s else "n/a"),
      "bench_p99_ms=" + (f"{statistics.median(p99s):.1f}" if p99s else "n/a"),
      "proxy_cpu_pct=" + (str(max(int(c) for c in cpu)) if cpu else "n/a"),
      "trapped_lines=" + str(log.count("wasm codec filter trapped")),
      "quarantines=" + str(log.count("wasm codec filter quarantined")))
PY
}

for run in $(seq 1 "$RUNS"); do
  for config in $CONFIGS; do
    dir="$OUT/$config-$run"
    echo "==> $config run $run -> $dir"
    binary="${INFRARUST_BINARY:-${CARGO_TARGET_DIR:-$REPO/target}/release/infrarust}"
    if [[ "$config" == before ]]; then
      binary="${INFRARUST_BINARY_BEFORE:?config before needs INFRARUST_BINARY_BEFORE}"
    fi
    env INFRARUST_BINARY="$binary" \
      "$REPO/tests/soak/run.sh" --scenario faulty-codec $(config_args "$config") \
      --duration "$DURATION" --bots 20 --bench-conns 5 --port-base "$PORT_BASE" --out "$dir" > "$dir.log" 2>&1
    echo "$config run $run: $(summarize "$dir")" | tee -a "$OUT/summary.txt"
  done
done
