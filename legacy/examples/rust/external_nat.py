#!/usr/bin/env python3
"""External connectivity: hosts reach the outside world through a NAT node.

Equivalent of examples/nat.py, backed by the Rust core (mnr):
  sudo python3 examples/rust/external_nat.py
The NAT (nat0) is linked to the first switch; every host gets a default
route via the NAT IP; nat0 masquerades with iptables (installed by the
Rust nodelib NAT on start).
"""
import mininet_rust as mnr

net = mnr.Mininet(topo="single,3", controller="none", nat=True)
net.build()
assert "nat0" in net.hosts(), net.hosts()
print(net.dump())

net.start()  # requires root
try:
    # from a host, ping the NAT gateway and an external address
    print(net.cmd("h1", "ping -c1 -W2 10.0.0.254 || true"))
    print(net.cmd("h1", "ping -c1 -W3 1.1.1.1 || echo 'no external route'"))
finally:
    net.stop()
