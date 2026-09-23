#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

PROFILE="${1:-}"
if [[ -z "$PROFILE" ]]; then
  echo "Usage: ./tierb.sh <profile>   e.g. ./tierb.sh v1_21_4" >&2
  echo "Profiles: $(grep -oP 'profiles: \["\K[^"]+' docker-compose.yml | sort -u | tr '\n' ' ')" >&2
  exit 1
fi

SECRET="$(node -e "import('./src/lab.js').then(m => process.stdout.write(m.FORWARDING_SECRET))")"

rm -rf .paper
mkdir -p .paper/config .paper/legacy
sed "s|\${CFG_VELOCITY_SECRET}|$SECRET|" fixtures/paper/global/paper-global.yml > .paper/config/paper-global.yml
sed "s|\${CFG_VELOCITY_SECRET}|$SECRET|" fixtures/paper/legacy/paper.yml > .paper/legacy/paper.yml
cp fixtures/paper/spigot.yml .paper/spigot.yml
chmod -R 0777 .paper

echo "==> starting profile $PROFILE"
podman compose --profile "$PROFILE" up -d

echo "==> waiting for backends to report healthy (cold start pulls a jar and generates a world)"
deadline=$(( $(date +%s) + 600 ))
while :; do
  mapfile -t states < <(podman ps -a --format '{{.Names}} {{.Status}}' | grep -E "e2e-(plain|velocity|bungee)-" || true)
  [[ ${#states[@]} -gt 0 ]] || { echo "no backend containers found" >&2; exit 1; }

  if printf '%s\n' "${states[@]}" | grep -q 'Exited'; then
    echo "a backend exited:" >&2
    printf '  %s\n' "${states[@]}" >&2
    exit 1
  fi
  if ! printf '%s\n' "${states[@]}" | grep -qv 'healthy'; then
    printf '  %s\n' "${states[@]}"
    echo "==> all backends healthy"
    exit 0
  fi
  if (( $(date +%s) > deadline )); then
    echo "timed out waiting for health:" >&2
    printf '  %s\n' "${states[@]}" >&2
    exit 1
  fi
  sleep 5
done
