# mininet-rust — `mnr`

A from-scratch **Rust** re-implementation of [Mininet](https://mininet.org) —
the network emulator — following containerlab's core lesson: **talk netlink,
never shell out**. One binary, `mnr`, plus an OMO-style graph TUI (`mnr-tui`)
and an [MCP](https://modelcontextprotocol.io/) server (`mnr-mcp`) so AI agents
can build and drive networks natively.

```console
$ target/release/mnr --topo single,3 --test pingall     # build + emulate
*** Ping: testing ping reachability
h1 -> h2 h3
h2 -> h1 h3
h3 -> h1 h2
*** Results: 0% dropped (6/6 received)
```

---

## Screenshots

### `mn-tui` — live graph, pan/zoom/select

```text
 mn-tui — topo tree,2,2 (dry-run plan)
                                          ┌───────────────────────────────┐
              ┌───────────────────────────┤            s1                 │
              │                           │  switch, stp=off prio=32768   │
              │  k = kind   tab = cycle   └───────┬───────────┬───────────┘
              │  +/− or </> = zoom              / │ \         │
        ┌─────┴─────┐                 ┌────────┴┐  ┌───────┴───┐  ┌────┴───────┐
        │ h1        │             ┌───┤ h2      │  │      h3   │  │ ...
        │ 10.0.0.1  │             │   │10.0.0.2 │  │10.0.0.3   │
        └───────────┘             └───┴─────────┘  └───────────┘
 frame 1/2 (headless, plan mode)
 q quit | arrows/hjkl pan | +/- or </> zoom | tab cycle | enter detail | r refresh | p pingall
```

*(exact frames: [`docs/images/tui_single3.txt`](docs/images/tui_single3.txt),
[`docs/images/tui_tree22.txt`](docs/images/tui_tree22.txt) — rendered by the
same `render_frame` code the interactive TUI uses; `just container-run` for a
live run with real pids/status.)*

## What you get

| binary         | what it does |
|----------------|--------------|
| `mnr`          | the CLI: `--topo <spec> [--switch <kind>] [--controller none] [--test pingall] [--dump]` |
| `mnr-tui`      | graph TUI — nodes as boxes, edges by live status; `--frames N` for headless renders, `--json` export |
| `mnr-mcp`      | [MCP](https://modelcontextprotocol.io) stdio server: `topo_plan`, `graph`, `net_start/stop`, `pingall`, `host_cmd`, … |
| `mnexec`       | netns-attach helper, ported to Rust (`apps/mnexec`) — no C anywhere |

```console
$ mnr --topo single,3 --test pingall
*** Ping: testing ping reachability
h1 -> h2 h3 ...
*** Results: 0% dropped (6/6 received)
$ mnr --topo single,3 --dump          # plan-only, no root needed
$ mnr --topo single,3 --switch lxbr   # kernel LinuxBridge instead of OVS
```

## Research experiments

[`example/`](example/README.md) provides OSPF, IS-IS, BGP, VLAN and
multi-tenant presets using FRRouting and the Rust network core. Experiments
retain commands, protocol state and per-node telemetry. The
[research monitor](example/tui/README.md) displays live run state in a
terminal; an optional [monitoring stack](example/monitoring/README.md) stores
host metrics and run events.

The presets run as root in a privileged container. Use trusted scenarios on a
disposable host or VM; a privileged container is not a security sandbox.

## No root? Run it rootless

```console
$ sh scripts/mnr-container.sh --topo single,3 --test pingall
```

Rootless podman/docker can map container root into a user namespace. Network
namespace support depends on the runtime and host; privileged containers are
not a security boundary for untrusted code. See [DECISIONS.md D25](docs/migration/DECISIONS.md).

## How it compares

| | Python mininet | mininet-rust |
|---|---|---|
| data path | shell-outs to `ip`/`tc`/`ovs-vsctl` | rtnetlink via `rtnetlink` crate — typed, no parsing |
| error surface | `Command` exit codes + parse | `Result<T, Error>` end-to-end, no unwrap in library code |
| dry-run | — | `--dump` renders the exact plan, verified byte-identical to Python |
| tests | runtime-only | unit + differential suites, with optional privileged integration tests |
| `Tc` qdiscs | `tc` shell-outs | raw netlink `TCA_OPTIONS` blobs, iproute2-compatible |

## Screenshots — CLI surface

```text
$ mnr --help
Usage: mnr [OPTIONS]

Options:
      --topo <SPEC>            single,k | linear,k | tree,d,f | minimal
      --switch <KIND[,k=v]>    default|ovs|ovsbr|user|ivs|lxbr
      --controller <SPEC>      none | ref | remote,IP | ovsc ...
      --test <NAME>            pingall | pingpair | iperf | none | build
      --dump                   print the plan and exit (no root needed)
  -h, --help                   Print help
```

## Docs

- [Ship guide](RUST.md) · [Decision log D1–D26](docs/migration/DECISIONS.md)
- [Technical validation report](docs/research/TECHNICAL_REPORT.md) — evidence, failure paths, limits
- [Scenario complexity map](docs/research/SCENARIO_MAP.md) — plan → connectivity → protocols → faults → scale → agents
- [Topology design catalogue](docs/research/TOPOLOGY_DESIGNS.md) — 12 designs: Clos, hierarchy, VRF/VXLAN tenancy, bonding, rings, hub-spoke, torus, service chains, IPv6, chaos
- [Research plans](docs/research/RESEARCH_PLANS.md) — ten programmes across all features
- [Benchmarks](docs/migration/BENCHMARKS.md) (python-vs-rust ladder with results)
- [Review verdict](docs/migration/REVIEW.md) (five-axis)
- TUI deep-dive: [docs/tui.rst](docs/tui.rst); MCP: [docs/mcp.rst](docs/mcp.rst)

## Install

```console
$ cargo build --workspace --release    # then use target/release/{mnr,mnr-tui,mnr-mcp,mnexec}
$ cargo test --workspace               # unit gates (root-gated suites are #[ignore])
```

or fully inside a container, no sudo at all:

```console
$ just container-build && just container-run --topo single,3 --test pingall
```

## Credits & license

An idea inherited from [mininet/mininet](https://github.com/mininet/mininet)
(BSD-3); this port is BSD-2-Clause. No affiliation with the upstream project.
