# TESTING.md — Test strategy and how to run it (Task 11, `tasks/plan.md`)

How to verify the Rust migration of Mininet at each tier of privilege. There
are **two tiers**: everything that runs unprivileged (this dev sandbox, normal
CI), and the **root-gated** suites that need a privileged Linux host or a
privileged CI job (netlink/netns integration tests + the Python-vs-Rust
differential topology harness in `scripts/`).

Scope note: this document and the scripts it describes live in `scripts/**`
and `docs/migration/**` only; the Python tree stays untouched as the
differential reference.

## 1. Unprivileged tier (dev sandbox, normal CI)

No root, no netns needed. These are the gates wired into
`.github/workflows/rust-ci.yaml`:

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
scripts/diff_topology.sh single,3
# -> prints SKIP (Rust mn emulation not wired yet) and exits 2 — expected
#    until apps/mn implements --topo/--dump; NOT a failure
scripts/run_root_tests.sh
# -> prints the root-required error and exits 3 without running anything
```

## 2. Root-gated tier (privileged Linux host / privileged CI)

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

runs the root-gated cargo suite **and** `scripts/diff_topology.sh` for the
three fixture specs (`single,3 linear,4 tree,2,2`, override with
`DIFF_TOPOS="..."`), then prints a summary table. Exit codes: `0` = no
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
  `net.stop()`), stdout captured. Requires the repo root on `PYTHONPATH` (the
  script sets it) and the usual Mininet system deps on the host
  (`util/install.sh`; the default switch `OVSSwitch` needs `ovs-vsctl` — set
  e.g. `DIFF_MN_SWITCH=lxbr` to use `LinuxBridge`/`brctl` instead).
- **Rust port** — `<mn binary> --topo <spec> ... --dump`
  (`target/release/mn` or `target/debug/mn`, else `cargo build -p mn`; override
  with `MN_RUST_BIN`). Equivalent to `cargo run -p mn -- --topo <spec> --dump`.

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

### 2.4 Privileged CI (GitHub Actions)

Option A — a `container:` job with `--privileged` (container jobs run as root):

```yaml
  root-tests:
    name: Root-gated tests (privileged container)
    runs-on: ubuntu-latest
    container:
      image: ubuntu:24.04
      options: --privileged          # needs netns + CAP_NET_ADMIN
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Python + mininet deps
        run: |
          apt-get update
          apt-get install -y --no-install-recommends python3 iproute2 \
            bridge-utils ethtool openvswitch-switch
      - name: Root-gated netlink tests + differential harness
        run: ./scripts/run_root_tests.sh
```

Option B — a hosted `ubuntu-latest` runner (not root, but passwordless sudo;
`privileged: true` is not available on hosted VM runners, so use `sudo`):

```yaml
  root-tests:
    name: Root-gated tests (root runner via sudo)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Python + mininet deps
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends python3 iproute2 \
            bridge-utils ethtool openvswitch-switch
      - name: Root-gated netlink tests + differential harness
        run: sudo ./scripts/run_root_tests.sh
```

Optionally gate the job on the workspace building first
(`needs: rust` where `rust` is the existing `rust-ci` job).

## 3. Sandbox note

This dev sandbox is **non-root with no netns**, so the root-gated suites cannot
run here; they are written and smoke-tested only:

```bash
bash -n scripts/diff_topology.sh scripts/run_root_tests.sh   # pass
scripts/diff_topology.sh single,3   # -> SKIP, exit 2 (Rust mn --dump not wired yet)
scripts/run_root_tests.sh           # -> root-required error, exit 3, nothing executed
```

The full MATCH/MISMATCH comparison happens on the privileged host/CI above.
