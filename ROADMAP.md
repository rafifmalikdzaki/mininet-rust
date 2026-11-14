# Roadmap

**Status:** real OVS switches and rootless execution are working. This roadmap
turns the design catalogue into executable, tested scenarios and then into
research results. It is ordered by risk and dependency, not by feature count.

## North star

A researcher can start from a clean Linux host with rootless Podman, run a
reproducible multi-switch / multi-tenant / routed topology, inject faults, and
recover evidence without host root. Every design has explicit acceptance tests,
a benchmark, and a report.

## What is working now

| Capability | Evidence |
|---|---|
| Real OVS switch per scenario | `vlan-ovs`, `multi-tenant-ovs`, `multi-tenant-fault-ovs`, `ospf-ovs`, `dual-home-ovs`, `clos-l2-ovs` |
| Ring/chord routing | `ring-isis`: six routers, 3 chords, ECMP, single-link cut |
| Rootless Podman | `mnr` `single,3` 6/6; research scenarios; rootless MCP lifecycle; artifacts owned by invoking user |
| OVS VLAN isolation | same-VLAN pass, cross-VLAN fail |
| Multi-tenant isolation | full positive + negative matrices across 3 tenants and 3 sites |
| Trunk fault/recovery | down/up cycle preserves isolation and restores reachability |
| Routing over OVS | OSPF adjacency, routes, loopback pings |
| Link redundancy | active-backup bond failover and recovery |
| Redundant fabric | 2 spine + 2 leaf OVS switches, STP convergence, full reachability |
| Differential benchmark | full 6-topology ladder, 3 samples, 12/12 cells 0% loss |
| Rootless report | `docs/research/REAL_SWITCHES_ROOTLESS.md` |

### Current design status

| Design | Status | Next action |
|---|---|---|
| D01 Clos L3 eBGP | **partial** — L2 leaf-spine with STP implemented (`clos-l2-ovs`) | add router-hosts, eBGP, ECMP, uplink/leaf failure |
| D02 three-tier hierarchy | planned | core/agg/access, OSPF areas, VLAN access/trunk |
| D03 VRF-lite tenancy | planned | prototype Linux VRF + FRR on OVS-attached PEs |
| D04 multi-site VLAN | **implemented** (OVS) | add independent per-tenant fault and restore tests |
| D05 VXLAN overlay | planned | two-VTEP kernel VXLAN smoke, then tenant overlay |
| D06 dual-homed links | **partial** — active-backup implemented | OVS LACP experiment; document reference platform |
| D07 ring/chords | **implemented** — `ring-isis` | add two-link partition and convergence timing |
| D08 hub-spoke BGP | planned | prefix filters, default route, no-transit assertion |
| D09 torus scale | planned | generator + 16/36-router runs, leak and timing budget |
| D10 service chain | planned | firewall/NAT tenant policy matrix |
| D11 IPv6 anycast | planned | OSPFv3/BGP, dual-stack, failover, IPv6 probes |
| D12 chaos | **partial** — timeouts, nonzero exits, trunk failure | signals, daemon kill, repeated start/stop, leak checks |

## Workstreams

### P0 — Real-switch foundation (done)

- per-scenario switch spec, default real OVS standalone;
- OVS daemons in the research image;
- rootless wrapper without `keep-id`, with `--nolistenport`;
- OVS VLAN, tenant, fault, routed, bonded, and leaf-spine scenarios;
- OVS benchmark defaults.

**Remaining P0 hardening:**

- add `command -v ovs-vsctl` and `ovs-vsctl show` to image/run preflight;
- add a host-module preflight with an actionable error;
- add a regression test for the wrapper's switch-default logic.

### P1 — Routing and complex topologies

1. **D07 ring IS-IS** — implemented as `ring-isis`; add two-link partition and
   convergence timing.
2. **D01 Clos eBGP** — 2 spines, 4 leaves, 8 servers, ECMP, leaf/uplink failure.
3. **D08 hub-spoke BGP** — default route, prefix filter, no transit.
4. **D02 three-tier OSPF** — areas, summaries, dual-homed aggregation.
5. **D09 torus** — generated 4×4 then 6×6 fabric; scale and leak tests.

**Acceptance for each:** all positive pairs pass, failure blast radius matches
the design, convergence attempts stay within a documented bound, artifacts
complete, teardown leaves no namespaces/veths/FRR processes.

### P2 — Tenancy

1. **D03 VRF-lite** — two PEs, four tenants, overlapping `10.20.0.0/24`;
   per-VRF route tables; 8/8 same-tenant and 48/48 cross-tenant checks.
2. **Shared-service leak** — deliberate route leak into all VRFs, then revert.
3. **D10 service chain** — tenant-aware firewall allow/deny matrix and NAT.
4. **Tenant scale** — 10 tenants, artifact-size and runtime budget.

**Acceptance:** no cross-tenant route or packet, explicit leak test proves the
matrix can detect a misconfiguration, and per-VRF evidence is retained.

### P3 — Link and layer resilience

1. **D06b OVS LACP** — determine whether the rootless OVS datapath and host
   bond negotiate LACP; document the supported combination.
2. **D07 ring** under single-link and two-link cuts.
3. **D12 chaos** — SIGINT/SIGTERM, FRR daemon kill, partial startup, repeated
   start/stop, existing-output-directory refusal.
4. **STP timing** — measure root election and forwarding delay; expose a
   readiness probe instead of fixed retries.

### P4 — Observability and evidence

1. Add switch-specific telemetry: OVS port counters, STP state, VLAN/FDB.
2. Correlate fault timestamps with FRR neighbor and route events.
3. Verify Alloy→Loki ingestion for OVS runs, including restart/rescan.
4. Add a machine-readable `summary.json` with matrices and convergence times.
5. Monitor OVS run directories at 100 MB+ without unbounded memory.

### P5 — AI/MCP and rootless

1. MCP rootless lifecycle is verified; add an MCP evaluation script: task success, invalid-call rate, recovery,
   time-to-result.
3. Verify root-gated tools fail closed outside the container.
4. Document single-client stdio semantics and no network transport.

### P6 — Performance and scale

- rerun the ladder with 5–10 samples and a release binary;
- compare OVS vs LinuxBridge vs direct links on the same host;
- extend to 100+ nodes and measure kernel/file-descriptor limits;
- publish CPU, RSS, artifact bytes, and convergence with a host fingerprint;
- keep the current 3-sample OVS table as a historical baseline.

### P7 — Security and reproducibility

- threat model for privileged OVS + arbitrary scenario commands;
- artifact symlink/ownership tests (some already covered);
- pin image digests and record OVS/FRR/kernel versions in provenance;
- rootless build path with a working registry or fully local image import;
- no secrets in scenarios or monitoring configuration.

## 30 / 60 / 90 day plan

### First 30 days

- D01 Clos eBGP (D07 ring IS-IS is now implemented);
- D03 VRF-lite prototype;
- chaos: signals and daemon kill;
- OVS preflight and wrapper `--switch=ovs` test;
- update benchmark to 5 samples with release build.

### 60 days

- D02 three-tier and D08 hub-spoke;
- D06b LACP support decision;
- D10 service chain;
- OVS telemetry and Loki ingestion;
- MCP rootless lifecycle evaluation.

### 90 days

- D05 VXLAN and D11 IPv6 anycast;
- D09 100-node scale run;
- full chaos matrix and leak budget;
- consolidated research report with reproducibility package.

## Rootless requirements

```console
$ podman info --format '{{.Host.Security.Rootless}}'   # true
$ lsmod | grep openvswitch                             # host module present
$ grep "$USER" /etc/subuid /etc/subgid                 # mappings present
```

- Rootless Podman (preferred) or rootless Docker.
- Host OVS kernel module loaded; the container cannot load it.
- Do not force `--userns=keep-id` for OVS.
- Use `--switch ovs,failMode=standalone` and `--nolistenport` without a
  controller.
- Keep the container `--privileged` only inside the user namespace; use a
  disposable host and trusted scenarios.

## Definition of done for a design

- Scenario JSON committed under `example/scenarios/`.
- Plan validation, rootless live run, positive matrix, negative matrix.
- Fault injection with expected blast radius and recovery bound.
- OVS/FRR/kernel provenance recorded.
- Teardown leak check.
- Benchmark row or scenario timing recorded.
- Report and scenario map updated.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Host lacks OVS kernel module | rootless OVS cannot start | preflight fail with actionable message; LinuxBridge fallback |
| STP convergence variability | false test failures | readiness polling and documented retry budget |
| OVS/FRR version drift | different behavior | pin images; record versions |
| Privileged container | host exposure | disposable host, trusted scenarios, no sensitive mounts |
| Benchmark single-host noise | misleading performance | multiple samples, fingerprint, release build |
| Scenario complexity | long JSON, hand-edited errors | generator + schema validation before execution |

## Document map

- `docs/research/TOPOLOGY_DESIGNS.md` — 12 design blueprints.
- `docs/research/SCENARIO_MAP.md` — complexity ladder.
- `docs/research/RESEARCH_PLANS.md` — ten research programmes.
- `docs/research/REAL_SWITCHES_ROOTLESS.md` — current OVS/rootless evidence.
- `example/README.md` — runner, scenario schema, presets.
- `docs/testing.rst` — test tiers and validation snapshot.
