# Scenario map: from deterministic plans to agent-driven networks

This map connects the features in this repository to network-research questions.
It is deliberately cumulative: a scenario at level **C4** should still assert
everything at C0–C3, or explicitly document why it cannot.

The repository has no embedded language model. “AI features” here means the
Model Context Protocol (MCP) server and the agent-facing tool contract; model
behaviour is supplied by the external client.

## Complexity ladder

| Level | Question | Representative topology | Primary feature exercised | Evidence |
|---|---|---|---|---|
| **C0 — Plan** | Is the requested graph the graph the model built? | `single,3`, `tree,2,2`, `torus,3,3,n1` | `mn-topo`, CLI `--dump`, TUI `--json`, MCP `topo_plan`/`net_build` | no kernel privileges required |
| **C1 — Connectivity** | Can every intended pair reach every other pair? | `single,3`, `linear,4` with LinuxBridge or OVS | `mn-netlink`, `mn-link`, `mn-node`, `mn-net` | live `pingall`, links `(OK OK)` |
| **C2 — Control plane** | Do routing protocols converge and install the expected routes? | 2–4 routers sharing a segment | FRR + experiment runner | OSPF/IS-IS/BGP presets, loopback pings |
| **C3 — Segmentation** | Can tenants with overlapping addresses communicate internally and not externally? | 3 sites × 3 tenants, VLAN-filtered trunks | Linux bridge VLAN filtering + runner | `vlan`, `multi-tenant` presets |
| **C4 — Fault** | Does the network detect, isolate, and recover from a failed component? | Multi-tenant trunk cut and restore | runner steps + link state | `multi-tenant-fault` preset |
| **C5 — Performance** | What are throughput, latency, jitter, loss, and convergence cost? | links with netem/tc, iperf3 | `TCLink`, benchmark ladder | requires new scenario steps |
| **C6 — Scale and soak** | Do lifecycle, artifacts, and observation stay correct for minutes and hundreds of nodes? | `tree,3,3` (40 nodes) upward | all lifecycle paths, monitor, telemetry | partial; benchmark ladder covers 40 nodes |
| **C7 — Adversarial** | Does bad input, a bad command, or a hostile component fail closed and clean up? | malformed JSON, timeouts, missing binaries, interrupted runs | validation, error paths, ownership | exercised in this report; more cases needed |
| **C8 — Agent orchestration** | Can an external agent complete a network task reliably and safely through MCP? | multi-step build/start/test/stop session | MCP protocol, state machine, root gate | protocol + live smoke tests |

```text
C0 plan
  └─ C1 connectivity
       ├─ C2 control plane ── C5 performance
       ├─ C3 segmentation ─── C4 fault
       └─ C6 scale/soak
C7 adversarial applies at every level
C8 agent orchestration composes every level through MCP
```

## Scenario catalogue

The table names the scenario, the assertions that make it useful, and the
current support. “New” means the repository provides the primitives but not a
ready-made scenario.

### C0 — Deterministic plans

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| P0.1 | Minimal / single / reversed | exact nodes, ordered ports, host IPs | CLI/TUI/MCP tests |
| P0.2 | Linear / tree / torus | switch counts, edge counts, IP allocation | `mn-topo` golden tests |
| P0.3 | Custom graph via MCP | named hosts/switches/links, reserved `c0`, 256-node/1024-link bounds | MCP tests; live smoke |
| P0.4 | Invalid graph | duplicate names/ports, self-links, missing endpoints, overlong interface names rejected before side effects | runner + MCP tests |
| P0.5 | Python differential | normalized Rust plan equals Python plan | `scripts/diff_topology.sh` (root) |

### C1 — Connectivity and lifecycle

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C1.1 | `single,3`, LinuxBridge | 6/6 ping success, link status `OK`, stop removes all state | live test passed |
| C1.2 | `linear,4`, OVS | full reachability, controller/switch startup | root-gated tests |
| C1.3 | No controller | hosts communicate without a control plane | live test passed |
| C1.4 | Interactive REPL | `nodes`/`dump`/`links`/`pingall` match expected shapes | golden + root tests |
| C1.5 | Start/stop idempotency | second stop is harmless; no orphan namespaces/veths | partial; chaos plan P5 |

### C2 — Control plane

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C2.1 | OSPF adjacency | neighbor reaches `Full`; loopback route via peer; ping succeeds | preset passed (32 attempts, ~52 s) |
| C2.2 | IS-IS adjacency | neighbor reaches `Up`; route installed; ping succeeds | preset passed (~36 s) |
| C2.3 | eBGP | session `Established`; prefix exchanged; ping succeeds | preset passed (~5 s) |
| C2.4 | IGP convergence after cost change | measure time to alternate path; no forwarding loop | new scenario |
| C2.5 | BGP withdraw/re-announce | route removal and restoration, no stale best path | new scenario |
| C2.6 | Protocol authentication / mismatch | bad key is rejected; adjacency never falsely reports success | new scenario |

### C3 — Segmentation and policy

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C3.1 | Two VLANs | intra-VLAN reachability, inter-VLAN isolation | `vlan` preset passed |
| C3.2 | Overlapping tenants | each tenant reaches its own services; cross-tenant pings fail | `multi-tenant` preset passed |
| C3.3 | Trunk carries all tenants | VLANs 10/20/30 cross sites without leaking | multi-tenant preset |
| C3.4 | Policy/ACL matrix | allow/deny matches intent for every ordered pair | new; requires ACL steps |
| C3.5 | Tenant scale-out | 10+ tenants, no VLAN/port collision, artifact volume bounded | new |

### C4 — Fault and recovery

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C4.1 | Trunk down/up | isolation during fault, restoration after repair, measurements before/during/after | `multi-tenant-fault` passed |
| C4.2 | Access-link failure | only the affected host loses connectivity | new |
| C4.3 | Router/daemon crash | routing reconverges; no zombie FRR process | new |
| C4.4 | Partial startup failure | cleanup removes every created namespace/link/process; nonzero exit | partial (runner cleanup on step failure) |
| C4.5 | SIGINT/SIGTERM | finish event records interruption; teardown completes | runner traps signals; needs test |

### C5 — Performance

| ID | Scenario | Metrics | Current support |
|---|---|---|---|
| C5.1 | TCP throughput | Gbit/s, CPU per Gbit/s | iperf3 available; new scenario |
| C5.2 | UDP loss/jitter | loss %, p50/p99 jitter | iperf3 available; new scenario |
| C5.3 | netem delay/loss/rate | configured vs measured delay/loss/bw | `TCLink` tests; new end-to-end scenario |
| C5.4 | Protocol convergence time | time from fault to restored reachability | measurable from `events.jsonl`; new analysis |
| C5.5 | Scale ladder | wall time and RSS for 4→40→100 nodes | benchmark ladder up to 40; extension new |

### C6 — Scale and soak

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C6.1 | 40-node tree | start/pingall/stop stays within time budget | benchmark ladder |
| C6.2 | 100+ nodes | name/port/interface constraints, file descriptors, kernel limits | new |
| C6.3 | Long hold | telemetry cadence and disk growth predictable | `--hold-secs` supported |
| C6.4 | Monitor during long run | stale/disconnected states truthful; memory bounded | monitor unit tests; soak new |
| C6.5 | Repeated runs | no leaked namespaces, veths, bridges, FRR pathspaces | new chaos/soak suite |

### C7 — Adversarial and failure-oriented

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C7.1 | Malformed scenario | rejected before run directory or namespace creation | verified |
| C7.2 | Command exits nonzero | attempts recorded, artifacts captured, run fails nonzero | verified |
| C7.3 | Command timeout | `timeout` kills, exit 124, cleanup succeeds | verified |
| C7.4 | Missing binary | exit 127, stderr captured | verified |
| C7.5 | `expect_stdout` mismatch | command exit 0 but step fails; retries bounded | verified |
| C7.6 | Symlink/ownership attacks | `O_NOFOLLOW` / no-clobber invariants hold | unit-level; root tests recommended |
| C7.7 | MCP unprivileged misuse | root-gated tools return tool errors, never escalate | protocol test |
| C7.8 | Untrusted scenario in privileged container | threat model documented; scenario is arbitrary root shell | documented; no sandbox |

### C8 — Agent orchestration

| ID | Scenario | Assertions | Current support |
|---|---|---|---|
| C8.1 | Plan-only agent | `topo_list` → `topo_plan` → inspect graph; no root | protocol tests |
| C8.2 | Custom graph build | `net_build` with `topology`; no implicit start | passed live |
| C8.3 | Build/start/test/stop | one session retains state; stop/release are explicit | live smoke passed |
| C8.4 | Error recovery | start before build, ping before start, invalid opts produce actionable errors without corrupting session | protocol tests; extend |
| C8.5 | Agent benchmark | task success, tool-call count, invalid-call rate, time-to-result | new; plan P6 |
| C8.6 | Concurrency | two clients cannot silently share one live network | not supported; stdio one-client model |

## What “modern networking research” maps to here

| Research theme | This repository’s angle | Missing to study it seriously |
|---|---|---|
| Intent-based networking | MCP turns intent-ish JSON into a validated plan, then a live network | intent language, policy compiler, closed-loop verification |
| Network digital twins | deterministic plan + reproducible artifact bundle per run | telemetry import, state comparison, what-if replay |
| AI-driven operations | agent-facing MCP tools with explicit state and errors | agent evaluation harness, guardrails, explanation traces |
| Programmable data planes | OVS/LinuxBridge control plane; TCLink netem | P4/eBPF datapath, per-flow telemetry |
| Network slicing / multi-tenancy | overlapping tenants + bridge VLAN filtering | resource isolation (CPU/bandwidth), SLA enforcement |
| Resilience / SRE | fault steps, per-attempt evidence, cleanup | automated chaos, SLOs, recovery-time distributions |
| Observability | per-node probes, run events, Prometheus/Loki stack | per-node network metrics, correlation IDs, retention policy |
| Security / zero trust | root-gated tools, stdio-only MCP, no untrusted-scenario sandbox | threat model, isolation boundary, signed artifacts |
| Deterministic networking | netem delay/loss/bandwidth | TSN-style schedules, hardware timestamping |
| Edge/cloud continuity | containerized emulator, rootless mode | multi-host/cluster emulation, mobility |

The honest boundary: this is a **single-host Linux network emulator with strong
evidence capture**, not a hardware testbed or a production digital twin.
