# RUST.md — the Rust port of Mininet

This repo carries a Rust re-implementation of Python Mininet 2.3.1b4 next to
the original Python tree, now grouped under `legacy/`. The Python code in
`legacy/mininet/` and `legacy/bin/mn` is **untouched** and serves as the
differential-testing reference; the old C `mnexec.c` helper is deleted —
`mnexec` is a pure-Rust binary now. The Rust code lives in a Cargo workspace
(`crates/`, `apps/`) and is built into `mnr`, `mnr-tui`, `mnr-mcp`, `mnexec`.

Migration plan and task list: `tasks/plan.md`. Deep dives:
`docs/migration/ARCHITECTURE.md` (component inventory + dependency graph),
`docs/migration/TESTING.md` (test tiers), `docs/migration/DECISIONS.md`
(decision log D1–D17), `docs/migration/PROGRESS.md` (ledger),
`docs/migration/BENCHMARKS.md` (perf harness).

## Build

Requires a recent stable Rust toolchain (`rust-toolchain.toml` pins
`stable`; `Cargo.lock` is committed):

```bash
cargo build --release          # workspace -> target/release/mnr, target/release/mnexec
cargo build --workspace        # debug build
```

## Run

`mn` needs **root** to create network namespaces/veth pairs (same as Python
`mn`):

```bash
sudo target/release/mnrr --topo single,3                 # build + start + interactive CLI
sudo target/release/mnrr --topo linear,4 --test pingall  # build + built-in test + stop
sudo target/release/mnrr --clean                         # clean up leftover netns/links
```

Without root, the parse → build → plan path works as a **dry run** via the
harness-only `--dump` flag (see D14 in `docs/migration/DECISIONS.md`):

```bash
target/release/mnr --topo single,3 --dump    # prints === topo === / === dump === / === links ===, exit 0, no root
```

`--dump` is exactly what `scripts/diff_topology.sh` probes to diff the Rust
side against the Python reference. With no `--test`, `mn` enters
the interactive CLI when stdin is a tty (also with `--xterms`, like bin/mn),
otherwise dumps and stops.

## Architecture map

Workspace members and dependency direction (no cycles; arrows point from
dependent to dependency):

```
apps/mn        (mn)        bin/mn CLI + cli.py REPL        <- mn-net, mn-topo, mn-util, clap
apps/mnexec    (mnexec)    netns-attach helper (Rust)     <- nix, libc
crates/net     (mn-net)    mininet/net.py orchestrator    <- mn-node, mn-link, mn-netlink, mn-topo, mn-util
crates/node    (mn-node)   mininet/node.py + nodelib      <- mn-link, mn-netlink, mn-topo, mn-util, nix
crates/link    (mn-link)   mininet/link.py (Intf/Link/TCLink) <- mn-netlink, mn-util, rtnetlink
crates/netlink (mn-netlink) rtnetlink wrappers (netns/veth/addr/route) <- rtnetlink, nix, tokio
crates/topo    (mn-topo)   mininet/topo.py + topolib.py   <- mn-util
crates/util    (mn-util)   mininet/util.py pure helpers   <- (none)
```

Kernel-facing rule (D2/D15): **all** namespace/veth/address/route/qdisc/
bridge operations go through rtnetlink (`crates/netlink`, `crates/link`) —
never `ip`/`tc`/`ifconfig` shell-outs. The only retained subprocesses are
`iptables` (NAT), `sysctl`, and `ovs-vsctl`/`ovs-ofctl` (OVS/OpenFlow control
plane, too large to port now).

Lifecycle (D16): model construction (`Topo::build`, `Mininet::buildFromTopo`)
is pure; all kernel side effects happen in the explicit `start()`; `stop()`
is the exact reverse of `start()` with full cleanup on error paths.

## Testing tiers

See `docs/migration/TESTING.md` for the full strategy.

**Unprivileged tier** (dev sandbox, normal CI — `.github/workflows/rust-ci.yaml`):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # unit + python3-driven differential tests (no root)
cargo build --workspace --release
```

**Root-gated tier** (privileged host or CI — `.github/workflows/rust-root-tests.yaml`,
a `container: ubuntu:24.04` job with `options: --privileged`):

```bash
MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1   # netlink/netns integration tests
sudo sh scripts/run_root_tests.sh   # the above + scripts/diff_topology.sh single,3 linear,4 tree,2,2
```

Root-gated tests are `#[ignore]` **and** check `MN_RUST_ROOT_TESTS=1`, so
plain `cargo test --workspace` stays green everywhere; `--test-threads=1`
because some tests fork children that change namespaces.
`scripts/diff_topology.sh <spec>` builds the same topology in Python mininet
and Rust `mn` and diffs the normalized `dump`/`links`/`pingall` output
(MATCH/MISMATCH; needs root on the Python side, `PYTHONPATH=.`).

## Current status vs Python mininet

Works today:

- `mn-util`, `mn-topo` — full parity with `mininet/util.py` / `topo.py` /
  `topolib.py` (differential-tested against the Python original on
  `fixtures/`).
- `mn-netlink` — netns create/enter/delete, veth pairs, addresses, routes
  via rtnetlink; `mnexec` replacement with identical CLI.
- `mn-link` — `Link`/`TCLink` model; netem qdiscs (delay/loss/jitter, and
  `bw` via netem rate — see D3) built as raw netlink.
- `mn-node`, `mn-net` — Node/Host/Switch/Controller model + `Mininet`
  orchestrator with a pure build and explicit `start()`/`stop()`; dry-run
  (`--dump`) works non-root.
- `mnr` CLI — full `bin/mn` option surface (clap; golden help + differential
  option-set tests), `--topo/--test/--clean/--dump`, and the interactive
  REPL command set (`nodes links dump net ports intfs hosts switches
  controllers ping pingall pingallfull pingpair pingpairfull iperf iperfudp
  dpctl sh xterm gterm x link switch wait time noecho py px source help exit`).

TODO / out of v1 scope (D17):

- **NOX/Ryu controllers** — stubs that return a marked TODO error (D17).
- **HTB bw shaping via netlink** — `bw` currently maps to netem rate; the HTB
  root+class path is a marked TODO pending privileged verification (D3).
- **miniedit GUI, cluster, mobility** — not ported. xterm consoles ARE ported
  (`term.rs`, `--xterms` spawns real xterms on a root-started net).
- `--innamespace` (control network) and `--custom` (exec Python files) are
  marked TODOs in `mn-net` / `apps/mn`.

## CI

- `.github/workflows/rust-ci.yaml` — fmt/clippy/test/release build on push/PR
  to `rust-migration` and `master` (unprivileged).
- `.github/workflows/rust-root-tests.yaml` — privileged container job: builds
  the workspace, runs `MN_RUST_ROOT_TESTS=1 cargo test --workspace --
  --ignored --test-threads=1`, then `sh scripts/run_root_tests.sh` (which
  also runs the Python-vs-Rust `diff_topology.sh` harness for `single,3
  linear,4 tree,2,2`). `allow-failure: false`.

## Python bindings & examples

The Rust core is also exposed to Python as `mininet_rust` (PyO3/maturin,
`bindings/python`): `maturin develop -m bindings/python/Cargo.toml`, then
`import mininet_rust as mnr; net = mnr.Mininet(topo="tree,2,2")`. Rust-backed
examples in `examples/rust/` — including `external_nat.py` (external
connections) and `cybersec_testbed.py` (cybersecurity testbed, `--plan` for a
no-root preview).
