# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are not yet
tagged.

## [Unreleased]

### Added

- Real Open vSwitch support for research scenarios. The scenario `switch`
  field defaults to `ovs,failMode=standalone`; legacy LinuxBridge scenarios
  declare `"switch":"lxbr"`.
- OVS startup in the research image and an entrypoint that initializes
  `ovsdb-server` and `ovs-vswitchd` before the experiment runner.
- Rootless Podman recipes: `just research-rootless` and `just bench-rootless`.
- Rootless OVS scenarios:
  - `vlan-ovs` — access-port VLAN isolation.
  - `multi-tenant-ovs` — three sites, three tenants, overlapping addresses.
  - `multi-tenant-fault-ovs` — trunk failure, isolation, and recovery.
  - `ospf-ovs` — OSPF adjacency and routes over a real switch.
  - `dual-home-ovs` — active-backup bonding and failover.
  - `clos-l2-ovs` — two-spine/two-leaf OVS fabric with STP.
  - `ring-isis` — six-router ring with chords, IS-IS, ECMP, and a link cut.
- `clos-ebgp` — L3 spine/leaf eBGP, two ECMP next hops, and an uplink cut.
- `vrf-lite` — two Linux VRFs per PE, overlapping tenant prefixes, and a
  positive/negative isolation matrix.
- OVS-based benchmark defaults and a rootless three-sample benchmark report.
- Real-switch/rootless validation report and a project roadmap covering
  D01–D12 designs, 30/60/90-day workstreams, and acceptance gates.
- Host OVS kernel-module setup guide for Arch, Debian/Ubuntu, and Fedora.
- Consolidated implementation report with full evidence, defects, benchmark
  methodology, rootless runbook, and remaining work.
- Topology design catalogue with Clos, hierarchy, VRF, VXLAN, bonding, ring,
  hub-spoke, torus, service-chain, IPv6, and chaos designs.

### Changed

- `scripts/mnr-container.sh` prefers real OVS, adds `--nolistenport`, and no
  longer forces `--userns=keep-id` (which prevented OVS from creating its
  runtime paths). It now fails early if OVS is requested but the host module
  is not loaded.
- The research runner and benchmark use real OVS by default; `BENCH_SWITCH`
  and `BENCH_RUNS` remain overridable.
- Benchmark input validation rejects non-integer or zero `BENCH_RUNS` instead
  of emitting an empty table.
- Documentation now records rootless prerequisites, host OVS module
  requirements, and benchmark caveats.

### Fixed

- Privileged root-test container now builds and installs `mnexec` before the
  ignored suites, fixing 11 live spawn failures.
- Incremental monitor loading counts an unterminated JSON line once and clears
  per-file parse errors after truncation.
- Python bindings `plan()` documentation now matches its text output; the test
  was renamed accordingly.

### Security

- The OVS container remains `--privileged` inside a user namespace. It is not
  a sandbox for untrusted scenarios. Host OVS module loading is not available
  inside a rootless container.
- The research image starts its own OVS daemons; do not mount the host OVS
  database into it.

## [0.1.0] — pre-existing Rust port baseline

The Rust workspace, `mnr`, `mnr-tui`, `mnr-mcp`, `mnexec`, Python bindings,
differential tests, research presets, monitor, and monitoring stack existed
before this changelog was introduced. See the migration ledger and technical
report for the detailed history.
