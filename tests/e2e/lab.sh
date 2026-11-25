#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
REMOTE="${LAB_REMOTE:-shado@192.168.50.226}"
ROOT="${LAB_REMOTE_ROOT:-infrarust-lab}"
SOCKET="${XDG_RUNTIME_DIR:-/tmp}/infrarust-lab-ssh"
SSH=(ssh -o BatchMode=yes -o ControlMaster=auto -o ControlPath="$SOCKET" -o ControlPersist=600 "$REMOTE")

usage() {
  cat <<'USAGE'
Usage: ./lab.sh <command>

Drives the remote lab: a host you own that runs the proxy, real Paper servers,
a Yggdrasil server and a fake release feed, while this machine runs the clients.
Never run on CI.

  sync                      Copy the sources and the generated stack to the host.
  keys                      Create the lab minisign key pair on the host (once).
  build <name> <version> [patch] [official] [docker]
                            Build a proxy binary (or image with docker=1) on the host.
  publish <name> [security] Package dist/<name> as a signed release of the fake feed.
  up [service...]           Start the stack (compose up -d).
  install <name>            Put dist/<name> under systemd as the native proxy.
  down                      Stop everything.
  tunnel                    Forward the lab ports to this machine over SSH.
  ssh <command...>          Run a command on the host.
  run [options]             Run the scenarios (src/lab/run.js).

LAB_REMOTE overrides the host (default shado@192.168.50.226).
LAB_LTO=thin builds with thin LTO: the same code, linked in minutes instead of
a quarter of an hour. The default is the release profile.
USAGE
}

remote() {
  "${SSH[@]}" "LAB_LTO=${LAB_LTO:-fat} bash $ROOT/src/tests/e2e/lab/remote.sh $*"
}

command="${1:-}"
shift || true
case "$command" in
  sync)
    home="$("${SSH[@]}" 'echo $HOME')"
    node "$HERE/src/lab/stage-cli.js" "$HERE/.lab/stage" "$home/$ROOT"
    "${SSH[@]}" "mkdir -p $ROOT/src $ROOT/stage"
    rsync -a --delete -e "ssh -o ControlPath=$SOCKET" \
      --exclude '/target' --exclude '/.git' --exclude '/.claude' --exclude '/docs' \
      --exclude 'node_modules' --exclude '.mccache' --exclude '.lab' --exclude '.lab-real' \
      --exclude '.nuxt' --exclude '.output' --exclude '/tests/e2e/reports' \
      "$REPO/" "$REMOTE:$ROOT/src/"
    rsync -a --delete -e "ssh -o ControlPath=$SOCKET" "$HERE/.lab/stage/" "$REMOTE:$ROOT/stage/"
    remote tools
    remote stage
    ;;
  keys | build | publish | up | down | install) remote "$command" "$@" ;;
  tunnel)
    ports=(25565 25566 8080 8081 25585 8900)
    forwards=()
    for port in "${ports[@]}"; do forwards+=(-L "127.0.0.1:$port:127.0.0.1:$port"); done
    exec ssh -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -N "${forwards[@]}" "$REMOTE"
    ;;
  ssh) "${SSH[@]}" "$@" ;;
  run) cd "$HERE" && exec node src/lab/run.js "$@" ;;
  -h | --help | "") usage ;;
  *) usage >&2; exit 2 ;;
esac
