Mininet — Rapid Prototyping for Software Defined Networks
==========================================================

*The best way to emulate almost any network on your laptop.*

Mininet emulates a complete network of hosts, links, and switches on a single
machine. Create a sample two-host, one-switch network with:

.. code-block:: console

   $ sudo mn                       # Python original
   $ sudo target/release/mnr      # Rust port (binary mnr) (branch rust-migration)

.. toctree::
   :maxdepth: 2
   :caption: Contents

   quickstart
   rust-guide
   tui
   mcp
   architecture
   testing
   migration
   bindings
   python-api

.. toctree::
   :maxdepth: 2
   :caption: Research program

   research/index

.. toctree::
   :maxdepth: 1
   :caption: Migration ledger (from docs/migration/)

   migration/ARCHITECTURE
   migration/DECISIONS
   migration/PROGRESS
   migration/TESTING
   migration/BENCHMARKS
   migration/REVIEW
   migration/ROADMAP

Indices
-------

* :ref:`genindex`
* :ref:`modindex`
* :ref:`search`
