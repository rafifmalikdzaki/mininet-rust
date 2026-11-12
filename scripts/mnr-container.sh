#!/bin/sh
# mnr-container.sh — run mnr (Rust mininet) inside a ROOTLESS container:
# no sudo, no root on the host. Rootless podman/docker map the container's
# root into a user namespace (via /etc/subuid + newuidmap); `--privileged`
# then grants CAP_NET_ADMIN inside that namespace only, which is all mininet
# needs to create netns/veth pairs.
#
# Usage:
#   sh scripts/mnr-container.sh --topo single,3 --test pingall
#   sh scripts/mnr-container.sh --topo tree,2,2          # interactive REPL
#   MNR_IMAGE=myimg sh scripts/mnr-container.sh ...      # custom image
#   MNR_RUNTIME=docker sh scripts/mnr-container.sh ...   # force runtime
#   sh scripts/mnr-container.sh --build                  # (re)build the image
#
# Requirements (one-time, no root needed):
#   podman:  newuidmap/newgidmap installed + your user in /etc/subuid,subgid
#            (most distros ship this by default; check `podman info`)
#   docker:  rootless Docker (dockerd-rootless-setuptool.sh install)
#
# Notes:
# - Unless you pass --switch yourself, the wrapper adds `--switch lxbr`
#   (kernel LinuxBridge): rootless containers usually cannot load the OVS
#   kernel datapath module on the host, and lxbr needs no kernel module.
#   Pass `--switch ovs` explicitly when the host has the module.
# - The current directory is mounted at /w so topo files / dumps / MCP state
#   written by mnr land back on the host.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
IMAGE=${MNR_IMAGE:-mnr-rootless:latest}
RUNTIME=${MNR_RUNTIME:-}

log() { echo "mnr-container: $*" >&2; }

# --- pick a container runtime (podman rootless preferred, then docker) ------
if [ -z "$RUNTIME" ]; then
    if command -v podman >/dev/null 2>&1; then
        RUNTIME=podman
    elif command -v docker >/dev/null 2>&1; then
        RUNTIME=docker
    else
        log "ERROR: neither podman nor docker found. Install rootless podman"
        log "  (apt install podman uidmap) or rootless docker, then retry."
        exit 3
    fi
fi

# --- build the image if needed ------------------------------------------------
if [ "${1:-}" = "--build" ]; then
    log "building $IMAGE from container/Dockerfile ($RUNTIME)"
    "$RUNTIME" build -t "$IMAGE" -f "$REPO_ROOT/container/Dockerfile" "$REPO_ROOT"
    exit 0
fi
if ! "$RUNTIME" image inspect "$IMAGE" >/dev/null 2>&1; then
    log "image $IMAGE not found; building it now (one-time)"
    "$RUNTIME" build -t "$IMAGE" -f "$REPO_ROOT/container/Dockerfile" "$REPO_ROOT"
fi

# --- default to the kernel LinuxBridge switch inside containers --------------
has_switch=0
for a in "$@"; do
    case "$a" in --switch | --switch=*) has_switch=1 ;; esac
done
if [ "$has_switch" -eq 0 ]; then
    log "note: no --switch given; adding '--switch lxbr' (no OVS kernel module"
    log "      in most rootless containers; pass --switch ovs to override)"
    set -- "$@" --switch lxbr
fi

# --- runtime flags ------------------------------------------------------------
# --privileged is confined to the container's user namespace in rootless
# mode: it grants CAP_NET_ADMIN *there*, never on the host.
FLAGS="--rm --privileged"
TTYFLAGS=""
if [ -t 0 ]; then TTYFLAGS="-it"; fi
USERNS=""
if [ "$RUNTIME" = podman ]; then
    # keep the host uid inside the container so files written to /w are
    # owned by you, not by a mapped root
    USERNS="--userns=keep-id"
fi

log "running: $RUNTIME run $FLAGS $TTYFLAGS $USERNS -v $PWD:/w -w /w $IMAGE mnr $*"
# shellcheck disable=SC2086
exec "$RUNTIME" run $FLAGS $TTYFLAGS $USERNS \
    -v "$PWD:/w" -w /w "$IMAGE" mnr "$@"
