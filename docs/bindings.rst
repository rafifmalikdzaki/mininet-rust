Python bindings (``mininet_rust``)
==================================

The Rust core is exposed to Python via PyO3/maturin — "Rust core, Python
frontend": Python code drives the netlink-based engine, no Python mininet
install needed.

.. code-block:: console

   $ pip install maturin
   $ maturin develop -m bindings/python/Cargo.toml     # or: maturin build + pip install

.. code-block:: python

   import mininet_rust as mnr

   net = mnr.Mininet(topo="tree,2,2", controller="none", nat=False)
   net.build()
   print(net.dump()); print(net.links())
   net.start()                    # requires root
   loss, out = net.pingall()
   print(net.cmd("h1", "ifconfig"))
   net.stop()

   print(mnr.plan("single,3"))    # JSON plan, no root needed

API: ``Mininet(topo, controller, link, switch, ipbase, nat)`` with
``build/start/stop/dump/links/plan_json/hosts/switches/pingall/ping/cmd``,
plus module functions ``plan(spec)`` and ``version()``.

Tests: ``python3 -m pytest bindings/python/test_mininet_rust.py`` (non-root
smoke tests; root behavior is covered by the Rust root-gated suites).

Ready-made labs: see ``examples/rust/`` — ``external_nat.py`` (external
connections through the NAT) and ``cybersec_testbed.py`` (attack/defense
testbed; ``--plan`` previews without root).
