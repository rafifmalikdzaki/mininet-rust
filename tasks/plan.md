# Implementation Plan: mininet-rust — Rust migration of Python Mininet 2.3.1b4

## Overview

Migrate the Python Mininet codebase in this repo (~16.8k LOC across `mininet/*.py`,
plus `mnexec.c` and `bin/mn`) to an idiomatic Rust implementation, following the
phased playbook (discovery → baseline → architecture → foundation → incremental
migration → integration → hardening). The Python tree is **retained untouched** as
the reference implementation for differential testing; nothing is deleted until
the Rust replacement passes its gates.

Reference architecture: [containerlab](https://containerlab.dev/) — a single static
binary that talks to the kernel via netlink instead of shelling out to `ip`/`tc`,
with a declarative topology description. We adopt the same core lessons: use the
`rtnetlink` crate for namespace/veth/qdisc operations, ship one binary, keep a
declarative topo format.

## Architecture Decisions

- **Strategy: incremental module migration (strangler), not a big-bang rewrite.**
  The system is a mature, root-privileged, kernel-coupled tool; a full rewrite is
  the highest-risk option. Rust modules land behind stable interfaces while Python
  remains runnable.
- **Workspace layout** (new, additive — does not disturb the Python package):
  `Cargo.toml` (workspace) with `crates/` (`topo`, `util`, `netlink`/`link`,
  `node`, `net`, `cli`) and `apps/mn` (binary). Pure logic (topo, util, parsing)
  is ported first because it is verifiable without root.
- **Replace shell-outs with netlink.** Python shells out to `ip`, `tc`, `ifconfig`,
  `ovs-vsctl` (~75 subprocess call sites, 53 in `node.py` alone). Rust uses
  `rtnetlink`/`netlink-packet-*` for veth pairs, addresses, routes, and TC qdiscs;
  OVS control stays behind `ovs-vsctl` initially (FFI surface too large to port now).
- **Replace `mnexec.c`** with a small Rust helper using `setns()`/`unshare()` via
  the `nix` crate (same job: enter a pid's net+mount namespace, exec a command).
- **Behavioral contract first.** `topo.py`'s `MultiGraph`/`Topo`, `util.py`
  helpers, and `bin/mn` CLI parsing get characterization tests captured from the
  Python original before any Rust port of the same logic.
- **Environment constraint (recorded assumption):** this sandbox is non-root with
  no netns support, so namespace/link integration tests cannot run here. Gates
  that run locally: `cargo fmt/check/clippy/test` for pure crates, CLI dry-run,
  differential tests against Python for pure logic. Netns-dependent verification
  is marked and must run on a root Linux host/CI.

## Task List

### Phase 0 — Discovery & baseline (mostly done, needs recording)
- [x] Task 1: Write `docs/migration/ARCHITECTURE.md` — component inventory of
  `mininet/*.py`, `mnexec.c`, `bin/mn`, test suite, CI; dependency graph
  (topo ← net ← node/link ← cli). *(discovery, S)*
- [x] Task 2: Capture behavioral baseline: run recordable Python tests
  (`mininet/test/test_util.py`, topo construction snippets) and save outputs as
  golden fixtures under `fixtures/`. *(verification, S)*

### Phase 1 — Rust foundation
- [x] Task 3: Scaffold Cargo workspace (`Cargo.toml`, `rust-toolchain.toml`,
  crate skeletons, `apps/mn` stub that prints usage), plus CI workflow
  (fmt/check/clippy/test on the workspace). *(implementation, M)*
- [x] Task 4: Port `mininet/util.py` pure helpers (irange, natural sort, MAC/IP
  parsing, version parsing) to `crates/util` with unit tests + differential test
  vs Python on generated inputs. *(implementation, M)*
- [x] Task 5: Port `mininet/topo.py` (`MultiGraph`, `Topo`, built-in topologies
  from `topolib.py`) to `crates/topo` with characterization tests from Python
  outputs (node/edge counts, port numbering, `build()` results). *(implementation, M)*

### Checkpoint: Foundation
- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` all pass
- [ ] Differential tests show topo/util parity with Python on fixtures
- [ ] Python tree untouched (`git status` clean outside `tasks/`, `crates/`, `docs/`)

### Phase 2 — Kernel-facing core (netlink)
- [x] Task 6: `crates/netlink`: namespace create/enter/delete, veth pair
  creation, interface up/down, address assignment via rtnetlink; `mnexec`
  replacement binary using `nix::sched::setns` + `execvp`. Unit-testable parts
  only in sandbox; netns paths behind a `root` integration test. *(implementation, L → split: 6a netlink wrapper, 6b mnexec replacement)*
- [x] Task 7: Port `mininet/link.py` link model (`Intf`, `Link`, `TCLink` params:
  bw/delay/loss/jitter, latency via netem) on top of `crates/netlink`.
  *(implementation, M)*
- [~] Task 8: Port `mininet/node.py` `Node`/`Host` core: namespace-backed
  process spawning, cmd execution, interface config — minus OVS specifics.
  *(implementation, L → split: 8a Node/Host, 8b Switch/Controller shells)*

### Phase 3 — Orchestration & CLI
- [x] Task 9: Port `mininet/net.py` `Mininet` orchestrator (addHost/addSwitch/
  addLink/start/stop, config ordering) on `crates/topo` + node/link crates,
  with a `--dry-run`/print mode testable without root. *(implementation, L)*
- [ ] Task 10 — CLI, in four verified increments (10a done: clap arg parsing):
  - **10a (done)** `bin/mn` optparse surface → clap in `apps/mn`, golden help +
    differential option-set vs `python3 bin/mn --help`. Commit `773e78e`.
  - **10b — wire `mn` to `mn-net`**: `mn --topo single,3` builds the net via
    mn-net and runs it; `--dump` prints the planned/actual topology in the exact
    Python `dump`/`links` format (this is what `scripts/diff_topology.sh`
    probes); `--test pingall|iperf|none` runs the built-in tests. Root-gated
    end-to-end, but the parse→build→dump path must work non-root (dry-run).
  - **10c — interactive loop**: port `mininet/cli.py` `CLI` class command set in
    priority order: `nodes`, `links`, `dump`, `net`, `ports`, `hosts`,
    `switches`, `controllers`, `ping <a> <b>`/`pingall`/`pingallfull`,
    `iperf`, `dpctl`, `route`, `xterm`, `exit/quit`, `help`, `source` — each as
    a small handler fn in `apps/mn/src/repl.rs`, one commit per 2-3 commands,
    golden-output tests per command against captured Python CLI sessions.
  - **10d — parity polish**: `sh <cmd>` passthrough, `intfs`, `py` escape (exec
    python snippet with net in scope — optional, can defer), error messages
    matching Python CLI. Deferred/out-of-v1: miniedit GUI, consoles, cluster,
    mobility (recorded in DECISIONS.md).
- [~] Task 11: Differential harness: script that builds identical small topos
  (single, linear, tree) in Python mininet and Rust mn on a root host and
  compares `dump`/`pingall`/`links` output. *(verification, M — runs in CI/root only; scripts landed in `a5bbd12`, execution pending a privileged host)*

### Phase 4 — Hardening & docs
- [ ] Task 12: CI gates for Rust (fmt/clippy/test/nextest, release build),
  `docs/migration/PROGRESS.md` ledger, `README` section with build/run
  instructions, decision log (`DECISIONS.md`: why netlink over shell-outs, why
  strangler over rewrite, what stays Python). *(delivery, S)*

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Root/netns unavailable in dev sandbox | High — link/node/net can't be integration-tested locally | Split crates so pure logic is testable; mark netns tests `#[ignore]`-by-default with `MN_RUST_ROOT_TESTS=1`; run them in CI on a privileged runner |
| Behavioral drift vs Python mininet | High — silent incompatibility | Differential harness (Task 11) + golden fixtures (Task 2); no legacy deletion until parity |
| OVS/OpenFlow control surface too large | Med — scope creep | Keep `ovs-vsctl`/`ovs-ofctl` as subprocess calls initially; document as retained component |
| CLI feature parity (miniedit, cluster, mobility) | Med — examples break | Port core CLI first; examples ported last, individually, as separate tasks |
| Python 2 compat paths in tests/CI confuse baseline | Low | Baseline captured with Python 3 only; note in BASELINE.md |

## Open Questions
- Target scope: full parity with `bin/mn` + `cli.py`, or a containerlab-style
  reduced CLI (create/destroy/inspect) as v1? (Recommend reduced CLI + core API.)
- Keep `examples/` working against the Rust binary, or treat them as out of
  scope for v1?
- Licensing/headers for ported logic (GPL/BSD original) — confirm before publishing.
