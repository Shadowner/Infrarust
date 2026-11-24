#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --version VERSION --tag TAG --published-at RFC3339 --security true|false --handover JSON|null --dist DIR --out DIR" >&2
  exit 2
}

fail() {
  echo "release-manifest: $*" >&2
  exit 1
}

artifacts=(
  "infrarust-linux-x86_64.tar.gz infrarust x86_64-unknown-linux-musl x86_64-unknown-linux-gnu"
  "infrarust-linux-arm64.tar.gz infrarust aarch64-unknown-linux-musl aarch64-unknown-linux-gnu"
  "infrarust-darwin-x86_64.tar.gz infrarust x86_64-apple-darwin"
  "infrarust-darwin-arm64.tar.gz infrarust aarch64-apple-darwin"
  "infrarust-windows-x86_64.zip infrarust.exe x86_64-pc-windows-msvc"
  "infrarust-windows-arm64.zip infrarust.exe aarch64-pc-windows-msvc"
)

version=""
tag=""
published_at=""
security=""
handover=""
dist=""
out=""

while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || usage
  case "$1" in
    --version) version="$2" ;;
    --tag) tag="$2" ;;
    --published-at) published_at="$2" ;;
    --security) security="$2" ;;
    --handover) handover="$2" ;;
    --dist) dist="$2" ;;
    --out) out="$2" ;;
    *) usage ;;
  esac
  shift 2
done

for name in version tag published_at security handover dist out; do
  [ -n "${!name}" ] || fail "missing --${name//_/-}"
done

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || fail "version $version is not semver"
[ "$tag" = "v$version" ] || fail "tag $tag does not match version $version"
[[ "$published_at" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$ ]] || fail "published_at $published_at is not an RFC 3339 timestamp"
case "$security" in
  true | false) ;;
  *) fail "security must be true or false, got $security" ;;
esac
jq -en --argjson handover "$handover" '$handover | type == "object" or type == "null"' > /dev/null 2>&1 ||
  fail "handover must be a JSON object or null, got $handover"
[ -d "$dist" ] || fail "dist directory $dist does not exist"

expected=()
for entry in "${artifacts[@]}"; do
  expected+=("${entry%% *}")
done

shopt -s nullglob
for path in "$dist"/infrarust-*; do
  file="${path##*/}"
  known=false
  for name in "${expected[@]}"; do
    if [ "$file" = "$name" ]; then
      known=true
    fi
  done
  [ "$known" = true ] || fail "unexpected archive $file in $dist"
done
shopt -u nullglob

for file in "${expected[@]}"; do
  [ -f "$dist/$file" ] || fail "missing archive $file in $dist"
  [ -s "$dist/$file" ] || fail "archive $file in $dist is empty"
done

list_entries() {
  case "$1" in
    *.tar.gz) tar -tzf "$1" ;;
    *.zip) unzip -Z1 "$1" ;;
    *) return 1 ;;
  esac
}

sums=""
entries="[]"
for entry in "${artifacts[@]}"; do
  read -r -a fields <<< "$entry"
  file="${fields[0]}"
  binary="${fields[1]}"
  targets=("${fields[@]:2}")
  path="$dist/$file"

  listing=$(list_entries "$path") || fail "cannot list the content of $file"
  count=$(grep -cxF -e "$binary" -e "./$binary" <<< "$listing" || true)
  [ "$count" -eq 1 ] || fail "$file must contain $binary exactly once, found $count"

  sha256=$(sha256sum -- "$path")
  sha256="${sha256%% *}"
  size=$(($(wc -c < "$path")))
  sums+="$sha256  $file"$'\n'

  entries=$(jq -c \
    --arg file "$file" \
    --arg binary "$binary" \
    --arg sha256 "$sha256" \
    --argjson size "$size" \
    '. + [{targets: $ARGS.positional, file: $file, binary: $binary, sha256: $sha256, size: $size}]' \
    --args "${targets[@]}" <<< "$entries")
done

repository="${GITHUB_REPOSITORY:-Shadowner/Infrarust}"
notes_url="https://github.com/$repository/releases/tag/$tag"

manifest=$(jq -n \
  --arg version "$version" \
  --arg tag "$tag" \
  --arg published_at "$published_at" \
  --arg notes_url "$notes_url" \
  --argjson security "$security" \
  --argjson handover "$handover" \
  --argjson artifacts "$entries" \
  '{schema: 1, version: $version, tag: $tag, published_at: $published_at, notes_url: $notes_url, security: $security}
  + (if $handover == null then {} else {handover: $handover} end)
  + {artifacts: $artifacts}')

mkdir -p "$out"
printf '%s' "$sums" > "$out/SHA256SUMS"
printf '%s\n' "$manifest" > "$out/manifest.json"
echo "wrote $out/manifest.json and $out/SHA256SUMS"
