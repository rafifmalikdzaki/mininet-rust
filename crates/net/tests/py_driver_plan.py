#!/usr/bin/env python3
"""Differential-test driver for the mn-net Rust port (task 9).

Reads topo specs on stdin, one per line, each optionally followed by an
ipBase:  `<spec>[ <ipbase>]`   e.g. `single,3` or `linear,4 192.168.0.0/16`

For each input line it prints on stdout:

    ===CASE <spec>|<ipbase>===
    <the exact dry-run plan text>
    ===END===

or `===ERROR <ExceptionClass>===` between the markers on failure.

The plan is computed PURELY from the repo's own mininet topo/util code (no
root, no kernel, no Mininet object — `Mininet.__init__` calls ensureRoot):
`buildTopo` for the topology, the same `addHost` IP-assignment arithmetic
net.py uses (`ipAdd(nextIP, ipBaseNum, prefixLen)` with
`nextIP = ((0xffffffff >> prefixLen) & ipBaseNum) or 1`), the topo's own
port table for port numbering/intf names, and the exact repr formats of
cli.py `do_dump`/`do_links` (as `scripts/diff_topology.sh`'s reference
driver prints them). The Rust side renders the same text via
`Mininet::dry_run(spec).plan()` and the two are compared byte-for-byte.
"""

import os
import sys

# Repo root is four levels up: crates/net/tests/py_driver_plan.py -> repo root.
sys.path.insert(
    0,
    os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ),
)

from mininet.util import buildTopo, ipAdd, netParse, natural  # noqa: E402
from mininet.topo import (  # noqa: E402
    LinearTopo,
    MinimalTopo,
    SingleSwitchReversedTopo,
    SingleSwitchTopo,
)
from mininet.topolib import TorusTopo, TreeTopo  # noqa: E402

# Same TOPOS dict as bin/mn.
TOPOS = {
    "minimal": MinimalTopo,
    "linear": LinearTopo,
    "reversed": SingleSwitchReversedTopo,
    "single": SingleSwitchTopo,
    "tree": TreeTopo,
    "torus": TorusTopo,
}

DEFAULT_IPBASE = "10.0.0.0/8"


def plan(spec, ipbase=DEFAULT_IPBASE):
    """Return the dry-run plan text for `spec` (mirrors Rust mn-net)."""
    topo = buildTopo(TOPOS, spec)
    ipBaseNum, prefixLen = netParse(ipbase)
    hostIP = (0xFFFFFFFF >> prefixLen) & ipBaseNum
    nextIP = hostIP if hostIP > 0 else 1

    out = []
    out.append("=== topo ===")
    out.append("spec: %s" % spec)
    out.append("class: %s" % type(topo).__name__)
    out.append("nodes: %s" % " ".join(topo.nodes()))
    out.append("hosts: %s" % " ".join(topo.hosts()))
    out.append("switches: %s" % " ".join(topo.switches()))

    links = []
    intfs = {}  # node -> [(port, intf name)] — the net's registered intfs
    for src, dst, info in topo.links(sort=True, withInfo=True):
        p1, p2 = info["port1"], info["port2"]
        links.append("%s-eth%d<->%s-eth%d" % (src, p1, dst, p2))
        intfs.setdefault(src, []).append((p1, "%s-eth%d" % (src, p1)))
        intfs.setdefault(dst, []).append((p2, "%s-eth%d" % (dst, p2)))
    out.append("links: %s" % " ".join(sorted(links, key=natural)))

    out.append("=== dump ===")
    # buildFromTopo adds hosts in topo.hosts() order; addHost assigns IPs
    # sequentially; configHosts sets the IP on the default (lowest-port) intf.
    for h in topo.hosts():
        ip = ipAdd(nextIP, ipBaseNum=ipBaseNum, prefixLen=prefixLen)
        nextIP += 1
        entries = sorted(intfs.get(h, []))
        first_port = entries[0][0] if entries else None
        parts = [
            "%s:%s" % (name, ip if port == first_port else None)
            for port, name in entries
        ]
        out.append("<Host %s: %s pid=None> " % (h, ",".join(parts)))
    for s in topo.switches():
        entries = sorted(intfs.get(s, []))
        # root-namespace switch: control intf lo (port 0, ip 127.0.0.1)
        parts = ["lo:127.0.0.1"] + ["%s:None" % name for _, name in entries]
        out.append("<Switch %s: %s pid=None> " % (s, ",".join(parts)))

    out.append("=== links ===")
    for src, dst, info in topo.links(sort=True, withInfo=True):
        out.append(
            "%s-eth%d<->%s-eth%d (MISSING MISSING)"
            % (src, info["port1"], dst, info["port2"])
        )
    return "\n".join(out) + "\n"


def main():
    for line in sys.stdin:
        line = line.rstrip("\n")
        if not line:
            continue
        parts = line.split(" ")
        spec = parts[0]
        ipbase = parts[1] if len(parts) > 1 else DEFAULT_IPBASE
        print("===CASE %s|%s===" % (spec, ipbase))
        try:
            sys.stdout.write(plan(spec, ipbase))
        except Exception as e:  # noqa: BLE001 - report the class like the topo driver
            print("===ERROR %s===" % type(e).__name__)
        print("===END===")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
