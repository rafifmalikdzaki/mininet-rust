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

## CI status (2026-11-13, head 3d7554a, PR #1 run 43306962340): ALL 4 GREEN
- rust-ci, code-check, mininet-tests, rust-root-tests all success.
- rust-root-tests: all root-gated cargo suites pass in the privileged
  container; `diff_topology.sh` MATCH for all 6 ladder specs (single,3,
  linear,4, linear,10, tree,2,2, tree,3,2, tree,3,3) with lxbr; benchmark
  ladder uploaded as the `bench-results` artifact (results now in
  BENCHMARKS.md).
- Local gates: fmt clean, clippy `-D warnings` 0 errors, `cargo test
  --workspace` 419+ passed (3 failures in this sandbox only, reproduced on
  the pre-change HEAD: leftover veth in the sandbox netns breaks 3 dry-run
  tests; they pass on CI).

## This session's commits (CI repair + latent root-test fixes, D23/D24)
| 3afcfd6 | netlink/link: qdiscs installed after the netns move, per-end in its own ns (Tc is now a unit struct on &Netlink) |
| 7044fcf | latent root-test fixes: veth-pair delete, netns move-back, pingall 2/2, bridge port naming, cgroup ro skip |
| 9f80cef | ci: iputils-ping + controller shim + cgroup2 remount in root-tests |
| 7a6a92e | docs: D23 |
| 70882dd | bridge lifecycle cleanup best-effort |
| a035f97 | modprobe openvswitch; run_mn timeout; pingall -W in repl tests |
| 10ff305 | NetSwitch::attach after link.attach; Node::started_flag for shell-less lxbr |
| 803ee87 | root app/tui/mcp tests use lxbr; pid assertions hosts-only |
| fc2930e | docs: D24 |
| f93246b | ci: python3-packaging (py3.12 has no distutils) |
| 20acb77 | ci: diff harness + benchmarks run with lxbr in the container; step timeouts |
| 3d7554a | diff_topology: append pingall section to Rust capture; normalize pid=None |

## Rootless containers (D25)
- `container/Dockerfile` (multi-stage rust builder -> ubuntu:24.04 runtime with
  mnr/mnexec/mnr-tui/mnr-mcp) + `container/entrypoint.sh` (starts OVS daemons
  when present) + `scripts/mnr-container.sh` (rootless podman/docker runner;
  auto `--switch lxbr`; mounts $PWD at /w). No sudo/root on the host — caps are
  confined to the container user namespace.
- CI: `.github/workflows/container.yaml` builds the image and runs
  single,3/linear,4 pingall + tree,2,2 --dump through the wrapper.
- Verified locally with docker: image builds, `--topo single,3 --test pingall`
  -> 0% dropped (6/6), `--topo tree,2,2 --dump` -> plan.

## Rootless containers (D25)
- `container/Dockerfile` (multi-stage rust builder -> ubuntu:24.04 runtime with
  mnr/mnexec/mnr-tui/mnr-mcp) + `container/entrypoint.sh` (starts OVS daemons
  when present) + `scripts/mnr-container.sh` (rootless podman/docker runner;
  auto `--switch lxbr`; mounts $PWD at /w). No sudo/root on the host — caps are
  confined to the container user namespace.
- CI: `.github/workflows/container.yaml` builds the image and runs
  single,3/linear,4 pingall + tree,2,2 --dump through the wrapper.
- Verified locally with docker: image builds, `--topo single,3 --test pingall`
  -> 0% dropped (6/6), `--topo tree,2,2 --dump` -> plan.
- Justfiles: root `justfile` (build/check/test/root-tests-docker via
  privileged container/diff/bench/container-*/docs/clean) + `container/justfile`
  (build/run/shell/smoke/clean); recipes dry-run + live verified (`just test` = 39
  ok suites, `container-run` pingall 6/6 at 0% loss).

## Next actions
1. Merge PR #1 (rust-migration -> master on the fork) — all 5 workflows green.
2. Post-merge roadmap: docs/migration/ROADMAP.md (HTB bw via netlink, NOX/Ryu
   stubs, miniedit/cluster/mobility, --innamespace/--custom, perf: cut per-node
   shell spawn cost — see BENCHMARKS.md reading).
