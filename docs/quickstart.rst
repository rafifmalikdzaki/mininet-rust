Quickstart
==========

Installation
------------

See ``INSTALL`` for the Python package. For the Rust port (branch
``rust-migration``) you only need a Rust toolchain:

.. code-block:: console

   $ cargo build --workspace --release
   $ sudo target/release/mnrr --topo single,3

The Rust binary needs root for real emulation (network namespaces, veth
pairs, qdiscs), exactly like the Python original. Several operations work
without root in dry-run mode — see :doc:`rust-guide`.

Your first network
------------------

.. code-block:: console

   $ sudo mn --topo single,3
   mininet> nodes
   mininet> dump
   mininet> pingall
   mininet> exit

Topologies are parametrized, e.g. a tree with depth 2 and fanout 3:

.. code-block:: console

   $ sudo mn --topo tree,depth=2,fanout=3

Links can be shaped (bandwidth, delay, loss):

.. code-block:: console

   $ sudo mn --topo linear,3 --link tc,bw=10,delay=5ms,loss=0.1

Cleanup
-------

If interfaces or processes are left behind after a crash:

.. code-block:: console

   $ sudo mn -c          # Python
   $ sudo mn --clean     # Rust port (same effect)
