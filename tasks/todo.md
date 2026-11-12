# Task List: mininet-rust Rust migration (branch rust-migration)

Plan: tasks/plan.md. Python tree untouched = differential reference.
Workers run on glm-worker (GLM 5.3); reviewers (principal-engineer,
design-optimizer) on mistral-large-4-high; orchestrator commits only
reviewed, gate-green work, atomic per category, no AI attribution.

## Done
- [x] T1 ARCHITECTURE.md + T2 baseline fixtures (57804b2)
- [x] T3 workspace scaffold + rust-ci (18681cd, f5996fb)
- [x] T4 mn-util + 5536-case differential (ecf9113)
- [x] T5 mn-topo + characterization/differential (61c929a)
- [x] T6a/6b mn-netlink + mnexec (42518a9, 0e6994a)
- [x] T7 mn-link pure-netlink netem (411e4b5)
- [x] T8a mn-node Node/Host, PE APPROVE (e5255df)
- [x] T10a clap args in apps/mn (773e78e)
- [x] T11-prep diff harness scripts + TESTING.md (a5bbd12)
- [x] design nits sweep (aafe084)

## In flight
- [ ] polish-fixes worker: pickle->JSON state (security), ratbuf cell reset, graph JSON alignment, dep trim
- [ ] T10d remaining cli.py commands + term.py xterms (agent repl-polish) - apps/mn only
- [ ] T12 privileged CI + RUST.md/BENCHMARKS.md (agent ci-docs)

## Queued
- [x] T13 mn-tui graph TUI (c6ee867)
- [x] T14 mn-mcp MCP server (e8b42fd)
- [x] T10b wire mn -> mn-net (5adb7e0): --topo/--dump/--test, non-root dry-run path
- [x] T10c repl.rs interactive loop (ed56dbc): nodes/links/dump/net/ports/hosts/switches/controllers/ping/pingall/iperf/dpctl/route/xterm/exit (2-3 cmds per commit, golden tests)
- [ ] T10d parity polish (agent repl-polish) (sh passthrough, intfs, py escape optional)
- [x] nodelib: LinuxBridge/NAT/CPULimitedHost (2539eaa)
- [ ] T11 full run on privileged host/CI (workflow rust-root-tests.yaml landed; needs a real CI run)
- [x] T12 privileged CI + RUST.md + BENCHMARKS + D14-D17 (d742d88) PROGRESS/DECISIONS/BENCHMARKS, README, privileged CI job, PR to master
- [ ] scope decision: miniedit/consoles/cluster/mobility out of v1 (DECISIONS.md)

## Checkpoint: current
- [x] fmt/clippy -D warnings/test green: 207 passed, 0 failed, 14 root-gated ignored
- [x] PE re-review APPROVE on mn-node; 10/10 issues fixed
- [x] working tree clean, Python tree untouched, 345 passed / 0 failed / 24 root-gated ignored
