#!/bin/sh
# bench_scenarios.sh — differential benchmark ladder: Python mininet (bin/mn)
# vs the Rust port (mnr), across topologies of increasing complexity.
#
# Root-only (both sides create netns/veth pairs). Usage:
#   sudo sh scripts/bench_scenarios.sh
#
# Env:
#   BENCH_TOPOS   space-separated topo specs (default: the ladder below)
#   BENCH_RUNS    timed runs per scenario per implementation (default 5,
#                 plus 1 warmup)
#   BENCH_SWITCH  switch spec for both sides (default: ovs,failMode=standalone;
#                 use lxbr only where OVS is unavailable)
#   MN_RUST_BIN   Rust binary (default target/release/mnr, then debug)
#   BENCH_OUT     output markdown file (default: stdout only)
#
# Output: markdown table  scenario | impl | nodes | wall s (median/min/max)
#          | ping loss. Exit 0 on success, 3 if not root, 2 on usage error.

set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO_ROOT"

if [ "$(id -u)" -ne 0 ]; then
    echo "bench_scenarios: must run as root (both implementations create netns/veth pairs). Nothing was run." >&2
    exit 3
fi

BENCH_TOPOS=${BENCH_TOPOS:-"single,3 linear,4 linear,10 tree,2,2 tree,3,2 tree,3,3"}
BENCH_RUNS=${BENCH_RUNS:-5}
BENCH_SWITCH=${BENCH_SWITCH:-ovs,failMode=standalone}
BENCH_OUT=${BENCH_OUT:-}

case $BENCH_RUNS in
    '' | *[!0-9]*)
        echo "bench_scenarios: BENCH_RUNS must be a positive integer (got '$BENCH_RUNS')" >&2
        exit 2
        ;;
esac
if [ "$BENCH_RUNS" -lt 1 ]; then
    echo "bench_scenarios: BENCH_RUNS must be at least 1" >&2
    exit 2
fi

MN_BIN=""
for cand in "${MN_RUST_BIN:-}" "$REPO_ROOT/target/release/mnr" "$REPO_ROOT/target/debug/mnr"; do
    if [ -n "$cand" ] && [ -x "$cand" ]; then MN_BIN=$cand; break; fi
done
if [ -z "$MN_BIN" ]; then
    cargo build -p mnr --release >&2
    MN_BIN=$REPO_ROOT/target/release/mnr
fi

# Python side pathCheck()s 'mnexec' at Host setup; the Rust port provides the
# binary (apps/mnexec) — the workspace release build above already made it.
if ! command -v mnexec >/dev/null 2>&1; then
    install -m 0755 "$REPO_ROOT/target/release/mnexec" /usr/local/bin/mnexec 2>/dev/null || true
fi

PY_MN="python3 $REPO_ROOT/legacy/bin/mn"
# The Python reference package lives in legacy/ now
export PYTHONPATH="$REPO_ROOT/legacy${PYTHONPATH:+:$PYTHONPATH}"

# --- helpers ---------------------------------------------------------------

now_ns() { date +%s%N; }

median() { printf '%s\n' "$@" | sort -n | awk -v n="$(printf '%s\n' "$@" | wc -l)" '{a[NR]=$1} END {if (n%2) print a[(n+1)/2]; else printf "%.3f\n", (a[n/2]+a[n/2+1])/2}'; }

# run_once <impl:py|rs> <spec> -> "wall_s loss_pct"
run_once() {
    impl=$1; spec=$2
    case $impl in
        py) cmd="$PY_MN --topo $spec --switch $BENCH_SWITCH --controller none --test pingall --nolistenport" ;;
        rs) cmd="$MN_BIN --topo $spec --switch $BENCH_SWITCH --controller none --test pingall --nolistenport" ;;
    esac
    t0=$(now_ns)
    # shellcheck disable=SC2086
    if ! out=$(sh -c "$cmd" 2>&1); then
        printf '%s\n' "$out" >&2
        printf 'bench_scenarios: %s %s failed to run\n' "$impl" "$spec" >&2
        return 1
    fi
    t1=$(now_ns)
    wall=$(awk -v a="$t0" -v b="$t1" 'BEGIN {printf "%.3f", (b-a)/1e9}')
    loss=$(printf '%s\n' "$out" | grep -o '[0-9]*% dropped' | tail -1 | grep -o '[0-9]*') || {
        printf '%s\n' "$out" >&2
        printf 'bench_scenarios: %s %s produced no ping-loss line\n' "$impl" "$spec" >&2
        return 1
    }
    echo "$wall $loss"
}

nodes_of() {
    spec=$1
    python3 - "$spec" <<'PYEOF'
import sys
sys.path.insert(0, ".")
from mininet.util import buildTopo
from mininet.topo import Topo
TOPOS = {"single": (lambda k=2: __import__("mininet.topo", fromlist=["SingleSwitchTopo"]).SingleSwitchTopo(k=k))}
from mininet.topo import SingleSwitchTopo, LinearTopo, SingleSwitchReversedTopo, MinimalTopo
from mininet.topolib import TreeTopo, TorusTopo
TOPOS = {"minimal": MinimalTopo, "single": SingleSwitchTopo, "reversed": SingleSwitchReversedTopo,
         "linear": LinearTopo, "tree": TreeTopo, "torus": TorusTopo}
t = buildTopo(TOPOS, sys.argv[1])
print(len(t.nodes()))
PYEOF
}

# --- main ------------------------------------------------------------------

report=$(mktemp)
trap 'rm -f "$report"' EXIT
{
echo "# Differential benchmark: Python mininet vs Rust mnr"
echo
echo "- date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "- host: $(uname -srm) | cpus: $(nproc) | $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"
echo "- python: $(python3 --version 2>&1) | rust binary: $MN_BIN ($($MN_BIN --version 2>/dev/null || echo '?'))"
echo "- commit: $(git rev-parse --short HEAD 2>/dev/null || echo '?') branch: $(git branch --show-current 2>/dev/null || echo '?')"
echo "- switch: $BENCH_SWITCH | runs per cell: $BENCH_RUNS (+1 warmup), metric: wall time of '--topo SPEC --controller none --test pingall' (start+pingall+stop) and ping loss"
echo
echo "| scenario | nodes | impl | wall s median | wall s min | wall s max | ping loss |"
echo "|---|---|---|---|---|---|---|"
for spec in $BENCH_TOPOS; do
    nodes=$(nodes_of "$spec")
    for impl in py rs; do
        run_once "$impl" "$spec" >/dev/null   # warmup; failure aborts the run
        walls=""; losses=""
        i=1
        while [ "$i" -le "$BENCH_RUNS" ]; do
            r=$(run_once "$impl" "$spec")
            w=${r%% *}; l=${r##* }
            walls="$walls $w"; losses="$losses $l"
            i=$((i+1))
        done
        med=$(median $walls)
        min=$(printf '%s\n' $walls | sort -n | head -1)
        max=$(printf '%s\n' $walls | sort -n | tail -1)
        loss=$(printf '%s\n' $losses | sort | uniq -c | sort -rn | head -1 | awk '{print $2"%"}')
        label=python; [ "$impl" = rs ] && label="rust (mnr)"
        echo "| $spec | $nodes | $label | $med | $min | $max | $loss |"
    done
done
echo
echo "Complexity ladder: single,3 (4 nodes) < linear,4 < linear,10 < tree,2,2 (7) < tree,3,2 (15) < tree,3,3 (40)."
} > "$report"
if [ -n "$BENCH_OUT" ]; then
    cp "$report" "$BENCH_OUT"
fi
cat "$report"
