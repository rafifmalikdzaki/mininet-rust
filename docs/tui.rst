mnr-tui — the live graph TUI
===========================

Inspired by how `OMO (omo.dev) <https://omo.dev/>`_ renders agent workflows as
live dependency graphs, ``mnr-tui`` renders the emulated network as a graph:

.. code-block:: console

   $ mnr-tui --topo tree,2,2            # dry-run plan, no root needed
   $ sudo mnr-tui --topo tree,2,2       # live: starts the net, refreshes 1/s
   $ mnr-tui --topo single,3 --json     # export the graph as JSON
   $ mnr-tui --topo single,3 --frames 5 # headless render for CI

Nodes are drawn as boxes (host = green, switch = yellow, controller = cyan,
selected = bold white); edges are unicode lines colored by link status
(``OK``/``MISSING``). Keys: ``q`` quit, arrows/``hjkl`` pan, ``+``/``-`` zoom,
``tab`` cycle nodes, ``enter`` select (detail pane with repr/intfs/pid),
``r`` refresh, ``p`` pingall.

The renderer is terminal-agnostic (``CellBuffer`` trait) with a headless
``StringGrid`` backend, so frames are golden-tested without a tty.
