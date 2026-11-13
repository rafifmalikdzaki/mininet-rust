# mininet-rust — `mnr`

[![Rust root tests](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/rust-root-tests.yaml/badge.svg)](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/rust-root-tests.yaml)
[![rust-ci](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/rust-ci.yaml/badge.svg)](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/rust-ci.yaml)
[![container-image](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/container.yaml/badge.svg)](https://github.com/rafifmalikdzaki/mininet-rust/actions/workflows/container.yaml)

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
| `mnr-tui`      | graph TUI — nodes as boxes, edges by live status; `--frames N` for headless CI renders, `--json` export |
| `mnr-mcp`      | [MCP](https://modelcontextprotocol.io) stdio server: `topo_plan`, `graph`, `net_start/stop`, `pingall`, `host_cmd`, … |
| `mnexec`       | the C mnexec helper (netns attach), built from source like upstream |

```console
$ mnr --topo single,3 --test pingall
*** Ping: testing ping reachability
h1 -> h2 h3 ...
*** Results: 0% dropped (6/6 received)
$ mnr --topo single,3 --dump          # plan-only, no root needed
$ mnr --topo single,3 --switch lxbr   # kernel LinuxBridge instead of OVS
```

## No root? Run it rootless

```console
$ sh scripts/mnr-container.sh --topo single,3 --test pingall
```

Rootless podman/docker map the container's root into a user namespace, so
`--privileged` grants `CAP_NET_ADMIN` **only there** — the host gains nothing.
Details in [DECISIONS.md D25](docs/migration/DECISIONS.md); CI smoke-tests the
same path in [`container.yaml`](.github/workflows/container.yaml).

## How it compares

| | Python mininet | mininet-rust |
|---|---|---|
| data path | shell-outs to `ip`/`tc`/`ovs-vsctl` | rtnetlink via `rtnetlink` crate — typed, no parsing |
| error surface | `Command` exit codes + parse | `Result<T, Error>` end-to-end, no unwrap in library code |
| dry-run | — | `--dump` renders the exact plan, verified byte-identical to Python |
| tests | runtime-only | 400+ unit + differential suites; every kernel path exercised in privileged CI |
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

- [Ship guide](RUST.md) · [Decision log D1–D25](docs/migration/DECISIONS.md)
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
