# Five-Axis Review — branch `rust-migration` (PR #1, head `34d5f35`)

Reviewer pass per `code-review-and-quality`: correctness, readability,
architecture, security, performance. Evidence gathered 2026-11-13 against
the pushed branch; CI run 43306962340 all-green (rust-ci, code-check,
mininet-tests, rust-root-tests).

## Verification evidence

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors.
- `cargo test --workspace` — 419+ passed; the only 3 failures
  (`stop_on_dry_run_net_is_a_safe_noop_cleanup`, `dry_run_lxbr_switch_repr_golden`,
  `dry_run_is_pure_and_stoppable_non_root`) reproduce identically on the
  pre-change HEAD in this sandbox (leftover veth `h1-eth0@h2-eth0` in the
  sandbox netns) — environmental, not regressions; all pass on CI.
- Privileged CI (ubuntu:24.04 container, `--privileged`): all root-gated
  suites pass; `scripts/diff_topology.sh` MATCH for all 6 ladder specs;
  benchmark artifact `bench-results` uploaded (see BENCHMARKS.md).
- `/tmp/actionlint` clean on all 4 workflows.

## Findings

### Correctness
- [ok] Behavioral parity is *proven*, not asserted: differential harness
  (Python `bin/mn` vs `mnr --dump` + pingall) MATCHes on single,3 → tree,3,3.
- [ok] Root-gated kernel behavior verified in privileged CI: veth/netns
  lifecycle, netem qdisc install/delete, bridge enslave, netns move.
- [ok] Fork-safety: no unwinding in forked children (catch_unwind +
  ManuallyDrop + raw `_exit`); runtimes built pre-fork (D22/D23).
- [ok] Error paths return `Result`; the ~44 non-test `.expect(...)` calls are
  all invariant-style with messages (e.g. "shell just set"), plus test-only
  `unwrap`s. Two bare `.unwrap()`s outside tests are benign
  (`ns.rs:98` `CString::new("")` infallible; `link.rs:344` `pid.unwrap()`
  guarded by `local == false`) — confirmed by reading.
- [info] `dry_run` is side-effect free by construction (switch start/stop are
  no-ops on non-started nets — D22 fix #2).

### Readability & simplicity
- [ok] Names and module layout follow the port's conventions; crate docs
  explain the Python mapping (`port of mininet/link.py ...`).
- [optional] Large files by total size: `switch.rs` 2320 lines, `repl.rs`
  2083, `node.rs` 1475, `net.rs` 1472, `nodelib.rs` 1170. They are mostly
  test modules and message builders; splitting now would churn the diff
  without behavior change. Revisit after merge if they keep growing.
- [ok] No dead-code artifacts found in the diff; no `// removed` debris.

### Architecture
- [ok] Workspace crates mirror the Python modules (util/topo/netlink/link/
  node/net/cli/tui/mcp + apps + python bindings); dependencies flow one way.
- [ok] Pure computation (spec parsing, topo build, dry-run model) is
  separated from kernel I/O (netlink/netns), which is why non-root
  `cargo test --workspace` is meaningful.
- [ok] `Tc` is now a zero-sized unit riding on the shared `&Netlink`
  (D23) — removes a whole runtime/connection per link; `NetSwitch::attach`
  dispatch closes the deferred-lifecycle gap (D24).
- [info] Known intentional deviations from Python, all documented in
  DECISIONS.md: `bw` via netem `TCA_NETEM_RATE64` instead of HTB
  (netlink-packet-route 0.34 lacks typed HTB), root-ns lxbr switch has no
  shell pid, root tests double-gated (`#[ignore]` + `MN_RUST_ROOT_TESTS=1`).

### Security
- [ok] No secrets/credentials in code or logs (grep clean).
- [ok] All `unsafe` blocks (6 files: ns.rs, link.rs, bridge.rs, node.rs,
  mnexec/ops.rs, link tests) carry `SAFETY:` comments; fork children use only
  async-signal-safe raw syscalls.
- [ok] User input (topo specs, link specs, repl commands) is parsed into typed
  models with explicit `InvalidInput` errors (e.g. the link-attr encoding
  guard in `add_link` rejects unencodable `k=v` attrs loudly).
- [ok] Dry-run mode performs zero kernel side effects (CI-verified).
- [info] NAT path intentionally still shells out to `iptables` (same as
  Python); flagged in ROADMAP for a netlink port.

### Performance
- [info] Differential benchmarks (BENCHMARKS.md): behavior identical, 0%
  loss both sides; wall-clock Rust is ~1.1x–2.7x slower than Python
  end-to-end on the ladder, dominated by per-node shell/netns spawn, not the
  netlink data path. Measured, documented, and queued in ROADMAP — not a
  blocker for merge since correctness gates are green.
- [ok] No unbounded loops/queues in the diff; hot paths (veth create, qdisc
  add) are single netlink round trips, no polling.

## Verdict

VERDICT: Approve — the branch is behaviorally equivalent to Python mininet
on the differential ladder, all four CI workflows are green including the
privileged root suite, and the known deviations/limitations are documented
in DECISIONS.md / ROADMAP.md. The performance gap is measured and has a
concrete next step (cut per-node spawn cost), so it does not block merge.
