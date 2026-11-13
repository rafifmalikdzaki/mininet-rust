Rust Migration Ledger
=====================

The migration on branch ``rust-migration`` is tracked in ``docs/migration/``:

* ``ARCHITECTURE.md`` — full inventory of the Python system and dependency graph
* ``DECISIONS.md`` — D1–D19: every consequential decision with reasoning
  (strangler strategy, netlink-over-shell-outs, fork safety, error model,
  TUI/MCP choices, …)
* ``PROGRESS.md`` — commit-by-commit ledger and current gate status
* ``TESTING.md`` — the two testing tiers and how to run them
* ``BENCHMARKS.md`` — methodology + results table (filled after the privileged CI run)

Key decisions in short: incremental strangler migration (never a big-bang
rewrite); the Python tree stays as the differential reference until parity is
proven; root-requiring tests are double-gated (``#[ignore]`` +
``MN_RUST_ROOT_TESTS=1``); every slice passed an adversarial principal-engineer
review and a design review before being committed.
