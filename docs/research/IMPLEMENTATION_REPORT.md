# Implementation report: real switches, rootless operation, and research scenarios

**Date:** 2026-11-14  
**Baseline:** `f008209` plus the pending ring/changelog work  
**Audience:** maintainers, researchers, and the next coding agent.

This is the consolidated engineering record for the real-switch/rootless phase.
It includes what changed, why, how it was verified, what failed, what remains,
and how to reproduce it. The shorter evidence report is
[Real switches and rootless](REAL_SWITCHES_ROOTLESS.md); the design catalogue is
[Topology designs](TOPOLOGY_DESIGNS.md); the next-work sequence is in
`ROADMAP.md` at the repository root.

## 1. Executive summary

The emulator can now run research scenarios with **real Open vSwitch switches**
inside a **rootless Podman** container, without host root and without mounting
the host OVS database. Real OVS is the default for switch nodes; LinuxBridge is
an explicit legacy mode.

Verified outcomes:

| Outcome | Result |
|---|---|
| Rootless `mnr` with OVS | `single,3` pingall 6/6, 0% loss |
| Rootless OVS VLAN isolation | same-VLAN pass; cross-VLAN fail |
| Rootless multi-tenant | 3 sites × 3 tenants, full positive/negative matrices |
| Rootless tenant fault | trunk down/up, isolation held, recovery passed |
| OSPF over OVS | adjacency, routes, loopback pings passed |
| Active-backup bond | active-member failure and recovery passed |
| Redundant leaf-spine | 4 OVS switches, STP, full reachability passed |
| Ring/chord IS-IS | 6 routers, 9 links, adjacency, ECMP ping, link cut passed |
| L3 Clos/eBGP | 2 spines, 2 leaves, 2 servers; ECMP and uplink cut passed |
| VRF-lite | 2 PEs, 2 tenants, overlapping prefixes; positive/negative matrix passed |
| Rootless MCP lifecycle | build/start/ping 2/2/stop/release passed |
| Rootless benchmark | 6 topologies × 3 samples, 12/12 cells 0% loss |
| Workspace tests | 447 passed, 0 failed, 29 ignored (root suite separately 29/29) |
| Docs | Sphinx build clean, no warnings |

The main integration defects were not data-plane correctness bugs. They were:

1. OVS default secure mode has no forwarding and waits forever for a controller.
2. Podman `--userns=keep-id` prevents OVS from creating its runtime paths.
3. The research image lacked OVS daemons.
4. The runner hardcoded LinuxBridge.
5. The privileged root gate built `mnexec` but did not put it on `PATH`.
6. The benchmark script did not validate `BENCH_RUNS`.

All six are fixed or documented.

## 2. Starting point

The repository already contained:

- Rust `mnr`, `mnr-tui`, `mnr-mcp`, `mnexec`, Python bindings;
- pure topology/util/netlink/link/node/net crates;
- research presets for OSPF, IS-IS, BGP, VLAN, and multi-tenant;
- LinuxBridge-based multi-tenant scenarios;
- a container image and a rootless wrapper that defaulted to LinuxBridge;
- unprivileged, root-gated, differential, and benchmark tooling;
- extensive design/validation documentation.

The gap was that **switches were not real switches on the research path** and
the rootless wrapper deliberately avoided OVS. The task was to close that gap
without hiding limitations behind optimistic documentation.

## 3. Implemented changes

### 3.1 Per-scenario switch selection

`experiment-runner`'s scenario schema now has:

```json
{
  "name": "example",
  "switch": "ovs,failMode=standalone",
  "nodes": [],
  "links": [],
  "steps": []
}
```

- `switch` is optional.
- Default: `ovs,failMode=standalone`.
- Legacy scenarios declare `"switch":"lxbr"`.
- The value is passed directly to `MininetOptions::switch_spec`.
- Empty switch specs are rejected before any run directory or namespace is
  created.

Unit tests cover the default, an explicit legacy value, and empty rejection.

### 3.2 Real OVS in the research image

`example/Dockerfile` now installs `openvswitch-switch` and uses
`example/entrypoint.sh`. The entrypoint:

- creates `/etc/openvswitch/conf.db` if absent;
- starts `ovsdb-server` and `ovs-vswitchd`;
- optionally runs `modprobe openvswitch` (best effort);
- execs `experiment-runner`.

The container starts its own OVS userspace. It does not mount or depend on the
host OVS database. The host must provide the OVS kernel module; a rootless
container cannot load it.

### 3.3 Rootless wrapper

`scripts/mnr-container.sh` now:

- defaults to `--switch ovs,failMode=standalone` when no switch is given;
- adds `--nolistenport` for OVS unless the caller chose a listen option;
- does **not** force `--userns=keep-id`;
- continues to use Podman first and Docker as fallback;
- maps container root to the invoking user for bind-mounted artifacts.

Why `failMode=standalone`: the research runner creates no controller. Secure
OVS mode does not forward and its connected check waits forever. Standalone
learning mode forwards without a controller. This was verified with both a
no-controller pingall and an OSPF adjacency.

Why no `keep-id`: with keep-id, OVS could not create `/var/run/openvswitch`.
Default rootless Podman still maps container root to the host user for `/w`, so
artifacts remained owned by the invoking user.

### 3.4 Rootless recipes

Added:

```console
$ just research-rootless ring-isis ring-001
$ just bench-rootless 3
```

`bench-rootless` runs the same benchmark script in the `mnr-rootless` image
with `MN_RUST_BIN=/usr/local/bin/mnr`. Both recipes require the corresponding
image to exist in Podman storage. On this host, images were built with Docker
and loaded with `docker save ... | podman load`.

### 3.5 Scenario files

| File | Nodes/links/steps | Purpose |
|---|---|---|
| `vlan-ovs.json` | 5 / 4 / 9 | OVS access-port VLAN isolation |
| `multi-tenant-ovs.json` | 12 / 11 / 30 | 3 sites, 3 tenants, overlapping addresses |
| `multi-tenant-fault-ovs.json` | 12 / 11 / 38 | trunk down/up and recovery |
| `ospf-ovs.json` | 3 / 2 / 5 | OSPF through a real OVS switch |
| `dual-home-ovs.json` | 3 / 3 / 5 | active-backup bond failover |
| `clos-l2-ovs.json` | 8 / 8 / 7 | redundant leaf-spine with STP |
| `clos-ebgp.json` | 6 / 6 / 12 | L3 leaf/spine eBGP, ECMP, uplink cut |
| `vrf-lite.json` | 6 / 6 / 12 | Linux VRF isolation, overlapping tenants |
| `ring-isis.json` | 6 / 9 / 12 | ring + chords, IS-IS, ECMP, link cut |

The legacy `vlan`, `multi-tenant`, and `multi-tenant-fault` scenarios remain
explicit LinuxBridge cases for comparison.

## 4. Scenario details and results

### 4.1 OVS VLAN (`vlan-ovs`)

Two VLANs on one OVS bridge:

- `h1`/`h2` on access VLAN 10;
- `h3`/`h4` on access VLAN 20;
- same-VLAN pings must pass;
- cross-VLAN pings must exit 1.

Result rootless: **success**, 9 steps. This is the smallest real-switch proof
and is used as a smoke test for the OVS image.

### 4.2 Multi-tenant OVS (`multi-tenant-ovs`)

Three sites, three tenants, all tenants reuse `10.20.0.0/24`. OVS access ports
carry tenant tags; inter-site trunks carry VLANs 10, 20, 30.

Test matrix:

- each tenant's hosts reach one another across sites;
- every ordered cross-tenant address pair fails;
- run ends with `finish.success: true`.

Result rootless: **success**, 30/30 steps. Cross-tenant checks intentionally
take ~2 s per unreachable address because `ping -W 2` is used.

### 4.3 Tenant trunk fault (`multi-tenant-fault-ovs`)

Adds:

1. full multi-tenant matrices;
2. trunk `s2-eth5` down;
3. expected reachability and isolation during the fault;
4. trunk up;
5. recovery pings.

Result rootless: **success**, 38/38 steps. The OVS port link-down/up sequence
was sufficient; no switch process restart was needed.

### 4.4 OSPF over OVS (`ospf-ovs`)

Two routers attach to one OVS bridge. OSPF hellos traverse the switch;
adjacency and loopback routes are checked.

Result rootless: **success**. The adjacency required 40 retry attempts and
about 51 s; the route and both loopback pings then passed. OVS standalone
flooding/learning converges more slowly than a direct veth link, so retry
counts are the meaningful readiness signal.

### 4.5 Dual-homed active-backup (`dual-home-ovs`)

One host has two OVS ports in an active-backup bond. The active member is read
from netlink (`ip -d link show bond0`), not sysfs (the sysfs bonding directory
is not visible in the host namespace). The test:

- verifies a ping through the bond;
- takes the active member down;
- verifies a new active member and continued ping;
- restores both members and pings again.

Result rootless: **success**, 5 steps. This is D06a; OVS LACP remains untested.

### 4.6 Redundant leaf-spine (`clos-l2-ovs`)

Two spines, two leaves, four hosts, all links in one L2 fabric. STP is enabled
on each OVS bridge.

Result rootless: **success**. STP needs a forwarding-delay window; the test
observed 11–12 retries (~16–18 s) before reachability, then all host pairs
passed. This is expected protocol convergence, not a link failure.

### 4.7 Ring + chords (`ring-isis`)

Six routers:

- ring `r1-r2-r3-r4-r5-r6-r1`;
- chords `r1-r4`, `r2-r5`, `r3-r6`;
- three interfaces per router, `/30` links, `10.1.1.1–6/32` loopbacks;
- IS-IS level 2, point-to-point interfaces, LSP generation interval 1 s.

Checks:

- `r1` has IS-IS neighbors `Up`;
- `r1` reaches `r3` (non-adjacent, multiple paths);
- `r2` reaches `r4`;
- take `r1-eth0` down; `r1` still reaches `r3`;
- restore and ping again.

Result rootless: **success**, 12 steps. The first adjacency/ping needed 12
retries; route and failure checks then passed. The scenario is intentionally
retry-tolerant because FRR and IS-IS startup dominate.

### 4.8 L3 Clos eBGP (`clos-ebgp`)

Reduced Clos:

- 2 spine routers (`sp1`, `sp2`), 2 leaf routers (`lf1`, `lf2`);
- 2 servers (`h1`, `h2`);
- leaf-to-both-spines full mesh;
- loopbacks `10.255.0.1/2/11/12`;
- eBGP: spines AS 65000, leaves 65001/65002;
- leaves advertise their server `/24`; `maximum-paths 2`;
- leaf route to the remote server prefix has **two nexthops**;
- h1 reaches h2;
- `lf1-eth0` uplink is cut, h1 still reaches h2 via the other spine;
- uplink restored, reachability remains.

Result: **success**, 12 steps. The ECMP assertion needed one retry before two
next hops appeared; one failure-path attempt initially used the wrong source
address (router loopback rather than server), which was fixed by pinging from
`h1`. This is D01 in reduced form; the catalogue's 4-leaf/8-server expansion
and leaf-failure matrix remain.

### 4.9 VRF-lite tenancy (`vrf-lite`)

Reduced D03:

- two PE routers, each with two Linux VRFs (`t1` table 101, `t2` table 102);
- two hosts per tenant, one at each site;
- both tenants intentionally use `10.20.1.0/24` and `10.20.2.0/24`;
- per-VRF static routes over separate point-to-point links;
- a1→a2 and b1→b2 pass;
- a1→b1, a1→b2, and b1→a1 fail with ping exit 1;
- per-VRF tables are inspected directly.

Result: **success**, 12 steps. This validates the VRF primitive and the negative
matrix. FRR inside VRFs, four tenants, and shared-service leaks remain for the
full D03 design.

## 5. Benchmarks

### 5.1 Environment

| Item | Value |
|---|---|
| Host kernel | Linux 7.2.15-zen1 |
| CPU | 20 logical CPUs, i9-12900H |
| Container | Docker-built Ubuntu image, loaded into rootless Podman 6.2.0 |
| OVS | 3.3.9 |
| Python | 3.12.3 |
| Switch | `ovs,failMode=standalone` |
| Samples | 1 warmup + 3 timed, per cell |
| Metric | wall time of start + pingall + stop |

### 5.2 Results

| Scenario | Nodes | Python median | Rust median | Python min/max | Rust min/max | Loss |
|---|---:|---:|---:|---|---|---:|
| `single,3` | 4 | 0.204 | 0.306 | 0.197/0.210 | 0.297/0.315 | 0% |
| `linear,4` | 8 | 0.408 | 0.543 | 0.397/0.412 | 0.529/0.579 | 0% |
| `linear,10` | 20 | 0.972 | 1.799 | 0.923/0.985 | 1.766/1.812 | 0% |
| `tree,2,2` | 7 | 0.314 | 0.503 | 0.308/0.355 | 0.491/0.511 | 0% |
| `tree,3,2` | 15 | 0.676 | 1.268 | 0.666/0.692 | 1.243/1.298 | 0% |
| `tree,3,3` | 40 | 2.363 | 6.422 | 2.330/2.373 | 6.372/6.517 | 0% |

A separate `just bench-rootless 1` smoke produced the same shape
(`single,3`: 0.179 Python vs 0.275 Rust; `tree,3,3`: 2.231 vs 6.335), confirming
the recipe and validation path.

### 5.3 Interpretation

This is a **correctness and relative-cost smoke**, not a performance verdict:

- one host, one container image, three samples;
- Rust binary from the image, Python from the container;
- OVS startup and process/netns creation dominate small topologies;
- historical direct-link numbers are not comparable.

The consistent finding is that Rust is slower in this image; the optimization
target is lifecycle/process overhead, not the netlink data path. Do not turn
this table into a marketing claim.

### 5.4 Benchmark validation defect

Invoking `just bench-rootless runs=1` passed the literal `runs=1` to the recipe
and then to `BENCH_RUNS`, producing an empty table. The script now rejects
non-integer and zero values with exit 2 before creating a report. The correct
use is positional: `just bench-rootless 1`.

## 6. Verification matrix

| Gate | Result |
|---|---|
| `just check` | pass: fmt + clippy workspace and monitor |
| `just test` | 447 passed, 0 failed, 29 ignored |
| Root-gated Docker suite | 29/29 passed |
| MCP protocol | 13 passed, 2 root-gated ignored |
| MCP rootless live | custom graph 2/2, 0% loss |
| Python bindings | 5/5 non-root smoke |
| Differential topology ladder | 6/6 MATCH |
| OVS/live scenarios | 9/9 implemented routed/tenant scenarios pass rootless |
| Docs | Sphinx build succeeds, 0 warnings |
| Shell syntax | `sh -n` clean |
| Docker fallback | OVS `single,3` 6/6 |
| Rootless OVS preflight | host module check and `lxbr` fallback verified |
| Podman rootless | OVS `single,3` 6/6; scenario runs pass |

## 7. Defects and resolutions

### 7.1 OVS secure mode hang

**Symptom:** `--switch ovs --controller none --test pingall` stopped at
`*** Waiting for switches to connect`.

**Cause:** OVS bridge used `fail_mode: secure` with a passive controller port
but no active controller. `connected()` never returned true and no forwarding
flow was installed.

**Fix:** default `ovs,failMode=standalone`, add `--nolistenport` when no
controller/listen option is requested.

**Evidence:** after fix, `single,3` 6/6; OVS `dump-flows` showed
`actions=NORMAL`.

### 7.2 Podman keep-id prevents OVS runtime setup

**Symptom:** `mkdir: cannot create directory '/var/run/openvswitch': Permission denied`.

**Cause:** `--userns=keep-id` mapping made the runtime paths inaccessible to
the container's effective root.

**Fix:** remove forced keep-id. Rootless default mapping still leaves
bind-mounted run files owned by the host user.

**Evidence:** rootless run artifacts `1000:984`, OVS started, run succeeded.

### 7.3 Missing OVS in research image

**Symptom:** scenarios with `switch` nodes failed or started only LinuxBridge.

**Cause:** image installed FRR but not OVS, and entrypoint exec'd the runner
directly.

**Fix:** install `openvswitch-switch`; add OVS startup entrypoint; copy it into
the image.

### 7.4 Hardcoded LinuxBridge

**Symptom:** all switch nodes were Linux bridges regardless of design.

**Cause:** `switch_spec: "lxbr"` in runner.

**Fix:** scenario field with OVS default; legacy scenarios explicit.

### 7.5 Root-test `mnexec` PATH

**Symptom:** 11 live tests failed with
`failed to spawn "mnexec ...": No such file or directory`.

**Cause:** root Docker recipe compiled but did not install `mnexec`.

**Fix:** build and install it before ignored tests; rerun 29/29.

### 7.6 Benchmark invalid input

**Symptom:** `BENCH_RUNS=runs=1` produced empty medians and a misleading report.

**Fix:** strict integer/positive validation, exit 2. Tested inside rootless
Podman.

### 7.7 Ring test readiness

**Symptom:** IS-IS adjacency step returned before forwarding routes were usable;
first ping attempts failed.

**Cause:** adjacency state precedes full route installation; startup time
varies.

**Fix:** keep bounded retries on the actual reachability assertion. Result:
12 retries, then pass. This is documented rather than hidden.

### 7.8 Missing host OVS module was only documented

**Symptom:** a rootless user could start a container with an OVS scenario only
to fail inside the container with an opaque datapath error.

**Fix:** `scripts/mnr-container.sh` now checks `/proc/modules` before starting
when OVS is requested, prints the exact `modprobe`/`modules-load.d` commands,
and exits 3. `--switch lxbr` remains the fallback. The check was verified on
the loaded host for both OVS and LinuxBridge paths.

## 8. Rootless operating model

### Host requirements

```console
$ podman info --format '{{.Host.Security.Rootless}}'   # true
$ lsmod | grep openvswitch                             # module present
$ grep "$USER" /etc/subuid /etc/subgid                 # mappings present
```

- The host must load `openvswitch`; a rootless container cannot `modprobe`.
- OVS userspace runs inside the container.
- `--privileged` is confined to the container's user namespace.
- Scenarios are arbitrary root shell code inside that namespace; treat them as
  trusted and use a disposable host.
- Do not mount sensitive host paths.

### Image flow

```console
$ docker build -f example/Dockerfile -t mininet-research:latest .
$ docker save mininet-research:latest | podman load
$ podman run --rm --init --privileged --network none \
    -v "$PWD/runs:/runs" \
    -v "$PWD/example/scenarios/multi-tenant-ovs.json:/scenario.json:ro" \
    mininet-research:latest /scenario.json /runs/mt-001
```

### Rootless wrapper flow

```console
$ sh scripts/mnr-container.sh --topo single,3 --controller none --test pingall
$ MNR_RUNTIME=docker sh scripts/mnr-container.sh --topo single,3 --controller none --test pingall
```

### Artifact layout

```text
runs/<run>/
  scenario.json
  events.jsonl
  telemetry.jsonl
  provenance.json
  steps/NNNN-attempt-MMMM.{json,stdout,stderr}
  frr/...
```

The run directory must not already exist. Step files are no-clobber; event and
telemetry streams append.

## 9. Remaining work

The full status is maintained in `ROADMAP.md`. Summary:

| Design | State | Next |
|---|---|---|
| D01 Clos L3 eBGP | reduced implementation | 4 leaves/8 servers, leaf failure |
| D02 three-tier | planned | OSPF areas, summaries, VLAN access |
| D03 VRF-lite | reduced implementation | 4 tenants, per-VRF FRR, shared-service leak |
| D04 multi-site VLAN | implemented | add independent per-tenant faults |
| D05 VXLAN | planned | two-VTEP smoke, then tenant overlay |
| D06 dual-home | active-backup implemented | OVS LACP decision |
| D07 ring/chords | implemented | two-link partition, convergence timing |
| D08 hub-spoke BGP | planned | default route, filters, no transit |
| D09 torus | planned | generator, 16/36 routers, scale budget |
| D10 service chain | planned | firewall/NAT policy matrix |
| D11 IPv6 anycast | planned | OSPFv3/BGP, dual-stack failover |
| D12 chaos | partial | signals, daemon kill, repeated cycles |

Immediate next increments:

1. D01 Clos eBGP.
2. D03 VRF-lite.
3. Chaos signal/daemon-kill suite.
4. OVS switch-readiness checks inside scenarios (host-module preflight is done).
5. Five-sample release benchmark and scale run.

## 10. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Host lacks OVS kernel module | rootless OVS cannot start | preflight and documented `modprobe` |
| Privileged container | host exposure | disposable VM, trusted scenarios, no sensitive mounts |
| OVS standalone semantics | differs from controller-driven OVS | label results as standalone; controller experiments later |
| STP/FRR convergence variance | flaky tests | retry on reachability, not fixed sleep |
| Benchmark noise | misleading comparison | multiple samples, release build, fingerprint |
| Hand-edited JSON | long scenarios are error-prone | generator and schema validation before run |
| Rootless image build | registry credentials may fail | build with Docker and load into Podman for now |

## 11. Reproducibility checklist

- [ ] `just check`
- [ ] `just test`
- [ ] `just root-tests-docker` on a privileged host
- [ ] `just research-rootless <scenario> <new-run>`
- [ ] `just bench-rootless 3`
- [ ] verify `lsmod | grep openvswitch` on the host
- [ ] verify run directory is new and parent-owned by the researcher
- [ ] retain the complete run directory and `provenance.json`
- [ ] update scenario map, roadmap, and changelog for new designs

## 12. Document index

| Document | Purpose |
|---|---|
| `README.md` | project entry point, quickstart, docs index |
| `CHANGELOG.md` | unreleased changes and baseline |
| `ROADMAP.md` | current status, designs, 30/60/90-day work |
| `docs/research/TOPOLOGY_DESIGNS.md` | 12 design blueprints |
| `docs/research/SCENARIO_MAP.md` | C0–C8 complexity ladder |
| `docs/research/RESEARCH_PLANS.md` | ten research programmes |
| `docs/research/REAL_SWITCHES_ROOTLESS.md` | OVS/rootless evidence and setup |
| `docs/research/TECHNICAL_REPORT.md` | earlier feature/robustness validation |
| `docs/research/IMPLEMENTATION_REPORT.md` | this consolidated report |
| `example/README.md` | runner schema, presets, operational notes |
| `docs/migration/BENCHMARKS.md` | historical and OVS benchmark data |
