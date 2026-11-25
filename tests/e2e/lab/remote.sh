#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SRC="$ROOT/src"
LAB="$SRC/tests/e2e/lab"
DOCKER="sudo -n docker"

tools() {
  $DOCKER run --rm -u "$(id -u):$(id -g)" -v "$ROOT:$ROOT" -w "$ROOT" infrarust-lab-tools "$@"
}

base_version() {
  sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml"
}

cmd_tools() {
  $DOCKER build -q -t infrarust-lab-tools -f "$LAB/tools.Dockerfile" "$LAB" > /dev/null
}

cmd_keys() {
  mkdir -p "$ROOT/keys"
  if [[ ! -f "$ROOT/keys/lab.key" ]]; then
    tools minisign -G -W -p "$ROOT/keys/lab.pub" -s "$ROOT/keys/lab.key" > /dev/null
    chmod 600 "$ROOT/keys/lab.key"
  fi
  tail -n 1 "$ROOT/keys/lab.pub"
}

cmd_build() {
  local name="$1" version="$2" patch="${3:-none}" official="${4:-1}" docker="${5:-0}"
  local pubkey
  pubkey="$(tail -n 1 "$ROOT/keys/lab.pub")"
  local args=(
    --build-arg "LAB_BASE_VERSION=$(base_version)"
    --build-arg "LAB_VERSION=$version"
    --build-arg "LAB_PUBKEY=$pubkey"
    --build-arg "LAB_OFFICIAL=$official"
    --build-arg "LAB_DOCKER=$docker"
    --build-arg "LAB_PATCH=$patch"
    --build-arg "LAB_LTO=${LAB_LTO:-fat}"
    -f "$LAB/Dockerfile"
  )
  if [[ "$docker" == "1" ]]; then
    $DOCKER build "${args[@]}" --target runtime -t "infrarust-lab:$name" "$SRC"
  else
    local fresh="$ROOT/dist/.$name.fresh"
    sudo -n rm -rf "$fresh"
    mkdir -p "$fresh"
    $DOCKER build "${args[@]}" --target binary --output "type=local,dest=$fresh" "$SRC"
    sudo -n chown -R "$(id -u):$(id -g)" "$fresh"
    chmod 755 "$fresh/infrarust"
    rm -rf "$ROOT/dist/$name"
    mv "$fresh" "$ROOT/dist/$name"
    "$ROOT/dist/$name/infrarust" --handover-probe < /dev/null
  fi
}

cmd_publish() {
  local name="$1" security="${2:-false}"
  local binary="$ROOT/dist/$name/infrarust"
  local probe version tag handover work
  probe="$("$binary" --handover-probe < /dev/null | head -n 1)"
  version="$(jq -r .version <<< "$probe")"
  handover="$(jq -c '.handover | if type == "object" then . else null end' <<< "$probe")"
  tag="v$version"
  work="$ROOT/releases/.work-$tag"
  rm -rf "$work" "$ROOT/releases/$tag"
  mkdir -p "$work/dist" "$work/stage" "$work/dummy"
  cp "$binary" "$work/stage/infrarust"
  tar -C "$work/stage" -czf "$work/dist/infrarust-linux-x86_64.tar.gz" infrarust
  printf 'not a binary\n' > "$work/dummy/infrarust"
  printf 'not a binary\n' > "$work/dummy/infrarust.exe"
  for archive in linux-arm64 darwin-x86_64 darwin-arm64; do
    tar -C "$work/dummy" -czf "$work/dist/infrarust-$archive.tar.gz" infrarust
  done
  tools bash -c "cd '$work/dummy' && zip -q ../dist/infrarust-windows-x86_64.zip infrarust.exe && zip -q ../dist/infrarust-windows-arm64.zip infrarust.exe"
  tools bash "$SRC/scripts/release-manifest.sh" \
    --version "$version" --tag "$tag" \
    --published-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --security "$security" --handover "$handover" \
    --dist "$work/dist" --out "$work/out"
  tools minisign -S -H -s "$ROOT/keys/lab.key" -m "$work/out/manifest.json" -t "infrarust $tag" < /dev/null > /dev/null
  mkdir -p "$ROOT/releases/$tag"
  cp "$work/dist/infrarust-linux-x86_64.tar.gz" "$work/out/manifest.json" "$work/out/manifest.json.minisig" "$work/out/SHA256SUMS" "$ROOT/releases/$tag/"
  rm -rf "$work"
  echo "published $tag (handover $handover)"
}

cmd_stage() {
  mkdir -p "$ROOT/stack" "$ROOT/releases" "$ROOT/native"
  rsync -a --delete --exclude /plugins "$ROOT/stage/proxy-docker/" "$ROOT/stack/proxy-docker/"
  mkdir -p "$ROOT/stack/proxy-docker/plugins"
  chmod -R a+rwX "$ROOT/stack/proxy-docker" 2> /dev/null || true
  rsync -a "$ROOT/stage/drasl/" "$ROOT/stack/drasl/"
  mkdir -p "$ROOT/stack/drasl/data"
  cp "$ROOT/stage/compose.yml" "$ROOT/stack/"
  if ! cmp -s "$ROOT/stage/release-server.mjs" "$ROOT/stack/release-server.mjs"; then
    cp "$ROOT/stage/release-server.mjs" "$ROOT/stack/"
    if [[ -n "$($DOCKER ps -q -f name=lab-releases)" ]]; then $DOCKER restart lab-releases > /dev/null; fi
  fi
  for seed in "$ROOT/stage/backend-seed"/*; do
    mkdir -p "$ROOT/stack/backends/$(basename "$seed")"
    cp -rn "$seed/." "$ROOT/stack/backends/$(basename "$seed")/"
  done
  rsync -a --delete --exclude infrarust --exclude 'infrarust.old' --exclude '.*' --exclude '/plugins' "$ROOT/stage/proxy-native/" "$ROOT/native/"
  sudo -n cp "$ROOT/stage/infrarust-lab.service" /etc/systemd/system/infrarust-lab.service
  sudo -n systemctl daemon-reload
}

cmd_up() {
  cd "$ROOT/stack"
  $DOCKER compose up -d "$@"
}

cmd_down() {
  cd "$ROOT/stack"
  sudo -n systemctl stop infrarust-lab 2> /dev/null || true
  $DOCKER compose --profile docker-proxy down
}

cmd_install() {
  local name="$1"
  sudo -n systemctl stop infrarust-lab 2> /dev/null || true
  rm -f "$ROOT/native/infrarust" "$ROOT/native/infrarust.old" "$ROOT/native"/.infrarust.*
  cp "$ROOT/dist/$name/infrarust" "$ROOT/native/infrarust"
  sudo -n systemctl start infrarust-lab
}

command="${1:-}"
shift || true
case "$command" in
  tools | keys | build | publish | stage | up | down | install) "cmd_$command" "$@" ;;
  *) echo "usage: remote.sh tools|keys|build|publish|stage|up|down|install" >&2; exit 2 ;;
esac
