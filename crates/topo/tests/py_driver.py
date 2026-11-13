"""Differential-test driver for the mn-topo Rust port.

Reads a JSON array of cases from stdin and writes a JSON array of results to
stdout. Each case is one of:

  {"fn": "topo", "spec": "single,3"}                          build via buildTopo(TOPOS, spec)
  {"fn": "make", "topo": "single", "args": [4], "kwargs": {...}}  TOPOS[name](*args, **kwargs)
  {"fn": "graph", "ops": [{"op": "add_node", ...}, ...]}      raw MultiGraph ops

Each result is {"ok": "<canonical text dump>"} or {"error": "<ExceptionClass>"}.
The text dumps are byte-identical to the dumps produced by the Rust test
harness (tests/common/mod.rs), which mirror Python repr() formatting.
"""

import json
import os
import sys

# Make the repo's mininet package importable regardless of cwd.
sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from mininet.util import buildTopo  # noqa: E402
from mininet.topo import (  # noqa: E402
    LinearTopo,
    MinimalTopo,
    MultiGraph,
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


def dump_topo(topo):
    lines = []
    lines.append("class: %s" % type(topo).__name__)
    lines.append("nodes: %r" % topo.nodes())
    lines.append("nodes_unsorted: %r" % list(topo.nodes(sort=False)))
    lines.append("hosts: %r" % topo.hosts())
    lines.append("switches: %r" % topo.switches())
    for n in topo.nodes(sort=False):
        lines.append("nodeInfo(%s): %r" % (n, topo.nodeInfo(n)))
    lines.append("links_ski:")
    for link in topo.links(sort=True, withKeys=True, withInfo=True):
        lines.append("  %r" % (link,))
    lines.append("links_unsorted:")
    for link in topo.links(sort=False, withKeys=True, withInfo=True):
        lines.append("  %r" % (link,))
    lines.append("links_sort_info:")
    for link in topo.links(sort=True, withInfo=True):
        lines.append("  %r" % (link,))
    lines.append("links_keys:")
    for link in topo.links(withKeys=True):
        lines.append("  %r" % (link,))
    lines.append("links_plain:")
    for link in topo.links():
        lines.append("  %r" % (link,))
    lines.append("ports: %r" % topo.ports)
    for (src, dst) in topo.links(sort=True):
        lines.append("port(%s, %s): %r" % (src, dst, topo.port(src, dst)))
    for attr in ("k", "n", "hostNum", "switchNum"):
        if hasattr(topo, attr):
            lines.append("%s: %r" % (attr, getattr(topo, attr)))
    return "\n".join(lines)


def dump_graph(g, keys_returned):
    lines = []
    lines.append("len: %d" % len(g))
    lines.append("nodes: %r" % list(g.nodes()))
    lines.append("nodes_data: %r" % list(g.nodes(data=True)))
    lines.append("edges_dk: %r" % g.edges(data=True, keys=True))
    lines.append("edges: %r" % g.edges())
    lines.append("edges_k: %r" % g.edges(keys=True))
    lines.append("edges_d: %r" % g.edges(data=True))
    lines.append("keys_returned: %r" % keys_returned)
    return "\n".join(lines)


def run_case(case):
    fn = case["fn"]
    try:
        if fn == "topo":
            topo = buildTopo(TOPOS, case["spec"])
            return {"ok": dump_topo(topo)}
        if fn == "make":
            if case["topo"] not in TOPOS:
                raise Exception("Invalid topo name %s" % case["topo"])
            cls = TOPOS[case["topo"]]
            topo = cls(*case.get("args", []), **case.get("kwargs", {}))
            return {"ok": dump_topo(topo)}
        if fn == "graph":
            g = MultiGraph()
            keys = []
            for op in case["ops"]:
                if op["op"] == "add_node":
                    g.add_node(op["node"], op.get("attrs"), **op.get("extra", {}))
                elif op["op"] == "add_edge":
                    keys.append(
                        g.add_edge(
                            op["src"], op["dst"], op.get("key"), op.get("attrs"),
                            **op.get("extra", {})
                        )
                    )
                else:
                    raise Exception("unknown op %s" % op["op"])
            return {"ok": dump_graph(g, keys)}
        raise Exception("unknown fn %s" % fn)
    except Exception as exc:  # noqa: BLE001 - report any Python exception
        return {"error": type(exc).__name__}


def main():
    cases = json.load(sys.stdin)
    json.dump([run_case(c) for c in cases], sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
