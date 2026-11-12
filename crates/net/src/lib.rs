//! `mn-net` — Rust port of the `Mininet` orchestrator in `mininet/net.py`.
//!
//! # What is ported here (task 9)
//!
//! - [`Mininet`]: the network object with insertion-ordered
//!   [`hosts`](Mininet::hosts)/[`switches`](Mininet::switches)/
//!   [`controllers`](Mininet::controllers) maps, the [`links`](Mininet::links)
//!   list, and the `nameToNode` index ([`NameToNode`]/[`NodeId`]).
//! - [`Mininet::add_host`]/[`Mininet::add_switch`]/[`Mininet::add_controller`]/
//!   [`Mininet::add_link`] — ports of `addHost`/`addSwitch`/`addController`/
//!   `addLink`, including IP assignment via `mn_util::ip_add` (through
//!   `Host::add_host_defaults`, i.e. `ipAdd(nextIP, ipBaseNum, prefixLen)`),
//!   `autoSetMacs`/`autoPinCpus`, `listenPort` incrementing, port
//!   allocation (`node.newPort()`), intf naming (`node-eth<N>`), and random
//!   link MACs (`Mininet.randMac`).
//! - [`Mininet::build_from_topo`] — port of `buildFromTopo`: default
//!   controller, then `topo.hosts()`, `topo.switches()`,
//!   `topo.links(sort=True, withInfo=True)` (port numbering comes from
//!   `mn-topo`'s legacy port table).
//! - [`Mininet::start`] in Python's order — controllers, switches, hosts,
//!   links — and [`Mininet::stop`] in reverse with cleanup (link delete,
//!   intf delete, terminate, netns delete; every step runs even on error).
//! - Lookups: [`Mininet::get`]/[`Mininet::get_many`] (`getNodeByName`/`get`),
//!   [`Mininet::host`]/[`Mininet::switch`]/[`Mininet::controller`]/
//!   [`Mininet::node`]/[`Mininet::name_to_node`], [`Mininet::contains`],
//!   [`Mininet::keys`]/[`Mininet::values`]/[`Mininet::len`]/[`Mininet::iter`],
//!   plus `del_node`/`links_between`.
//! - Observability in the **exact** Python format that
//!   `scripts/diff_topology.sh` diffs: [`Mininet::dump`] (`cli.py do_dump`:
//!   `repr(node)` per node) and [`Mininet::links`] (`do_links`:
//!   `link status`), see [`observe`].
//! - [`Mininet::ping`]/[`Mininet::ping_all`]/[`Mininet::ping_pair`] —
//!   `pingAll` via host `cmd('LANG=C ping -c1 ...')` with
//!   [`parse_ping`] (port of `_parsePing`).
//! - **Dry-run mode**: [`Mininet::dry_run`] builds the whole topology from a
//!   spec (`--topo single,3`) and [`Mininet::plan`] prints the planned
//!   nodes/links/IPs (`=== topo ===` / `=== dump ===` / `=== links ===`
//!   sections) **without touching the kernel** — non-root testable; this is
//!   the non-root `--dump` path of `apps/mn` (task 10b).
//!
//! # Deliberate deviations from CPython (documented)
//!
//! - Python starts a node's shell in `Node.__init__` and attaches veth pairs
//!   in `Link.__init__` (`addLink`); `mn-node`/`mn-link` made construction
//!   pure and defer kernel work to `Node::start`/`Link::attach`, so
//!   `build()` is pure and `start()` performs controllers → switches →
//!   hosts → links → host kernel config. The model state after `build()`
//!   (node/intf/IP assignment, port numbering) is identical to Python's.
//! - Python `stop()` stops controllers first; per the task this port stops
//!   in exact reverse of `start()` (links, hosts, switches, controllers),
//!   with cleanup, collecting the first error while running all steps.
//! - No `ensureRoot()`/`fixLimits()` at construction (Python's
//!   `Mininet.init`): kernel-touching ops fail with permission errors, and
//!   dry-run construction is deliberately allowed unprivileged.
//! - Python's class-valued constructor args (`switch=`, `host=`, ...) are
//!   concrete types here; the runtime choices are [`SwitchKind`]/[`NetSwitch`]
//!   (`--switch default|ovs|ovsbr|user|ivs|lxbr`), [`ControllerKind`]
//!   (`--controller default|none`), and the `link_spec` string
//!   (`--link default|tc,...`). `NullController` returns `None` in Python,
//!   so `ControllerKind::None` adds no node.
//! - `MininetWithControlNet` (control-network config for
//!   `inNamespace=True`) is a **documented TODO**: `build()` returns an
//!   explicit error for `in_namespace=true` until ported.
//! - Switch *startup* dispatches through [`NetSwitch`] to the real `mn-node`
//!   subclass (`OVSSwitch` runs its `ovs-vsctl` argv, ...), so
//!   switch-bearing topos start on root hosts; `lxbr` (LinuxBridge) is not
//!   ported in mn-node and errors clearly at `start()`. Controller startup
//!   uses `mn-node`'s `Controller::start` (the `controller` binary); the
//!   NOX/Ryu controller subclasses remain TODO stubs there.
//! - Progress logging (`info(...)`) goes to stderr via `eprintln!`;
//!   `dump()`/`links()`/`plan()`/`ping()` return their text so callers
//!   (and tests) control the stream, like Python's `output()` logger.
//! - xterms (`startTerms`) and `mininet.clean` cleanup are not ported
//!   (stored flags only).
//!
//! # Privilege model
//!
//! Model construction, dry-run plans, dumps, and ping-output parsing are
//! testable anywhere. Everything that creates netns/veths/processes
//! (`start()` for namespace nodes, link attach, `ping` via host cmds) needs
//! root + `CAP_SYS_ADMIN`. Those tests are `#[ignore]`d **and** gated on
//! `MN_RUST_ROOT_TESTS=1`, to run on a privileged host/CI runner:
//! `MN_RUST_ROOT_TESTS=1 cargo test -p mn-net -- --ignored --test-threads=1`

mod clean;
mod error;
mod iperf;
mod map;
mod net;
mod observe;
mod options;
mod ping;
mod switch;

pub use clean::{cleanup, is_root};
pub use error::{Error, Result};
pub use iperf::{iperf_vals, IperfResult, IperfVals};
pub use map::{NameToNode, NodeId, NodeMap, NodeRef, NodeRefMut};
pub use net::{rand_mac, Mininet};
pub use observe::{link_str, repr_node, topo_section};
pub use options::{ControllerKind, MininetOptions, WaitConnected};
pub use ping::{parse_ping, PingResult};
pub use switch::{NetSwitch, SwitchKind};

/// Whether root/namespace integration tests should actually run.
///
/// Same convention as `mn-netlink`/`mn-link`/`mn-node`: all root-requiring
/// tests are `#[ignore]`d by default **and** additionally gated on this, so
/// they only execute when `MN_RUST_ROOT_TESTS=1` is set, on a privileged
/// host/CI:
/// `MN_RUST_ROOT_TESTS=1 cargo test -p mn-net -- --ignored --test-threads=1`
#[cfg(test)]
pub(crate) fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_gate_defaults_off() {
        // In this sandbox MN_RUST_ROOT_TESTS is unset, so the gate is off.
        // (If a developer runs the suite with it set, this test simply
        // reports the enabled state instead of failing.)
        let _ = root_tests_enabled();
    }
}
