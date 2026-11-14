# TESTING.md — Test strategy and how to run it (Task 11, `tasks/plan.md`)

How to verify the Rust migration of Mininet at each tier of privilege. There
are **two tiers**: unprivileged checks and **root-gated** suites that need a privileged Linux environment
(netlink/netns integration tests and the Python-vs-Rust differential topology
harness in `scripts/`).

Scope note: this document and the scripts it describes live in `scripts/**`
and `docs/migration/**` only; the Python tree stays untouched as the
differential reference.

## 1. Unprivileged tier

No root or network namespaces are needed for the local Rust checks:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # pure-logic unit tests + Python-driven differential tests
cargo build --workspace --release
```

The **differential tests against Python for pure logic** are Rust integration
tests that shell out to `python3` against the untouched reference tree:

- `crates/util/tests/differential.rs` (+ `py_driver.py`) — `mininet/util.py`
  helpers vs `crates/util` on generated inputs (`fixtures/util_cases.txt`).
- `crates/topo/tests/differential.rs` (+ `py_driver.py`) — `mininet/topo.py` +
  `topolib.py` vs `crates/topo` on the fixture specs (`fixtures/topo_*.txt`).

They run as part of plain `cargo test --workspace` (no root) as long as
`python3` is on `PATH`; the tests locate the repo root themselves via
`CARGO_MANIFEST_DIR` (and set it as the child's working directory), so the
working directory you run cargo from does not matter.

Script-level smoke checks (also fine unprivileged):

```bash
bash -n scripts/diff_topology.sh scripts/run_root_tests.sh   # syntax
# The differential harness requires a privileged Linux environment.
scripts/run_root_tests.sh
# -> prints the root-required error and exits 3 when not run as root
```

## 2. Root-gated tier (privileged Linux)

Some verification is impossible without `CAP_NET_ADMIN` + netns, so it is
explicitly gated and must run on a privileged host:

| Suite | Gate | Command |
|---|---|---|
| netlink/netns integration tests (`crates/netlink`, `crates/link`, …) | `#[ignore]` **and** `MN_RUST_ROOT_TESTS=1` env var; serialized | `MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1` |
| differential topology harness (Python `mn` vs Rust `mn`) | root (both sides create netns/veths) | `scripts/diff_topology.sh <spec>` |

The root-gated cargo tests self-skip unless `MN_RUST_ROOT_TESTS=1` is set, so
`cargo test --workspace` stays green unprivileged and `--ignored` alone is not
enough — always set the env var on the privileged host.

### 2.1 One-shot runner

```bash
sudo scripts/run_root_tests.sh
```

runs the root-gated cargo suite **and** `scripts/diff_topology.sh` for
six topology specifications (override with `DIFF_TOPOS="..."`),
then prints a summary table. Exit codes: `0` = no
failures (SKIPs allowed), `1` = at least one FAIL/MISMATCH/ERROR, `3` = not
root / harness error (nothing is executed when not root).

### 2.2 Running the pieces individually

```bash
# root-gated netlink/netns tests only
sudo env MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1

# differential harness for one topo spec (must be root)
sudo scripts/diff_topology.sh single,3
sudo scripts/diff_topology.sh linear,4
sudo scripts/diff_topology.sh tree,2,2
```

`diff_topology.sh` exit codes:

| Exit | Meaning |
|---:|---|
| 0 | **MATCH** — normalized Python and Rust dumps are identical |
| 1 | **MISMATCH** — normalized dumps differ (unified diff printed to stderr) |
| 2 | **SKIP** — Rust `mn` emulation not wired yet (`--dump` unsupported); not a failure |
| 3 | **ERROR** — usage/environment/harness failure (e.g. not root when the Rust side is wired) |

### 2.3 What the differential harness compares

For a topo spec (e.g. `single,3`) the script runs both sides with the same
class flags (`DIFF_MN_SWITCH`/`DIFF_MN_HOST`/`DIFF_MN_CONTROLLER`/`DIFF_MN_LINK`/
`DIFF_MN_IPBASE`, defaults `default`/`proc`/`none`/`default`/`10.0.0.0/8`):

- **Python reference** — an embedded driver equivalent to
  `python3 bin/mn --topo <spec> --controller none --test none`
  (`buildTopo` + `Mininet` + `net.start()` + `dump` + `links` + `pingall` +
  `net.stop()`), stdout captured. The script sets `PYTHONPATH` to the
  legacy reference tree; the usual Mininet system dependencies are needed
  (`legacy/util/install.sh`; the default switch `OVSSwitch` needs `ovs-vsctl` — set
  e.g. `DIFF_MN_SWITCH=lxbr` to use `LinuxBridge`/`brctl` instead).
- **Rust port** — `<mn binary> --topo <spec> ... --dump`
  (`target/release/mnr` or `target/debug/mnr`, else `cargo build -p mnr`; override
  with `MN_RUST_BIN`). Equivalent to `cargo run -p mnr -- --topo <spec> --dump`.

Both outputs are normalized before diffing — `pid=<n>` → `pid=<PID>`, ISO
timestamps stripped, `completed in …` lines dropped, trailing whitespace
stripped, and list-like lines (`nodes:`, `hosts:`, `switches:`, `links:`, ping
`h1 -> h2 h3` destination lists) field-sorted.

**Output contract for the Rust side** (what `--dump` must print so the diff is
meaningful) — same four sections the Python driver emits:

```
=== topo ===
spec: single,3
class: SingleSwitchTopo
nodes: h1 h2 h3 s1
hosts: h1 h2 h3
switches: s1
links: h1-eth0<->s1-eth1 h2-eth0<->s1-eth2 h3-eth0<->s1-eth3
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=12345>        (pid is normalized away)
...
=== links ===
h1-eth0<->s1-eth1 (OK OK)
...
=== pingall ===
*** Ping: testing ping reachability
h1 -> h2 h3
...
*** Results: 0% dropped (6/6 received)
```


## 3. Isolation note

Run kernel-facing experiments in a disposable Linux environment with the
necessary privileges. Network namespaces and privileged containers are not
security boundaries for untrusted code. Keep container and host Cargo target
directories separate because test binaries embed their build-time source paths.
