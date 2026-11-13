#!/usr/bin/env python3
"""simpleperf, Rust-backed: iperf-style throughput between h1 and h2.

  sudo python3 examples/rust/simpleperf.py
Falls back to ping RTT if iperf3 is not installed in the namespaces.
"""
import mininet_rust as mnr

net = mnr.Mininet(topo="single,2", controller="none", link="tc,bw=100,delay=1ms")
net.build()
net.start()
try:
    print(net.cmd("h2", "iperf3 -s -1 -D 2>/dev/null || iperf -s -D 2>/dev/null || echo no-iperf"))
    out = net.cmd("h1", "iperf3 -c 10.0.0.2 -t 3 2>/dev/null || iperf -c 10.0.0.2 -t 3 2>/dev/null || ping -c3 10.0.0.2")
    print(out)
finally:
    net.stop()
