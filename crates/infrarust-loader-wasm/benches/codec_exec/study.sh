#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-${TMPDIR:-/tmp}/codec-exec-study/$(date +%Y%m%d-%H%M%S)}"
RUNS="${RUNS:-3}"
PART="${PART:-all}"
mkdir -p "$OUT"
RUN="python3 $HERE/run.py"

CONFIGS="${CONFIGS:-default}"
EXEC="$CONFIGS"
ISOLATION="$CONFIGS"
CREATE_ATTACK="$CONFIGS"
BURST="$CONFIGS"

echo "load before: $(cut -d' ' -f1-3 /proc/loadavg)" | tee "$OUT/meta.txt"

if [[ "$PART" == all || "$PART" == micro ]]; then
  for scenario in hot rr create sizes; do
    $RUN "$scenario" --configs "$EXEC" --runs "$RUNS" --env STUDY_WORKERS=2 --out "$OUT/$scenario.json" | tee "$OUT/$scenario.md"
  done
  $RUN load --configs "$EXEC" --runs "$RUNS" --env STUDY_WORKERS=0 --out "$OUT/load.json" | tee "$OUT/load.md"
  $RUN load --configs "${CONFIGS%%,*}" --runs "$RUNS" --env STUDY_WORKERS=0 --env STUDY_LOAD_REGISTRY=none --out "$OUT/load-none.json" | tee "$OUT/load-none.md"
  $RUN idle --configs "$CONFIGS" --runs "$RUNS" --out "$OUT/idle.json" | tee "$OUT/idle.md"
fi

if [[ "$PART" == all || "$PART" == isolation ]]; then
  for profile in sparse sparse-w2pinned w2pinned w2 wdefault; do
    $RUN isolation --configs "$ISOLATION" --profile "$profile" --runs "$RUNS" --out "$OUT/isolation-$profile.json" | tee "$OUT/isolation-$profile.md"
  done
fi

if [[ "$PART" == all || "$PART" == create-attack ]]; then
  for profile in sparse w2pinned; do
    $RUN isolation --configs "$CREATE_ATTACK" --profile "$profile" --runs "$RUNS" --env STUDY_ATTACK=create --out "$OUT/create-attack-$profile.json" | tee "$OUT/create-attack-$profile.md"
  done
fi

if [[ "$PART" == all || "$PART" == burst ]]; then
  for profile in w2pinned wdefault; do
    $RUN isolation --configs "$BURST" --profile "$profile" --runs "$RUNS" --env STUDY_ATTACK=burst --env STUDY_ATTACK_PERIOD_MS=1000 --out "$OUT/burst-$profile.json" | tee "$OUT/burst-$profile.md"
  done
fi

echo "load after: $(cut -d' ' -f1-3 /proc/loadavg)" | tee -a "$OUT/meta.txt"
echo "results in $OUT"
