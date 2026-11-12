# Mininet: Rapid Prototyping for Software Defined Networks

*The best way to emulate almost any network on your laptop.*

[![Python tests](https://github.com/mininet/mininet/workflows/mininet-tests/badge.svg)](https://github.com/mininet/mininet/actions)
[![Rust CI](https://github.com/mininet/mininet/actions/workflows/rust-ci.yaml/badge.svg)](https://github.com/mininet/mininet/actions/workflows/rust-ci.yaml)
[![Rust root tests](https://github.com/mininet/mininet/actions/workflows/rust-root-tests.yaml/badge.svg)](https://github.com/mininet/mininet/actions/workflows/rust-root-tests.yaml)

Mininet 2.3.1b4 — original Python implementation, plus a Rust port on branch
[`rust-migration`](RUST.md) (installed as **`mnr`** so it never clashes with
an installed Python `mn`).

## What is Mininet?

Mininet emulates a complete network of hosts, links, and switches on a single
machine:

```console
$ sudo mn                 # Python CLI
$ sudo mnr                # Rust CLI (branch rust-migration)
```

Hosts are `bash` processes in their own Linux network namespaces; links are
veth pairs; switches are software switches (Open vSwitch, or a kernel bridge
with `--switch lxbr`). OpenFlow controllers prototyped in Mininet transfer to
hardware with minimal changes.

## Quick start

```console
$ sudo mn --topo single,3                  # 3 hosts, 1 switch
mininet> nodes
mininet> pingall
mininet> exit
$ sudo mn --topo tree,depth=2,fanout=3     # parametrized tree
$ sudo mn --topo linear,3 --link tc,bw=10,delay=5ms,loss=0.1
$ sudo mn -c                               # clean up leftovers
```

The interactive CLI (`mininet>`) supports `nodes`, `links`, `dump`, `pingall`,
`iperf`, `sh`, `xterm`, `py`, and per-node passthrough (`h1 ifconfig -a`).

## The Rust port (`mnr`) — branch `rust-migration`

A from-scratch Rust re-implementation following
[containerlab](https://containerlab.dev/)'s core lesson: **talk to the kernel
via netlink, never shell out to `ip`/`tc`**.

```console
$ cargo build --workspace --release
$ sudo target/release/mnr --topo tree,2,2     # real emulation (needs root)
$ target/release/mnr --topo single,3 --dump    # dry-run plan, no root needed
$ target/release/mn-tui --topo tree,2,2       # OMO-style live graph TUI
$ target/release/mn-mcp                        # MCP server for AI agents
```

Highlights:

- **Byte-exact parity, proven by differential tests** — the Rust suites drive
  the original Python code and assert identical outputs (5 536 util cases,
  52 topo cases, 11 net-plan specs, golden CLI sessions); root-gated suites
  diff `mnr --dump` against Python `mn` on a complexity ladder.
- **`mn-tui`** — live topology graph TUI (OMO-style: nodes as boxes, edges by
  link status, pan/zoom/select), headless `--frames`/`--json` for CI.
- **`mn-mcp`** — [MCP](https://modelcontextprotocol.io/) server exposing
  `topo_plan`, `graph`, `net_start/stop`, `pingall`, `host_cmd`, … to any
  AI tool: `vibe mcp add mn-mcp --transport stdio --command "$PWD/target/release/mn-mcp"`.
- Full decision log, architecture map, and progress ledger in
  [`docs/migration/`](docs/migration/PROGRESS.md).

See [RUST.md](RUST.md) for build/run details and the ported-component table.

## Installation (Python)

See [INSTALL](INSTALL). Python 3 and Python 2 are both supported by 2.3.1b4.

## Documentation

- **Sphinx docs (this branch):** `pip install -r docs/requirements.txt && make -C docs html`
  → `docs/_build/html/index.html` (quickstart, Rust guide, TUI, MCP,
  architecture, testing, migration ledger).
- **Python API:** `help()` docstrings, or `make doc` (Doxygen).
- Web walkthrough and FAQ: <http://mininet.org>, <http://faq.mininet.org>.

## Testing

```console
$ cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace   # Rust, any machine
$ sudo sh scripts/run_root_tests.sh     # root-gated netns tests + differential ladder + benchmarks
```

CI: Python tests (`run-tests.yaml`, `code-check.yaml`), Rust gates
(`rust-ci.yaml`), privileged root/differential/benchmark job
(`rust-root-tests.yaml`).

## Support & contributing

Community-supported — join `mininet-discuss`:
<https://mailman.stanford.edu/mailman/listinfo/mininet-discuss>

Hosted at <https://github.com/mininet>. Bug reports, fixes, and PRs welcome
(see [CONTRIBUTORS](CONTRIBUTORS)).

*Bob Lantz, on behalf of the Mininet Contributors*
