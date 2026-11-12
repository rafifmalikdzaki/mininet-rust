# Task List: mininet-rust Rust migration

Plan: `tasks/plan.md`. Python tree stays untouched as the differential reference.

## Phase 0 — Discovery & baseline
- [ ] Task 1: Write docs/migration/ARCHITECTURE.md (inventory + dependency graph of mininet/*.py, mnexec.c, bin/mn, tests, CI) — S — deps: none
- [ ] Task 2: Capture behavioral baseline fixtures from Python (test_util.py, topo outputs) into fixtures/ — S — deps: Task 1

## Phase 1 — Rust foundation
- [ ] Task 3: Scaffold Cargo workspace + rust-toolchain + apps/mn stub + Rust CI workflow — M — deps: none
- [ ] Task 4: Port mininet/util.py pure helpers to crates/util + unit + differential tests — M — deps: Task 3
- [ ] Task 5: Port mininet/topo.py + topolib.py to crates/topo + characterization tests — M — deps: Task 3, Task 4

## Checkpoint: Foundation
- [ ] cargo fmt --check / clippy -D warnings / cargo test --workspace pass
- [ ] Differential parity topo/util vs Python on fixtures
- [ ] Python tree untouched outside tasks/, crates/, docs/

## Phase 2 — Kernel-facing core
- [ ] Task 6a: crates/netlink namespace + veth + addr/qdisc wrapper (rtnetlink) — M — deps: Task 3
- [ ] Task 6b: mnexec replacement (setns/exec via nix) — S — deps: Task 6a
- [ ] Task 7: Port link.py model (Intf/Link/TCLink, netem params) — M — deps: 6a
- [ ] Task 8a: Port Node/Host core (namespace processes, cmd, intf config) — L — deps: 6a, 6b, 7
- [ ] Task 8b: Port Switch/Controller shells (OVS stays subprocess-based) — M — deps: 8a

## Phase 3 — Orchestration & CLI
- [ ] Task 9: Port net.py Mininet orchestrator + root-free dry-run mode — L — deps: 5, 8b
- [ ] Task 10: Port bin/mn arg parsing + cli.py core interactive loop to apps/mn — M — deps: 9
- [ ] Task 11: Differential harness Python-vs-Rust (dump/pingall/links on small topos, root host/CI) — M — deps: 9, 10

## Phase 4 — Hardening & docs
- [ ] Task 12: Rust CI gates, PROGRESS.md ledger, README build/run, DECISIONS.md — S — deps: all above

## Checkpoint: Complete
- [ ] All workspace gates pass; differential parity demonstrated on root host
- [ ] docs/migration/ reflects actual code; handoff notes written
