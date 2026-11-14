"""Smoke tests for the mininet_rust bindings (run: python3 -m pytest bindings/python/test_mininet_rust.py).

Non-root only: exercises the pure/dry-run surface. Root behavior is covered by
the Rust root-gated suites (MN_RUST_ROOT_TESTS=1).
"""
import mininet_rust as mnr


def test_version():
    assert "2.3.1b4" in mnr.version()


def test_build_single3():
    net = mnr.Mininet(topo="single,3", controller="none")
    net.build()
    assert net.hosts() == ["h1", "h2", "h3"]
    assert net.switches() == ["s1"]
    assert "h1" in net.dump()
    assert "s1-eth1" in net.links()


def test_nat_adds_nat0():
    net = mnr.Mininet(topo="single,2", controller="none", nat=True)
    assert "nat0" in net.hosts()
    assert "nat0-eth0" in net.links()


def test_plan_dump():
    plan = mnr.plan("tree,2,2")
    assert "=== topo ===" in plan and "tree" in plan.lower()


def test_bad_topo_raises():
    try:
        mnr.Mininet(topo="bogus")
    except RuntimeError as e:
        assert "bogus" in str(e)
    else:
        raise AssertionError("expected RuntimeError")
