# Decision Log — mininet-rust migration

Every consequential decision, with reasoning and revisit conditions. Branch `rust-migration`.

## D1 — Strategy: strangler/incremental migration, not big-bang rewrite
- **Decision:** Port module-by-module into a new Cargo workspace; the Python tree
  stays untouched as the differential-testing reference. Nothing is deleted until
  the Rust replacement passes its gates.
- **Why:** The system is root-privileged and kernel-coupled (netns, veth, qdiscs);
  a full rewrite is the highest-risk option and unverifiable in a non-root sandbox.
- **Revisit:** If the Rust side reaches full parity and CI is green, delete legacy
  Python in a final consolidation phase.

## D2 — Containerlab's core lesson: netlink, never shell out for kernel config
- **Decision:** All namespace/veth/addr/route/qdisc operations go through
  `rtnetlink`/`netlink-packet-route` (`crates/netlink`, `crates/link`). No `ip`,
  `tc`, `ifconfig` shell-outs for anything the kernel API can do.
- **Why:** containerlab proved a single binary talking rtnetlink is faster, more
  reliable, and more testable than parsing CLI tool output. Python mininet has
  ~103 string-literal subprocess call sites (node.py 47, nodelib.py 29, ...).
- **Exception (retained):** OVS/OpenFlow control plane stays on `ovs-vsctl` /
  `ovs-ofctl` subprocesses — that surface is too large to port now and is not
  kernel config. Documented as retained component.
- **Revisit:** If OVS control becomes a bottleneck, evaluate openflow crates later.

## D3 — Netem qdisc built as raw netlink, not via `tc`
- **Decision:** `crates/link/src/tc.rs` hand-builds the netem `TCA_OPTIONS` blob
  (24-byte `tc_netem_qopt` + nested `TCA_NETEM_LATENCY64`/`JITTER64` NLAs), verified
  against kernel `sch_netem.c` and iproute2 `q_netem.c` v6.12 sources.
- **Problem found:** `netlink-packet-route` 0.34 has no typed netem qdisc; only
  `TcAttribute::Other(DefaultNla)` accepts the raw blob.
- **Interim deviation:** `bw` is applied via netem's `TCA_NETEM_RATE64`
  (Mbps x 125000 bytes/s) instead of Python's HTB root+class — HTB netlink blobs
  are unmodelled by the crate and unverifiable non-root. `use_htb` parsed but the
  HTB path is a marked TODO.
- **Revisit:** Implement HTB via netlink when a privileged CI can verify it.

## D4 — `mnexec.c` replaced by Rust `apps/mnexec`
- **Decision:** 1:1 flag port (`-c -d -n -p -a -g -r -v -h`) using the `nix` crate;
  usage string byte-identical to the C original (verified by diff against a
  gcc-built mnexec).
- **Why:** One less C artifact; same setns/unshare/cgroup/exec semantics in a
  memory-safe language. `setns` by pid remains the mechanism (attach), plus
  netns-by-name in mn-netlink.

## D5 — Root-gating convention for tests
- **Decision:** Every test requiring root/netns is `#[ignore]` **and** early-returns
  unless `MN_RUST_ROOT_TESTS=1`; run with `-- --ignored --test-threads=1` on a
  privileged host. This sandbox is uid 1000 without netns, so these cannot run here.
- **Why:** Keeps `cargo test --workspace` green and meaningful on any machine while
  making the privileged suite explicit. `scripts/run_root_tests.sh` wraps it.
- **Problem found (fixed):** single `#[ignore]` alone was not enough — PE review
  verified the env gate actually gates by running it as non-root and watching it fail.

## D6 — Fork safety pattern
- **Problem found (PE review, HIGH):** forked children originally allocated and even
  created a tokio runtime after `fork()` (deadlock risk under parallel test threads),
  and the holder/setns race could land the node shell in the ROOT namespace —
  silent host-side misconfiguration in a root tool.
- **Solution (now the standard pattern):** pre-allocate all `CString`s/paths and the
  tokio current-thread runtime **before** `fork()`; child does raw syscalls only;
  readiness pipe handshake (child writes 1 byte after `setns` succeeds; parent
  blocks with 10 s timeout, EOF = failure); defense-in-depth
  `readlink(/proc/<pid>/ns/net) == /var/run/netns/<name>` verification after shell
  spawn; `check_fork_safety()` thread-count gate (hard error under
  `MN_RUST_STRICT_FORK=1`).
- **Where:** `crates/netlink/src/ns.rs`, `crates/node/src/node.rs`.

## D7 — Error model conventions
- **Decision:** Flat per-crate `Error` enums with `Display`/`source()`/`From`;
  per-crate `pub type Result<T>` alias. Python exceptions map to `Err`/`None`
  (a library cannot `exit(1)`); `TopoError::python_exception()` maps back to the
  Python exception class so differential tests can assert identical failure types.
- **Problem found (fixed):** `impl From<String> for Error` was a foot-gun
  (any String silently became the catch-all) — removed; explicit variants required.

## D8 — Process pipes: no deadlocks
- **Problem found (PE review, HIGH):** `pexec` read stdout to EOF before stderr —
  >64 KiB stderr deadlocks.
- **Solution:** `Child::wait_with_output()` (std drains both concurrently).
  Regression test floods 400 KiB of stderr. `crates/node/src/node.rs`.

## D9 — Node lifecycle: pure model + explicit `start()`
- **Decision:** `Node::new()` is pure; all side effects in `start()`/`stop()`.
  `terminate()` always runs full cleanup (killpg SIGHUP -> wait -> killpg SIGKILL
  -> reap holder -> netns_del) and returns the first error after cleanup — no leaks
  on error paths. `popen`/`pexec`/`with_ns_nl` on an unstarted in-namespace node
  return `Err(NotRunning)`, never silently run in the root namespace.
- **Why:** testability without root; no silent wrong-namespace execution.

## D10 — Behavioral parity technique
- **Decision:** Differential tests shell out to `python3` driving the ORIGINAL
  Python modules (cwd = repo root) on generated inputs: mn-util 5536 cases,
  mn-topo 52 cases, plus characterization tests against byte-reproduced fixtures
  (`fixtures/topo_*.txt`). Any deviation is classified (equivalent / tolerance /
  intentional / legacy bug / regression) and documented in crate docs.
- **Known documented deviations:** i64/i128 instead of CPython bignums; ASCII-only
  int/float parsing; `natural` Num-vs-Text total order instead of TypeError;
  library returns instead of process exit.

## D11 — CLI plan (Task 10) in four increments
- 10a (done): clap port of `bin/mn` args, golden help + differential option set.
- 10b: wire `mn` to `mn-net`; `--dump` prints Python-identical dump/links format
  (probed by `scripts/diff_topology.sh`); must work non-root as dry-run.
- 10c: `apps/mn/src/repl.rs` interactive loop port of cli.py — priority commands
  nodes/links/dump/net/ports/hosts/switches/controllers/ping/pingall/iperf/
  dpctl/route/xterm/exit, 2-3 commands per commit with golden-output tests.
- 10d: parity polish (`sh` passthrough, `intfs`, optional `py` escape).
- Out of v1 (to be recorded here): miniedit GUI, consoles, cluster, mobility.

## D12 — Workspace layout and review workflow
- **Layout:** `crates/{util,topo,netlink,link,node,net}`, `apps/{mn,mnexec}`;
  dependency direction util <- topo; netlink base; link <- util+netlink; node <- all;
  mn <- util+topo(+net later). No cycles (verified by design review).
- **Workflow:** workers implement one bounded slice -> principal-engineer
  (adversarial, on mistral-large-4-high) + design-pattern-optimizer review ->
  orchestrator verifies gates and commits atomically per category, no AI
  attribution. Workers intended to run on GLM 5.3 via the `glm-worker` subagent
  profile (`~/.vibe/agents/glm-worker.toml`, needs `/reload` to be discovered).
- **Why:** reviews caught 2 HIGH + 8 MEDIUM issues before they were committed
  (root-ns race, pipe deadlock, cleanup leaks, PATH resolution, clobbering
  defaults); the gate-then-commit flow keeps master always green.

## D13 — Toolchain
- **Decision:** `rust-toolchain.toml` pins `stable`; edition 2021; `Cargo.lock`
  committed; CI runs fmt/check/clippy `-D warnings`/test/release build
  (`.github/workflows/rust-ci.yaml`).

## D14 — `--dump` is a harness-only flag beyond `bin/mn`'s optparse
- **Decision:** `mn --dump` prints the `=== topo ===` / `=== dump ===` /
  `=== links ===` sections (the `cli.py` `do_dump`/`do_links` format that
  `scripts/diff_topology.sh` expects) and exits. It is **not** one of
  `bin/mn`'s optparse options — it exists so the differential harness has a
  scriptable, non-root probe of parse → build → plan. It is whitelisted as
  the single allowed extra option in the help differential test
  (`apps/mn/tests/help_differential.rs`, `allowed_extra = {"dump"}`), so the
  golden `--help` comparison still fails on any *other* drift.
- **Why:** the real Python equivalent of "print the topology without a live
  net" is the interactive CLI's `dump` command, which needs a started net;
  the harness needs the same output root-free to run in the unprivileged
  tier. Non-root, `--dump` takes the dry-run (no-kernel) path and exits 0.
- **Where:** `apps/mn/src/cli.rs` (flag), `apps/mn/src/runner.rs` (dry-run
  wiring to mn-net).
- **Revisit:** if upstream ever wants a `mn --dump`-style flag, align its
  semantics with `cli.py dump` on a live net instead of the plan.

## D15 — iptables/sysctl/OVS stay subprocesses; netlink for all kernel link/addr/route/qdisc/bridge ops
- **Decision:** refining D2: **every** kernel link/address/route/qdisc/
  bridge/veth/netns operation goes through rtnetlink (`crates/netlink`,
  `crates/link`) — no `ip`/`tc`/`ifconfig`/`brctl` shell-outs. The retained
  subprocesses are exactly three families, none of which are plain kernel
  link config: `iptables` (NAT rules, `crates/node` NAT), `sysctl` (a few
  host sysctls), and `ovs-vsctl`/`ovs-ofctl` (the OVS/OpenFlow control
  plane, which is a userspace daemon's CLI, not kernel config).
- **Why:** the containerlab lesson applies to kernel configuration, where
  parsing CLI-tool output is slow and fragile; iptables-nft and the OVSDB
  control protocol are separate, much larger surfaces whose Rust crates are
  not yet verifiable in this environment.
- **Revisit:** if NAT or OVS control shows up in benchmarks
  (`docs/migration/BENCHMARKS.md`), evaluate `nftnl`/`mnl` and an OVSDB
  client crate as their own tasks.

## D16 — Deferred lifecycle: pure model build, explicit `start()`; `stop()` is the exact reverse of `start()` with full cleanup
- **Decision:** at the orchestrator level (extending D9): `Topo::build` and
  `Mininet::buildFromTopo` are pure model construction — zero kernel side
  effects. All side effects live in the explicit `start()`; `stop()` undoes
  `start()` in the exact reverse order and always runs full cleanup
  (kill process groups → reap → delete veths/qdiscs → delete netns), even
  on error paths, returning the first error encountered during cleanup.
- **Why:** the parse → build → plan path (including `mn --dump`, D14) is
  then testable without root, and a failed `start()` can never leak
  half-created namespaces or links.
- **Where:** `crates/net/src/net.rs`, `crates/node/src/node.rs`,
  `crates/link`.
- **Revisit:** a `Drop` impl may later call `stop()` defensively — `stop()`
  must therefore stay idempotent (it already is: it errors/no-ops on an
  unstarted net rather than touching the root namespace).

## D17 — Out of v1 scope: miniedit GUI, consoles, cluster, mobility, NOX/Ryu, HTB-via-netlink
- **Decision:** v1 targets the `bin/mn` + `cli.py` core (create / destroy /
  inspect / built-in tests) with the `default`/`ovs`/`lxbr` switches,
  `proc`/`cfs`/`rt` hosts, and `Link`/`TCLink` params. Explicitly deferred,
  each a self-contained Python module to port later behind the same
  interfaces:
  - **miniedit GUI** (`mininet/miniedit.py`),
  - **consoles via other backends** — `mininet/term.py` IS ported (xterm/socat
    argv builders + `--xterms` wiring, Task 10d); only non-xterm console
    backends remain out of scope,
  - **cluster** (`mininet/cluster.py`, `--cluster`/`--placement`),
  - **mobility** (`mininet/mobility.py`),
  - **NOX/Ryu controller runtimes** (`crates/node/src/controller.rs` —
    stubs whose `start()` returns a marked TODO error; the rest of the
    Controller surface is ported),
  - **HTB qdisc via netlink** (D3 — `bw` maps to netem rate until a
    privileged CI can verify HTB blobs),
  - plus `--innamespace` (control network) and `--custom` (exec Python
    files) as marked TODOs in `mn-net`/`apps/mn`.
- **Why:** the surface is too large for one migration pass; deferring keeps
  the v1 gate (differential parity on topos + CLI + netlink core) achievable
  and reviewable.
- **Revisit:** per feature, when an example/test needs it; each lands with
  its own decision entry and differential tests.

## D18 — OMO-style live graph TUI (Task 13)
- **Decision:** `mn-tui` (ratatui) renders the emulated network as a live graph,
  the way OMO (omo.dev) renders agent workflows as dependency graphs: nodes as
  boxes (host green / switch yellow / controller cyan / selected bold),
  edges as unicode lines colored by link status, detail pane on select.
- **Why:** graph-first UI matches how the system is reasoned about; headless
  `--frames`/`--json` make it CI-testable without a tty. Non-root renders the
  dry-run plan; live mode root-gated.

## D19 — MCP server for native AI-tool integration (Task 14)
- **Decision:** `mn-mcp` speaks MCP (JSON-RPC 2.0 over stdio) exposing
  topo_list/topo_plan/graph/net_build/net_start/net_stop/pingall/ping/dump/
  links/host_cmd/net_release, so any MCP-capable AI agent can drive simulations.
  Mutating tools (net_start/stop, host_cmd) hard-fail unless euid 0.
- **Why:** lets AI tools (incl. Vibe itself via `vibe mcp add mn-mcp`) create
  topos, run sims, inspect graphs. Trust model: stdio-only transport, client is
  the local spawning user; never expose over a network transport.
- **Revisit:** align the graph JSON dialect with mn-tui GraphView (polish task).
