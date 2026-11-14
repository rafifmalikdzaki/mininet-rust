# Research plans

These are independent but composable research programmes. Each one names the
existing feature it exercises, a falsifiable question, experiments that can be
run on one Linux host, metrics, and a concrete deliverable. None assumes an
embedded AI model: the AI-facing surface is the MCP tool contract and the
external agent that drives it.

## Portfolio at a glance

| Plan | Programme | Features | Horizon | Depends on |
|---|---|---|---|---|
| **P1** | Differential fidelity and fuzzing | `mn-util`, `mn-topo`, CLI, legacy Python | 4–6 weeks | none |
| **P2** | Linux datapath and link semantics | `mn-netlink`, `mn-link`, TCLink, node/switch | 6–8 weeks | P1 |
| **P3** | Routing control-plane convergence | research runner, FRR presets, telemetry | 6–10 weeks | P1, P2 |
| **P4** | Multi-tenant isolation and policy | bridge VLANs, overlapping tenants, runner | 6–8 weeks | P3 |
| **P5** | Resilience, cleanup, and chaos | lifecycle, signals, runner, containers | 6–8 weeks | P2, P4 |
| **P6** | Agent/MCP reliability and safety | `mnr-mcp`, schemas, session state | 4–6 weeks | P1 |
| **P7** | Observability and evidence integrity | runner telemetry, monitor, Prometheus/Loki | 5–7 weeks | P3, P5 |
| **P8** | Performance, scale, and resource cost | benchmark ladder, TUI, core | 6–8 weeks | P1, P2 |
| **P9** | Portability, packaging, bindings | containers, rootless, PyO3, docs | 4–6 weeks | none |
| **P10** | Security and privilege-boundary analysis | privileged scenarios, MCP root gate, artifacts | 4–6 weeks | P5, P6 |

Recommended waves:

1. **Evidence wave** — P1, P6, P9: make the current correctness and agent
   claims reproducible before adding new scenarios.
2. **Network wave** — P2, P3, P4: understand data-plane, control-plane, and
   isolation behaviour.
3. **Operations wave** — P5, P7, P8, P10: resilience, observability, scale, and
   threat boundaries.

## P1 — Differential fidelity and fuzzing

**Question.** Where does the Rust implementation diverge from Python Mininet,
and are divergences bounded, explainable, and regression-tested?

**Current evidence.** The unprivileged suite passed 445 tests with 0 failures
on this host. A fresh privileged-container run matched Python on all six
topology specs (`single,3`, `linear,4`, `linear,10`, `tree,2,2`, `tree,3,2`,
`tree,3,3`).

**Hypotheses.**

- H1.1 Randomized topology specs expose divergences not covered by the six
  fixed fixtures.
- H1.2 Parameterized host/link/controller combinations produce differences in
  representation rather than behaviour, which can be normalized and documented.
- H1.3 Repeated runs have deterministic normalized output; wall-clock fields are
  the only nondeterminism.

**Experiments.**

1. Generate 100+ valid and invalid specs from the topology grammar; run Rust
   `--dump` and the Python driver; classify normalized diffs.
2. Fuzz CLI option combinations (`--switch`, `--host`, `--link`, `--ipbase`,
   `--mac`, `--arp`); compare option parsing and refusal messages.
3. Replay golden REPL sessions under load and with a non-tty stdin.
4. Seed a corpus of malformed specs and assert the Rust side never panics.

**Metrics.** mismatch rate by topology class; unclassified diff count; panic
count; normalized-output determinism across 20 repeats.

**Deliverables.** A reproducible fuzz target, a minimized divergence corpus,
updated `scripts/diff_topology.sh` modes, and a fidelity section in the report.

**Risks.** Python-side global state and namespace names can make the reference
flaky; isolate each comparison in a fresh process and a unique run directory.

## P2 — Linux datapath and link semantics

**Question.** Does the netlink implementation preserve the intended semantics
of veths, bridges, namespaces, addresses, routes, qdiscs, and switch variants
under failure and load?

**Hypotheses.**

- H2.1 LinuxBridge and OVS produce identical reachability for the supported
  topologies, with different control-plane and port semantics.
- H2.2 TCLink `delay`, `loss`, `jitter`, and `bw` match configured values within
  measurement error.
- H2.3 Partial link failures leave no stale qdiscs, addresses, or interfaces.

**Experiments.**

1. Matrix: `{lxbr, ovs, ovsbr}` × `{linear, tree, single}` × `{none, controller}`
   with full pingall and link-status assertions.
2. netem matrix: delay 10/50/100 ms, loss 1/5/10%, jitter, rate limits; measure
   with ping and iperf3; compare configured vs observed.
3. Delete a veth peer, bring a port down, re-add it; assert cleanup and recovery.
4. Run start/stop 100 times; count leaked namespaces, interfaces, and bridges.

**Metrics.** reachability matrix, configured-vs-measured delay/loss/bandwidth,
cleanup leak count, start/stop latency.

**Deliverables.** A datapath conformance suite and a documented support matrix
for switch/link combinations.

**Risks.** The host kernel and OVS datapath module determine what can run;
record kernel, OVS, and container privileges with every result.

## P3 — Routing control-plane convergence

**Question.** How do OSPF, IS-IS, and BGP converge in this emulator, and which
topology or timer properties dominate convergence and route stability?

**Current evidence.** All presets passed in the privileged research image:
OSPF ~52 s (32 adjacency attempts), IS-IS ~36 s (26 attempts), BGP ~5 s (2
attempts), VLAN ~6 s, multi-tenant ~43 s, multi-tenant fault ~50 s. Attempt
counts, not wall time, are the useful convergence proxy.

**Hypotheses.**

- H3.1 Adjacency time is dominated by FRR startup and hello/dead timers, not by
  the emulator.
- H3.2 A link-cost change converges without forwarding loops in a two-path
  topology.
- H3.3 eBGP withdraw/re-announce leaves no stale best path.

**Experiments.**

1. Repeat each preset 10–20 times; report attempt-count distributions and
   wall-time percentiles.
2. Build a four-router square with equal and unequal costs; change a cost and
   measure reconvergence to the alternate path.
3. BGP: announce, withdraw, re-announce a prefix; sample `show bgp` and pings.
4. Vary FRR timers in a controlled copy of the preset; separate timer effects
   from emulator effects.
5. Add authentication mismatch and assert the adjacency never falsely reports
   success.

**Metrics.** attempts-to-convergence, p50/p95 wall time, route-install time,
packet loss during reconvergence, stale-route count.

**Deliverables.** A convergence dataset, parameterized scenarios, and a
statistical summary per protocol.

**Risks.** FRR’s daemon startup and pathspace behaviour can dominate; never
claim a protocol improvement caused by the emulator without isolating timers.

## P4 — Multi-tenant isolation and policy

**Question.** Can tenants with overlapping address space be kept isolated while
sharing physical links, and can policy be verified exhaustively?

**Current evidence.** `vlan` and `multi-tenant` passed; the multi-tenant preset
checks intra-tenant reachability and cross-tenant failure, including VLAN
trunks. The fault preset verifies isolation during and after a trunk failure.

**Hypotheses.**

- H4.1 Bridge VLAN filtering prevents leakage for all ordered tenant pairs, not
  only the sampled pairs.
- H4.2 A misconfigured trunk (missing VLAN) fails closed and is detectable from
  the telemetry and ping matrix.
- H4.3 ACL/policy matrices can be checked exhaustively at 3–5 tenants without
  combinatorial runtime becoming prohibitive.

**Experiments.**

1. Generate the full tenant × tenant connectivity matrix; compare with expected
   isolation; include same-address tenants.
2. Remove one VLAN from a trunk and verify only that tenant degrades.
3. Add ingress/egress ACLs and test allow/deny for every ordered pair.
4. Scale to 10 tenants and 30 hosts; measure runtime, artifact size, and
   failures.
5. Repeat after trunk flaps; assert no VLAN leakage or stale FDB entries.

**Metrics.** false-allow and false-deny counts, isolation confidence, runtime,
artifact bytes, FDB/VLAN state after teardown.

**Deliverables.** An isolation matrix generator, ACL scenarios, and a
tenant-scale report.

**Risks.** Linux bridge VLAN filtering semantics differ from OVS; run the matrix
on both datapaths before generalizing.

## P5 — Resilience, cleanup, and chaos

**Question.** When startup, a command, a daemon, or the runner is interrupted,
does the system converge to a clean state without orphaning kernel or process
resources?

**Current evidence.** Malformed scenarios are rejected before side effects;
nonzero exits, timeouts (124), missing commands (127), and `expect_stdout`
mismatches fail the run with captured artifacts and cleanup. A full
chaos matrix has not been run.

**Hypotheses.**

- H5.1 SIGINT/SIGTERM during start, a step, or hold always emits `finish` and
  removes namespaces/interfaces.
- H5.2 Killing a router daemon triggers reconvergence without leaving a
  namespace process.
- H5.3 Repeating a failed run with the same output directory is refused cleanly
  rather than overwriting evidence.

**Experiments.**

1. Kill the runner at controlled phases (before start, during start, during a
   long step, during hold); inspect kernel state and artifacts.
2. `kill -9` FRR daemons and nodes; measure reconvergence and zombie processes.
3. Run 50 consecutive start/stop cycles; compare namespace/veth/bridge counts.
4. Stress output-directory edge cases: existing directory, symlink, read-only
   parent, ownership mismatch.
5. Power-loss simulation: truncate `events.jsonl` and `telemetry.jsonl`; verify
   the monitor reports rather than invents history.

**Metrics.** leaked namespaces/veths/bridges/processes, cleanup time, finish
event completeness, exit-code correctness.

**Deliverables.** A chaos harness with phase injection and a cleanup invariant
checklist.

**Risks.** A privileged container is not a security boundary. Run only on a
disposable host and never against sensitive mounts.

## P6 — Agent/MCP reliability and safety

**Question.** Can an external agent drive the emulator through MCP predictably,
recover from invalid states, and stay within its privilege boundary?

**Current evidence.** 13 protocol tests passed (2 root-gated ignored); a live
privileged custom-graph session built, started, pinged 2/2 with 0% loss,
stopped, and released. Unprivileged `net_start` is rejected. There is no
embedded model or prompt logic to evaluate.

**Hypotheses.**

- H6.1 Tool errors are actionable and state-preserving: an invalid build does
  not destroy a previously built network.
- H6.2 A schema-constrained agent can complete build → start → test → stop
  without undocumented calls.
- H6.3 Tool-call count and invalid-call rate distinguish a good agent workflow
  from a lucky one.

**Experiments.**

1. Scripted agent tasks: plan-only, custom graph, connectivity test, recovery
   after calling `pingall` before `net_start`.
2. Adversarial tasks: unknown options, reserved `c0`, overlong names, graph
   bounds, calls after release, malformed JSON-RPC.
3. Instrument an external MCP client to record tool sequence, latency, errors,
   and final state; repeat with multiple clients/models.
4. Concurrency: two clients on one stdio process; document the single-session
   contract and test failure modes.
5. Safety review: ensure `net_start`/`host_cmd` cannot run without root and
   that no tool silently shells out to a privileged helper.

**Metrics.** task success rate, median tool calls, invalid-call rate, recovery
rate after an injected error, wall time, destructive-action count.

**Deliverables.** An MCP evaluation harness and a published tool-contract
conformance suite.

**Risks.** Model variability can swamp tool effects; report per-model results
and keep a deterministic scripted baseline.

## P7 — Observability and evidence integrity

**Question.** Are run events, telemetry, provenance, and dashboards sufficient
to reconstruct what happened, and at what overhead?

**Current evidence.** OSPF produced 46 events, 444 telemetry records, and a
724 KB run directory; multi-tenant produced 1,116 telemetry records. The
monitor’s 12 unit tests cover incremental append, truncation, stale/disconnected
states, and no-newline line accounting. The monitoring stack started healthy;
Prometheus reported `prometheus`, `alloy`, and `research-host` targets up;
Grafana exposed the provisioned dashboard and both datasources. Alloy exposed
10 event-file and 10 telemetry-file series, and a Loki query returned actual
`step` and `finish` records; restart/rescan and retention behaviour remain to
be tested.

**Hypotheses.**

- H7.1 Telemetry overhead is below a stated fraction of scenario wall time at
  one sample per step plus hold cadence.
- H7.2 Every artifact can be reproduced from `scenario.json` + provenance +
  event stream.
- H7.3 Monitor memory stays bounded on 100 MB+ event files.

**Experiments.**

1. Measure sampling overhead with holds of 0, 30, 120 s and with telemetry
   disabled/enabled.
2. Corrupt/truncate each artifact and verify the monitor reports instead of
   fabricating.
3. Validate the event schema and cross-reference step IDs to files.
4. Feed a synthetic run to Alloy/Loki and verify ingestion labels and queries.
5. Simulate clock jumps and compare runner timestamps with `date -u` in
   provenance.

**Metrics.** overhead %, artifact bytes/event, ingestion lag, monitor RSS and
load time, unreconstructable runs.

**Deliverables.** An evidence schema, an integrity checker, and a monitoring
integration test.

**Risks.** The monitoring images are third-party and versioned independently;
pin digests before publishing reproducibility claims.

## P8 — Performance, scale, and resource cost

**Question.** How do wall time, CPU, memory, and UI responsiveness scale with
node count and event volume?

**Current evidence.** A live `single,3` run completed in 0.228 s. A fresh
one-sample smoke benchmark on `single,3`, `tree,2,2`, and `tree,3,3` completed
with 0% loss; the debug Rust binary was slower on all three in this one-sample
run. This is a correctness smoke, not a performance dataset.

**Hypotheses.**

- H8.1 Per-node startup cost is sublinear in the tested range because namespace
  and veth creation dominate.
- H8.2 The monitor’s incremental refresh remains under 10 ms at 100k events.
- H8.3 Artifact volume, not network size, is the main disk-pressure driver.

**Experiments.**

1. Re-run `scripts/bench_scenarios.sh` with 5–20 repetitions on a fixed host;
   report median/min/max and confidence intervals.
2. Extend the ladder to `tree,3,3` (40), `tree,4,3` (121), and a 100-host star;
   record kernel limits and failures.
3. Measure `mnr` RSS during start, pingall, and idle.
4. Measure monitor initial load and refresh on synthetic 1 MB/10 MB/100 MB
   event files.
5. Test TUI responsiveness at 100 nodes with pan/zoom frame timing.

**Metrics.** wall time, CPU seconds, peak RSS, refresh latency, frame time,
artifact bytes, failure rate.

**Deliverables.** A benchmark dataset with environment fingerprint and a scale
limit report.

**Risks.** Container/host scheduling noise; use medians, report host CPU count,
and avoid cross-host comparisons without a fingerprint.

## P9 — Portability, packaging, and bindings

**Question.** Can the Rust core be built and used across the supported
environments without hidden host dependencies?

**Current evidence.** Workspace release build, research image, and rootless
container image built successfully; docs build with Sphinx with zero warnings.
The PyO3 extension was built with maturin for CPython 3.14; non-root
`version`, `plan`, `Mininet`, `dump`, `links`, and `plan_json` were exercised.
Root lifecycle through the binding was not exercised (covered instead by the
Rust live suites).

**Hypotheses.**

- H9.1 The container images are reproducible from a clean checkout with pinned
  dependencies.
- H9.2 Rootless networking works on a documented kernel/container-runtime
  matrix.
- H9.3 Python bindings expose the same lifecycle and fail with the same errors
  as the CLI.

**Experiments.**

1. Build both images from a clean checkout; record sizes and build times.
2. Test rootless Docker and Podman on at least two distributions; document
  subuid/subgid and `CAP_NET_ADMIN` results.
3. Build a Python wheel with maturin; run a lifecycle script and compare with
  the CLI.
4. Build docs with `-W`; fix warnings.
5. Verify architecture of all shipped binaries and check for unexpected dynamic
  dependencies.

**Metrics.** build success, image/binary size, startup overhead, binding
task success, docs warning count.

**Deliverables.** A support matrix, reproducible image recipes, and a binding
smoke suite.

**Risks.** Rootless podman and Docker differ in user-namespace mapping; a
single successful host is not a portability claim.

## P10 — Security and privilege-boundary analysis

**Question.** What is the actual attack surface of a privileged emulation
container, the MCP server, and the artifact pipeline, and which guarantees are
enforced versus merely documented?

**Hypotheses.**

- H10.1 The MCP root gate prevents unprivileged namespace creation through
  every tool.
- H10.2 Scenario files are arbitrary root shell code by design; only isolation
  of the host, not scenario sandboxing, protects the operator.
- H10.3 Artifact creation resists symlink and ownership tricks in the tested
  paths.

**Experiments.**

1. Attempt path traversal, symlink swap, hardlink, and pre-existing-file attacks
   on run directories and step artifacts.
2. Call every MCP tool without root, with wrong types, and after release;
   verify no escalation and no state corruption.
3. Inspect container capabilities and mounts; enumerate what a scenario can
   reach; compare with the documented threat model.
4. Test monitoring exposure: loopback bindings, unauthenticated Loki, Grafana
   bootstrap password semantics.
5. Review dependency and image provenance; pin digests and check for secrets in
   build args and artifacts.

**Metrics.** escalation count (target zero), artifact overwrite count, exposed
listeners, undocumented privileges, dependency CVEs.

**Deliverables.** A threat model, a security test suite, and a hardening
checklist.

**Risks.** Do not run destructive attacks against production hosts; use a
disposable VM and clearly label the privilege boundary as non-sandboxing.

## Feature coverage matrix

| Feature | Primary plans | Current validation |
|---|---|---|
| `mn-util`, `mn-topo` | P1 | unit + differential tests |
| `mn-netlink`, `mn-link`, TCLink | P1, P2 | unit; root/datapath matrix partial |
| `mn-node`, `mn-net`, lifecycle | P2, P5 | unit + live `single,3`; chaos new |
| CLI `mnr` and REPL | P1, P2 | golden, option, live pingall |
| `mnexec` | P1, P2 | unit; installed for root gate |
| TUI core and `mnr-tui` | P8 | headless golden, JSON, live frames |
| MCP server and schema | P6, P10 | 13 protocol tests, live custom graph |
| Research runner | P3, P4, P5, P7 | all presets, error injection |
| Research monitor | P7 | 12 unit tests, PTY smoke |
| Monitoring stack | P7, P10 | compose config, services/dashboard live |
| Containers/rootless images | P9 | both images built; root gate |
| Python bindings | P9 | wheel + non-root smoke; root new |
| Legacy differential harness | P1 | fresh 6/6 MATCH; fuzz new |
| Docs and evidence | P7, P9 | Sphinx build, report |
