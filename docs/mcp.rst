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
     - build a net from a spec (dry-run model, no root)
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

Security model: stdio transport only; the client is the local user who spawned
the server. ``net_start``/``net_stop``/``host_cmd`` hard-fail unless euid 0.
Never expose ``mnr-mcp`` over a network transport.

The graph JSON is the same canonical shape the TUI uses
(``mnr-tui --json``), defined once in ``crates/tui`` and shared with
``crates/mcp``.
