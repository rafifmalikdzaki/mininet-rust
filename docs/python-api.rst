Python API (original)
=====================

The Python package in ``mininet/`` is unchanged. As before:

.. code-block:: python

   from mininet.net import Mininet
   from mininet.topo import SingleSwitchTopo

   net = Mininet(topo=SingleSwitchTopo(k=3))
   net.start()
   net.pingAll()
   net.stop()

Full API documentation via Python ``help()`` docstrings, or build it with
``make doc`` (Doxygen, see ``doc/doxygen.cfg``). The web walkthrough lives at
http://mininet.org.
