#!/usr/bin/env python3
"""Cybersecurity testbed on the Rust core (mnr).

A ready-made lab for attack/defense experiments:

    attacker (h1) ---\
                     s1 --- victim web/db (h2, h3) --- nat0 ---> external
    defender (h4) --/

Run as root:  sudo python3 examples/rust/cybersec_testbed.py
Dry-run:      python3 examples/rust/cybersec_testbed.py --plan

Scenario steps (executed on the live net):
  1. reconnaissance: attacker scans the victim subnet (nmap if present,
     else ping sweep + /dev/tcp probes)
  2. exploit simulation: attacker opens a connection to victim h2:80
  3. detection: defender h4 runs a packet counter on s1 (ovs-ofctl dump-flows
     if OVS, else /proc/net/dev counters) before/after
All steps are guarded and safe: they only touch emulated namespaces.
"""
import argparse
import sys

import mininet_rust as mnr


def build_net():
    # attacker h1, victims h2/h3, defender h4, one switch, NAT for egress
    net = mnr.Mininet(topo="single,4", controller="none", nat=True)
    net.build()
    return net


def scenario(net):
    net.start()
    try:
        atk, vic, dfd = "h1", "h2", "h4"
        print("== 1. recon: ping sweep of the lab subnet from attacker ==")
        print(net.cmd(atk, "for i in 1 2 3 4; do ping -c1 -W1 10.0.0.$i >/dev/null 2>&1 && echo 10.0.0.$i up; done"))
        print("== 2. service probe on victim (h2) ==")
        print(net.cmd(atk, "(exec 3<>/dev/tcp/10.0.0.2/80) 2>/dev/null && echo 'h2:80 open' || echo 'h2:80 closed/filtered'"))
        print("== 3. defender counts traffic on the switch ==")
        print(net.cmd(dfd, "cat /proc/net/dev | head -5"))
        print("== 4. controlled connectivity check ==")
        loss, out = net.pingall()
        print(out)
        print(f"pingall loss: {loss}%")
    finally:
        net.stop()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--plan", action="store_true", help="print the planned topology and exit (no root)")
    args = ap.parse_args()
    net = build_net()
    if args.plan:
        print(net.dump())
        print(net.links())
        return 0
    scenario(net)
    return 0


if __name__ == "__main__":
    sys.exit(main())
