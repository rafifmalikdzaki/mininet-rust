The Rust Port (``rust-migration``)
==================================

The branch ``rust-migration`` contains a from-scratch Rust re-implementation of
Mininet alongside the untouched Python original (which remains the
differential-testing reference). The design follows `containerlab
<https://containerlab.dev/>`_'s core lesson: **talk to the kernel via netlink,
never shell out to** ``ip`` / ``tc`` / ``ifconfig``.

Layout
------

.. code-block:: text

   crates/  util topo netlink link node net tui mcp   (libraries)
   apps/    mn (CLI)  mnexec (namespace exec helper)  mnr-tui  mnr-mcp

Dependency direction: ``util ← topo``, ``netlink`` at the base,
``link ← util+netlink``, ``node ← all``, ``net ← topo+node+link``,
``mn ← net``, ``mnr-tui``/``mnr-mcp ← net``.

Build & run
-----------

.. code-block:: console

   $ cargo build --workspace --release
   $ sudo target/release/mnrr --topo tree,2,2
   $ target/release/mnr --topo single,3 --dump     # dry-run, no root needed
   $ sudo target/release/mnrr --test pingall

The CLI mirrors ``bin/mn``'s full option surface (verified by golden +
differential tests against ``python3 bin/mn --help``) and the interactive REPL
mirrors ``mininet/cli.py`` byte-for-byte (``nodes``, ``links``, ``dump``,
``pingall``, ``iperf``, ``sh``, ``py``, ``xterm``, …).

What is ported
--------------

.. list-table::
   :header-rows: 1

   * - Component
     - Status
   * - ``util.py`` helpers
     - ported, 5536-case differential vs Python
   * - ``topo.py`` / ``topolib.py``
     - ported, byte-identical fixtures + differential
   * - namespaces / veth / addr / route / qdisc
     - netlink (rtnetlink), no ``ip``/``tc`` shell-outs
   * - ``link.py`` incl. netem
     - pure-netlink netem qdisc
   * - ``node.py`` (Node/Host/Switch/Controller)
     - ported; OVS control stays ``ovs-vsctl``
   * - ``nodelib.py``
     - LinuxBridge (netlink), NAT (iptables), CPULimitedHost (cgroup v2)
   * - ``net.py`` orchestrator
     - ported, Python-order start/stop, exact dump/links
   * - ``cli.py`` REPL + ``bin/mn``
     - ported, golden-tested vs live CPython sessions
   * - ``mnexec.c``
     - ported as ``apps/mnexec`` (byte-identical usage)
   * - ``mnr-tui``
     - OMO-style live graph TUI (see :doc:`tui`)
   * - ``mnr-mcp``
     - MCP server for AI tools (see :doc:`mcp`)

Known TODOs: HTB bandwidth shaping via netlink (bw currently maps to netem
rate), NOX/Ryu controller runtimes, miniedit GUI, cluster/mobility. See
``docs/migration/DECISIONS.md`` (D17).

Python bindings & labs
----------------------

``bindings/python`` (PyO3 + maturin) exposes the Rust core to Python as
``mininet_rust`` — see :doc:`bindings`. Rust-backed examples live in
``examples/rust/``, including ``external_nat.py`` (external connections via
the NAT) and ``cybersec_testbed.py`` (cybersecurity testbed:
attacker/victims/defender + NAT egress; ``--plan`` works without root).
