#!/usr/bin/env bash
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

TARGET="${CARGO_TARGET_DIR:-$REPO/target}"
BINARY="${INFRARUST_BINARY:-$TARGET/release/infrarust}"
WASM_DIR="${SOAK_WASM_DIR:-$REPO/target/soak-wasm/wasm32-wasip2/release}"
STATS_WASM="${SOAK_STATS_WASM:-$REPO/target/soak-stats/wasm32-wasip2/release/infrarust_plugin_stats_wasm.wasm}"
MC_BENCH="${MC_BENCH:-$TARGET/release/mc-bench}"

SCENARIO=baseline
DURATION=600
BOTS=100
PORT_BASE=41500
OUT=""
BENCH_CONNS=20
BENCH_RATE=10
BENCH_WINDOW=60
SHUTDOWN=after
SESSION_MIN=8
SESSION_MAX=30
CHAT_MS=1500
CMD_MS=4000
BURST_BOTS=0
BURST_AT=0
BURST_DURATION=60
INSTANCE_POOL=0
ATTACKERS=10
FLAKY_KNOBS=""
POST_PROBE=0
FLAKY_CPU_BUDGET=250ms
CODEC_BUDGET=200ms
EPOCH_TICK=""

usage() {
  cat <<'USAGE'
Usage: tests/soak/run.sh [options]

  --scenario <name>            baseline, faulty, faulty-codec or faulty-events
  --duration <s>               how long the bots churn (default 600)
  --bots <n>                   concurrent bot slots (default 100)
  --port-base <p>              proxy port; backends use p+1..p+3, admin API p+9
  --out <dir>                  where logs and samples land
  --bench-conns <n>            mc-bench connections per window, 0 disables it
  --shutdown after|load        SIGTERM the proxy after the bots stop, or while they run
  --burst-bots <n>             extra bots started at --burst-at seconds for --burst-duration
  --instance-pool <n>          [wasm] instance_pool
  --attackers <n>              bots from 127.0.0.2 whose connections the faulty codec filter traps or spins on
  --codec-budget <d|default>   codec_cpu_budget of soak-codec-faulty (default 200ms)
  --epoch-tick <d>             [wasm] epoch_tick (default: the proxy default)
  --flaky-knobs <file>         soak.txt for soak-flaky instead of the scenario's
  --post-probe <s>             after the load stops, run 5 fresh bots for this long before shutdown
  --flaky-cpu-budget <d|default>  cpu_budget of soak-flaky (default 250ms)
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scenario) SCENARIO="$2"; shift 2 ;;
    --duration) DURATION="$2"; shift 2 ;;
    --bots) BOTS="$2"; shift 2 ;;
    --port-base) PORT_BASE="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --bench-conns) BENCH_CONNS="$2"; shift 2 ;;
    --bench-rate) BENCH_RATE="$2"; shift 2 ;;
    --shutdown) SHUTDOWN="$2"; shift 2 ;;
    --session-min) SESSION_MIN="$2"; shift 2 ;;
    --session-max) SESSION_MAX="$2"; shift 2 ;;
    --chat-ms) CHAT_MS="$2"; shift 2 ;;
    --cmd-ms) CMD_MS="$2"; shift 2 ;;
    --burst-bots) BURST_BOTS="$2"; shift 2 ;;
    --burst-at) BURST_AT="$2"; shift 2 ;;
    --burst-duration) BURST_DURATION="$2"; shift 2 ;;
    --instance-pool) INSTANCE_POOL="$2"; shift 2 ;;
    --attackers) ATTACKERS="$2"; shift 2 ;;
    --codec-budget) CODEC_BUDGET="$2"; shift 2 ;;
    --epoch-tick) EPOCH_TICK="$2"; shift 2 ;;
    --flaky-knobs) FLAKY_KNOBS="$2"; shift 2 ;;
    --post-probe) POST_PROBE="$2"; shift 2 ;;
    --flaky-cpu-budget) FLAKY_CPU_BUDGET="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option $1" >&2; usage; exit 2 ;;
  esac
done

PROXY_PORT=$PORT_BASE
BACKEND_A=$((PORT_BASE + 1))
BACKEND_B=$((PORT_BASE + 2))
BENCH_BACKEND=$((PORT_BASE + 3))
API_PORT=$((PORT_BASE + 9))
API_KEY="soak-api-key-0123456789abcdef"

if [[ -z "$OUT" ]]; then
  OUT="${TMPDIR:-/tmp}/infrarust-soak/$(date +%Y%m%d-%H%M%S)-$SCENARIO"
fi
mkdir -p "$OUT"
LAB="$OUT/lab"
rm -rf "$LAB"
mkdir -p "$LAB/servers" "$LAB/plugins"

for port in $PROXY_PORT $BACKEND_A $BACKEND_B $BENCH_BACKEND $API_PORT; do
  if ss -ltnH "sport = :$port" | grep -q .; then
    echo "port $port is already in use; pick another --port-base" >&2
    exit 2
  fi
done

for f in "$BINARY" "$STATS_WASM" "$WASM_DIR/soak_witness.wasm" "$WASM_DIR/soak_flaky.wasm" "$WASM_DIR/soak_quarantine.wasm" "$WASM_DIR/soak_codec.wasm" "$WASM_DIR/soak_codec_faulty.wasm"; do
  if [[ ! -e "$f" ]]; then
    echo "missing $f (run tests/soak/build.sh first)" >&2
    exit 2
  fi
done

PIDS=()
cleanup() {
  for pid in "${PIDS[@]}"; do
    kill -TERM "$pid" 2>/dev/null
  done
  sleep 1
  for pid in "${PIDS[@]}"; do
    kill -KILL "$pid" 2>/dev/null
  done
}
trap cleanup EXIT

cp "$STATS_WASM" "$LAB/plugins/stats.wasm"
cp "$WASM_DIR/soak_witness.wasm" "$WASM_DIR/soak_flaky.wasm" "$WASM_DIR/soak_quarantine.wasm" "$WASM_DIR/soak_codec.wasm" "$LAB/plugins/"
mkdir -p "$LAB/plugins/soak-flaky" "$LAB/plugins/soak-quarantine"

FAULTY_CODEC=0
FAULTY_EVENTS=0
case "$SCENARIO" in
  baseline) ;;
  faulty) FAULTY_CODEC=1; FAULTY_EVENTS=1 ;;
  faulty-codec) FAULTY_CODEC=1 ;;
  faulty-events) FAULTY_EVENTS=1 ;;
  *) echo "unknown scenario $SCENARIO" >&2; exit 2 ;;
esac

if [[ $FAULTY_CODEC -eq 1 ]]; then
  cp "$WASM_DIR/soak_codec_faulty.wasm" "$LAB/plugins/"
fi
if [[ $FAULTY_EVENTS -eq 1 ]]; then
  cat > "$LAB/plugins/soak-flaky/soak.txt" <<'EOF'
chat_trap_every=40
login_spin_every=25
login_grow_every=45
preconnect_busy_ms=20
disconnect_trap_every=60
tick_ms=500
tick_trap_every=97
command_trap_every=7
leak_kb_per_event=16
EOF
  cat > "$LAB/plugins/soak-quarantine/soak.txt" <<'EOF'
chat_trap_every=30
command_trap_every=5
tick_ms=1000
tick_trap_every=50
EOF
else
  : > "$LAB/plugins/soak-flaky/soak.txt"
  : > "$LAB/plugins/soak-quarantine/soak.txt"
fi

if [[ -n "$FLAKY_KNOBS" ]]; then
  cp "$FLAKY_KNOBS" "$LAB/plugins/soak-flaky/soak.txt"
fi

cat > "$LAB/infrarust.toml" <<EOF
bind = "127.0.0.1:$PROXY_PORT"
servers_dir = "$LAB/servers"
plugins_dir = "$LAB/plugins"
connect_timeout = "3s"

[rate_limit]
enabled = false

[permissions]
player_commands = ["server"]

[web]
enable_api = true
enable_webui = false
bind = "127.0.0.1:$API_PORT"
api_key = "$API_KEY"

[web.rate_limit]
requests_per_minute = 1000000

[wasm]
instance_pool = $INSTANCE_POOL
$(if [[ -n "$EPOCH_TICK" ]]; then printf 'epoch_tick = "%s"\n' "$EPOCH_TICK"; fi)

[plugins.soak-witness]
permissions = ["chat-intercept"]

[plugins.soak-flaky]
permissions = ["chat-intercept"]

[plugins.soak-quarantine]
permissions = ["chat-intercept"]

[plugins.soak-codec]
permissions = ["codec-filter"]

[plugins.soak-codec-faulty]
permissions = ["codec-filter"]

$(if [[ "$CODEC_BUDGET" != "default" ]]; then printf '[plugins.soak-codec-faulty.wasm]\ncodec_cpu_budget = "%s"\n' "$CODEC_BUDGET"; fi)

[plugins.soak-flaky.wasm]
$(if [[ "$FLAKY_CPU_BUDGET" != "default" ]]; then printf 'cpu_budget = "%s"\n' "$FLAKY_CPU_BUDGET"; fi)
memory_limit_mb = 32

[plugins.soak-flaky.wasm.recovery]
max_restarts = 1000
window = "1m"

[plugins.soak-quarantine.wasm.recovery]
max_restarts = 3
window = "1m"
backoff_initial = "2s"
backoff_max = "20s"
EOF

cat > "$LAB/servers/soak-a.toml" <<EOF
name = "soak-a"
domains = ["soak-a.local"]
addresses = ["127.0.0.1:$BACKEND_A"]
proxy_mode = "offline"
EOF
cat > "$LAB/servers/soak-b.toml" <<EOF
name = "soak-b"
domains = ["soak-b.local"]
addresses = ["127.0.0.1:$BACKEND_B"]
proxy_mode = "offline"
EOF
cat > "$LAB/servers/soak-bench.toml" <<EOF
name = "soak-bench"
domains = ["soak-bench.local"]
addresses = ["127.0.0.1:$BENCH_BACKEND"]
proxy_mode = "offline"
EOF

echo "==> scenario=$SCENARIO duration=${DURATION}s bots=$BOTS out=$OUT"

node "$HERE/backend.mjs" --ports "$BACKEND_A,$BACKEND_B" --stats "$OUT/backend.json" > "$OUT/backend.log" 2>&1 &
PIDS+=($!)
if [[ "$BENCH_CONNS" -gt 0 && -x "$MC_BENCH" ]]; then
  "$MC_BENCH" serve-backend --host 127.0.0.1 --port "$BENCH_BACKEND" > "$OUT/bench-backend.log" 2>&1 &
  PIDS+=($!)
fi

for _ in $(seq 1 100); do
  if ss -ltnH "sport = :$BACKEND_B" | grep -q .; then break; fi
  sleep 0.1
done

PROXY_START=$(date +%s.%N)
if [[ -n "${SOAK_PROXY_VLIMIT_KB:-}" ]]; then
  (ulimit -v "$SOAK_PROXY_VLIMIT_KB"; exec "$BINARY" --config "$LAB/infrarust.toml" --log-level info) < /dev/null > "$OUT/proxy.log" 2>&1 &
else
  "$BINARY" --config "$LAB/infrarust.toml" --log-level info < /dev/null > "$OUT/proxy.log" 2>&1 &
fi
PROXY_PID=$!
PIDS+=($PROXY_PID)
echo "$PROXY_PID" > "$OUT/proxy.pid"

READY=0
for _ in $(seq 1 600); do
  if ! kill -0 "$PROXY_PID" 2>/dev/null; then
    echo "proxy exited during startup" >&2
    tail -40 "$OUT/proxy.log" >&2
    exit 1
  fi
  if ss -ltnH "sport = :$PROXY_PORT" | grep -q . && ss -ltnH "sport = :$API_PORT" | grep -q .; then
    READY=1
    break
  fi
  sleep 0.1
done
if [[ $READY -ne 1 ]]; then
  echo "proxy never started listening" >&2
  tail -40 "$OUT/proxy.log" >&2
  exit 1
fi
echo "==> proxy up in $(awk -v a="$PROXY_START" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}') s (pid $PROXY_PID)"
sleep 2

PLUGINS_JSON=$(curl -s -H "Authorization: Bearer $API_KEY" "http://127.0.0.1:$API_PORT/api/v1/plugins")
echo "$PLUGINS_JSON" > "$OUT/plugins-at-start.json"
EXPECTED="stats soak-witness soak-flaky soak-quarantine soak-codec"
if [[ $FAULTY_CODEC -eq 1 ]]; then EXPECTED="$EXPECTED soak-codec-faulty"; fi
for id in $EXPECTED; do
  state=$(echo "$PLUGINS_JSON" | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const p=(JSON.parse(s).data||[]).find(x=>x.id==='$id');process.stdout.write(p?p.state:'missing')})")
  echo "    plugin $id: $state"
  if [[ "$state" != "enabled" ]]; then
    echo "plugin $id did not load; see $OUT/proxy.log" >&2
    exit 1
  fi
done

WATCH="$OUT/watch.txt"
: > "$WATCH"
echo "backend=${PIDS[0]}" >> "$WATCH"
node "$HERE/sampler.mjs" --pid "$PROXY_PID" --log "$OUT/proxy.log" --out "$OUT/samples.jsonl" --every 5 --watch "$WATCH" \
  --witness "$LAB/plugins/soak-witness/counts.txt" --backend "$OUT/backend.json" \
  --api "http://127.0.0.1:$API_PORT" --api-key "$API_KEY" &
SAMPLER_PID=$!
PIDS+=($SAMPLER_PID)

node "$HERE/bots.mjs" --port "$PROXY_PORT" --domain soak-a.local --switch-to soak-b --bots "$BOTS" \
  --duration "$DURATION" --session-min "$SESSION_MIN" --session-max "$SESSION_MAX" \
  --chat-ms "$CHAT_MS" --cmd-ms "$CMD_MS" --out "$OUT/bots.jsonl" --summary "$OUT/bots-summary.json" \
  > "$OUT/bots.log" 2>&1 &
BOTS_PID=$!
PIDS+=($BOTS_PID)
echo "bots=$BOTS_PID" >> "$WATCH"

ATTACK_PID=""
if [[ "$ATTACKERS" -gt 0 ]]; then
  node "$HERE/bots.mjs" --port "$PROXY_PORT" --domain soak-a.local --local-address 127.0.0.2 --bots "$ATTACKERS" \
    --duration "$DURATION" --ramp 5 --tag x --session-min 2 --session-max 4 --chat-ms 1000000 --commands '' \
    --switch-chance 0 --abrupt-chance 0 --abort-chance 0 \
    --out "$OUT/attackers.jsonl" --summary "$OUT/attackers-summary.json" > "$OUT/attackers.log" 2>&1 &
  ATTACK_PID=$!
  PIDS+=($ATTACK_PID)
fi

BURST_PID=""
if [[ "$BURST_BOTS" -gt 0 ]]; then
  (
    sleep "$BURST_AT"
    exec node "$HERE/bots.mjs" --port "$PROXY_PORT" --domain soak-a.local --switch-to soak-b --bots "$BURST_BOTS" \
      --duration "$BURST_DURATION" --ramp 2 --tag u --session-min 5 --session-max 15 \
      --out "$OUT/burst.jsonl" --summary "$OUT/burst-summary.json"
  ) > "$OUT/burst.log" 2>&1 &
  BURST_PID=$!
  PIDS+=($BURST_PID)
  echo "burst=$BURST_PID" >> "$WATCH"
fi

BENCH_LOOP_PID=""
if [[ "$BENCH_CONNS" -gt 0 && -x "$MC_BENCH" ]]; then
  (
    END=$(( $(date +%s) + DURATION ))
    i=0
    while [[ $(date +%s) -lt $((END - BENCH_WINDOW)) ]]; do
      i=$((i + 1))
      echo "=== window $i t=$(date +%s)"
      "$MC_BENCH" load --host 127.0.0.1 --port "$PROXY_PORT" --server-address soak-bench.local \
        --protocol 758 --concurrency "$BENCH_CONNS" --rate "$BENCH_RATE" --duration "$((BENCH_WINDOW - 5))" --warmup 3 2>&1
    done
  ) > "$OUT/bench.log" 2>&1 &
  BENCH_LOOP_PID=$!
  PIDS+=($BENCH_LOOP_PID)
fi

if [[ "$SHUTDOWN" == "load" ]]; then
  sleep "$DURATION"
else
  wait "$BOTS_PID"
  if [[ -n "$BURST_PID" ]]; then wait "$BURST_PID"; fi
  if [[ -n "$ATTACK_PID" ]]; then wait "$ATTACK_PID"; fi
  if [[ -n "$BENCH_LOOP_PID" ]]; then wait "$BENCH_LOOP_PID"; fi
  sleep 6
fi

if [[ "$POST_PROBE" -gt 0 && "$SHUTDOWN" != "load" ]]; then
  echo "==> post-load probe for ${POST_PROBE}s"
  node "$HERE/bots.mjs" --port "$PROXY_PORT" --domain soak-a.local --switch-to soak-b --bots 5 \
    --duration "$POST_PROBE" --ramp 1 --tag p --session-min 5 --session-max 10 \
    --abrupt-chance 0 --abort-chance 0 \
    --out "$OUT/post.jsonl" --summary "$OUT/post-summary.json" > "$OUT/post.log" 2>&1
fi

cp "$LAB/plugins/soak-witness/counts.txt" "$OUT/witness-before-shutdown.txt" 2>/dev/null
curl -s -H "Authorization: Bearer $API_KEY" "http://127.0.0.1:$API_PORT/api/v1/plugins" > "$OUT/plugins-at-end.json"
curl -s -H "Authorization: Bearer $API_KEY" "http://127.0.0.1:$API_PORT/api/v1/players/count" > "$OUT/players-at-end.json"
ACTIVE_AT_SHUTDOWN=$(ss -tnH state established "( sport = :$PROXY_PORT )" | wc -l)

SHUTDOWN_START=$(date +%s.%N)
kill -TERM "$PROXY_PID"
EXIT_CODE=""
for _ in $(seq 1 600); do
  if ! kill -0 "$PROXY_PID" 2>/dev/null; then
    wait "$PROXY_PID"
    EXIT_CODE=$?
    break
  fi
  sleep 0.1
done
SHUTDOWN_S=$(awk -v a="$SHUTDOWN_START" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}')
if [[ -z "$EXIT_CODE" ]]; then
  EXIT_CODE="hung"
  kill -KILL "$PROXY_PID" 2>/dev/null
fi
echo "==> shutdown with $ACTIVE_AT_SHUTDOWN client sockets open took ${SHUTDOWN_S}s, exit code $EXIT_CODE"
echo "{\"activeClientSockets\": $ACTIVE_AT_SHUTDOWN, \"seconds\": $SHUTDOWN_S, \"exit\": \"$EXIT_CODE\"}" > "$OUT/shutdown.json"

sleep 6
kill -TERM "$SAMPLER_PID" 2>/dev/null
if [[ "$SHUTDOWN" == "load" ]]; then
  kill -TERM "$BOTS_PID" 2>/dev/null
  [[ -n "$BURST_PID" ]] && kill -TERM "$BURST_PID" 2>/dev/null
  [[ -n "$ATTACK_PID" ]] && kill -TERM "$ATTACK_PID" 2>/dev/null
  [[ -n "$BENCH_LOOP_PID" ]] && pkill -TERM -P "$BENCH_LOOP_PID" 2>/dev/null
  wait "$BOTS_PID" 2>/dev/null
fi
cp "$LAB/plugins/soak-witness/counts.txt" "$OUT/witness-final.txt" 2>/dev/null
cp "$LAB/plugins/soak-flaky/recoveries.txt" "$OUT/flaky-recoveries.txt" 2>/dev/null
cp "$LAB/plugins/soak-quarantine/recoveries.txt" "$OUT/quarantine-recoveries.txt" 2>/dev/null

node "$HERE/report.mjs" "$OUT" > "$OUT/report.txt" 2>&1
cat "$OUT/report.txt"
