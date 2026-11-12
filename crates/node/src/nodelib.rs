//! `nodelib` — port of `mininet/nodelib.py`: [`LinuxBridge`] and [`NAT`].
//!
//! # LinuxBridge
//!
//! [`LinuxBridge`] is a [`Switch`] variant backed by a kernel **Linux
//! bridge** instead of OVS. Python drives it with `brctl addbr/addif/stp/
//! setbridgeprio` subprocess calls; per the migration plan's containerlab
//! lesson (kernel link config goes over rtnetlink, never `ip`/`brctl`),
//! the Rust port creates the bridge and enslaves veths with **rtnetlink
//! messages**:
//!
//! - [`mn_netlink::bridge_add_msg`] — `RTM_NEWLINK` creating `type bridge`
//!   (optionally with STP state + priority, i.e. `brctl stp
//!   on`/`setbridgeprio`);
//! - [`mn_netlink::enslave_msg`] / [`mn_netlink::disenslave_msg`] —
//!   `IFLA_MASTER` set/clear (`brctl addif`/`delif`);
//! - [`mn_netlink::link_up_msg`] — up/down (`ifconfig <dev> up|down`).
//!
//! The bridge link ops live upstream in `mn-netlink` (`mn_netlink::bridge`
//! items, re-exported at its root): all message builders are **pure and
//! unit-tested** there; the real netlink ops run in
//! [`LinuxBridge::start`]/[`stop`]/[`attach`]/[`detach`] through the
//! [`mn_netlink::BridgeNl`] facade (and `mn_netlink::run_bridge_in_netns`
//! for the in-namespace path) and are root-gated in tests (`#[ignore]` +
//! `MN_RUST_ROOT_TESTS=1`).
//!
//! # NAT
//!
//! [`NAT`] is a [`Host`] (net.py `addNAT` does `addHost(name, cls=NAT,
//! ...)`) that provides connectivity to the external network. The
//! iptables rule set is a **retained shell-out** (documented): unlike
//! kernel link/addr/qdisc config, iptables has **no stable netlink API**,
//! so the containerlab lesson does not apply here and the rules still run
//! as `iptables`/`sysctl` subprocesses, exactly like nodelib.py. Every
//! argv vector is a **pure, unit-tested builder** checked line-by-line
//! against nodelib.py ([`nat_forward_drop_argv`], [`nat_masquerade_argv`],
//! ...); execution goes through `Node::pexec` and is root-gated in tests.
//!
//! Python passes the MASQUERADE negation shell-quoted (`"'!'"`); since
//! `pexec` execs argv directly without a shell, the argument is the bare
//! `!` — same argv the shell would have produced.
//!
//! The NAT's own default route: the NAT runs in the root namespace by
//! default, so its external (`local_intf`) interface keeps the host's
//! default route, and net.py `addNAT` points every host's default route at
//! the NAT IP (`host.setDefaultRoute('via <natIP>')` — see [`NAT::nat_ip`]).
//! "dhcp off" is [`NAT::set_manual_config`]: it appends
//! `iface <intf> inet manual` to `/etc/network/interfaces` so
//! network-manager does not DHCP-manage the external interface.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU16, Ordering};

// The bridge netlink plumbing (message builders + BridgeNl facade +
// run_bridge_in_netns) lives upstream in mn-netlink and is re-exported
// here; the raw packet types come via mn_netlink::link_packet so this
// crate needs no direct rtnetlink/netlink-packet-route dependency.
use mn_netlink::{
    any_port_forwarding, bridge_add_msg, disenslave_msg, enslave_msg, enslaved_port_states,
    link_up_msg, msg_ifname, run_bridge_in_netns, BridgeNl,
};

use crate::controller::Controller;
use crate::error::{Error, Result};
use crate::host::Host;
use crate::switch::Switch;

// ---------------------------------------------------------------------
// LinuxBridge
// ---------------------------------------------------------------------

/// `LinuxBridge` — port of `class LinuxBridge(Switch)` in nodelib.py: a
/// switch backed by a kernel Linux bridge (optionally with spanning tree),
/// created over rtnetlink instead of `brctl`.
///
/// `config`/`config_default` are inherited from [`Switch`]/[`Node`] via
/// `Deref` (Python inherits `Node.config` the same way).
#[derive(Debug)]
pub struct LinuxBridge {
    switch: Switch,
    /// Use spanning tree protocol (`stp=False`).
    pub stp: bool,
    /// Bridge priority for STP (`prio`, else `LinuxBridge.nextPrio`).
    pub prio: u16,
}

/// Python `LinuxBridge.nextPrio = 100`, incremented per defaulted instance.
static NEXT_PRIO: AtomicU16 = AtomicU16::new(100);

impl LinuxBridge {
    /// Port of `LinuxBridge(name, stp=False, prio=None, **kwargs)` with the
    /// usual switch defaults (derived dpid, no opts/listen port).
    pub fn new(
        name: &str,
        stp: bool,
        prio: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_full(name, stp, prio, None, "", None, in_namespace, params)
    }

    /// [`LinuxBridge::new`] with explicit switch args (`dpid`/`opts`/
    /// `listen_port` — Python's `**kwargs` into `Switch.__init__`).
    #[allow(clippy::too_many_arguments)]
    pub fn new_full(
        name: &str,
        stp: bool,
        prio: Option<u16>,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let prio = match prio {
            Some(p) => p,
            None => NEXT_PRIO.fetch_add(1, Ordering::SeqCst),
        };
        let mut switch = Switch::new(name, dpid, opts, listen_port, in_namespace, params)?;
        switch.node_mut().set_kind("LinuxBridge");
        Ok(Self { switch, stp, prio })
    }

    // ---- setup() (dependency check + firewall warning) ----

    /// The bridge-nf-call sysctl `LinuxBridge.setup()` checks for `table`
    /// (`'arp'`/`'ip'`/`'ip6'`): `net.bridge.bridge-nf-call-<table>tables`.
    pub fn bridge_nf_sysctl(table: &str) -> String {
        format!("net.bridge.bridge-nf-call-{table}tables")
    }

    /// `/proc` path of the same sysctl (read directly, no subprocess).
    pub fn bridge_nf_proc_path(table: &str) -> String {
        format!("/proc/sys/net/bridge/bridge-nf-call-{table}tables")
    }

    /// Python: `out = quietRun(cmd).strip(); if out.endswith('1'): warn`.
    pub fn sysctl_output_enabled(out: &str) -> bool {
        out.trim_end().ends_with('1')
    }

    /// Port of `LinuxBridge.setup()` (minus the `brctl` pathCheck, which is
    /// moot — we do not shell out to brctl): returns one warning per table
    /// whose `bridge-nf-call-*` sysctl is enabled, since that breaks
    /// bridged traffic. Reads `/proc/sys/net/bridge/...` directly.
    pub fn setup() -> Vec<String> {
        let mut warnings = Vec::new();
        for table in ["arp", "ip", "ip6"] {
            if let Ok(out) = std::fs::read_to_string(Self::bridge_nf_proc_path(table)) {
                if Self::sysctl_output_enabled(&out) {
                    warnings.push(format!(
                        "Warning: Linux bridge may not work with {} = {}",
                        Self::bridge_nf_sysctl(table),
                        out.trim_end()
                    ));
                }
            }
        }
        warnings
    }

    // ---- connected() ----

    /// Port of `LinuxBridge.connected()`: with STP, are we forwarding yet?
    /// (netlink: any enslaved port in `Forwarding` state); without STP,
    /// always true.
    pub fn connected(&mut self) -> bool {
        if !self.stp {
            return true;
        }
        self.stp_connected().unwrap_or(false)
    }

    fn stp_connected(&mut self) -> Result<bool> {
        let name = self.switch.name.clone();
        let mut result = false;
        self.with_bridge_nl(|nl| {
            let msgs = nl.link_msgs()?;
            let br_idx = msgs
                .iter()
                .find(|m| msg_ifname(m) == Some(name.as_str()))
                .map(|m| m.header.index)
                .ok_or_else(|| mn_netlink::Error::Other(format!("{name}: no such bridge")))?;
            result = any_port_forwarding(&enslaved_port_states(&msgs, br_idx));
            Ok(())
        })?;
        Ok(result)
    }

    // ---- start/stop/attach/detach (netlink ops) ----

    /// Port of `LinuxBridge.start(controllers)` — netlink form:
    /// delete any stale bridge, create the bridge (with STP opts), enslave
    /// this switch's own interfaces (`if self.name in i.name`), bring the
    /// bridge up.
    pub fn start(&mut self, _controllers: &[&Controller]) -> Result<()> {
        let name = self.switch.name.clone();
        let stp = self.stp;
        let prio = self.prio;
        // Python: `for i in self.intfList(): if self.name in i.name: addif`
        let ports: Vec<String> = self
            .switch
            .intf_list()
            .iter()
            .map(|i| i.name.clone())
            .filter(|n| n.contains(&name))
            .collect();
        self.with_bridge_nl(|nl| {
            // `ifconfig <br> down; brctl delbr <br>` — stale bridge cleanup
            // (missing bridge is not an error).
            if let Some(idx) = nl.link_index(&name)? {
                let _ = nl.link_set(link_up_msg(idx, false));
                nl.link_del(idx)?;
            }
            // `brctl addbr <br>` (+ `setbridgeprio`/`stp on` when stp)
            nl.link_add(bridge_add_msg(&name, stp, prio)?)?;
            let br_idx = nl
                .link_index(&name)?
                .ok_or_else(|| mn_netlink::Error::Other(format!("{name}: bridge not created")))?;
            // `brctl addif <br> <intf>` for our own interfaces
            for port in &ports {
                if let Some(pidx) = nl.link_index(port)? {
                    nl.link_set(enslave_msg(pidx, br_idx))?;
                }
            }
            // `ifconfig <br> up`
            nl.link_set(link_up_msg(br_idx, true))?;
            Ok(())
        })
    }

    /// Port of `LinuxBridge.stop(deleteIntfs=True)`: bring the bridge down
    /// and delete it (`brctl delbr`), then the base `Switch.stop`.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        let name = self.switch.name.clone();
        let r = self.with_bridge_nl(|nl| {
            if let Some(idx) = nl.link_index(&name)? {
                let _ = nl.link_set(link_up_msg(idx, false));
                nl.link_del(idx)?;
            }
            Ok(())
        });
        self.switch.stop_with(delete_intfs);
        r
    }

    /// Enslave `intf` to the bridge (`brctl addif`) and bring the port up
    /// (like `OVSSwitch.attach`'s `ifconfig <intf> up`).
    pub fn attach(&mut self, intf: &str) -> Result<()> {
        let br = self.switch.name.clone();
        let intf = intf.to_string();
        self.with_bridge_nl(|nl| {
            let br_idx = nl
                .link_index(&br)?
                .ok_or_else(|| mn_netlink::Error::Other(format!("{br}: no such bridge")))?;
            let pidx = nl
                .link_index(&intf)?
                .ok_or_else(|| mn_netlink::Error::Other(format!("no such interface: {intf}")))?;
            nl.link_set(enslave_msg(pidx, br_idx))?;
            nl.link_set(link_up_msg(pidx, true))?;
            Ok(())
        })
    }

    /// Release `intf` from the bridge (`brctl delif`).
    pub fn detach(&mut self, intf: &str) -> Result<()> {
        let intf = intf.to_string();
        self.with_bridge_nl(|nl| {
            if let Some(pidx) = nl.link_index(&intf)? {
                nl.link_set(disenslave_msg(pidx))?;
            }
            Ok(())
        })
    }

    /// Python `LinuxBridge.dpctl` ran `brctl <args>`. The Rust port does not
    /// shell out to brctl, so this returns an explicit error pointing at the
    /// netlink-backed methods.
    pub fn dpctl(&mut self, args: &[String]) -> Result<String> {
        Err(Error::Other(format!(
            "{}: LinuxBridge.dpctl({args:?}) is not supported — the Rust port \
             manages the bridge over netlink (start/stop/attach/detach), not brctl",
            self.switch.name
        )))
    }

    /// Run `f` with a [`BridgeNl`] in the switch's namespace: the root
    /// namespace for `in_namespace=false` switches, else the node's netns
    /// via [`run_bridge_in_netns`] (requires a started node). `f` works in
    /// `mn_netlink::Result`; errors surface here as [`Error::Netlink`].
    fn with_bridge_nl<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&BridgeNl) -> mn_netlink::Result<()>,
    {
        let node = self.switch.node_mut();
        match (node.in_namespace, node.pid()) {
            (true, Some(pid)) => run_bridge_in_netns(pid, f).map_err(Error::Netlink),
            (true, None) => Err(Error::NotRunning(node.name.clone())),
            (false, _) => {
                let nl = BridgeNl::new().map_err(Error::Netlink)?;
                f(&nl).map_err(Error::Netlink)
            }
        }
    }

    /// The inner base switch.
    pub fn switch(&self) -> &Switch {
        &self.switch
    }

    /// Mutable access to the inner base switch.
    pub fn switch_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl Deref for LinuxBridge {
    type Target = Switch;
    fn deref(&self) -> &Switch {
        &self.switch
    }
}

impl DerefMut for LinuxBridge {
    fn deref_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl fmt::Display for LinuxBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.switch.fmt(f)
    }
}

// ---------------------------------------------------------------------
// NAT — iptables/sysctl argv builders (pure, unit-tested vs nodelib.py)
// ---------------------------------------------------------------------

fn v(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

/// iptables rule op: install (`-I`/`-A`) or delete (`-D`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatOp {
    /// `-I` (insert, used for the DROP rule).
    Insert,
    /// `-A` (append, used for the ACCEPT/MASQUERADE rules).
    Append,
    /// `-D` (delete, used by `terminate`).
    Delete,
}

impl NatOp {
    /// The iptables flag for this op.
    pub fn flag(self) -> &'static str {
        match self {
            NatOp::Insert => "-I",
            NatOp::Append => "-A",
            NatOp::Delete => "-D",
        }
    }
}

/// `sysctl -n net.ipv4.ip_forward` — read the forwarding state
/// (nodelib.py `__init__`: `self.cmd('sysctl -n net.ipv4.ip_forward')`).
pub fn sysctl_read_ip_forward_argv() -> Vec<String> {
    v(&["sysctl", "-n", "net.ipv4.ip_forward"])
}

/// `sysctl net.ipv4.ip_forward=<value>` — Python passes the assignment as
/// one argument after `sysctl`.
pub fn sysctl_write_ip_forward_argv(value: &str) -> Vec<String> {
    let arg = format!("net.ipv4.ip_forward={value}");
    v(&["sysctl", arg.as_str()])
}

/// `iptables -F` (flush, `flush=True`).
pub fn iptables_flush_argv() -> Vec<String> {
    v(&["iptables", "-F"])
}

/// `iptables -t nat -F` (flush the nat table).
pub fn iptables_flush_nat_argv() -> Vec<String> {
    v(&["iptables", "-t", "nat", "-F"])
}

/// `iptables -P <chain> <policy>` (default policies when flushing).
pub fn iptables_policy_argv(chain: &str, policy: &str) -> Vec<String> {
    v(&["iptables", "-P", chain, policy])
}

/// `iptables <op> FORWARD -i <local> -d <subnet> -j DROP` — nodelib.py:
/// `'iptables -I FORWARD', '-i', localIntf, '-d', subnet, '-j DROP'`
/// (and `-D` in `terminate`).
pub fn nat_forward_drop_argv(op: NatOp, local: &str, subnet: &str) -> Vec<String> {
    v(&[
        "iptables",
        op.flag(),
        "FORWARD",
        "-i",
        local,
        "-d",
        subnet,
        "-j",
        "DROP",
    ])
}

/// `iptables <op> FORWARD -i <local> -s <subnet> -j ACCEPT` — nodelib.py:
/// `'iptables -A FORWARD', '-i', localIntf, '-s', subnet, '-j ACCEPT'`.
pub fn nat_forward_accept_src_argv(op: NatOp, local: &str, subnet: &str) -> Vec<String> {
    v(&[
        "iptables",
        op.flag(),
        "FORWARD",
        "-i",
        local,
        "-s",
        subnet,
        "-j",
        "ACCEPT",
    ])
}

/// `iptables <op> FORWARD -o <local> -d <subnet> -j ACCEPT` — nodelib.py:
/// `'iptables -A FORWARD', '-o', localIntf, '-d', subnet, '-j ACCEPT'`.
pub fn nat_forward_accept_dst_argv(op: NatOp, local: &str, subnet: &str) -> Vec<String> {
    v(&[
        "iptables",
        op.flag(),
        "FORWARD",
        "-o",
        local,
        "-d",
        subnet,
        "-j",
        "ACCEPT",
    ])
}

/// `iptables -t nat <op> POSTROUTING -s <subnet> ! -d <subnet> -j MASQUERADE`
/// — nodelib.py: `'iptables -t nat -A POSTROUTING', '-s', subnet, "'!'",
/// '-d', subnet, '-j MASQUERADE'`. Python's `"'!'"` is shell quoting; as an
/// exec argv the negation argument is the bare `!` (same argv the shell
/// would have produced, and what `terminate`'s `'\\'!\\''` means too).
pub fn nat_masquerade_argv(op: NatOp, subnet: &str) -> Vec<String> {
    v(&[
        "iptables",
        "-t",
        "nat",
        op.flag(),
        "POSTROUTING",
        "-s",
        subnet,
        "!",
        "-d",
        subnet,
        "-j",
        "MASQUERADE",
    ])
}

/// `/etc/network/interfaces`, edited by `NAT.setManualConfig`.
pub const NETWORK_INTERFACES: &str = "/etc/network/interfaces";

/// The stanza `setManualConfig` appends: `\niface <intf> inet manual\n` —
/// "manual" = network-manager/DHCP leaves the NAT's external interface
/// alone ("dhcp off").
pub fn manual_config_line(intf: &str) -> String {
    format!("\niface {intf} inet manual\n")
}

/// Python: `if (line) not in config: append it`.
pub fn manual_config_needed(config: &str, line: &str) -> bool {
    !config.contains(line)
}

/// `NAT` — port of `class NAT(Node)` in nodelib.py: a [`Host`] providing
/// connectivity between Mininet and the external network via iptables
/// filter+nat rules and `net.ipv4.ip_forward`.
///
/// Typically created in the root namespace (`in_namespace=false`), linked
/// to the first switch; net.py `addNAT` then sets every host's default
/// route `via <natIP>` (see [`NAT::nat_ip`]).
#[derive(Debug)]
pub struct NAT {
    host: Host,
    /// Mininet subnet (`subnet='10.0/8'`).
    pub subnet: String,
    /// External interface (`localIntf=None` → the default intf at config).
    pub local_intf: Option<String>,
    /// Flush iptables before installing the NAT rules (`flush=False`).
    pub flush: bool,
    /// Saved `net.ipv4.ip_forward` value, restored by [`NAT::terminate`].
    /// Python captures it in `__init__`; here the shell is not up at
    /// construction, so it is captured at the start of [`NAT::config`] —
    /// still before anything changes it.
    pub forward_state: String,
}

impl NAT {
    /// Port of `NAT(name, subnet='10.0/8', localIntf=None, flush=False,
    /// **params)`.
    pub fn new(
        name: &str,
        subnet: Option<&str>,
        local_intf: Option<&str>,
        flush: bool,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Self {
        Self {
            host: Host::new(name, in_namespace, params),
            subnet: subnet.unwrap_or("10.0/8").to_string(),
            local_intf: local_intf.map(|s| s.to_string()),
            flush,
            forward_state: "1".to_string(),
        }
    }

    /// The inner host.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// Mutable access to the inner host.
    pub fn host_mut(&mut self) -> &mut Host {
        &mut self.host
    }

    /// The NAT's IP on the Mininet side (`params['ip']` without the
    /// prefix) — net.py `addNAT` uses it as the hosts' default gateway
    /// (`host.setDefaultRoute('via <natIP>')`).
    pub fn nat_ip(&self) -> Option<String> {
        self.host
            .params
            .get("ip")
            .and_then(|s| s.split('/').next().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
    }

    /// Port of `NAT.setManualConfig`: append the `iface <intf> inet
    /// manual` stanza to `/etc/network/interfaces` if missing ("dhcp off"),
    /// then restart network-manager so it stops managing the interface.
    pub fn set_manual_config(&mut self, intf: &str) -> Result<()> {
        let line = manual_config_line(intf);
        let existing = std::fs::read_to_string(NETWORK_INTERFACES).unwrap_or_default();
        if manual_config_needed(&existing, &line) {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(NETWORK_INTERFACES)?;
            file.write_all(line.as_bytes())?;
            // Python restarts network-manager only when the stanza was
            // actually appended (nodelib.py:100-105) — keep that guard.
            // Best-effort, like Python's cmd (no raise).
            let _ = self.host.node_mut().pexec(&v(&[
                "sh",
                "-c",
                "service network-manager restart || netplan apply",
            ]));
        }
        Ok(())
    }

    /// Run one iptables/sysctl argv through the node. Python's `cmd` does
    /// not raise on non-zero exit; the merged output is returned.
    fn run_rule(&mut self, argv: &[String]) -> Result<String> {
        let out = self.host.node_mut().pexec(argv)?;
        Ok(out.merged())
    }

    /// Port of `NAT.config(**params)`: pick the local (external) intf,
    /// write the manual-config stanza, apply `Node.config(**params)`,
    /// optionally flush iptables, install the NAT rules, enable IP
    /// forwarding.
    pub fn config(&mut self) -> Result<BTreeMap<String, String>> {
        if self.local_intf.is_none() {
            let intf = self
                .host
                .default_intf()
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "{}: NAT has no default interface (localIntf)",
                        self.host.name
                    ))
                })?
                .name
                .clone();
            self.local_intf = Some(intf);
        }
        let local = self.local_intf.clone().expect("local_intf just set");
        let subnet = self.subnet.clone();

        self.set_manual_config(&local)?;

        // Capture the current forwarding state before anything changes it
        // (Python does this in __init__; see struct docs).
        if let Ok(out) = self.run_rule(&sysctl_read_ip_forward_argv()) {
            let t = out.trim().to_string();
            if !t.is_empty() {
                self.forward_state = t;
            }
        }

        // super().config(**params) — Node.configDefault
        let r = self.host.config_default()?;

        if self.flush {
            self.run_rule(&sysctl_write_ip_forward_argv("0"))?;
            self.run_rule(&iptables_flush_argv())?;
            self.run_rule(&iptables_flush_nat_argv())?;
            // Default entries for unmatched traffic
            self.run_rule(&iptables_policy_argv("INPUT", "ACCEPT"))?;
            self.run_rule(&iptables_policy_argv("OUTPUT", "ACCEPT"))?;
            self.run_rule(&iptables_policy_argv("FORWARD", "DROP"))?;
        }

        // Install the NAT rules (nodelib.py NAT.config)
        self.run_rule(&nat_forward_drop_argv(NatOp::Insert, &local, &subnet))?;
        self.run_rule(&nat_forward_accept_src_argv(NatOp::Append, &local, &subnet))?;
        self.run_rule(&nat_forward_accept_dst_argv(NatOp::Append, &local, &subnet))?;
        self.run_rule(&nat_masquerade_argv(NatOp::Append, &subnet))?;

        // Instruct the kernel to perform forwarding
        self.run_rule(&sysctl_write_ip_forward_argv("1"))?;
        Ok(r)
    }

    /// Port of `NAT.terminate`: remove the NAT rules, restore the saved
    /// `ip_forward` state, then terminate the host/node. Rule-removal
    /// failures are not fatal (Python uses `cmd`, which does not raise);
    /// the first error is returned after cleanup.
    pub fn terminate(&mut self) -> Result<()> {
        let mut first_err: Option<Error> = None;
        if let Some(local) = self.local_intf.clone() {
            let subnet = self.subnet.clone();
            let rules = [
                nat_forward_drop_argv(NatOp::Delete, &local, &subnet),
                nat_forward_accept_src_argv(NatOp::Delete, &local, &subnet),
                nat_forward_accept_dst_argv(NatOp::Delete, &local, &subnet),
                nat_masquerade_argv(NatOp::Delete, &subnet),
            ];
            for argv in &rules {
                if let Err(e) = self.run_rule(argv) {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
            let restore = sysctl_write_ip_forward_argv(&self.forward_state);
            if let Err(e) = self.run_rule(&restore) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        self.host.terminate()?;
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Deref for NAT {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.host
    }
}

impl DerefMut for NAT {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.host
    }
}

impl fmt::Display for NAT {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.host.fmt(f)
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;
    use mn_link::Intf;
    // Test-only names via the mn-netlink re-exports (so this crate needs no
    // direct netlink-packet-route dependency).
    use mn_netlink::link_packet::{
        BridgePortState, BridgeStpState, InfoBridge, InfoBridgePort, InfoKind, InfoPortData,
        LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
    };
    use mn_netlink::msg_bridge_data;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // ---- LinuxBridge message builders ----

    #[test]
    fn bridge_add_msg_matches_brctl_addbr() {
        // brctl addbr s1: RTM_NEWLINK, kind "bridge", name s1, created DOWN.
        let msg = bridge_add_msg("s1", false, 100).unwrap();
        assert_eq!(msg_ifname(&msg), Some("s1"));
        assert!(
            msg.attributes.iter().any(|a| matches!(
                a,
                LinkAttribute::LinkInfo(infos)
                    if infos.iter().any(|i| matches!(i, LinkInfo::Kind(InfoKind::Bridge)))
            )),
            "kind must be bridge: {msg:?}"
        );
        assert!(
            !msg.header.flags.contains(LinkFlags::Up),
            "brctl addbr creates the bridge down: {msg:?}"
        );
        // No STP attributes without stp.
        let data = msg_bridge_data(&msg).cloned().unwrap_or_default();
        assert!(!data.iter().any(|d| matches!(d, InfoBridge::StpState(_))));
        assert!(!data.iter().any(|d| matches!(d, InfoBridge::Priority(_))));

        // brctl setbridgeprio 104 + stp on: StpState=KernelStp(1), Priority.
        let msg = bridge_add_msg("s1", true, 104).unwrap();
        let data = msg_bridge_data(&msg).expect("bridge info data").clone();
        assert!(
            data.contains(&InfoBridge::StpState(BridgeStpState::KernelStp)),
            "stp on -> IFLA_BR_STP_STATE=1 (kernel STP): {data:?}"
        );
        assert!(data.contains(&InfoBridge::Priority(104)), "{data:?}");

        // ifname validation (IFNAMSIZ) applies.
        assert!(bridge_add_msg("this-name-is-way-too-long", false, 100).is_err());
        assert!(bridge_add_msg("a/b", false, 100).is_err());
    }

    #[test]
    fn link_up_enslave_disenslave_msgs() {
        // ifconfig <dev> up / down
        let up = link_up_msg(7, true);
        assert_eq!(up.header.index, 7);
        assert!(up.header.flags.contains(LinkFlags::Up));
        let down = link_up_msg(7, false);
        assert!(!down.header.flags.contains(LinkFlags::Up));

        // brctl addif br port  ==  IFLA_MASTER = br index on the port
        let m = enslave_msg(5, 3);
        assert_eq!(m.header.index, 5);
        assert!(m.attributes.contains(&LinkAttribute::Controller(3)));
        // brctl delif == IFLA_MASTER = 0 (nomaster)
        let m = disenslave_msg(5);
        assert!(m.attributes.contains(&LinkAttribute::Controller(0)));
    }

    #[test]
    fn enslaved_port_states_and_forwarding_like_showstp() {
        // Hand-built dump: port 5 enslaved to bridge 3, state forwarding;
        // port 6 enslaved to bridge 3, state blocking; port 9 enslaved to
        // another bridge.
        let port = |idx: u32, master: u32, state: BridgePortState| {
            let mut msg = LinkMessage::default();
            msg.header.index = idx;
            msg.attributes = vec![
                LinkAttribute::Controller(master),
                LinkAttribute::LinkInfo(vec![LinkInfo::PortData(InfoPortData::BridgePort(vec![
                    InfoBridgePort::State(state),
                ]))]),
            ];
            msg
        };
        let msgs = vec![
            port(5, 3, BridgePortState::Forwarding),
            port(6, 3, BridgePortState::Blocking),
            port(9, 4, BridgePortState::Forwarding),
        ];
        let states = enslaved_port_states(&msgs, 3);
        assert_eq!(
            states,
            vec![BridgePortState::Forwarding, BridgePortState::Blocking]
        );
        // Python: 'forwarding' in showstp output -> any forwarding port.
        assert!(any_port_forwarding(&states));
        assert!(!any_port_forwarding(&[BridgePortState::Blocking]));
        assert!(!any_port_forwarding(&[]));
    }

    #[test]
    fn linux_bridge_model_matches_python() {
        // prio auto-assignment: nextPrio starts at 100 and increments.
        let b1 = LinuxBridge::new("s1", false, None, false, BTreeMap::new()).unwrap();
        let b2 = LinuxBridge::new("s2", true, None, false, BTreeMap::new()).unwrap();
        assert_eq!(b1.prio, 100);
        assert_eq!(b2.prio, 101);
        assert!(!b1.stp);
        assert!(b2.stp);
        // Switch model underneath: portBase 1, derived dpid, kind tag.
        assert_eq!(b1.port_base, 1);
        assert_eq!(b1.dpid, "0000000000000001");
        assert_eq!(b1.kind(), "LinuxBridge");
        assert!(b1.to_string().starts_with("<LinuxBridge s1: "));
        // Explicit prio wins over the counter.
        let b3 = LinuxBridge::new("s3", true, Some(42), false, BTreeMap::new()).unwrap();
        assert_eq!(b3.prio, 42);
        // connected(): no stp -> True without touching the kernel.
        let mut b4 = LinuxBridge::new("s4", false, Some(100), false, BTreeMap::new()).unwrap();
        assert!(b4.connected());
        // dpctl is an explicit error (no brctl shell-out).
        let err = b4.dpctl(&s(&["show"])).unwrap_err().to_string();
        assert!(err.contains("not supported"), "got {err}");
        // in-namespace + not started: bridge ops report NotRunning.
        let mut b5 = LinuxBridge::new("s5", false, Some(100), true, BTreeMap::new()).unwrap();
        let err = b5.start(&[]).unwrap_err();
        assert!(matches!(err, Error::NotRunning(_)), "got {err:?}");
    }

    #[test]
    fn linux_bridge_setup_sysctl_names_match_python() {
        // nodelib.py: for table in 'arp', 'ip', 'ip6':
        //   cmd = 'sysctl net.bridge.bridge-nf-call-%stables' % table
        assert_eq!(
            LinuxBridge::bridge_nf_sysctl("arp"),
            "net.bridge.bridge-nf-call-arptables"
        );
        assert_eq!(
            LinuxBridge::bridge_nf_sysctl("ip"),
            "net.bridge.bridge-nf-call-iptables"
        );
        assert_eq!(
            LinuxBridge::bridge_nf_sysctl("ip6"),
            "net.bridge.bridge-nf-call-ip6tables"
        );
        assert_eq!(
            LinuxBridge::bridge_nf_proc_path("ip"),
            "/proc/sys/net/bridge/bridge-nf-call-iptables"
        );
        // out.strip().endswith('1')
        assert!(LinuxBridge::sysctl_output_enabled(
            "net.bridge.bridge-nf-call-iptables = 1\n"
        ));
        assert!(!LinuxBridge::sysctl_output_enabled(
            "net.bridge.bridge-nf-call-iptables = 0\n"
        ));
        // setup() reads /proc directly; on hosts without the bridge
        // netfilter modules the files are absent -> no warnings, no panic.
        let _ = LinuxBridge::setup();
    }

    // ---- NAT argv builders, exactly vs nodelib.py ----

    #[test]
    fn nat_sysctl_argvs_match_python() {
        // __init__: self.cmd( 'sysctl -n net.ipv4.ip_forward' )
        assert_eq!(
            sysctl_read_ip_forward_argv(),
            s(&["sysctl", "-n", "net.ipv4.ip_forward"])
        );
        // config: self.cmd( 'sysctl net.ipv4.ip_forward=1' )
        assert_eq!(
            sysctl_write_ip_forward_argv("1"),
            s(&["sysctl", "net.ipv4.ip_forward=1"])
        );
        assert_eq!(
            sysctl_write_ip_forward_argv("0"),
            s(&["sysctl", "net.ipv4.ip_forward=0"])
        );
        // terminate: 'sysctl net.ipv4.ip_forward=%s' % forwardState
        assert_eq!(
            sysctl_write_ip_forward_argv("0"),
            s(&["sysctl", "net.ipv4.ip_forward=0"])
        );
    }

    #[test]
    fn nat_flush_argvs_match_python() {
        // flush block: 'iptables -F', 'iptables -t nat -F', policies
        assert_eq!(iptables_flush_argv(), s(&["iptables", "-F"]));
        assert_eq!(
            iptables_flush_nat_argv(),
            s(&["iptables", "-t", "nat", "-F"])
        );
        assert_eq!(
            iptables_policy_argv("INPUT", "ACCEPT"),
            s(&["iptables", "-P", "INPUT", "ACCEPT"])
        );
        assert_eq!(
            iptables_policy_argv("OUTPUT", "ACCEPT"),
            s(&["iptables", "-P", "OUTPUT", "ACCEPT"])
        );
        assert_eq!(
            iptables_policy_argv("FORWARD", "DROP"),
            s(&["iptables", "-P", "FORWARD", "DROP"])
        );
    }

    #[test]
    fn nat_forward_rule_argvs_match_python_exactly() {
        // config: iptables -I FORWARD -i <local> -d <subnet> -j DROP
        assert_eq!(
            nat_forward_drop_argv(NatOp::Insert, "eth0", "10.0/8"),
            s(&["iptables", "-I", "FORWARD", "-i", "eth0", "-d", "10.0/8", "-j", "DROP"])
        );
        // config: iptables -A FORWARD -i <local> -s <subnet> -j ACCEPT
        assert_eq!(
            nat_forward_accept_src_argv(NatOp::Append, "eth0", "10.0/8"),
            s(&["iptables", "-A", "FORWARD", "-i", "eth0", "-s", "10.0/8", "-j", "ACCEPT"])
        );
        // config: iptables -A FORWARD -o <local> -d <subnet> -j ACCEPT
        assert_eq!(
            nat_forward_accept_dst_argv(NatOp::Append, "eth0", "10.0/8"),
            s(&["iptables", "-A", "FORWARD", "-o", "eth0", "-d", "10.0/8", "-j", "ACCEPT"])
        );
        // terminate deletes the same rules with -D
        assert_eq!(
            nat_forward_drop_argv(NatOp::Delete, "eth0", "10.0/8"),
            s(&["iptables", "-D", "FORWARD", "-i", "eth0", "-d", "10.0/8", "-j", "DROP"])
        );
        assert_eq!(
            nat_forward_accept_src_argv(NatOp::Delete, "eth0", "10.0/8")[1],
            "-D"
        );
        assert_eq!(
            nat_forward_accept_dst_argv(NatOp::Delete, "eth0", "10.0/8")[1],
            "-D"
        );
    }

    #[test]
    fn nat_masquerade_argv_matches_python_bare_bang() {
        // config: 'iptables -t nat -A POSTROUTING', '-s', subnet, "'!'",
        //          '-d', subnet, '-j MASQUERADE'
        // The shell-quoted "'!'" becomes the bare `!` argument in exec argv.
        assert_eq!(
            nat_masquerade_argv(NatOp::Append, "10.0/8"),
            s(&[
                "iptables",
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-s",
                "10.0/8",
                "!",
                "-d",
                "10.0/8",
                "-j",
                "MASQUERADE"
            ])
        );
        // terminate: 'iptables -t nat -D POSTROUTING', '-s', subnet,
        //            '\'!\'', '-d', subnet, '-j MASQUERADE' -> same bare `!`
        assert_eq!(
            nat_masquerade_argv(NatOp::Delete, "10.0/8"),
            s(&[
                "iptables",
                "-t",
                "nat",
                "-D",
                "POSTROUTING",
                "-s",
                "10.0/8",
                "!",
                "-d",
                "10.0/8",
                "-j",
                "MASQUERADE"
            ])
        );
        assert_eq!(NatOp::Insert.flag(), "-I");
        assert_eq!(NatOp::Append.flag(), "-A");
        assert_eq!(NatOp::Delete.flag(), "-D");
    }

    #[test]
    fn nat_manual_config_line_matches_python() {
        // nodelib.py: line = '\niface %s inet manual\n' % intf
        assert_eq!(manual_config_line("eth0"), "\niface eth0 inet manual\n");
        assert!(manual_config_needed(
            "auto lo\n",
            "\niface eth0 inet manual\n"
        ));
        assert!(!manual_config_needed(
            "auto lo\n\niface eth0 inet manual\n",
            "\niface eth0 inet manual\n"
        ));
        assert_eq!(NETWORK_INTERFACES, "/etc/network/interfaces");
    }

    #[test]
    fn nat_model_matches_python() {
        let nat = NAT::new(
            "nat0",
            None,
            None,
            false,
            false,
            [("ip".to_string(), "10.0.0.254/8".to_string())]
                .into_iter()
                .collect(),
        );
        assert_eq!(nat.subnet, "10.0/8"); // default
        assert_eq!(nat.local_intf, None);
        assert!(!nat.flush);
        assert_eq!(nat.kind(), "Host");
        assert!(nat.to_string().starts_with("<Host nat0: "));
        // net.py addNAT: natIP = nat.params['ip'].split('/')[0]
        assert_eq!(nat.nat_ip().as_deref(), Some("10.0.0.254"));
        let nat2 = NAT::new(
            "nat1",
            Some("192.168.0/24"),
            Some("ens3"),
            true,
            false,
            BTreeMap::new(),
        );
        assert_eq!(nat2.subnet, "192.168.0/24");
        assert_eq!(nat2.local_intf.as_deref(), Some("ens3"));
        assert!(nat2.flush);
        assert_eq!(nat2.nat_ip(), None);
        // in-namespace NAT that is not started: config reports NotRunning
        // via the default-intf lookup path only when it has an intf; with
        // no intfs at all it is InvalidInput.
        let mut nat3 = NAT::new("nat3", None, None, false, true, BTreeMap::new());
        let err = nat3.config().unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
    }

    // ------------------------------------------------------------------
    // Root-gated tests: real netlink bridge ops / NAT subprocess path.
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + bridge/veth support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn linux_bridge_lifecycle_over_netlink() {
        if !root_tests_enabled() {
            return;
        }
        let tag = format!("mnbr{}", std::process::id() % 100000);
        let br_name = format!("{tag}b");
        // Python's LinuxBridge.start enslaves interfaces whose *name*
        // contains the switch name (`if self.name in i.name`), so the port
        // must be named like "<br>-eth1" — "{tag}p" would be skipped.
        let port = format!("{br_name}p");
        let peer = format!("{tag}q");

        // A veth pair stands in for the switch port; the port end stays in
        // the root ns like a LinuxBridge's enslaved interface.
        let nl = mn_netlink::Netlink::new().unwrap();
        nl.veth_create_replacing(&port, &peer, None, None).unwrap();

        let mut br = LinuxBridge::new(&br_name, true, Some(4096), false, BTreeMap::new()).unwrap();
        br.add_intf(Intf::new(&port, &br_name, Some(1)), Some(1))
            .unwrap();

        br.start(&[]).expect("LinuxBridge.start as root");

        let bnl = BridgeNl::new().unwrap();
        let msgs = bnl.link_msgs().unwrap();
        let bmsg = msgs
            .iter()
            .find(|m| msg_ifname(m) == Some(br_name.as_str()))
            .expect("bridge exists after start");
        assert_eq!(
            bmsg.header.index,
            bnl.link_index(&br_name).unwrap().unwrap()
        );
        assert!(bmsg.header.flags.contains(LinkFlags::Up), "bridge is up");
        // kind bridge + STP attrs from bridge_add_msg
        assert!(bmsg.attributes.iter().any(|a| matches!(
            a,
            LinkAttribute::LinkInfo(infos)
                if infos.iter().any(|i| matches!(i, LinkInfo::Kind(InfoKind::Bridge)))
        )));
        let data = msg_bridge_data(bmsg).expect("bridge data").clone();
        assert!(data.contains(&InfoBridge::StpState(BridgeStpState::KernelStp)));
        assert!(data.contains(&InfoBridge::Priority(4096)));
        // The port is enslaved (IFLA_MASTER == bridge index).
        let pmsg = msgs
            .iter()
            .find(|m| msg_ifname(m) == Some(port.as_str()))
            .expect("port exists");
        assert!(
            pmsg.attributes
                .contains(&LinkAttribute::Controller(bmsg.header.index)),
            "port not enslaved to bridge {br_name} (index {}): port attrs {:?}",
            bmsg.header.index,
            pmsg.attributes
        );

        // attach/detach round-trip on a second port.
        let port2 = format!("{tag}r");
        let peer2 = format!("{tag}s");
        nl.veth_create_replacing(&port2, &peer2, None, None)
            .unwrap();
        br.attach(&port2).expect("attach");
        let msgs = bnl.link_msgs().unwrap();
        let br_idx = bnl.link_index(&br_name).unwrap().unwrap();
        let p2 = msgs
            .iter()
            .find(|m| msg_ifname(m) == Some(port2.as_str()))
            .unwrap();
        assert!(p2.attributes.contains(&LinkAttribute::Controller(br_idx)));
        assert!(
            p2.header.flags.contains(LinkFlags::Up),
            "attach brings port up"
        );
        br.detach(&port2).expect("detach");
        let msgs = bnl.link_msgs().unwrap();
        let p2 = msgs
            .iter()
            .find(|m| msg_ifname(m) == Some(port2.as_str()))
            .unwrap();
        // After disenslaving, the kernel simply omits IFLA_MASTER (rather
        // than reporting it as 0).
        assert!(
            !p2.attributes
                .iter()
                .any(|a| matches!(a, LinkAttribute::Controller(m) if *m != 0)),
            "port must be disenslaved after detach: {:?}",
            p2.attributes
        );

        br.stop(true).expect("LinuxBridge.stop as root");
        assert!(
            bnl.link_index(&br_name).unwrap().is_none(),
            "bridge deleted"
        );

        // stop(delete_intfs=true) already removed the enslaved ports, and
        // deleting one veth end removes the pair anyway — all deletes here
        // are best-effort cleanup.
        let _ = nl.link_delete(&port);
        let _ = nl.link_delete(&port2);
        let _ = nl.link_delete(&peer);
        let _ = nl.link_delete(&peer2);
    }

    #[test]
    #[ignore = "requires root + iptables on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn nat_subprocess_path_root() {
        if !root_tests_enabled() {
            return;
        }
        // Read-only end-to-end check of the retained shell-out path (no
        // rules are installed or flushed here): the sysctl read NAT does
        // in config(), plus an iptables listing.
        let mut nat = NAT::new("nat0", None, Some("lo"), false, false, BTreeMap::new());
        let out = nat.run_rule(&sysctl_read_ip_forward_argv()).unwrap();
        assert!(
            out.trim() == "0" || out.trim() == "1",
            "sysctl -n net.ipv4.ip_forward returned {out:?}"
        );
        let out = nat
            .run_rule(&s(&["iptables", "-t", "nat", "-L", "POSTROUTING", "-n"]))
            .unwrap();
        assert!(!out.trim().is_empty(), "iptables listing works as root");
    }
}
