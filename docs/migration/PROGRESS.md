# Migration Progress Ledger — branch `rust-migration`

Last updated: 2026-11-12 (session log below). Gates: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
all green — **207 passed / 0 failed / 14 ignored** (ignored = root-gated
`MN_RUST_ROOT_TESTS=1`). Python tree untouched (differential reference).

## Commit log (atomic, per category)
| Commit | Slice | Notes |
|--------|-------|-------|
| 40d8a0c | plan + task list | tasks/plan.md, tasks/todo.md |
| 18681cd | workspace scaffold + rust-ci | crates util/topo stubs, apps/mn stub |
| f5996fb | /target ignore + Cargo.lock | build hygiene |
| 57804b2 | ARCHITECTURE.md + fixtures | Tasks 1-2, byte-reproduced baselines |
| ecf9113 | mn-util (Task 4) | 16 fns, 23 unit + 5536-case differential |
| 773e78e | mn CLI args (Task 10a) | clap, golden help, differential options |
| 42518a9 | mn-netlink + mnexec (6a/6b) | rtnetlink core, mnexec.c replacement |
| 61c929a | mn-topo (Task 5) | 48 tests, fixtures byte-identical |
| 411e4b5 | mn-link (Task 7) | pure-netlink netem, LinkConfig |
| e5255df | mn-node (Task 8a) | PE APPROVE after 10 fixes |
| 0e6994a | mn-netlink hardening | fork-safety, handshake, error model |
| aafe084 | design nits | behavior-preserving, differentials unchanged |
| a5bbd12 | diff harness scripts + TESTING.md | Task 11 prep, SKIP/root-guard verified |
| 8719f6d | plan update | 4-increment CLI plan |

## Review history
- principal-engineer REJECT on first mn-node review: 2 HIGH (holder/setns root-ns
  race; pexec pipe deadlock) + 8 MEDIUM (NotRunning fallbacks, fork-after-alloc,
  terminate leak, mnexec PATH resolution, ip/mac clobbering, dpid first-digit-run,
  Switch default_intf/stop, killpg) — all fixed, re-review **APPROVE**.
- design-optimizer: 19 NITS (premature GraphSink trait, Box<dyn Iterator>, dup
  parsers, visibility leaks, oversized fns) — all applied or consciously skipped
  with reasons; behavior proven unchanged by differential tests.

## In flight
- Task 8b: Switch/Controller real ports (agents: switch-ctrl) — crates/node only.
- Task 9: mn-net orchestrator + non-root dry-run `--dump` (agent: net-orch) —
  crates/net + root Cargo.toml.

## Blocked / environment
- Sandbox is uid 1000, no netns: 14 root-gated tests can't run here; they run via
  `scripts/run_root_tests.sh` on a privileged host/CI. `diff_topology.sh` exits
  SKIP until `mn --dump` is wired (Task 10b).
- `glm-worker` subagent profile created (`~/.vibe/agents/glm-worker.toml`,
  active_model zai-glm-5-3-flash) but needs `/reload` in the Vibe session to be
  discoverable; until then workers run on the session model.

## Next actions
1. Land 8b + 9 after PE/design review; commit atomically.
2. Task 10b: wire apps/mn to mn-net (--topo/--dump/--test), non-root dry-run.
3. Task 10c: repl.rs interactive loop, 2-3 commands per commit, golden tests.
4. nodelib (LinuxBridge/NAT/CPULimitedHost), then privileged CI run of T11.
5. T12: DECISIONS/PROGRESS/BENCHMARKS upkeep, README, privileged CI job, PR to master.
