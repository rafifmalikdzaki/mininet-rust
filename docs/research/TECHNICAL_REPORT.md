# Technical report: feature validation and robustness

**Date:** 2026-11-14  
**Revision:** `709b2c3` plus the documentation and gate changes in this work  
**Scope:** feature validation, failure-path testing, robustness analysis, and
research planning for the Rust Mininet emulator and its research workbench.

**Follow-up:** real OVS switches and rootless Podman are now implemented and
validated separately in [Real switches and rootless](REAL_SWITCHES_ROOTLESS.md).
The LinuxBridge live result below is retained as historical validation.

## Executive summary

| Claim | Result |
|---|---|
| Unprivileged suite (workspace + monitor) | **445 passed, 0 failed, 29 ignored** (43 suites) |
| Monitor subset | **12 passed, 0 failed** |
| Privileged ignored suites after gate fix | **29 passed, 0 failed** |
| CLI dry-run plan | passed: `single,3 --dump` produced topo/dump/links sections |
| Headless TUI | passed: JSON graph and two rendered frames for `tree,2,2` |
| Live TUI | passed: `single,3` frame with real host PID and `up` status |
| MCP protocol | **13 passed, 2 ignored**; live privileged custom graph passed |
| Differential topology ladder | **6/6 MATCH** (fresh, privileged container, LinuxBridge) |
| Python bindings | wheel built; 5/5 non-root smoke tests passed |
| Live emulation | `single,3` pingall **6/6, 0% loss**, 0.228 s |
| Research presets | **6/6 passed**: OSPF, IS-IS, BGP, VLAN, multi-tenant, multi-tenant fault |
| Malformed scenario validation | 5/5 rejected before side effects |
| Command failure paths | nonzero, timeout (124), missing binary (127), stdout mismatch all captured and failed the run |
| Monitoring stack | Compose valid; healthy; Prometheus targets up; Grafana dashboard/datasources visible; Loki ingested event + telemetry series |
| Docs | Sphinx HTML build succeeds with no warnings |
| Short benchmark smoke | 3 topologies × 1 run: all runs 0% loss |

No functional regression was found in the paths exercised. The main actionable
finding was a **local root-test gate defect**, not an emulator defect: the
privileged Docker recipe built `mnexec` but did not install it on `PATH`, so 11
live tests failed to spawn hosts. The recipe now builds and installs it; the
rerun passed all 29 ignored tests, including live `mnr`, MCP, and TUI paths.

## Environment

| Item | Value |
|---|---|
| Host | Linux 7.2.15-zen1-1-zen x86_64 |
| CPU / RAM | 20 logical CPUs / 46 GiB |
| Rust | 1.95.0 |
| Docker | 29.9.3 |
| Host privilege | normal user (`uid 1000`); no passwordless sudo |
| Privileged execution | Docker `--privileged`, disposable containers |
| Python for differential/docs | CPython 3.14.10 |
| Images built | `mininet-research:test`, `mnr-rootless:test` |

The host user can access Docker but cannot `sudo`. Live emulation was therefore
tested inside privileged containers, not directly on the host. This tests the
same kernel features but does not establish behaviour on a bare root host.

## System under test

| Component | Responsibility | What was exercised |
|---|---|---|
| `mn-util`, `mn-topo` | pure helpers, topology model | unit + differential tests |
| `mn-netlink` | netns, veth, addresses, routes | unit; root suite in container |
| `mn-link` | links, netem/TCLink | unit; root suite; no end-to-end tc matrix |
| `mn-node`, `mn-net` | host/switch/controller, lifecycle | unit; live `single,3` |
| `mnr` CLI + REPL | planning/start/stop/tests/shell | dry-run, live pingall, golden REPL |
| `mnexec` | namespace/exec helper | built and installed; used by live containers |
| `mnr-tui` | dry-run/live graph UI | JSON, frames, live frame in container |
| `mnr-mcp` | MCP stdio server | protocol tests; live custom graph |
| `experiment-runner` | research scenario orchestrator | all presets + error injection |
| `research-monitor` | read-only run dashboard | unit tests + PTY smoke |
| monitoring stack | Prometheus/Grafana/Loki/Alloy | compose start, health, targets, dashboard |
| containers | research and rootless images | both built from current tree |
| Python bindings | PyO3 surface | wheel built, non-root plan/dump/links smoke |
| legacy tree | Python differential reference | unit-level differential tests |

## Results

### 1. Unprivileged correctness gates

`just test` ran 43 suites (workspace plus the standalone research monitor).
Aggregated results:

```text
passed=445 failed=0 ignored=29
```

Ignored tests are root/network-namespace or live-network tests; they are not
silently skipped failures. The monitor accounts for 12 of the passing tests.
The 29 ignored tests were then executed with
`just root-tests-docker` and all passed.

`just check` (format + clippy with `-D warnings`, workspace and standalone
monitor) passed. `cargo fmt` was clean.

### 2. CLI and plan surface

`mnr --topo single,3 --dump` exited 0 without root and printed the expected
three sections:

```text
=== topo ===
spec: single,3
class: SingleSwitchTopo
nodes: h1 h2 h3 s1
...
=== links ===
h1-eth0<->s1-eth1 (MISSING MISSING)
...
```

`mnr-tui --topo tree,2,2 --frames 2` rendered headlessly; `--json` exported the
canonical graph (7 nodes, IPs, interfaces, planned status). In a privileged
container, a live `single,3` frame showed `pid: 18` and `status: up`, then
cleaned up.

### 3. Live emulation

Inside the rootless image, run as container root:

```console
$ sh scripts/mnr-container.sh --topo single,3 --controller none --test pingall
*** Results: 0% dropped (6/6 received)
*** completed in 0.228 seconds
```

The wrapper defaulted to LinuxBridge; OVS startup logs were also observed from
the image entrypoint. The run stopped hosts, switch, and controller and exited
0.

### 4. MCP / agent surface

The protocol suite covers handshake bytes, notifications, ping, all tool
schemas, topo planning, error categories, session state, custom graphs, invalid
graph preservation, unknown options, and non-root root-gate rejection.

A live privileged session used a caller-supplied graph:

- `net_build` with `{"hosts":["h1","h2"],"switches":["s1"],...}` returned
  `built: true` and class `Topo`;
- `net_start` reported host PIDs 17 and 20;
- `pingall` returned `0% dropped (2/2 received)`;
- `net_stop` and `net_release` completed.

This is the strongest end-to-end evidence that an external agent-style client
can drive a full lifecycle. It is not evidence about model quality: the
repository contains no model or prompt logic.

### 5. Research presets

All six presets ran in `mininet-research:test` with `--privileged --network none`
and a bind-mounted run directory:

| Preset | Nodes | Links | Steps | Measured wall | Result | Notable retries |
|---|---:|---:|---:|---:|---|---|
| OSPF | 2 | 1 | 5 | ~52 s | pass | adjacency 32 attempts, route 9 |
| IS-IS | 2 | 1 | 5 | ~36 s | pass | adjacency 26 attempts |
| BGP | 2 | 1 | 5 | ~5 s | pass | session 2 attempts |
| VLAN | 5 | 4 | 9 | ~6 s | pass | none |
| Multi-tenant | 12 | 11 | 30 | ~43 s | pass | none |
| Multi-tenant fault | 12 | 11 | 38 | ~50 s | pass | none |

The OSPF run is a useful calibration: 46 events, 44 step-attempt files, 444
telemetry records, and a 724 KB run directory. Retry counts are a more stable
convergence signal than wall time; wall time includes FRR startup and the
runner’s one-second retry cadence.

### 6. Failure-path and adversarial tests

| Case | Injected behaviour | Observed |
|---|---|---|
| Empty node list | invalid scenario | rejected before run directory creation |
| Duplicate node | invalid scenario | `duplicate node h1` |
| Unknown link endpoint | invalid scenario | `invalid link h1--nope` |
| Interface name overflow | 9-char host + port 100 | rejected with kernel 15-char explanation |
| Unknown top-level field | `extra` | rejected by serde with field list |
| Command exit 7 with retry | 3 attempts | each attempt recorded; run failed; cleanup reported |
| Command timeout | `sleep 5`, `timeout_secs: 1` | exit 124, one attempt, run failed |
| Missing binary | `this-command-does-not-exist` | exit 127, stderr captured, run failed |
| `expect_stdout` mismatch | `echo no`, expect `yes` | exit 0 but `success: false` after retry |
| Artifact ownership | bind mount owned by uid 1000 | artifacts owner `dzakirm:users`, mode 644/755 |

The runner stops at the first failed step and still emits a `finish` event with
the error. It does not continue to later steps, so a multi-failure scenario must
be split into separate runs. This is intentional fail-fast behaviour but should
be documented for experiment design.

### 7. Monitor and evidence capture

The monitor was started under a PTY against the OSPF run. It entered the
alternate screen and rendered the Experiment frame; the smoke window was too
short to evaluate the individual views, but the binary did not crash on a real
artifact set. Unit tests cover the harder logic: incremental append, truncation
reload, stale/disconnected states, step/attempt history, and absolute parse-error
line numbers.

### 8. Monitoring stack

`docker compose config` validates only with the required
`RESEARCH_RUNS_DIR` and `GRAFANA_ADMIN_PASSWORD`. With an ephemeral project:

- all five services reached healthy state;
- Prometheus `/-/ready` returned ready;
- `up` queries showed `prometheus`, `alloy`, and `research-host` all at `1`;
- Grafana `/api/health` returned 200;
- the provisioned **Mininet research overview** dashboard was searchable;
- Prometheus and Loki datasources were provisioned with the expected UIDs.

Alloy exposed 10 event-file series and 10 telemetry-file series; a Loki query
returned real `finish` and `step` records. The log path is verified for a short
smoke run. Rescan/duplicate semantics after an Alloy restart and long-run
retention remain untested.

### 9. Privileged Rust suites and the gate defect

The first `just root-tests-docker` run failed 11 tests across five targets. Every
failure had the same cause:

```text
failed to spawn "mnexec -cd -a ...": No such file or directory
```

The container compiled the workspace but did not install `mnexec` on `PATH`.
This is a gate defect: the tests are valid, but the recipe did not prepare the
runtime they require. The recipe now:

1. builds `mnexec` explicitly;
2. installs it to `/usr/local/bin/mnexec`;
3. runs the ignored workspace suites.

The rerun passed **29/29** ignored tests with 0 failures, including
`root_single3_pingall_end_to_end`, `root_custom_graph_live_lifecycle`,
`root_live_net_start_pingall_stop_release`, and
`live_single3_graph_reports_up`. This defect did not affect the shipped research
image, which already installs `mnexec`.

### 10. Differential ladder and benchmark smoke

A fresh privileged container ran the six-spec differential ladder with
`DIFF_MN_SWITCH=lxbr`. All six matched Python after normalization:

```text
single,3  MATCH   linear,4  MATCH   linear,10 MATCH
tree,2,2  MATCH   tree,3,2  MATCH   tree,3,3  MATCH
```

A shortened benchmark (`BENCH_RUNS=1`, three topologies, debug Rust binary,
LinuxBridge, container Python 3.12.3) completed with 0% ping loss on every
cell:

| Scenario | Nodes | Python wall | Rust wall |
|---|---:|---:|---:|
| `single,3` | 4 | 0.228 s | 0.261 s |
| `tree,2,2` | 7 | 0.400 s | 0.456 s |
| `tree,3,3` | 40 | 2.438 s | 6.331 s |

This is a **functional smoke test, not a performance claim**: one sample per
cell, debug Rust, and a containerized CPU. It does confirm that both
implementations run the same ladder and that the benchmark script propagates
successful measurements correctly.

### 11. Python bindings

`maturin develop` built the PyO3 extension for CPython 3.14. The non-root
surface exercised:

- `version()` → `2.3.1b4 (rust)`;
- `plan("single,3")` → dry-run plan text;
- `Mininet(topo="tree,2,2", controller="none")` → 4 hosts, 3 switches,
  12 link lines, expected dump and `plan_json` metadata.

The five non-root binding smoke tests passed. The runtime check found a
documentation mismatch: `plan()` was documented as JSON but returns the same
text sections as `mnr --dump`. The docs, module comment, and the misnamed
`test_plan_json` were corrected; `plan_json` remains the JSON metadata method.

## Robustness assessment

### What holds

- **Validation before side effects.** Scenario structure, names, ports,
  interface-length limits, links, and steps are checked before the run directory
  or network is created.
- **Explicit lifecycle.** Build is side-effect free; start/stop are separate;
  live failures still trigger cleanup and a `finish` event.
- **Bounded execution.** Each step is wrapped in `timeout --kill-after=2s`;
  timeouts and signals become recorded failures, not hangs.
- **Evidence is append-only and per-attempt.** Step files are created with
  no-clobber semantics; event and telemetry streams are append-only; artifacts
  inherit the parent directory owner.
- **Monitor degrades honestly.** Missing, malformed, truncated, and stale sources
  are reported; a start without finish is not claimed alive.
- **MCP is stateful and explicit.** Root-gated tools reject non-root; invalid
  builds do not replace an existing network; errors are tool results, not
  protocol corruption.
- **Containers are reproducible enough to rebuild.** Both images built from the
  current tree without host package installation.

### Limits and risks

1. **A privileged container is not a security sandbox.** Scenario commands are
   arbitrary root shell code with host-kernel reach. Use disposable hosts and
   trusted scenarios only.
2. **No embedded AI.** “AI features” are the MCP tool contract. Agent quality is
   external and unmeasured by this report.
3. **No concurrency model.** MCP is one stdio session per process; two clients
   sharing a live network would race. Document and test the single-client
   contract.
4. **No resource quotas.** CPU, memory, disk, and process limits for scenarios
   are not enforced by the runner.
5. **Retry model is fixed-interval.** One-second retries with a bounded window;
   no backoff or jitter. This is simple and reproducible but can produce many
   attempts for FRR convergence.
6. **Fail-fast steps.** A single failed step prevents later steps, so independent
   faults require separate scenarios.
7. **Telemetry can be large.** A 12-node, 38-step run produced 1,044 telemetry
   records; long holds will grow linearly. Retention and disk budgets are the
   operator’s responsibility.
8. **Monitoring soak behaviour untested.** Ingestion, labels, and queries are
   verified; Alloy restart/rescan duplicates and retention under long runs are
   not.
9. **Benchmark scope.** The fresh benchmark is a one-run debug smoke on a
   containerized host; it is not a statistically meaningful performance study.
   The six-spec differential ladder is fresh and all matched.

## Recommendations, prioritized

| Priority | Recommendation | Rationale |
|---|---|---|
| P0 | Keep the `mnexec` install step in the root gate and add a smoke assertion that `command -v mnexec` succeeds | prevents 11 false failures |
| P1 | Add a chaos suite for SIGINT/SIGTERM, daemon kill, and repeated start/stop with leak checks | highest unverified risk |
| P1 | Test Alloy restart/rescan duplicates and long-run retention | observability soak risk |
| P2 | Make step failure policy configurable (`continue_on_failure`) and document fail-fast | supports multi-fault experiments |
| P2 | Add explicit CPU/memory/disk limits or warnings for scenarios | protects the host |
| P2 | Add an MCP conformance/evaluation harness with scripted and model-driven clients | makes agent claims measurable |
| P3 | Expand benchmark ladder to 100+ nodes and publish host fingerprints | scale claims need evidence |
| P3 | Keep docs at zero Sphinx warnings and add `-W` to the local docs gate | prevents silent documentation rot |
| P3 | Add exhaustive tenant/ACL matrices and protocol churn scenarios | next research frontier |

## Reproduction commands

```bash
# Local gates
just check
just test

# Privileged root suites (builds/installs mnexec)
just root-tests-docker

# Research image and one preset
just research-build
just research ospf ospf-001

# Monitor a completed or held run
cargo run --manifest-path example/tui/Cargo.toml -- runs/ospf-001

# Monitoring stack (ephemeral)
export RESEARCH_RUNS_DIR="$PWD/runs"
export GRAFANA_ADMIN_PASSWORD='choose-a-strong-secret'
docker compose -f example/monitoring/compose.yaml up -d
curl -fsS http://127.0.0.1:9090/-/ready
curl -fsS http://127.0.0.1:3000/api/health
docker compose -f example/monitoring/compose.yaml down -v

# Docs
python3 -m venv /tmp/mnr-docs && /tmp/mnr-docs/bin/pip install -r docs/requirements.txt
/tmp/mnr-docs/bin/python -m sphinx -b html docs docs/_build/html
```

## Evidence boundaries

- Unit, protocol, and monitor results are from the host.
- Live `mnr`, TUI, MCP, and research-preset results are from privileged Docker
  containers on this host.
- The root-suite, differential, live, and benchmark results depend on the Docker
  image and kernel; a bare-metal root host may expose different namespace or OVS
  behaviour.
- The benchmark table above is a smoke result, not a performance dataset. Use
  `docs/migration/BENCHMARKS.md` for the historical dataset and rerun with
  multiple samples on the target host.
- No model/agent-quality result is claimed; only the MCP contract and lifecycle.
