Testing
=======

Two tiers (see ``docs/migration/TESTING.md``):

Unprivileged (any machine)
--------------------------

.. code-block:: console

   $ cargo fmt --all -- --check
   $ cargo clippy --workspace --all-targets -- -D warnings
   $ cargo test --workspace          # unit + Python-driven differential tests
   $ target/debug/mnr --topo single,3 --dump   # dry-run, no root

The suites include differential tests that drive the **original Python code**
and assert identical outputs (5536 util cases, 52 topo cases, 11 net-plan
specs), plus golden tests captured from live CPython CLI sessions.

Privileged (root / disposable Linux host)
-----------------------------------------

.. code-block:: console

   $ sudo sh scripts/run_root_tests.sh
   # MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1
   # + scripts/diff_topology.sh single,3 linear,4 tree,2,2
   #   (diffs Rust mnr output against the Python original)

Checks run locally; there is no GitHub Actions pipeline. Network namespaces
and privileged containers are not security boundaries for untrusted code.

The containerized root gate builds and installs ``mnexec`` before running the
ignored suites; without it, live tests fail to spawn hosts even though the
workspace compiles. Use:

.. code-block:: console

   $ just check
   $ just test
   $ just root-tests-docker          # privileged, installs mnexec, all #[ignore]d tests

Validation snapshot (2026-11-14, host + privileged Docker)
----------------------------------------------------------

* unprivileged workspace + monitor: 445 passed, 0 failed, 29 ignored;
* privileged ignored suites: 29 passed, 0 failed after the ``mnexec`` gate fix;
* all six research presets passed, plus failure injection (nonzero exit,
  timeout, missing binary, stdout mismatch) and invalid-scenario rejection;
* MCP protocol suite 13 passed; live custom-graph build/start/ping/stop/release
  passed;
* differential topology ladder 6/6 MATCH; Python bindings 5/5 non-root smoke;
* real OVS scenarios pass rootless: VLAN, multi-tenant, trunk fault, OSPF,
  active-backup bonding, and redundant leaf-spine/STP;
* Sphinx docs build with zero warnings;
* full evidence, limits, and recommendations: :doc:`research/TECHNICAL_REPORT`;
* real-switch/rootless report: :doc:`research/REAL_SWITCHES_ROOTLESS`.

Benchmarks & differential scenarios
-----------------------------------

``scripts/run_root_tests.sh`` diffs Rust ``mnr --dump`` against the Python
reference across an increasing-complexity ladder: ``single,3``, ``linear,4``,
``linear,10``, ``tree,2,2``, ``tree,3,2``, ``tree,3,3``.

``scripts/bench_scenarios.sh`` (root) benchmarks both implementations on the
same ladder and emits a markdown table (wall time of
``--topo SPEC --controller none --test pingall`` + ping loss, median/min/max
over N runs). See :doc:`migration/BENCHMARKS` for methodology; set
``BENCH_OUT`` to save the table.
