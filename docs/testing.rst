Testing
=======

Two tiers (see ``docs/migration/TESTING.md``):

Unprivileged (any machine)
--------------------------

.. code-block:: console

   $ cargo fmt --all -- --check
   $ cargo clippy --workspace --all-targets -- -D warnings
   $ cargo test --workspace          # 420 passed, 0 failed
   $ target/debug/mnr --topo single,3 --dump   # dry-run, no root

The suites include differential tests that drive the **original Python code**
and assert identical outputs (5536 util cases, 52 topo cases, 11 net-plan
specs), plus golden tests captured from live CPython CLI sessions.

Privileged (root / CI)
----------------------

.. code-block:: console

   $ sudo sh scripts/run_root_tests.sh
   # MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1
   # + scripts/diff_topology.sh single,3 linear,4 tree,2,2
   #   (diffs Rust mnr output against the Python original)

CI: ``.github/workflows/rust-ci.yaml`` (fmt/clippy/test/release) and
``.github/workflows/rust-root-tests.yaml`` (privileged container with OVS).
