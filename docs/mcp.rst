mnr-mcp — drive simulations from AI tools
=========================================

``mnr-mcp`` is a `Model Context Protocol <https://modelcontextprotocol.io/>`_
server (JSON-RPC 2.0 over stdio) that exposes the network graph and the
simulator to any MCP-capable AI agent — including this CLI:

.. code-block:: console

   $ vibe mcp add mnr-mcp --transport stdio --command "$(pwd)/target/release/mnr-mcp"

Tools
-----

.. list-table::
   :header-rows: 0

   * - ``topo_list``
     - built-in topology names
   * - ``topo_plan``
     - plan a topo spec → graph JSON ``{nodes, edges}``
   * - ``graph``
     - live graph with node/link status (needs a built net)
   * - ``net_build``
     - build a built-in spec or custom graph (dry-run model, no root)
   * - ``net_start``
     - start it for real — **requires root**
   * - ``net_stop``
     - stop — **requires root**
   * - ``pingall`` / ``ping``
     - connectivity tests (need a running net)
   * - ``dump`` / ``links``
     - cli.py-format text output
   * - ``host_cmd``
     - run a command on a host — **requires root**
   * - ``net_release``
     - release the session net

For an arbitrary topology, call ``net_build`` with exactly one of ``spec`` or
``topology``. This example builds two hosts and a switch:

.. code-block:: json

   {"topology":{"hosts":["h1","h2"],"switches":["s1"],"links":[["h1","s1"],["h2","s1"]]},"opts":{"controller":"none","switch":"lxbr"}}

Names begin with a letter and contain at most seven ASCII letters, digits or
underscores; ``c0`` is reserved. The graph is bounded to 256 nodes and 1024
links. Check the returned plan before starting it.

Security model: stdio transport only; the client is the local user who spawned
the server. ``net_start``/``net_stop``/``host_cmd`` hard-fail unless euid 0.
Never expose ``mnr-mcp`` over a network transport.

The graph JSON is the same canonical shape the TUI uses
(``mnr-tui --json``), defined once in ``crates/tui`` and shared with
``crates/mcp``.

Validation
----------

The protocol suite covers handshake/notifications, every tool schema, error
categories, session state, custom graphs, and non-root rejection. A live
privileged session also built a custom two-host graph, started it, ran
``pingall`` at 0% loss, stopped it, and released it. There is no embedded model
in this repository: agent behaviour is supplied by the MCP client.

See :doc:`research/TECHNICAL_REPORT` for evidence, limits, and the agent/MCP
research plan (P6).
