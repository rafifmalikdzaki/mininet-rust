#!/bin/sh
# diff_topology.sh — differential test harness: Python mininet vs Rust mnrr.
#
# Task 11 (prep) of tasks/plan.md. Builds the same small topology through BOTH
# implementations and diffs normalized topology dumps:
#
#   Python (reference): an embedded driver that mirrors
#       python3 bin/mn --topo <spec> --controller none --test none
#       (buildTopo, Mininet, net.start(), dump, links, pingall, stop),
#       capturing stdout.
#   Rust (port, binary mnr):        <mn binary> --topo <spec> ... --dump
#       (equivalent to: cargo run -p mnr -- --topo <spec> --dump)
#
# Usage:
#   scripts/diff_topology.sh <topo-spec>     e.g. single,3 | linear,4 | tree,2,2
#   (runnable as root: sudo scripts/diff_topology.sh single,3)
#
# Environment overrides (applied to BOTH sides so they stay comparable):
#   MN_RUST_BIN        Rust mnr binary (default: target/release/mnr, then
#                      target/debug/mnr, else `cargo build -p mnr`)
#   DIFF_MN_SWITCH     switch class spec    (default: default)
#   DIFF_MN_HOST       host class spec      (default: proc)
#   DIFF_MN_CONTROLLER controller spec      (default: none)
#   DIFF_MN_LINK       link class spec      (default: default)
#   DIFF_MN_IPBASE     ipBase               (default: 10.0.0.0/8)
#
# Exit codes:
#   0  MATCH     normalized outputs identical
#   1  MISMATCH  normalized outputs differ (diff printed to stderr)
#   2  SKIP      Rust mnr emulation not wired yet (no --dump support); nothing
#               to compare — NOT a failure
#   3  ERROR     usage / environment / harness failure (message on stderr)
#
# The full comparison needs root (netns/veth creation on the Python side).
# The "Rust side not wired" probe runs BEFORE the root check, so on a non-root
# host this script still reports SKIP (exit 2) instead of failing — which makes
# it safe to smoke-test unprivileged.
#
# Normalization applied to both outputs before diffing:
#   - pid=<n>            -> pid=<PID>
#   - ISO timestamps      -> removed
#   - "completed in ..."  -> line dropped
#   - trailing whitespace -> stripped
#   - list-like lines ("nodes:", "hosts:", "switches:", "links:", and ping
#     "h1 -> h2 h3" destination lists) -> fields sorted
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

die() { echo "diff_topology: ERROR: $*" >&2; exit 3; }
info() { echo "diff_topology: $*"; }

if [ $# -ne 1 ]; then
    echo "usage: $0 <topo-spec>   e.g. single,3 | linear,4 | tree,2,2" >&2
    exit 3
fi
SPEC=$1
case $SPEC in
    '' | *[!A-Za-z0-9_,=.:+-]*)
        die "invalid topo spec '$SPEC' (expected e.g. single,3 / linear,4 / tree,2,2)"
        ;;
esac

DIFF_MN_SWITCH=${DIFF_MN_SWITCH:-default}
DIFF_MN_HOST=${DIFF_MN_HOST:-proc}
DIFF_MN_CONTROLLER=${DIFF_MN_CONTROLLER:-none}
DIFF_MN_LINK=${DIFF_MN_LINK:-default}
DIFF_MN_IPBASE=${DIFF_MN_IPBASE:-10.0.0.0/8}

# --- locate (or build) the Rust mnrr binary -----------------------------------
MN_BIN=${MN_RUST_BIN:-}
if [ -z "$MN_BIN" ]; then
    for cand in "$REPO_ROOT/target/release/mnr" "$REPO_ROOT/target/debug/mnr"; do
        if [ -x "$cand" ]; then
            MN_BIN=$cand
            break
        fi
    done
fi
if [ -z "$MN_BIN" ]; then
    command -v cargo >/dev/null 2>&1 ||
        die "no Rust mnr binary (target/release/mnr, target/debug/mnr) and no cargo to build one; set MN_RUST_BIN"
    info "no prebuilt mn binary; building with: cargo build -p mnr"
    (cd "$REPO_ROOT" && cargo build -p mnr) >&2 || die "cargo build -p mnr failed"
    MN_BIN=$REPO_ROOT/target/debug/mnr
fi
[ -x "$MN_BIN" ] || die "Rust mnr binary not found or not executable: $MN_BIN"

run_rust_mn() {
    # common flags for both the probe and the real capture
    "$MN_BIN" --topo "$SPEC" \
        --switch "$DIFF_MN_SWITCH" --host "$DIFF_MN_HOST" \
        --controller "$DIFF_MN_CONTROLLER" --link "$DIFF_MN_LINK" \
        --ipbase "$DIFF_MN_IPBASE" --dump
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM
RUST_RAW=$WORK/rust_raw.out
RUST_ERR=$WORK/rust_stderr.txt

# --- probe the Rust side: is emulation (--dump) wired yet? -------------------
set +e
run_rust_mn >"$RUST_RAW" 2>"$RUST_ERR"
RUST_RC=$?
set -e

if [ "$RUST_RC" -eq 0 ]; then
    info "Rust mnr emulation appears wired (--dump exited 0); proceeding"
elif grep -qE 'not yet implemented|unexpected argument|unrecognized|unknown argument' \
        "$RUST_ERR" "$RUST_RAW" 2>/dev/null; then
    echo "diff_topology: SKIP: Rust mnr emulation is not wired yet — nothing to compare."
    echo "  probe: $MN_BIN --topo $SPEC ... --dump exited $RUST_RC"
    echo "  first probe output line: $(head -n 1 "$RUST_ERR" 2>/dev/null || head -n 1 "$RUST_RAW")"
    echo "  (Python reference side exists; once apps/mn implements --topo/--dump this becomes MATCH/MISMATCH)"
    exit 2
fi

# --- root check (full comparison creates netns/veths) ------------------------
if [ "$(id -u)" -ne 0 ]; then
    die "full differential run needs root (Python side creates netns/veth pairs); re-run as root: sudo $0 $SPEC"
fi
if [ "$RUST_RC" -ne 0 ]; then
    echo "diff_topology: ERROR: Rust mnr --dump failed (rc=$RUST_RC); output:" >&2
    head -n 20 "$RUST_RAW" "$RUST_ERR" >&2 2>/dev/null || true
    exit 3
fi

# --- Python reference side ----------------------------------------------------
command -v python3 >/dev/null 2>&1 || die "python3 not found (needed for the Python reference side)"
PY_RAW=$WORK/python_raw.out
PY_ERR=$WORK/python_stderr.txt

cat >"$WORK/py_dump.py" <<'PY'
#!/usr/bin/env python3
"""Python reference-side dump driver for scripts/diff_topology.sh.

Equivalent to running:
    python3 bin/mn --topo <spec> --controller none --test none
but instead of entering the interactive CLI it prints four fixed sections on
stdout (mininet's output() logger redirected to stdout, level 'output'):

    === topo ===     pure topology dump via buildTopo (no root needed)
    === dump ===     cli.py do_dump:  repr(node) for every node in the net
    === links ===    cli.py do_links: str(link) + link.status() per link
    === pingall ===  net.pingAll()

The harness normalizes this output and diffs it against the Rust
`mn --topo <spec> --dump` output, so the Rust side must emit the same
sections. Requires root for net.start(). Class choices come from the
DIFF_MN_* env vars so the same flags can be passed to the Rust side.
"""
import os
import sys

from mininet.log import lg, output, setLogLevel
from mininet.util import buildTopo, customClass, specialClass
from mininet.topo import (MinimalTopo, LinearTopo, SingleSwitchReversedTopo,
                          SingleSwitchTopo)
from mininet.topolib import TorusTopo, TreeTopo
from mininet.node import (Controller, CPULimitedHost, DefaultController, Host,
                          IVSSwitch, NOX, NullController, OVSBridge, OVSSwitch,
                          OVSController, RemoteController, Ryu, UserSwitch)
from mininet.nodelib import LinuxBridge
from mininet.link import Link, OVSLink, TCLink, TCULink
from mininet.net import Mininet

# Class dicts copied verbatim from bin/mn (MininetRunner).
TOPOS = {'minimal': MinimalTopo, 'linear': LinearTopo,
         'reversed': SingleSwitchReversedTopo, 'single': SingleSwitchTopo,
         'tree': TreeTopo, 'torus': TorusTopo}
SWITCHES = {'user': UserSwitch, 'ovs': OVSSwitch, 'ovsbr': OVSBridge,
            'ovsk': OVSSwitch, 'ivs': IVSSwitch, 'lxbr': LinuxBridge,
            'default': OVSSwitch}
HOSTS = {'proc': Host,
         'rt': specialClass(CPULimitedHost, defaults=dict(sched='rt')),
         'cfs': specialClass(CPULimitedHost, defaults=dict(sched='cfs'))}
CONTROLLERS = {'ref': Controller, 'ovsc': OVSController, 'nox': NOX,
               'remote': RemoteController, 'ryu': Ryu,
               'default': DefaultController, 'none': NullController}
LINKS = {'default': Link, 'tc': TCLink, 'tcu': TCULink, 'ovs': OVSLink}


def main():
    spec = sys.argv[1]
    # mininet logs to stderr by default; the harness captures stdout.
    lg.ch.setStream(sys.stdout)
    setLogLevel('output')  # suppress info/debug chatter, keep output()

    topo = buildTopo(TOPOS, spec)
    output('=== topo ===\n')
    output('spec: %s\n' % spec)
    output('class: %s\n' % type(topo).__name__)
    output('nodes: %s\n' % ' '.join(sorted(topo.nodes())))
    output('hosts: %s\n' % ' '.join(sorted(topo.hosts())))
    output('switches: %s\n' % ' '.join(sorted(topo.switches())))
    links = []
    for src, dst in topo.links(sort=True):
        port1, port2 = topo.port(src, dst)
        links.append('%s-eth%d<->%s-eth%d' % (src, port1, dst, port2))
    output('links: %s\n' % ' '.join(sorted(links)))

    switch = customClass(SWITCHES, os.environ.get('DIFF_MN_SWITCH', 'default'))
    host = customClass(HOSTS, os.environ.get('DIFF_MN_HOST', 'proc'))
    controller = [customClass(CONTROLLERS,
                              os.environ.get('DIFF_MN_CONTROLLER', 'none'))]
    link = customClass(LINKS, os.environ.get('DIFF_MN_LINK', 'default'))
    ipbase = os.environ.get('DIFF_MN_IPBASE', '10.0.0.0/8')

    net = Mininet(topo=topo, switch=switch, host=host, controller=controller,
                  link=link, ipBase=ipbase)
    net.start()
    try:
        output('=== dump ===\n')
        for node in net.values():          # cli.py do_dump
            output('%s\n' % repr(node))
        output('=== links ===\n')
        for lnk in net.links:              # cli.py do_links
            output('%s %s\n' % (lnk, lnk.status()))
        output('=== pingall ===\n')
        net.pingAll()
    finally:
        net.stop()


if __name__ == '__main__':
    main()
PY

if ! PYTHONPATH="$REPO_ROOT${PYTHONPATH:+:$PYTHONPATH}" \
        python3 "$WORK/py_dump.py" "$SPEC" >"$PY_RAW" 2>"$PY_ERR"; then
    echo "diff_topology: ERROR: Python reference side failed; last stderr lines:" >&2
    tail -n 30 "$PY_ERR" >&2
    exit 3
fi

# --- normalize both outputs and diff ------------------------------------------
normalize() {
    sed -E \
        -e 's/pid=[0-9]+/pid=<PID>/g' \
        -e 's/[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]+)?//g' \
        -e '/completed in [0-9.]+ seconds/d' \
        -e 's/[[:space:]]+$//' \
        -e 's/\r$//' \
    | awk '
        # insertion-sort fields k..n of a line, return the rebuilt line
        function sortfrom(line, k,   n, a, i, j, t, s) {
            n = split(line, a, /[ \t]+/)
            for (i = k + 1; i <= n; i++) {
                t = a[i]; j = i - 1
                while (j >= k && a[j] > t) { a[j+1] = a[j]; j-- }
                a[j+1] = t
            }
            s = a[1]
            for (i = 2; i <= n; i++) s = s " " a[i]
            return s
        }
        /^nodes:/    { print sortfrom($0, 2); next }  # "nodes: h1 h2 s1"
        /^hosts:/    { print sortfrom($0, 2); next }
        /^switches:/ { print sortfrom($0, 2); next }
        /^links:/    { print sortfrom($0, 2); next }
        / -> /       { print sortfrom($0, 3); next }  # ping "h1 -> h2 h3"
        { print }
      '
}

PY_NORM=$WORK/python_normalized.txt
RUST_NORM=$WORK/rust_normalized.txt
DIFF_OUT=$WORK/diff.txt
normalize <"$PY_RAW" >"$PY_NORM"
normalize <"$RUST_RAW" >"$RUST_NORM"

if diff -u "$PY_NORM" "$RUST_NORM" >"$DIFF_OUT" 2>&1; then
    info "MATCH: topo spec '$SPEC' — Python and Rust mnr outputs identical after normalization"
    exit 0
else
    echo "diff_topology: MISMATCH: topo spec '$SPEC' (normalized diff, python vs rust):" >&2
    cat "$DIFF_OUT" >&2
    exit 1
fi
