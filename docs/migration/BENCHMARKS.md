# BENCHMARKS.md — performance: Rust `mn` vs Python mininet

Task 12 (docs part) of `tasks/plan.md`. **Status: scaffold.** The results
table below is a placeholder — fill it in after the first privileged CI run
(`.github/workflows/rust-root-tests.yaml`) or a run on a dedicated root host.

Goal: quantify the containerlab-lesson payoff (single static binary, netlink
instead of ~100 subprocess shell-outs) — i.e. how much faster parse → build →
start → dump → stop is in Rust `mn` vs `python3 bin/mn` on identical small
topologies.

## Methodology

- **What is measured:** end-to-end wall-clock of a full `mn` run per
  implementation on the same topo spec: build topology, start the net
  (netns/veth/qdisc creation), `dump`+`links`, `pingall` for the `--test`
  variants, stop. Both sides run on the **same controlled host** (the
  privileged CI container or a dedicated, otherwise idle Linux host — no
  other workloads, CPU governor pinned if possible).
- **Repeated runs, hyperfine-style:** 3 warm-up runs (page cache, OVS db,
  cargo/rustc caches) discarded, then **N = 20** measured runs per
  (implementation, spec, variant). Report median / min / max. If `hyperfine`
  is available, prefer it (`hyperfine --warmup 3 --runs 20 '<cmd>'`);
  otherwise a `time` loop (no extra deps — this repo installs nothing via
  pip):

  ```bash
  # median/min/max of N runs of a command, wall-clock seconds (/usr/bin/time -f %e)
  bench() { # bench <label> <cmd...>
      label=$1; shift
      for i in 1 2 3; do "$@" >/dev/null 2>&1; done          # warm-up
      for i in $(seq 20); do /usr/bin/time -f %e -o "$label.times" "$@" >/dev/null 2>&1; done
      sort -n "$label.times" | awk -v l="$label" 'NR==1{min=$1} {a[NR]=$1} END{printf "%s median=%.3f min=%.3f max=%.3f\n", l, a[int((NR+1)/2)], min, a[NR]}'
      rm -f "$label.times"
  }
  ```

- **Commands measured** (root required; run from the repo root with
  `PYTHONPATH=.` — exactly the environment the privileged CI job sets up):

  ```bash
  # Python reference (same driver shape as scripts/diff_topology.sh)
  sudo env PYTHONPATH=. python3 bin/mn --topo single,3 --test none
  sudo env PYTHONPATH=. python3 bin/mn --topo linear,4 --test pingall
  sudo env PYTHONPATH=. python3 bin/mn --topo tree,2,2  --test none

  # Rust port
  sudo target/release/mn --topo single,3 --test none
  sudo target/release/mn --topo linear,4 --test pingall
  sudo target/release/mn --topo tree,2,2  --test none
  ```

  Plus the **non-root dry-run path** (parse → build → plan → `--dump`), which
  needs no privileges and is what most CI/dev machines can run:

  ```bash
  target/release/mn --topo tree,2,2 --dump                                   # Rust
  python3 -c 'import sys; sys.path.insert(0,"."); from mininet.topo import TreeTopo; t=TreeTopo(2,2); print(t.nodes())'  # Python topo build only
  ```

- **Correctness gate first:** only benchmark after `scripts/diff_topology.sh
  <spec>` reports MATCH for that spec — timing a mismatched implementation
  is meaningless. `cargo bench` is intentionally **not** used: there are no
  wired micro-benchmarks; all measurements are end-to-end CLI runs via the
  scripts above + `time`.
- **Normalize environment:** same kernel, same OVS version, same
  `DIFF_MN_SWITCH/HOST/CONTROLLER/LINK/IPBASE` class flags on both sides
  (defaults: `default`/`proc`/`none`/`default`/`10.0.0.0/8`), same host.

## Results

| Benchmark | Impl | Spec | Runs (warm-up) | Median (s) | Min (s) | Max (s) | Notes |
|---|---|---|---|---|---|---|---|
| full run, `--test none` | Python `bin/mn` | single,3 | 20 (3) | TBD | TBD | TBD | fill after privileged CI run |
| full run, `--test none` | Rust `mn` | single,3 | 20 (3) | TBD | TBD | TBD | |
| full run, `--test pingall` | Python `bin/mn` | linear,4 | 20 (3) | TBD | TBD | TBD | |
| full run, `--test pingall` | Rust `mn` | linear,4 | 20 (3) | TBD | TBD | TBD | |
| full run, `--test none` | Python `bin/mn` | tree,2,2 | 20 (3) | TBD | TBD | TBD | |
| full run, `--test none` | Rust `mn` | tree,2,2 | 20 (3) | TBD | TBD | TBD | |
| dry-run `--dump` (no root) | Python topo build | tree,2,2 | 20 (3) | TBD | TBD | TBD | |
| dry-run `--dump` (no root) | Rust `mn --dump` | tree,2,2 | 20 (3) | TBD | TBD | TBD | |

Record the host (kernel, CPU, OVS version), the commit, and the CI job URL
next to the numbers when filling this in.

## Interpretation notes (to complete with results)

- Expected: Rust wins on process-spawn-heavy paths (one binary, no
  `mnexec`/`ip`/`tc` per operation) and on startup (no Python interpreter +
  module import). Python may still win on tiny single-switch dry paths where
  interpreter startup dominates less than netns setup — measure, don't assume.
- `pingall`/`iperf` results are dominated by kernel datapath, not the
  control plane — differences there are noise unless large.
