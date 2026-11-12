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

## Recent commits
| 3459e45 | mn-net orchestrator + dry-run (Task 9) |
| e3d6dde | Switch/Controller port (Task 8b) |
| 5adb7e0 | mn wired to mn-net, --dump/--test/--clean (Task 10b) |
| 2539eaa | nodelib: LinuxBridge/NAT/CPULimitedHost |
| ed56dbc | cli.py interactive loop port (Task 10c) |
| 5908e6e | lxbr seam + netlink upstreaming + cleanups |

Gates now: 410 passed / 0 failed / 29 ignored (root-gated).

## Latest commits (Tasks 10d/12/13/14)
| c2be88b | cli.py parity: term.rs xterms, link/switch/wait/time/py/px/x (T10d) |
| d742d88 | privileged CI workflow, RUST.md, BENCHMARKS, D14-D17 (T12) |
| c6ee867 | mnr-tui OMO-style graph TUI, headless --frames/--json (T13) |
| e8b42fd | mnr-mcp MCP server, 12 graph+sim tools, stdio JSON-RPC (T14) |

## Latest
- 386dd1a rename CLI mn -> mnr; Sphinx docs site (docs/, furo) + modernized README; bench_scenarios.sh ladder (single,3..tree,3,3) + CI artifact; diff ladder extended; BENCHMARKS.md methodology.

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
1. PR #1 (rust-migration -> master on the fork) is open; CI was red and is
   now fixed locally-verified (D22): watch the privileged `rust-root-tests`
   job and the benchmark ladder artifact.
2. Fill docs/migration/BENCHMARKS.md results table from the CI `bench-results`
   artifact (python mn vs mnr ladder) once the privileged job runs green.
3. Reviewer pass (five-axis) over the branch diff, then merge PR #1.
4. Post-merge roadmap: see docs/migration/ROADMAP.md (HTB via netlink, NOX/Ryu
   stubs, miniedit/cluster/mobility, --innamespace/--custom).
