# ARCHITECTURE.md — Current Python System (Mininet 2.3.1b4)

Discovery document for Task 1 of `tasks/plan.md` (Phase 0). It describes the
**current, unmodified Python implementation** on branch `rust-migration`
(commit `18681cd5c6fa10056b1e14c29e73777d41c7719b`), as read from the source
code on 2026-11-12 (not from the README). The Python tree is the reference
implementation for differential testing during the Rust migration and must stay
untouched.

- Version string: `VERSION = "2.3.1b4"` (`mininet/net.py:112`), also what
  `bin/mn --version` prints and what the Makefile bakes into `mnexec -v`.
- Runtime used for the behavioral fixtures: CPython 3.14.9 (see
  `fixtures/README.md`).

## 1. Top-level layout

```
mininet/            Python package (the system under migration)
  examples -> ../examples   (symlink; cluster mode + demos live in examples/)
  __main__.py -> ../bin/mn  (symlink; `python3 -m mininet` == `bin/mn`)
bin/mn              CLI entry point (451 lines, installed by setup.py as a script)
mnexec.c            C helper binary (227 lines, built by `make mnexec`)
mininet/test/       core unittest suite (7 files, 977 LOC)
examples/test/      example regression tests (runner + 27 files)
.github/            issue templates (no CI pipeline)
util/               install.sh, versioncheck.py, vm/ images, patches
Makefile            codecheck / test / slowtest / mnexec / install targets
setup.py            setuptools packaging; version read from mininet.net.VERSION
debian/             Debian packaging
```

## 2. Module inventory (`mininet/*.py`)

Line counts from `wc -l` on 2026-11-12. Package modules total **5,517 LOC**
(excluding the `__main__.py` symlink, which `wc` double-counts as `bin/mn`).

| File | LOC | Responsibility |
|---|---:|---|
| `log.py` | 172 | Logging. Custom `OUTPUT = 25` level between INFO and WARNING, `LEVELS` dict (`debug/info/output/warning/warn/error/critical`), `StreamHandlerNoNewline`, singleton logger `lg`, `setLogLevel()`. No mininet imports — base of the graph. |
| `util.py` | 741 | Process/command plumbing + pure helpers. Subprocess wrappers `run/checkRun/oldQuietRun/errRun/errFail/quietRun/which/isShellBuiltin` (poll()-based reading, `CmdResult` namedtuple); veth creation `makeIntfPair()` / `moveIntf()` via `ip link`; dump helpers (`dumpNodeConnections/dumpNetConnections/dumpPorts`); MAC/IP codec + parsing (`macColonHex`, `ipStr`, `ipNum`, `ipAdd`, `ipParse`, `netParse`); natural sort (`natural`, `naturalSeq`); arg-string parsing (`splitArgs`, `customClass`, `specialClass`, `buildTopo`); `pmonitor`; resource limits (`fixLimits`, `sysctlTestAndSet`, `rlimitTestAndSet`, `mountCgroups`); `ensureRoot`, `waitListening`, `unitScale`/`fmtBps`; pexpect compatibility shim. Imports only `mininet.log`. |
| `topo.py` | 358 | Topology model. `MultiGraph` (in-house replacement for `networkx.MultiGraph`: `add_node/add_edge/nodes/edges_iter/edges`), and `Topo` (`addNode/addHost/addSwitch/addLink`, port allocation `addPort`/`port`, `links/iterLinks`, `nodeInfo`, `hosts/switches/isSwitch`, natural sorting). Built-in topologies: `SingleSwitchTopo`, `SingleSwitchReversedTopo`, `MinimalTopo`, `LinearTopo`. Pure data structure — no subprocess, no root. Imports `irange/natural/naturalSeq` from `util`. |
| `topolib.py` | 84 | Extra topologies: `TreeTopo` (depth/fanout recursive `addTree`), `TorusTopo` (x×y grid with loops, warns about STP), `TreeNet()` convenience. Imports `topo` and `net`. |
| `moduledeps.py` | 72 | Kernel-module and binary checks: `lsmod/rmmod/modprobe`, `moduleDeps()` (loads `openvswitch`/`ofdatapath`/`tun`), `pathCheck()` (verifies required binaries exist, e.g. `ovs-vsctl`, `mnexec`, `ifconfig`). Imports `util`, `log`. |
| `term.py` | 81 | Per-node terminals via `socat` + `xterm`/`gnome-terminal` (`makeTerms`, `runX11`, `tunnelX11`), `cleanUpScreens()`. Imports `log`, `util`. |
| `link.py` | 582 | Interfaces and links. `Intf` (self-configuring interface: `setIP/setMAC` via `ifconfig`, `rename` via `ip link set`, `isUp`, `config`, `delete` via `ip link del`, addr parsing from `ifconfig` output); `TCIntf` (traffic control: `tc qdisc` htb/hfsc/tbf for bw, netem for delay/jitter/loss/queue, RED/ECN, `ethtool -K` offload off); `Link` (veth pair between two nodes, port numbering, `intfName` = `<node>-eth<N>`); `OVSIntf`, `OVSLink` (OVS-owned ports), `TCLink`, `TCULink` (user-switch variant). Imports `log`, `util.makeIntfPair`. |
| `node.py` | 1,617 | Node classes — the kernel-coupled core. `Node` base: spawns a bash shell on a pty via `mnexec -cd[n] env PS1=\x7f bash --norc --noediting -is mininet:<name>`; command I/O protocol `cmd/sendCmd/pexec/monitor/waitOutput` (sentinel `chr(127)` prompt matching, `mnexec -p` pid parsing); `privateDirs` bind mounts; `config()` (IP/MAC/default route via `ifconfig`/`ip route`). Subclasses: `Host`, `CPULimitedHost` (cgroup cfs/rt via `cgcreate/cgclassify/chrt`), `Switch`, `UserSwitch` (`dpctl`/`ofprotocol`/`ofdatapath`), `OVSSwitch`/`OVSKernelSwitch` (one big `ovs-vsctl` command, `ovs-ofctl`, dpid handling), `OVSBridge`, `IVSSwitch` (`ivs-ctl`), `Controller`, `OVSController`, `NOX`, `Ryu`, `RemoteController`, `DefaultController`, `NullController`, `findController()`. Imports `log`, `util`, `moduledeps`, `link`. |
| `nodelib.py` | 154 | Additional node types: `LinuxBridge` (switch backed by `brctl addbr/addif/stp/setbridgeprio`, `pathCheck('brctl')`) and `NAT` (NAT node: `iptables` filter+nat rules, default route via `ip route`, `ifconfig` on nat0-eth0, sysctl ip_forward). Imports `node`, `moduledeps`, `util`, `log`. |
| `net.py` | 1,023 | Orchestrator. `Mininet(topo, switch, host, controller, link, ipBase, ...)`: `addHost/addSwitch/addController/addNAT/addLink/delNode...`, `buildFromTopo()` (iterates `topo.hosts()/switches()/links(sort=True, withInfo=True)` + `nodeInfo`), `configHosts()`, `build()`, `start()` (controllers → switches, incl. `batchStartup`, `waitConnected`), `stop()` (links → switches → hosts), tests/measurements `ping/pingAll/pingPair/pingFull/iperf/runCpuLimitTest/monitor/configLinkStatus`, `interact()` → `CLI`, `randMac()`, `MininetWithControlNet` subclass (control network on `192.168.123.1/24` subnet, routed variant). Imports `cli`, `node`, `nodelib`, `link`, `util`, `term`, `log`. |
| `cli.py` | 503 | Interactive CLI (`class CLI(Cmd)`, prompt `mininet> `). Commands: `nodes ports net sh py px pingall pingpair pingallfull pingpairfull iperf iperfudp intfs dump link xterm x gterm exit quit EOF noecho source dpctl time links switch wait help`; substitutes node names with IPs in host commands; `CLI(mn, script=...)` for `--pre/--post`. Imports `log`, `term`, `util`. |
| `clean.py` | 129 | Cleanup (`mn --clean`): `killprocs()` (pkill patterns for controller/OVS/ofprotocol/...), `ovs-vsctl del-br` per bridge, `ip link del` for leftover mininet veths, unmount private dirs, `cleanUpScreens()`. All via `sh()` = `/bin/sh -c`. Imports `log`, `term`, `util`. |
| `__init__.py` | 1 | Docstring only (pylint silencer). |
| `__main__.py` | — | Symlink to `../bin/mn` (so `python3 -m mininet ...` works). |

## 3. `mnexec.c` (227 lines) and `bin/mn` (451 lines)

**`mnexec.c`** — small C helper compiled by `make mnexec`
(`cc -Wall -Wextra -DVERSION="<mn --version>" mnexec.c -o mnexec`) that does the
things that are slow or impossible in Python before `execvp()` of the target
command:

- `-c` close all fds > 2 (scan `/proc/self/fd`)
- `-d` detach from controlling tty (`setsid()`, fork if process-group leader)
- `-n` `unshare(CLONE_NEWNET|CLONE_NEWNS)`, remount `/` private, mount sysfs
- `-p` print `\x01<pid>\n` (sentinel `Node.startShell`/`pexec` use to find pids)
- `-a pid` `setns()` into `/proc/<pid>/ns/net` and `ns/mnt` (chroot fallback),
  then `chdir` back to the original cwd — this is how commands run *inside* a
  node's namespace
- `-g group` write pid into cgroup `tasks` files (cpu/cpuacct/cpuset)
- `-r rtprio` `SCHED_RR` via `sched_setscheduler`

**`bin/mn`** — the user-facing CLI (also reachable as `python3 -m mininet` and
installed as the `mn` script by `setup.py`). `MininetRunner`:

1. `parseArgs()` — `optparse.OptionParser` with dict-driven choices:
   `--topo` (`TOPOS`: minimal/linear/reversed/single/tree/torus),
   `--switch` (`user/ovs/ovsbr/ovsk/ivs/lxbr/default`), `--host` (`proc/rt/cfs`),
   `--controller` (`ref/ovsc/nox/remote/ryu/default/none`, append),
   `--link` (`default/tc/tcu/ovs`), plus `--clean --custom --test --xterms
   --ipbase --mac --arp --verbosity --innamespace --listenport --nolistenport
   --pre --post --pin --nat --version --wait --twait --cluster --placement`.
2. `begin()` — `buildTopo(TOPOS, opts.topo)` (e.g. `--topo single,3` →
   `splitArgs("single,3")` → `SingleSwitchTopo(3)`), `customClass()` for
   switch/host/controller/link, `Mininet(topo=..., switch=..., host=...,
   controller=..., link=..., ipBase=..., ...)`, optional `addNAT()`,
   `mn.start()`, then either `runTests()` (`--test pingall+iperf...`, with
   `ALTSPELLING` mapping to `Mininet` methods) or the interactive `CLI(mn)`,
   then `mn.stop()`. `--clean` → `cleanup()` and exit. `KeyboardInterrupt` /
   any exception → `cleanup()`.

## 4. Test suite

`mininet/test/` (977 LOC total), discovered and run by `runner.py`
(`ensureRoot()` + `cleanup()` first, `unittest.defaultTestLoader.discover`):

| File | LOC | What it covers | Needs root? |
|---|---:|---|---|
| `runner.py` | 34 | Test discovery/runner driver (`-v`, `-quick`) | yes (calls ensureRoot) |
| `test_util.py` | 40 | `quietRun()` output-length behavior via `echo -n` (3 tests: empty, one-read, multiple-reads) — **pure, runs without root** | no |
| `test_nets.py` | 108 | All-pairs ping for each topo × switch type (user/ovs/ivs) | yes |
| `test_hifi.py` | 269 | Topologies with link (bw/delay/loss) and CPU-limit options | yes |
| `test_ptyleak.py` | 32 | Regression: no pty leak across addHost/delHost cycles | yes |
| `test_switchdpidassignment.py` | 102 | Regression: switch dpid assignment (default/explicit/zero) | yes |
| `test_walkthrough.py` | 392 | End-to-end walkthrough incl. `tshark -v` version check, pexpect-driven `mn` session | yes |

`examples/test/` adds 27 more root-requiring example tests + its own `runner.py`
(run by `make slowtest` / CI). `make test` runs `test_nets.py` + `test_hifi.py`
only. Only `test_util.py` is runnable in the non-root sandbox — it is captured
as a golden fixture (`fixtures/test_util_out.txt`).

## 5. Quality checks (local)

Run `just check` (format + clippy `-D warnings`) and `just test` (workspace
tests) locally. Kernel-facing tests run with `just root-tests-docker` or
`just root-tests-host` on a privileged Linux host. No GitHub Actions pipeline
is configured.

## 6. External command surface (shell-outs)

The Python code never talks netlink directly; it shells out. Grep for
string-literal subprocess call sites
(`(self.)?(cmd|cmdPrint|pexec|quietRun|errRun|errFail|checkRun|oldQuietRun)('...`)
counts **103 sites**: `node.py` 47, `nodelib.py` 29, `net.py` 9, `cli.py` 6,
`link.py` 6, `util.py` 3, `moduledeps.py` 2, `term.py` 1 — plus list-form calls
(e.g. `self.cmd('ip', 'link', 'set', ...)`) and ~15 `sh()` calls in `clean.py`.
(The plan's "~75 sites, 53 in node.py" counts a narrower pattern; the counts
above are the reproducible grep.)

Commands by area:

| Area | Commands (as issued) | Where |
|---|---|---|
| Namespace/process spawn | `mnexec -cd[n] env PS1=\x7f bash --norc --noediting -is mininet:<name>`; `mnexec -p <cmd>`; `mnexec -da <pid> <cmd>`; `mnexec -g <name> ...` | `node.py` (Node.startShell, pexec, cmd, CPULimitedHost) |
| veth / link mgmt | `ip link add name %s [address %s] type veth peer name %s [address %s] netns %s`; `ip link del %s`; `ip link set %s netns %s`; `ip link set %s name %s`; `ip link show` | `util.makeIntfPair/moveIntf`, `link.py` |
| Interface config | `ifconfig <intf> <ip>/<plen> up`; `ifconfig <intf> down|up`; `ifconfig <intf> hw ether <mac>`; `ifconfig lo up`; `ifconfig` / `ifconfig <intf>` (parsed for IP/MAC); `ip route del default; ip route add default via ...` | `link.py` Intf, `node.py` Node.config/Host |
| Traffic control | `tc qdisc add dev %s root handle 5:0 htb default 1`; `tc class add ... htb rate %fMbit burst 15k` (also `hfsc sc rate ... ul rate ...`, `tbf rate ... burst 15000 latency ...`); RED/ECN `qdisc ... handle 6:`; `tc qdisc add ... handle 10: netem delay %s %s loss %.5f limit %d`; `ethtool -K %s gro off txo off rxo off` | `link.py` TCIntf.bwCmds/delayCmds/config |
| Open vSwitch | `ovs-vsctl -t 1 show`; `ovs-vsctl --version`; `ovs-vsctl -- get Bridge <br> ...`; `ovs-vsctl del-br <br>`; one-shot `ovs-vsctl -- add-br ... -- add-port ... -- set-controller ...` batch (node.py:1273); `ovs-ofctl <args> <br> ...` | `node.py` OVSSwitch, `cli.py` do_dpctl, `clean.py` |
| User/IVS datapath | `dpctl ...`, `ofprotocol unix:/tmp/...`, `ofdatapath -i ...`, `ivs-ctl show/...` | `node.py` UserSwitch/IVSSwitch |
| Linux bridge | `brctl addbr/delbr/addif/stp <on|off>/setbridgeprio/showstp` | `nodelib.py` LinuxBridge |
| NAT / firewall | `iptables -F`, `-t nat -F`, `-P INPUT/OUTPUT/FORWARD ...`, `-I/-A/-D FORWARD ...`, `-t nat -A/-D POSTROUTING ...`; `sysctl -w net.ipv4.ip_forward=1` | `nodelib.py` NAT |
| Cleanup | `killall[-9]`, `pkill -f`, `kill -9`, `ovs-vsctl --timeout=1 list-br` + `del-br`, `ip link del`, `umount`, `mn` leftover scan | `clean.py` |
| Misc | `mount --bind`, `mount -n -t tmpfs`, `umount`, `mkdir -p`, `chrt -p`, `cgcreate/cgclassify/cgdelete`, `cgroupfs-mount`, `lsmod/rmmod/modprobe`, `ping -c1`, `iperf`, `netstat -natp`, `nproc`, `route`, `arp`, `stty`, `renice`, `which`, `bash -c enable`, `socat`/`xterm`/`gnome-terminal` (terms), `xhost` (X11), `tshark -v` (walkthrough test), `grep -c processor /proc/cpuinfo` | various |

**Migration relevance:** the `ip`/`tc`/`ifconfig` surface (veth pairs, addr
assignment, qdiscs) is what the Rust port replaces with `rtnetlink`; OVS
control (`ovs-vsctl`/`ovs-ofctl`) stays a shell-out initially (per
`tasks/plan.md`); `mnexec` is replaced by a `nix`-based `setns`/`unshare`
helper.

## 7. Dependency graph

Actual import edges (X → Y = "X imports Y"), from the source:

```
bin/mn (= mininet/__main__.py) ─► clean, cli, log, net, node, nodelib,
                                 link, topo, topolib, util,
                                 examples.cluster, examples.clustercli
topolib  ─► topo, net
net      ─► cli, log, node, nodelib, link, util, term
nodelib  ─► node, moduledeps, util, log
node     ─► link, moduledeps, util, log
cli      ─► log, term, util
clean    ─► log, term, util
term     ─► log, util
link     ─► log, util
moduledeps ─► util, log
topo     ─► util
util     ─► log
log      ─► (stdlib only)
```

Layered view (bottom = most depended-upon):

```
Layer 0  log
Layer 1  util
Layer 2  topo · moduledeps · term · link
Layer 3  node · cli · clean
Layer 4  nodelib · net
Layer 5  topolib · bin/mn
```

This matches the plan's shorthand `topo ← net ← node/link ← cli/mn` as a
*conceptual* layering: pure topology/util at the bottom, the kernel-facing
node/link layer in the middle, orchestration (`net`) above that, and the
CLI/entry point (`cli`, `bin/mn`) at the top. One nuance the shorthand hides:
`net.py` does **not** import `topo.py` — `Mininet` receives a `Topo` *instance*
(duck-typed; only `bin/mn`/`topolib` wire the classes together), so `topo` is
fully decoupled from `net` and is the safest first Rust port. `net.py` imports
`cli` (for `interact()`), so `cli` sits *below* `net` in the import graph even
though it drives it at runtime.

## 8. Runtime data flow (one `mn --topo single,3 --test pingall` run)

1. `bin/mn` parses args → `buildTopo(TOPOS, "single,3")` → `SingleSwitchTopo(3)`
   builds a `MultiGraph`: nodes h1..h3 + s1, edges with port numbers
   (`Topo.addPort`: hosts get port 0, switches start at 1).
2. `Mininet(topo=..., switch=OVSSwitch, host=Host, controller=[...], link=Link,
   ipBase="10.0.0.0/8", ...)`.
3. `mn.start()` → `build()` → `buildFromTopo()`: `addController("c0")` →
   `addHost` for `topo.hosts()` → `addSwitch` for `topo.switches()` →
   `addLink(**params)` for `topo.links(sort=True, withInfo=True)`; then
   `configHosts()` (default IPs `10.0.0.x/8`, MACs, default routes).
4. Each `Node.__init__` → `startShell()`: pty + `mnexec -cdn ... bash -is`;
   `cmd("unset HISTFILE; stty -echo; set +m")`.
5. Each `Link.__init__` → `makeIntfPair("h1-eth0","s1-eth1", netns=<s1 pid>)`
   (`ip link add ... type veth peer ... netns ...`), `moveIntf` into host ns,
   `Intf.config()` (`ifconfig ... up`, MAC), `Node` registers intf/port.
6. `start()`: controllers start, `OVSSwitch.start()` issues the big
   `ovs-vsctl` batch and adds ports; `waitConnected` polls
   `ovs-ofctl show`/`dpctl`.
7. `runTests` → `mn.pingAll()` → per-host `ping -c1 <ip>` via `host.cmd`,
   output parsed by `_parsePing`; or interactive `CLI(mn)` loop.
8. `mn.stop()`: `link.stop()` (`ip link del`), switch `stop()/terminate()`
   (`ovs-vsctl del-br`, kill), `host.terminate()` (kill shell); exceptions →
   `cleanup()` in `clean.py`.

## 9. What is pure vs. kernel-coupled (input to the migration split)

- **Pure / root-free (port first, differentially testable):** `topo.py`
  (`MultiGraph`, `Topo`, built-ins), `topolib.py` topologies, and from
  `util.py`: `macColonHex/ipStr/ipNum/ipAdd/ipParse/netParse`, `natural/
  naturalSeq`, `splitArgs/makeNumeric/customClass/specialClass/buildTopo`,
  `unitScale/fmtBps`, `irange`, `checkInt/checkFloat`. Golden outputs for these
  are in `fixtures/`.
- **Root/kernel-coupled (needs netns; sandbox cannot run):** everything in
  `node.py`/`link.py`/`nodelib.py` that shells out to `ip`/`tc`/`ifconfig`/
  `ovs-vsctl`/`brctl`/`iptables`, `mnexec.c`, `net.py` start/stop, `clean.py`,
  and all root-requiring tests. These get root-host/CI gates in the Rust port.
