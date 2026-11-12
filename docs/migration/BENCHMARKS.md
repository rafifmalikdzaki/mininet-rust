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

*To be filled from the privileged CI run (this dev sandbox is non-root).*

| scenario | nodes | impl | wall s median | wall s min | wall s max | ping loss |
|---|---|---|---|---|---|---|
| single,3 | 4 | python | TBD | TBD | TBD | TBD |
| single,3 | 4 | rust (mnr) | TBD | TBD | TBD | TBD |
| … | | | | | | |

## Notes / limitations

- Wall time includes process spawn, netns/veth/qdisc setup, pingall, teardown.
- `--test pingall` on a switch-bearing topo requires the switch to start;
  the ladder uses the default `ovs` switch, so OVS must be running (CI starts
  ovsdb-server/ovs-vswitchd). Host-to-host ping crosses the OVS bridge.
- For pure link-setup cost without OVS, add `--switch lxbr` variants later.
