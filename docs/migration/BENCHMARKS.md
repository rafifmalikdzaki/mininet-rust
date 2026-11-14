# Benchmarks — Python mininet vs Rust `mnr` (differential)

Goal: prove the Rust port is behaviorally identical (see `scripts/diff_topology.sh`)
and measure whether the netlink-based implementation is *faster* than shelling out
to `ip`/`tc` — across topologies of **increasing complexity**.

## Scenario ladder

| # | spec | nodes | why |
|---|------|-------|-----|
| 1 | `single,3` | 4 | smoke: 1 switch, 3 hosts |
| 2 | `linear,4` | 5 | chain of switches |
| 3 | `linear,10` | 11 | wider chain (more veths/qdiscs) |
| 4 | `tree,2,2` | 7 | 2-level tree |
| 5 | `tree,3,2` | 15 | 3-level tree |
| 6 | `tree,3,3` | 40 | largest ladder rung |

## Methodology

- **Command per cell:** `<impl> --topo <spec> --controller none --test pingall`
  (captures net start + pingall + teardown in one wall-clock number).
- **Impls:** `python3 bin/mn` (reference) vs `target/release/mnr` (Rust).
- **Repetitions:** 1 warmup + `BENCH_RUNS` (default 5) timed runs; report
  median / min / max wall seconds and the modal ping-loss line.
- **Controls:** same host, same kernel, both binaries release-built, root or a
  privileged container. Current runs use real OVS (`ovs,failMode=standalone`)
  with `--nolistenport`; legacy runs used LinuxBridge.
- **Host capture:** the script records `uname -srm`, CPU model, core count,
  Python version, mnr version, git commit, and date into the report header.
- **Correctness first:** benchmarks only run after `diff_topology.sh` MATCHes
  the same specs — a fast but wrong port is not a win.

## Run

```sh
sudo sh scripts/bench_scenarios.sh            # prints + writes markdown table
sudo BENCH_TOPOS="single,3 tree,3,3" BENCH_RUNS=10 sh scripts/bench_scenarios.sh
sudo BENCH_OUT=bench-results.md sh scripts/bench_scenarios.sh
```

Run it on a privileged Linux host. The script reports a failure string rather
than a benchmark row when an invocation cannot be measured.

## Results

From the privileged CI run on PR #1 (run 43306962340, artifact
`bench-results`, commit `3d7554a`; host: Linux 7.0.0-1019-azure, 4 vCPU Xeon
Platinum 8573C, Python 3.12.3, `--switch lxbr` on both sides because the CI
container has no OVS kernel datapath module; 5 runs + 1 warmup per cell;
metric: wall time of `--topo <spec> --controller none --test pingall`):

| scenario | nodes | impl | wall s median | wall s min | wall s max | ping loss |
|---|---|---|---|---|---|---|
| single,3 | 4 | python | 0.148 | 0.145 | 0.150 | 0% |
| single,3 | 4 | rust (mnr) | 0.160 | 0.157 | 0.187 | 0% |
| linear,4 | 8 | python | 0.280 | 0.273 | 0.300 | 0% |
| linear,4 | 8 | rust (mnr) | 0.299 | 0.289 | 0.316 | 0% |
| linear,10 | 20 | python | 0.741 | 0.707 | 0.749 | 0% |
| linear,10 | 20 | rust (mnr) | 1.116 | 1.105 | 1.126 | 0% |
| tree,2,2 | 7 | python | 0.236 | 0.234 | 0.249 | 0% |
| tree,2,2 | 7 | rust (mnr) | 0.289 | 0.269 | 0.305 | 0% |
| tree,3,2 | 15 | python | 0.536 | 0.520 | 0.549 | 0% |
| tree,3,2 | 15 | rust (mnr) | 0.762 | 0.745 | 0.779 | 0% |
| tree,3,3 | 40 | python | 1.924 | 1.905 | 1.943 | 0% |
| tree,3,3 | 40 | rust (mnr) | 5.142 | 5.115 | 5.162 | 0% |

**Reading:** behavior matches exactly (see the differential harness MATCHes in
the same CI run: `scripts/diff_topology.sh` MATCH for all six specs), and ping
loss is 0% on both sides everywhere. Wall-clock, the Rust port is currently
~1.1x (single,3) to ~2.7x (tree,3,3) *slower* end-to-end than Python — the gap
grows with node count because each host start spawns an `mnexec`-attached
shell plus per-node netlink round trips, while Python amortizes setup in one
`mnexec` batch per node. The netlink data path itself is not the bottleneck
at these sizes; process/netns spawn dominates. This is a known, measured
trade-off of the deferred-lifecycle design, not a correctness issue — and it
is where future optimization effort has the highest value (see ROADMAP).


## Real OVS, rootless (2026-11-14)

Docker-built image loaded into rootless Podman, with Python 3.12.3, OVS 3.3.9,
and Podman 6.2.0. Three timed samples plus one warmup per cell; all twelve cells
had 0% ping loss. Full methodology and caveats:
`docs/research/REAL_SWITCHES_ROOTLESS.md`.

| scenario | nodes | impl | median s | min/max s |
|---|---:|---|---:|---|
| single,3 | 4 | python | 0.204 | 0.197/0.210 |
| single,3 | 4 | rust | 0.306 | 0.297/0.315 |
| linear,4 | 8 | python | 0.408 | 0.397/0.412 |
| linear,4 | 8 | rust | 0.543 | 0.529/0.579 |
| linear,10 | 20 | python | 0.972 | 0.923/0.985 |
| linear,10 | 20 | rust | 1.799 | 1.766/1.812 |
| tree,2,2 | 7 | python | 0.314 | 0.308/0.355 |
| tree,2,2 | 7 | rust | 0.503 | 0.491/0.511 |
| tree,3,2 | 15 | python | 0.676 | 0.666/0.692 |
| tree,3,2 | 15 | rust | 1.268 | 1.243/1.298 |
| tree,3,3 | 40 | python | 2.363 | 2.330/2.373 |
| tree,3,3 | 40 | rust | 6.422 | 6.372/6.517 |

This is a correctness/relative-cost smoke, not a performance verdict; see the
report for the OVS startup and container caveats.

## Local re-run (2026-11-13, privileged docker container on the dev host)

Host: Linux 7.2.15-zen1 x86_64, 20 cpus (12th Gen Intel i9-12900H), Python 3.12.3,
`--switch lxbr` both sides, 3 runs + 1 warmup per cell, release build with
`lto=true, codegen-units=1, strip=true`. All six `diff_topology.sh` specs MATCH;
root-gated cargo suites: 28 passed / 0 failed.

| scenario | nodes | impl | wall s median | ping loss |
|---|---|---|---|---|
| single,3 | 4 | python | 0.344 | 0% |
| single,3 | 4 | rust (mnr) | 0.246 | 0% |
| linear,4 | 8 | python | 0.690 | 0% |
| linear,4 | 8 | rust (mnr) | 0.498 | 0% |
| linear,10 | 20 | python | 1.753 | 0% |
| linear,10 | 20 | rust (mnr) | 1.552 | 0% |
| tree,2,2 | 7 | python | 0.587 | 0% |
| tree,2,2 | 7 | rust (mnr) | 0.452 | 0% |
| tree,3,2 | 15 | python | 1.282 | 0% |
| tree,3,2 | 15 | rust (mnr) | 1.101 | 0% |
| tree,3,3 | 40 | python | 4.455 | 0% |
| tree,3,3 | 40 | rust (mnr) | 6.084 | 0% |

On this host the Rust port is ~1.1-1.4x *faster* than Python for 4-15 nodes;
only the 40-node tree,3,3 rung (702 pings) is slower (~1.37x) — pingall
dominates there and Python's fan-out wins. Peak RSS on tree,3,3 dry-run:
python 11.7 MB vs mnr 7.3 MB.

## Binary size / footprint (release profile hardening, 2026-11-13)

`[profile.release]` gained `lto = true`, `codegen-units = 1`, `strip = true`:

| binary | before | after | delta |
|---|---|---|---|
| mnr | 4,959,480 B | 3,123,664 B | -37% |
| mnr-tui | 5,288,032 B | 3,299,696 B | -38% |
| mnr-mcp | 4,275,512 B | 2,644,560 B | -38% |
| mnexec | 528,184 B | 371,152 B | -30% |

## Notes / limitations

- Wall time includes process spawn, netns/veth/qdisc setup, pingall, teardown.
- Run the ladder locally with `BENCH_SWITCH=lxbr` (kernel LinuxBridge) when the
  host cannot load the OVS kernel datapath module; use an environment with OVS
  for OVS-based comparisons. Use `just bench` on a privileged host;
  `just root-tests` runs the root suite and the differential ladder.
- The `tree,3,3` rung (40 nodes, 702 pings) is the heaviest; per-run variance
  is small (min/max within ~1%), so medians are stable.
