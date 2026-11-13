# Roadmap — mininet-rust (branch `rust-migration`)

*Written by the reviewer/architect pass, 2026-11-12. Current gates: `cargo fmt --check`,
`clippy -D warnings`, `cargo test --workspace` → **420 passed / 0 failed / 29 ignored**
(root-gated `MN_RUST_ROOT_TESTS=1`). 26 commits on the branch; Python tree untouched.*

## 1. Executive summary

The Rust port is **functionally complete for the v1 scope**: all core modules
(util, topo, netlink, link, node, net, CLI+REPL, mnexec, nodelib), the `mnr`
binary, the `mnr-tui` live graph TUI, and the `mnr-mcp` MCP server are ported,
reviewed, and differentially tested against the original Python. What remains
is (a) a few surface gaps, (b) Python-facing bindings, (c) examples/testbed,
(d) the first privileged CI run, (e) the PR to the fork's `master`.

## 2. Problems found & fixed during the migration

| Problem | Severity | Status |
|---|---|---|
| Holder/setns race could land a node shell in the **root namespace** | HIGH | fixed (readiness pipe handshake + `/proc/<pid>/ns/net` verification) |
| `pexec` deadlock on >64 KiB stderr (stdout read first) | HIGH | fixed (`wait_with_output`), regression test |
| Pickle state file in shared `/tmp` = root code-exec via `py`/`px` | HIGH | fixed (JSON state in 0700 O_EXCL dir; planted-payload test) |
| `terminate()` error path leaked holder process + netns | MED | fixed (run-all-cleanup, first error returned) |
| Fork children allocating/creating tokio runtime post-fork | MED | fixed (pre-fork alloc, runtime moved in, thread gate) |
| `mnexec` not resolved on PATH → clean-machine test failure | MED | fixed (PATH resolution, skip on NotFound) |
| `add_host_defaults` clobbered caller ip/mac | MED | fixed (`entry().or_insert`) |
| Duplicate workspace package names (mn-tui lib vs app) broke `cargo metadata` | MED | fixed (`-app` packages + `[[bin]]`) |
| Binary name `mn` clashed with installed Python mininet | MED | fixed → `mnr` (D20); `mnr-tui`/`mnr-mcp` (D20b) |
| 19 design nits (premature trait, boxed iterator, dup parsers, …) | LOW | applied behavior-preserving |

## 3. Known remaining gaps

| Gap | Severity | Notes |
|---|---|---|
| Root-gated suites (29 tests) never executed on a privileged host yet | HIGH | `rust-root-tests.yaml` exists; needs one real CI run |
| `--nat` / `addNAT` wiring | HIGH | **done** (`8090dcc`): add_nat + config_nats + dry-run test |
| Python bindings | MED | **done**: `bindings/python` (mininet_rust, PyO3), pytest smoke tests green |
| Rust-backed examples | MED | **done**: `examples/rust/` (emptynet, external_nat, cybersec_testbed, simpleperf) |
| HTB `bw=` via netlink is TODO (netem rate interim) | LOW | needs privileged verification |
| NOX/Ryu controller runtimes are stubs | LOW | out of v1 (D17) |
| miniedit GUI, cluster, mobility, `--innamespace`, `--custom` | LOW | out of v1 (D17) |
| `BENCHMARKS.md` results table empty | LOW | filled from CI `bench-results` artifact |
| Differential ladder + bench ladder never run as root | MED | same CI run |

## 4. Next steps (ordered)

1. **Wire `--nat`**: `Mininet::add_nat` in `crates/net` (port of `net.py addNAT`:
   addHost cls=NAT with subnet=ipBase, link to first switch, set `via <natIP>`
   default route on namespaced hosts) + runner `--nat` flag; root-gated test.
2. **Python bindings** (`bindings/python`, PyO3 + maturin): expose
   `mininet_rust` module — `Topo`/`TreeTopo` builders, `Mininet` with
   `dry_run/plan/dump/start/stop/pingall`. This gives Python users the Rust
   core with familiar abstractions ("Rust core + Python frontend").
3. **Examples & cybersec testbed**: `examples/` ports that run on the bindings
   (emptynet, simpleperf, nat/external-connection, linuxrouter) + a
   `cybersec_testbed` example (attacker/victim/switch + NAT egress, scripted
   scan/defense steps). Rust-native examples under `examples/rust/`.
4. **Privileged CI run**: push branch, watch `rust-root-tests.yaml` —
   root-gated tests, `diff_topology.sh` ladder (single,3…tree,3,3),
   `bench_scenarios.sh` ladder → fill `BENCHMARKS.md`.
5. **PR `rust-migration` → fork `master`** (origin `rafifmalikdzaki/mininet-rust`),
   with the verification evidence in the body.

## 5. Roadmap phases

| Phase | Scope | Exit criteria |
|---|---|---|
| A — Finish the surface | `--nat`, PyO3 bindings, examples + cybersec testbed, docs | workspace gates green; bindings importable from python3; examples run (dry-run non-root) |
| B — Verify on root | privileged CI run | 29 root-gated tests pass; diff ladder all MATCH; bench table produced |
| C — Land it | push + PR to fork master, review, merge | CI green on master; PR merged |
| D — Harden & extend | HTB via netlink, NOX/Ryu, miniedit/cluster decision, optional legacy cleanup after a parity window | D-log updated; benchmarks show no regression vs Python |

## 6. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Privileged CI unavailable → root paths unverified | keep double-gated tests; run `scripts/run_root_tests.sh` on any root host; never claim verified without output |
| PyO3 bindings drift from Rust API | thin bindings over `mn-net` only; differential test bindings vs Python mininet on plan/dump |
| Examples bit-rot | golden/dry-run tests for each example |
| Branch divergence from fork master | PR early (Phase C), small merges |
