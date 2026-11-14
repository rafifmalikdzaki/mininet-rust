# Research program

This section turns the repository from a feature list into an evidence-producing
network research workbench. It combines a validation report, a scenario map with
increasing complexity, and a portfolio of research plans.

```{toctree}
:maxdepth: 1

TECHNICAL_REPORT
SCENARIO_MAP
TOPOLOGY_DESIGNS
REAL_SWITCHES_ROOTLESS
IMPLEMENTATION_REPORT
RESEARCH_PLANS
```

## Start here

| If you want to… | Read |
|---|---|
| Know what was tested and what remains unproven | [Technical report](TECHNICAL_REPORT.md) |
| Choose a scenario by complexity or research theme | [Scenario map](SCENARIO_MAP.md) |
| Design a complex topology, routing scheme, or tenant layout | [Topology design catalogue](TOPOLOGY_DESIGNS.md) |
| Run real OVS switches rootless and see the evidence | [Real switches and rootless](REAL_SWITCHES_ROOTLESS.md) |
| Read the consolidated implementation record | [Implementation report](IMPLEMENTATION_REPORT.md) |
| Plan the next experiment or a longer programme | [Research plans](RESEARCH_PLANS.md) |
| Run the workbench | `example/README.md` and `example/tui/README.md` |
| Understand the MCP agent surface | [MCP documentation](../mcp.rst) |
| See the next implementation sequence | `ROADMAP.md` at the repository root |

## Evidence at a glance

- 445 unprivileged tests passed, 0 failed; 29 ignored root tests passed in the
  privileged Docker gate after the `mnexec` installation fix (29/29).
- 6/6 differential topology specs matched Python in a fresh privileged run.
- 13 MCP protocol tests passed; a live custom-graph MCP session built, started,
  pinged at 0% loss, stopped, and released.
- The Python binding wheel built and its 5/5 non-root smoke tests passed
  (plan/dump/links/plan_json).
- 6/6 research presets passed: OSPF, IS-IS, BGP, VLAN, multi-tenant, and
  multi-tenant trunk fault.
- Malformed scenarios, nonzero exits, timeouts, missing binaries, and
  `expect_stdout` mismatches fail closed with artifacts and cleanup.
- Real OVS switches, rootless Podman, OVS VLAN/multi-tenant/fault, routed OVS,
  active-backup bonding, and leaf-spine STP are validated; see the
  real-switch report.
- The monitoring stack starts healthy with all Prometheus targets up, a
  provisioned Grafana dashboard, and verified Alloy→Loki event/telemetry ingestion.

## The short version of the research agenda

1. **Make evidence reproducible** — fuzz the differential surface, pin the
   environment, and validate artifacts.
2. **Know the datapath and control plane** — measure switch/link semantics and
   protocol convergence instead of assuming them.
3. **Move from connectivity to operations** — isolation matrices, fault
   injection, cleanup invariants, and cost.
4. **Treat the agent surface as an interface under test** — tool success,
   recovery, safety, and privilege boundaries.
