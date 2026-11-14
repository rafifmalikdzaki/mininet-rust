# Real OVS switches, rootless execution, and validation

**Date:** 2026-11-14  
**Base revision:** `c0a9cb2` plus the real-switch changes  
**Scope:** OVS in the research runner, rootless operation, tenant/fault
scenarios, routed OVS, link redundancy, redundant fabric, and benchmarks.

## Result

| Area | Result |
|---|---|
| Real switch default | `ovs,failMode=standalone` for scenario switch nodes |
| Rootless runtime | Podman 6.2.0 rootless, `--privileged` inside the user namespace |
| OVS kernel datapath | available; `ovs-vswitchd` 3.3.9 uses `system@ovs-system` |
| Rootless `mnr` `single,3` | **6/6 received, 0% loss**, 0.305 s |
| OVS VLAN isolation | intra-VLAN pass, cross-VLAN fail, run success |
| OVS multi-tenant | 30/30 steps, full positive + negative tenant matrix, success |
| OVS tenant fault | 38/38 steps, trunk down/up and recovery, success |
| OSPF over OVS | adjacency `Full`, routes installed, loopback pings pass |
| Dual-homed active-backup | traffic survives active-member failure, recovers |
| Redundant leaf-spine + STP | 4 OVS switches, full host reachability after STP convergence |
| Benchmark ladder | 6 topologies, 3 samples each, all 12 cells 0% loss |
| Rootless MCP lifecycle | custom graph build/start/ping 2/2/stop/release passed |
| Invalid benchmark input | `BENCH_RUNS=bogus` rejected with exit 2 |
| Artifact ownership | rootless run files owned by the invoking user (1000:984) |

No functional regression was found in the exercised switch paths. The main
failures were integration/configuration issues, fixed below.

## Why the first rootless OVS attempts failed

1. **OVS in secure mode with no controller hung at “Waiting for switches to
   connect.”** The bridge had a passive `ptcp` controller and `fail_mode:
   secure`, so no forwarding and no successful controller connection.
   Fix: use `ovs,failMode=standalone` and suppress the passive port with
   `--nolistenport` when no controller exists.
2. **Podman `--userns=keep-id` prevented OVS from creating
   `/var/run/openvswitch`.** With keep-id, container root is not mapped in a
   way that lets the daemons create their runtime paths.
   Fix: do not force keep-id. Rootless Podman still maps container root to the
   invoking user for bind-mounted files, so `/w` artifacts remain host-owned.
3. **The research image did not contain OVS.** Fix: install
   `openvswitch-switch` and add an entrypoint that initializes/starts
   `ovsdb-server` and `ovs-vswitchd` before the runner.
4. **Switch nodes were hardcoded to LinuxBridge.** Fix: scenarios now carry a
   `switch` field; the default is real OVS standalone, and legacy LinuxBridge
   scenarios declare `"switch":"lxbr"`.

## Implemented scenarios

| Scenario | Topology | What it proves | Rootless result |
|---|---|---|---|
| `vlan-ovs` | 1 OVS, 4 hosts, 2 VLANs | access-port tagging and isolation | success, 9 steps |
| `multi-tenant-ovs` | 3 OVS, 9 hosts, 3 tenants | overlapping addresses, tagged trunks, full matrix | success, 30 steps |
| `multi-tenant-fault-ovs` | same + redundant trunk | trunk down/up, isolation preserved, recovery | success, 38 steps |
| `ospf-ovs` | 2 routers, 1 OVS | OSPF adjacency and routes over a real switch | success, 40 adjacency attempts, ~51 s |
| `dual-home-ovs` | 1 OVS, 2 hosts, 2 links | active-backup bond failover and recovery | success, 5 steps |
| `clos-l2-ovs` | 2 spines + 2 leaves, 4 hosts | redundant fabric, STP loop prevention | success after 11 retries, ~35 s |

The legacy `vlan`, `multi-tenant`, and `multi-tenant-fault` scenarios remain
explicit LinuxBridge scenarios for comparison. New designs should use OVS.

### Evidence details

- OVS VLAN scenario: same-VLAN pings return 0% loss; cross-VLAN pings exit 1
  with the runner’s explicit isolation check.
- Multi-tenant: all 9 same-tenant host checks pass; the three cross-tenant
  matrices (18 ordered addresses per tenant) fail as expected.
- OSPF over OVS: the adjacency took 40 attempts; the route and both loopback
  pings then passed. Retry count is more stable than wall time.
- Dual-home: the active slave is read from `ip -d link show bond0` (sysfs
  bonding state is not visible in the host namespace), then taken down; the
  backup becomes active and traffic continues.
- Leaf-spine: STP moves ports through learning/forwarding; the test retries
  reachability until forwarding is established (~12 retries in the `just`
  recipe run). This is expected convergence, not a link failure.
- Rootless MCP: `mnr-mcp` in the rootless image built a custom graph with
  `switch=ovs,failMode=standalone`, started it, pinged 2/2 at 0% loss, stopped,
  and released it.

## Rootless benchmark

Run inside rootless Podman on the host:

```console
$ podman run --rm --privileged --network none \
    -v "$PWD:/w" -w /w -e MN_RUST_BIN=/usr/local/bin/mnr -e BENCH_RUNS=3 \
    mnr-rootless:test sh scripts/bench_scenarios.sh
```

Environment: Linux 7.2.15-zen1, 20 logical CPUs (i9-12900H), Python 3.12.3 in
the container, OVS 3.3.9, `ovs,failMode=standalone`, `--nolistenport`,
3 timed samples plus one warmup per cell.

| Scenario | Nodes | Python median (s) | Rust median (s) | Python min/max | Rust min/max | Loss |
|---|---:|---:|---:|---|---|---:|
| `single,3` | 4 | 0.204 | 0.306 | 0.197/0.210 | 0.297/0.315 | 0% |
| `linear,4` | 8 | 0.408 | 0.543 | 0.397/0.412 | 0.529/0.579 | 0% |
| `linear,10` | 20 | 0.972 | 1.799 | 0.923/0.985 | 1.766/1.812 | 0% |
| `tree,2,2` | 7 | 0.314 | 0.503 | 0.308/0.355 | 0.491/0.511 | 0% |
| `tree,3,2` | 15 | 0.676 | 1.268 | 0.666/0.692 | 1.243/1.298 | 0% |
| `tree,3,3` | 40 | 2.363 | 6.422 | 2.330/2.373 | 6.372/6.517 | 0% |

**Interpretation:** this is a correctness and relative-cost smoke, not a
performance verdict. OVS startup, container scheduling, and a single host
dominate the small topologies. Rust is slower on every cell in this image. The
historical direct-link benchmark reported different results; do not mix them.

## Reproduction runbook

### Host prerequisites

```console
$ podman info --format '{{.Host.Security.Rootless}}'   # true
$ lsmod | grep openvswitch                             # module present
$ cat /etc/subuid /etc/subgid                          # subuid/subgid mappings
```

The rootless container cannot load the OVS kernel module. The host must already
provide it. If the module is absent, rootless OVS cannot use the kernel
datapath; use a rootful disposable host or run the LinuxBridge fallback.

### Build and run

```console
$ docker build -f example/Dockerfile -t mininet-research:ovs .
$ docker save mininet-research:ovs | podman load
$ mkdir -p runs
$ podman run --rm --init --privileged --network none \
    -v "$PWD/runs:/runs" \
    -v "$PWD/example/scenarios/multi-tenant-ovs.json:/scenario.json:ro" \
    mininet-research:ovs /scenario.json /runs/mt-001
```

The research image entrypoint starts OVS. `--privileged` is confined to the
container user namespace under rootless Podman. Scenario commands are still
arbitrary root shell code inside that namespace; use trusted scenarios only.

### Just recipes

```console
$ just research-rootless clos-l2-ovs clos-l2-001
$ just bench-rootless 1
$ just container-run --topo single,3 --controller none --test pingall
```

The research/benchmark recipes require the images to exist in Podman storage.
On this host the image was built with Docker and loaded with
`docker save ... | podman load`, because rootless Podman registry credentials
were unavailable.

### `mnr` wrapper

```console
$ sh scripts/mnr-container.sh --topo single,3 --controller none --test pingall
```

Defaults now:

- real OVS: `ovs,failMode=standalone`;
- no passive controller listener (`--nolistenport`);
- no forced `--userns=keep-id` (OVS needs its runtime paths);
- `lxbr` remains available with an explicit `--switch lxbr`.

## Limitations and risks

1. **Host OVS kernel module required.** Rootless cannot `modprobe` it.
2. **Standalone OVS only.** No OpenFlow controller, LACP, or EVPN control plane
   is exercised. No controller means learning-switch behavior.
3. **LACP not implemented.** The dual-home design uses active-backup. OVS LACP
   is a separate experiment.
4. **STP convergence is slow.** Leaf-spine reachability took ~11 retries; tests
   must not treat initial ping loss as a design failure.
5. **Benchmark single host and single image.** Multiple runs, multiple hosts,
   and release builds are needed for performance claims.
6. **Rootless build path.** This host’s Podman registry credentials failed, so
   the image was built with Docker and loaded into Podman. A fully rootless
   build needs a working rootless builder/registry.
7. **Privileged container is not a sandbox.** OVS daemons and scenarios run with
   broad capabilities inside the user namespace.

## Related files

- `example/entrypoint.sh` — OVS startup before the runner.
- `example/Dockerfile` — OVS package and entrypoint.
- `scripts/mnr-container.sh` — real-switch rootless wrapper defaults.
- `example/scenarios/vlan-ovs.json`
- `example/scenarios/multi-tenant-ovs.json`
- `example/scenarios/multi-tenant-fault-ovs.json`
- `example/scenarios/ospf-ovs.json`
- `example/scenarios/dual-home-ovs.json`
- `example/scenarios/clos-l2-ovs.json`
- `scripts/bench_scenarios.sh` — OVS standalone benchmark defaults.
