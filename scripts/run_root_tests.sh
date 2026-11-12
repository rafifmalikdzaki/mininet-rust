#!/bin/sh
# run_root_tests.sh — root-gated test suites for the mininet Rust migration.
#
# Task 11 (prep) of tasks/plan.md. ROOT-ONLY. On a privileged Linux host runs:
#
#   1. MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1
#      — the #[ignore]d netlink/netns integration tests (crates/netlink,
#      crates/link, ...). They are double-gated: #[ignore] AND the
#      MN_RUST_ROOT_TESTS=1 env var; --test-threads=1 because some tests fork
#      children that change namespaces.
#   2. The differential topology harness scripts/diff_topology.sh for the
#      three fixture specs: single,3 linear,4 tree,2,2
#      (MATCH/MISMATCH/SKIP per spec; SKIP = Rust mnr --dump not wired yet).
#
# ...and prints a summary table.
#
# Usage:
#   sudo scripts/run_root_tests.sh
#   DIFF_TOPOS="single,3 linear,4" sudo scripts/run_root_tests.sh   # override specs
#
# Exit codes:
#   0  everything ran; no FAIL/MISMATCH/ERROR (SKIPs are OK and reported)
#   1  at least one suite FAILED / MISMATCHed / errored
#   3  not run as root, or harness error (nothing is executed in that case)
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

# --- root guard: fail before running anything ---------------------------------
if [ "$(id -u)" -ne 0 ]; then
    echo "run_root_tests: ERROR: this script must run as root (current uid=$(id -u))." >&2
    echo "  It executes the root-gated netlink/netns tests and the Python-vs-Rust" >&2
    echo "  differential topology harness, both of which create network namespaces" >&2
    echo "  and veth pairs (CAP_NET_ADMIN). Nothing was run. Re-run as root, e.g.:" >&2
    echo "    sudo $0" >&2
    exit 3
fi

command -v cargo >/dev/null 2>&1 || { echo "run_root_tests: ERROR: cargo not found in PATH" >&2; exit 3; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

PASS=0
FAIL=0
SKIP=0

printf '%-58s %-10s %s\n' "SUITE" "RESULT" "DETAIL"
printf '%-58s %-10s %s\n' "-----" "------" "------"

# --- 1. root-gated cargo tests -------------------------------------------------
CARGO_LOG=$WORK/cargo_root_tests.log
if (cd "$REPO_ROOT" && MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1) \
        >"$CARGO_LOG" 2>&1; then
    CARGO_RC=0
else
    CARGO_RC=$?
fi
if [ "$CARGO_RC" -eq 0 ]; then
    RES=PASS
    PASS=$((PASS + 1))
    DETAIL=$(grep -h 'test result:' "$CARGO_LOG" | tr '\n' '; ' | cut -c1-160)
else
    RES=FAIL
    FAIL=$((FAIL + 1))
    DETAIL="exit $CARGO_RC (full log below)"
    echo "----- cargo root-gated test log (tail) -----" >&2
    tail -n 40 "$CARGO_LOG" >&2
    echo "--------------------------------------------" >&2
fi
printf '%-58s %-10s %s\n' "cargo: MN_RUST_ROOT_TESTS=1 ... -- --ignored --test-threads=1" "$RES" "$DETAIL"

# --- 2. differential topology harness over the fixture specs -------------------
SPECS=${DIFF_TOPOS:-'single,3 linear,4 tree,2,2'}
for SPEC in $SPECS; do
    LOG=$WORK/diff_topology_$SPEC.log
    set +e
    (cd "$REPO_ROOT" && sh scripts/diff_topology.sh "$SPEC") >"$LOG" 2>&1
    RC=$?
    set -e
    case $RC in
        0) RES=MATCH;    PASS=$((PASS + 1)) ;;
        1) RES=MISMATCH; FAIL=$((FAIL + 1))
           echo "----- diff_topology.sh $SPEC output -----" >&2
           cat "$LOG" >&2
           echo "-----------------------------------------" >&2 ;;
        2) RES=SKIP;     SKIP=$((SKIP + 1)) ;;
        *) RES=ERROR;    FAIL=$((FAIL + 1))
           echo "----- diff_topology.sh $SPEC output -----" >&2
           cat "$LOG" >&2
           echo "-----------------------------------------" >&2 ;;
    esac
    DETAIL=$(tail -n 1 "$LOG" | cut -c1-160)
    printf '%-58s %-10s %s\n' "diff: scripts/diff_topology.sh $SPEC" "$RES" "$DETAIL"
done

# --- summary --------------------------------------------------------------------
echo
echo "=== run_root_tests summary ==="
printf 'passed: %d   failed: %d   skipped: %d\n' "$PASS" "$FAIL" "$SKIP"
if [ "$FAIL" -gt 0 ]; then
    echo "run_root_tests: FAIL ($FAIL suite(s) failed/mismatched/errored)" >&2
    exit 1
fi
echo "run_root_tests: OK ($PASS passed, $SKIP skipped, 0 failed)"
exit 0
