#!/usr/bin/env python3
"""emptynet, Rust-backed: build a 3-host/1-switch net with the mnr core.

Run:  python3 examples/rust/emptynet.py          (dry-run plan, no root)
      sudo python3 examples/rust/emptynet.py       (real emulation)
Requires the mininet_rust module (maturin develop -m bindings/python/Cargo.toml).
"""
import mininet_rust as mnr

net = mnr.Mininet(topo="single,3", controller="none")
net.build()
print(net.dump())
print(net.links())

if mnr.__dict__.get("_ROOT", False):
    net.start()
    loss, out = net.pingall()
    print(out)
    print(f"pingall loss: {loss}%")
    net.stop()
