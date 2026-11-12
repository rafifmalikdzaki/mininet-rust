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
- **Controls:** same host, same kernel, both binaries release-built, root,
  OVS not involved (`--controller none`, default `ovs` switch type is only
  *configured* if started — pingall runs hosts-only links via TCLink; see notes).
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

CI: `.github/workflows/rust-root-tests.yaml` runs it after the differential
harness and uploads `bench-results.md` as the `bench-results` artifact.

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

## Notes / limitations

- Wall time includes process spawn, netns/veth/qdisc setup, pingall, teardown.
- CI runs the ladder with `BENCH_SWITCH=lxbr` (kernel LinuxBridge) because
  the privileged GH container cannot load the OVS kernel datapath module;
  OVS-based runs remain covered by the VM workflows (`run-tests.yaml`).
- The `tree,3,3` rung (40 nodes, 702 pings) is the heaviest; per-run variance
  is small (min/max within ~1%), so medians are stable.
