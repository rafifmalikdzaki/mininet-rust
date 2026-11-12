# Behavioral baseline fixtures (Task 2, `tasks/plan.md`)

Golden outputs captured from the **original, unmodified Python Mininet** so the
Rust port can be differentially tested against them. Nothing outside
`fixtures/` and `docs/migration/` was created or modified.

## Provenance (common to all fixtures)

- Repo: `/home/dzakirm/Research/networking/mininet-rust`
- Branch / commit: `rust-migration` @ `18681cd5c6fa10056b1e14c29e73777d41c7719b`
  ("Scaffold Rust workspace (Task 3)", 2026-11-12) — the Python tree at this
  commit is pristine upstream Mininet 2.3.1b4 (`VERSION = "2.3.1b4"`,
  `mininet/net.py:112`).
- Capture date: **2026-11-12** (UTC)
- Interpreter: **Python 3.14.9** (`3.14.9 (main, Oct 25 2026, 14:52:47)
  [GCC 16.2.1 20260810]`), invoked as `python3`
- Host: Linux 7.2.14-zen1-1-zen x86_64, non-root sandbox (no netns needed for
  these fixtures — they only exercise pure Python code paths)
- Working directory for every command: the repo root
  (`/home/dzakirm/Research/networking/mininet-rust`)

## Fixture: `topo_single3.txt` (678 bytes)

Exact command (run from the repo root; output redirected to the fixture):

```bash
python3 -c '
from mininet.util import buildTopo
from mininet.topo import SingleSwitchTopo, LinearTopo, SingleSwitchReversedTopo, MinimalTopo
from mininet.topolib import TreeTopo, TorusTopo
TOPOS = { "minimal": MinimalTopo, "linear": LinearTopo, "reversed": SingleSwitchReversedTopo, "single": SingleSwitchTopo, "tree": TreeTopo, "torus": TorusTopo }
topo = buildTopo( TOPOS, "single,3" )
print( "buildTopo arg:", "single,3" )
print( "topo class:", type( topo ).__name__ )
print( "nodes():", topo.nodes() )
print( "hosts():", topo.hosts() )
print( "switches():", topo.switches() )
for n in topo.nodes():
    print( "nodeInfo(%s):" % n, topo.nodeInfo( n ) )
print( "links(sort=True, withKeys=True, withInfo=True):" )
for link in topo.links( sort=True, withKeys=True, withInfo=True ):
    print( "  ", link )
print( "ports:", topo.ports )
for ( src, dst ) in topo.links( sort=True ):
    print( "port(%s, %s):" % ( src, dst ), topo.port( src, dst ) )
' > fixtures/topo_single3.txt
```

This uses the same entry path `bin/mn` uses for `--topo single,3`
(`buildTopo(TOPOS, opts.topo)` with the `TOPOS` dict copied verbatim from
`bin/mn`) and the same `Topo` accessors `Mininet.buildFromTopo()` consumes
(`hosts()`, `switches()`, `links(sort=True, withInfo=True)`, `nodeInfo()`),
plus `ports` / `port(src, dst)` for the port-numbering contract.

## Fixture: `topo_linear4.txt` (1364 bytes)

Identical command to the above with the topo string changed to `"linear,4"`
(every `"single,3"` → `"linear,4"`, including the `buildTopo` call and the
printed `buildTopo arg` line), redirected to `fixtures/topo_linear4.txt`.
Equivalent to `mn --topo linear,4` → `LinearTopo(k=4, n=1)`.

## Fixture: `topo_tree.txt` (1185 bytes)

Identical command with the topo string changed to `"tree,2,2"`, redirected to
`fixtures/topo_tree.txt`. Equivalent to `mn --topo tree,2,2` →
`TreeTopo(depth=2, fanout=2)` (from `mininet/topolib.py`).

## Fixture: `util_cases.txt` (2659 bytes)

Exact command (run from the repo root):

```bash
python3 -c '
from mininet.util import ( macColonHex, ipStr, ipNum, ipAdd, ipParse, netParse,
                           natural, naturalSeq, splitArgs, makeNumeric,
                           unitScale, fmtBps, irange )
print( "== macColonHex ==" )
for v in [ 0, 1, 0xffffffffffff, 0x000102030405, 0x0a00000001, 0xdeadbeef1234 ]:
    print( "macColonHex(%#x) = %s" % ( v, macColonHex( v ) ) )
print( "== ipStr / ipNum ==" )
for v in [ 0, 0x0a000001, 0x0a000000, 0xffffffff, 0xc0a80001 ]:
    s = ipStr( v )
    print( "ipStr(%#x) = %s ; ipNum(%s) = %#x" % ( v, s, s, ipNum( *[ int( x ) for x in s.split( "." ) ] ) ) )
print( "== ipAdd ==" )
print( "ipAdd(1) =", ipAdd( 1 ) )
print( "ipAdd(2) =", ipAdd( 2 ) )
print( "ipAdd(255) =", ipAdd( 255 ) )
print( "ipAdd(1, prefixLen=16) =", ipAdd( 1, prefixLen=16 ) )
print( "ipAdd(65535, prefixLen=16) =", ipAdd( 65535, prefixLen=16 ) )
print( "ipAdd(1, prefixLen=24, ipBaseNum=0xc0a80000) =", ipAdd( 1, prefixLen=24, ipBaseNum=0xc0a80000 ) )
print( "== ipParse ==" )
for s in [ "10.0.0.1", "10.1", "127.0.0.1", "192.168.1.100", "0.0.0.0", "255.255.255.255" ]:
    print( "ipParse(%r) = %#x" % ( s, ipParse( s ) ) )
print( "== netParse ==" )
for s in [ "10.0.0.0/8", "192.168.0.0/16", "10.1.2.3", "172.16.5.0/12" ]:
    addr, plen = netParse( s )
    print( "netParse(%r) = (%#x, %d)" % ( s, addr, plen ) )
print( "== natural sort ==" )
sample = [ "h10", "h2", "h1", "s10", "s2", "s1", "h1-eth10", "h1-eth2", "c0", "10.0.0.2", "10.0.0.10" ]
print( "input:", sample )
print( "sorted(key=natural):", sorted( sample, key=natural ) )
pairs = [ ("h2", "s1"), ("h10", "s1"), ("h1", "s1"), ("s2", "h10") ]
print( "pairs sorted by naturalSeq:", sorted( pairs, key=naturalSeq ) )
print( "== splitArgs ==" )
for s in [ "single,3", "linear,4", "tree,2,2", "ovs,protocols=OpenFlow13", "tc,bw=10,delay=5ms,loss=0.1", "none", "rt,sched=rt,cores=0-1" ]:
    print( "splitArgs(%r) =" % s, splitArgs( s ) )
print( "== makeNumeric ==" )
print( "makeNumeric:", [ ( s, makeNumeric( s ), type( makeNumeric( s ) ).__name__ ) for s in [ "10", "0.5", "5ms", "abc", "-3" ] ] )
print( "== unitScale / fmtBps ==" )
for bps in [ 0, 999, 1000, 999999, 10**6, 1.5 * 10**8, 10**9, 2.5 * 10**9, 10**12 ]:
    print( "fmtBps(%s) = %s" % ( bps, fmtBps( bps ) ) )
print( "fmtBps(10**9, prefix=\"M\") =", fmtBps( 10**9, prefix="M" ) )
print( "unitScale(500) =", unitScale( 500 ) )
print( "unitScale(10**9) =", unitScale( 10**9 ) )
print( "unitScale(2500, prefix=\"k\") =", unitScale( 2500, prefix="k" ) )
print( "== irange ==" )
print( "list(irange(1, 5)) =", list( irange( 1, 5 ) ) )
print( "list(irange(3, 3)) =", list( irange( 3, 3 ) ) )
' > fixtures/util_cases.txt
```

Covers the pure helpers of `mininet/util.py` on fixed inputs: `macColonHex`,
`ipStr`/`ipNum`, `ipAdd`, `ipParse` (including the short-form quirk
`ipParse("10.1") == ipParse("10.0.0.1")`), `netParse`, `natural`/`naturalSeq`
sorting, `splitArgs`, `makeNumeric`, `unitScale`/`fmtBps`, `irange`.

## Fixture: `test_util_out.txt` (101 bytes)

Exact command (run from the repo root, stdout+stderr merged, exit code 0):

```bash
PYTHONPATH=. python3 mininet/test/test_util.py > fixtures/test_util_out.txt 2>&1
```

Captured content:

```
...
----------------------------------------------------------------------
Ran 3 tests in 0.010s

OK
```

Notes:

- `PYTHONPATH=.` is required: on Python 3.14 running a script by path puts the
  *script's* directory (`mininet/test/`) on `sys.path`, not the cwd, so a bare
  `python3 mininet/test/test_util.py` from the repo root fails with
  `ModuleNotFoundError: No module named 'mininet'`. (CI avoids this by
  installing the package via `util/install.sh` first.) This is itself part of
  the baseline behavior.
- unittest writes its report to stderr, hence `2>&1`.
- The `Ran 3 tests in X.XXXs` line contains wall-clock time and is the **only**
  non-deterministic line; re-runs are otherwise byte-identical and always end
  in `OK` with exit code 0.

## Reproducibility check (performed 2026-11-12)

Each fixture command was executed **twice** and the two runs diffed, then
diffed against the saved fixture:

- `topo_single3.txt`, `topo_linear4.txt`, `topo_tree.txt`, `util_cases.txt`:
  two runs **byte-identical** and identical to the saved fixture
  (`diff` empty). All non-empty (678 / 1364 / 1185 / 2659 bytes).
- `test_util_out.txt`: two runs identical except the `in X.XXXs` timing
  value; both runs `OK`, exit 0 (101 bytes saved).

## Intended use

- `topo_*.txt` — characterization tests for the Rust `crates/topo` port
  (node/edge counts, link keys, port numbering, `port()` lookups, natural
  sort order of `links(sort=True)`).
- `util_cases.txt` — differential tests for the Rust `crates/util` port of
  the pure helpers.
- `test_util_out.txt` — baseline for the `quietRun`/subprocess-reading
  behavior when `crates/util` gains an exec helper.
