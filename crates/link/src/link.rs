//! Link model — the Rust port of `Link` / `TCLink` / `TCULink` in
//! `mininet/link.py`.
//!
//! A basic link is a veth pair. Python's `Link.__init__` calls
//! `makeIntfPair` (`mininet/util.py`: `ip link add ... type veth peer ...
//! netns <pid>`) and then each `Intf` configures itself inside its node
//! (`moveIntf`: `ip link set <intf> netns <pid>`). Here [`Link::attach`]
//! does the equivalent entirely over netlink via [`mn_netlink`]:
//!
//! 1. `veth_create_replacing` (makeIntfPair, incl. optional MACs),
//! 2. `link_set_up` on both ends,
//! 3. `addr_add` for any configured IP/prefix,
//! 4. netem qdisc via [`crate::Tc`] for TC links (delay/jitter/loss/limit/bw),
//! 5. `move_to_netns_pid` of each end into its node's namespace (moveIntf).
//!
//! Steps 2–4 deliberately run *before* the move: an interface's
//! up/address/qdisc configuration travels with the veth end into the target
//! namespace, so no in-namespace `setns` round-trip is needed from the
//! orchestrator. [`Link::delete`] deletes the pair (deleting either end of
//! a veth pair removes both), like Python's `Link.delete`.

use std::fmt;

use mn_netlink::Netlink;

use crate::intf::{intf_name, Intf, IntfStatus};
use crate::params::{parse_link_spec, LinkKind, LinkParams};
use crate::tc::{NetemConfig, Tc};
use crate::{Error, Result};

/// Aggregate link status derived from the two endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// Both ends up.
    Up,
    /// Both ends down.
    Down,
    /// One end up/down and the other missing (or mixed).
    Partial,
    /// Neither end exists.
    Missing,
}

/// Kernel-operation handles used by [`Link::attach`]: the `mn-netlink`
/// facade for veth/address/move ops and the `mn-link` TrafficControl facade
/// for the netem qdisc.
#[derive(Debug)]
pub struct LinkOps {
    /// veth create/delete, link up/down, addr add, move-to-netns.
    pub nl: Netlink,
    /// qdisc add/del/list (netem).
    pub tc: Tc,
}

impl LinkOps {
    /// Open both netlink facades. Opening sockets needs no privileges.
    pub fn new() -> Result<Self> {
        Ok(Self {
            nl: Netlink::new()?,
            tc: Tc::new()?,
        })
    }
}

/// Mbps to bytes/sec for netem's `TCA_NETEM_RATE64` (tc parses `10Mbit` as
/// 10 * 10^6 bit/s = 1_250_000 B/s).
fn mbps_to_bytes_per_sec(mbps: f64) -> u64 {
    (mbps * 1_000_000.0 / 8.0) as u64
}

/// Builder for a [`Link`]: the two endpoints (node names plus optional
/// ports and optional explicit interface names), the link kind and the TC
/// params. Keeps the constructor argument lists short.
///
/// Port allocation (`Node.newPort`) belongs to the node crate; here ports
/// default to 0 (and interface names to `node-eth<port>`) when not given.
///
/// ```
/// use mn_link::{Link, LinkConfig};
/// # fn main() -> Result<(), mn_link::Error> {
/// let link = Link::new(LinkConfig::new("h1", "h2").ports(Some(0), Some(1)))?;
/// let tc = Link::new(LinkConfig::new("h1", "h2").spec("tc,bw=10,delay=5ms")?)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct LinkConfig {
    node1: String,
    node2: String,
    port1: Option<u32>,
    port2: Option<u32>,
    intf1: Option<String>,
    intf2: Option<String>,
    kind: LinkKind,
    params: LinkParams,
}

impl LinkConfig {
    /// New link config between `node1` and `node2`: default link kind and
    /// params, no ports set (ports default to 0, names to `node-eth<port>`).
    pub fn new(node1: &str, node2: &str) -> Self {
        Self {
            node1: node1.to_string(),
            node2: node2.to_string(),
            port1: None,
            port2: None,
            intf1: None,
            intf2: None,
            kind: LinkKind::Default,
            params: LinkParams::default(),
        }
    }

    /// Port numbers of both ends (`None` leaves the port unset).
    pub fn ports(mut self, port1: Option<u32>, port2: Option<u32>) -> Self {
        self.port1 = port1;
        self.port2 = port2;
        self
    }

    /// Explicit interface names for both ends (Python
    /// `intfName1`/`intfName2`); without them the names are
    /// `node-eth<port>`.
    pub fn intf_names(mut self, intf1: impl Into<String>, intf2: impl Into<String>) -> Self {
        self.intf1 = Some(intf1.into());
        self.intf2 = Some(intf2.into());
        self
    }

    /// Link class (default / tc / tcu / ovs).
    pub fn kind(mut self, kind: LinkKind) -> Self {
        self.kind = kind;
        self
    }

    /// TC params applied to both ends (TCLink/TCULink).
    pub fn params(mut self, params: LinkParams) -> Self {
        self.params = params;
        self
    }

    /// Set the link kind and TC params from a spec string like
    /// `"tc,bw=10,delay=5ms"` (see [`parse_link_spec`]).
    pub fn spec(mut self, spec: &str) -> Result<Self> {
        let parsed = parse_link_spec(spec)?;
        self.kind = parsed.kind;
        self.params = parsed.params;
        Ok(self)
    }

    /// Build the [`Link`], validating the interface names.
    pub fn build(self) -> Result<Link> {
        let LinkConfig {
            node1,
            node2,
            port1,
            port2,
            intf1,
            intf2,
            kind,
            params,
        } = self;
        let i1 = intf1.unwrap_or_else(|| intf_name(&node1, port1.unwrap_or(0)));
        let i2 = intf2.unwrap_or_else(|| intf_name(&node2, port2.unwrap_or(0)));
        mn_netlink::validate_ifname(&i1).map_err(Error::Netlink)?;
        mn_netlink::validate_ifname(&i2).map_err(Error::Netlink)?;
        if i1 == i2 {
            return Err(Error::InvalidInput(format!(
                "link ends must have different interface names ({i1:?})"
            )));
        }
        Ok(Link {
            node1: node1.clone(),
            intf1: Intf::new(i1, &node1, port1),
            node2: node2.clone(),
            intf2: Intf::new(i2, &node2, port2),
            kind,
            params,
            attached: false,
        })
    }
}

/// A basic link: a veth pair between two nodes. Port of
/// `mininet/link.py::Link`.
#[derive(Debug, Clone)]
pub struct Link {
    /// First node name.
    pub node1: String,
    /// Second node name.
    pub node2: String,
    /// Interface on node1.
    pub intf1: Intf,
    /// Interface on node2.
    pub intf2: Intf,
    /// Link class (default / tc / tcu / ovs).
    pub kind: LinkKind,
    /// TC params applied to both ends (TCLink/TCULink).
    pub params: LinkParams,
    attached: bool,
}

impl Link {
    /// Create a link model from a [`LinkConfig`] builder, e.g.
    /// `Link::new(LinkConfig::new("h1", "h2").ports(Some(0), Some(1)))`.
    pub fn new(config: LinkConfig) -> Result<Self> {
        config.build()
    }

    /// Build a link from a spec string, e.g.
    /// `Link::from_spec("h1", "h2", Some(0), Some(0), "tc,bw=10,delay=5ms")`.
    pub fn from_spec(
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        spec: &str,
    ) -> Result<Self> {
        LinkConfig::new(node1, node2)
            .ports(port1, port2)
            .spec(spec)?
            .build()
    }

    /// Port number of end 1 / end 2 (Python reads them off the intfs).
    pub fn port1(&self) -> Option<u32> {
        self.intf1.port
    }

    /// Port number of end 2.
    pub fn port2(&self) -> Option<u32> {
        self.intf2.port
    }

    /// Whether [`Link::attach`] has run and [`Link::delete`] has not.
    pub fn is_attached(&self) -> bool {
        self.attached
    }

    /// Attach the link in the kernel: create the veth pair, configure both
    /// ends, install the netem qdisc for TC links, then move each end into
    /// its node's network namespace.
    ///
    /// `node1_pid` / `node2_pid` are the pids whose netns each end belongs
    /// to (Python `node.pid`); `None` leaves the end in the current
    /// namespace (root-namespace nodes).
    ///
    /// TODO(port): when `params.bw` is set and `params.use_htb` is true,
    /// Python installs an HTB root qdisc (`handle 5:0 htb default 1`) with
    /// class `5:1` (`rate <bw>Mbit burst 15k`) and hangs netem under it.
    /// That path needs raw `tc_htb_opt`/rtab blobs (RTM_NEWTCLASS), which
    /// netlink-packet-route 0.34 does not model; this slice applies `bw`
    /// via netem's `TCA_NETEM_RATE64` instead (see crate docs).
    pub fn attach(
        &mut self,
        ops: &LinkOps,
        node1_pid: Option<u32>,
        node2_pid: Option<u32>,
    ) -> Result<()> {
        // makeIntfPair: delete stale ends, then create the pair (with MACs).
        ops.nl.veth_create_replacing(
            &self.intf1.name,
            &self.intf2.name,
            self.intf1.mac,
            self.intf2.mac,
        )?;

        // moveIntf: move each end into its node's netns first. The kernel
        // drops IFF_UP, addresses AND qdiscs on a netns move (verified
        // empirically with iproute2: a netem qdisc installed before the move
        // is gone afterwards), and Python configures the interface inside
        // the node after the move — so configure (qdisc + up + addr) each
        // end in its own namespace afterwards.
        self.move_ends(ops, node1_pid, node2_pid)?;
        self.configure_ends(ops, node1_pid, node2_pid)?;

        self.intf1.status = IntfStatus::Up;
        self.intf2.status = IntfStatus::Up;
        self.attached = true;
        Ok(())
    }

    /// Configure each end in the namespace it lives in after `move_ends`
    /// (locally for root-namespace ends, via a forked child for namespaced
    /// nodes): install the netem qdisc for TC links, bring the end up, and
    /// assign any configured IP/prefix. This must happen **after** the move
    /// because the kernel drops IFF_UP, addresses and qdiscs on a netns
    /// move.
    fn configure_ends(
        &self,
        ops: &LinkOps,
        node1_pid: Option<u32>,
        node2_pid: Option<u32>,
    ) -> Result<()> {
        for (intf, pid) in [(&self.intf1, node1_pid), (&self.intf2, node2_pid)] {
            let name = intf.name.clone();
            let ip = intf.ip;
            let prefix_len = intf.prefix_len;
            // Snapshot the netem config for the end so the closure below is
            // self-contained.
            let netem = if self.kind.uses_tc() && self.params.has_tc() {
                let bw = self.params.effective_bw_mbps(&intf.node);
                Some(NetemConfig {
                    delay: self.params.delay,
                    jitter: self.params.jitter,
                    loss_percent: self.params.loss,
                    limit: self.params.max_queue_size,
                    rate_bytes_per_sec: bw.map(mbps_to_bytes_per_sec),
                })
            } else {
                None
            };
            let tc = ops.tc;
            let configure = move |nl: &Netlink| -> Result<()> {
                if let Some(cfg) = &netem {
                    // Python clears a pre-existing root qdisc unless it is a
                    // priomap/noqueue qdisc.
                    let root_kind = tc
                        .qdisc_list(nl, &name)?
                        .into_iter()
                        .find(|q| q.parent == crate::tc::TC_H_ROOT)
                        .map(|q| q.kind);
                    if !matches!(root_kind.as_deref(), Some("noqueue") | Some("prio")) {
                        tc.del_root_qdisc(nl, &name)?;
                    }
                    tc.add_netem(nl, &name, cfg)?;
                }
                nl.link_set_up(&name, true)?;
                if let (Some(ip), Some(prefix_len)) = (ip, prefix_len) {
                    nl.addr_add(&name, ip, prefix_len)?;
                }
                Ok(())
            };
            let local = match pid {
                None => true,
                Some(p) => p == std::process::id(),
            };
            if local {
                configure(&ops.nl)?;
            } else {
                // run_in_netns's closure must return mn_netlink::Result; the
                // qdisc ops return crate::Error, so flatten to Other here
                // (the outer `?` restores Error::Netlink via From).
                mn_netlink::run_in_netns(pid.unwrap(), move |nl| {
                    configure(nl).map_err(|e| mn_netlink::Error::Other(e.to_string()))
                })?;
            }
        }
        Ok(())
    }

    /// moveIntf: move each end into its node's netns (`None` leaves the end
    /// in the current namespace, root-namespace nodes).
    fn move_ends(
        &self,
        ops: &LinkOps,
        node1_pid: Option<u32>,
        node2_pid: Option<u32>,
    ) -> Result<()> {
        if let Some(pid) = node2_pid {
            ops.nl.move_to_netns_pid(&self.intf2.name, pid)?;
        }
        if let Some(pid) = node1_pid {
            if pid != std::process::id() {
                ops.nl.move_to_netns_pid(&self.intf1.name, pid)?;
            }
        }
        Ok(())
    }

    /// Delete the link (Python `Link.delete`: `intf1.delete()` +
    /// `intf2.delete()`). Deleting either end of a veth pair removes both,
    /// so an end that was moved into another namespace needs no special
    /// handling; if neither end is present locally the link is already gone.
    pub fn delete(&mut self, nl: &Netlink) -> Result<()> {
        for intf in [&self.intf1, &self.intf2] {
            if nl.link_index(&intf.name)?.is_some() {
                nl.link_delete(&intf.name)?;
                break;
            }
        }
        self.attached = false;
        self.intf1.status = IntfStatus::Missing;
        self.intf2.status = IntfStatus::Missing;
        Ok(())
    }

    /// Python `Link.stop()` — just deletes the link.
    pub fn stop(&mut self, nl: &Netlink) -> Result<()> {
        self.delete(nl)
    }

    /// Re-query kernel state of both ends and return the aggregate status.
    pub fn refresh_status(&mut self, nl: &Netlink) -> Result<LinkStatus> {
        for intf in [&mut self.intf1, &mut self.intf2] {
            intf.status = match nl.link_get(&intf.name)? {
                None => IntfStatus::Missing,
                Some(info) if info.is_up => IntfStatus::Up,
                Some(_) => IntfStatus::Down,
            };
        }
        Ok(self.status())
    }

    /// Aggregate status from the cached per-end statuses.
    pub fn status(&self) -> LinkStatus {
        match (self.intf1.status, self.intf2.status) {
            (IntfStatus::Up, IntfStatus::Up) => LinkStatus::Up,
            (IntfStatus::Down, IntfStatus::Down) => LinkStatus::Down,
            (IntfStatus::Missing, IntfStatus::Missing) => LinkStatus::Missing,
            _ => LinkStatus::Partial,
        }
    }

    /// Python `Link.status()`: `"(%s %s)" % (intf1.status(), intf2.status())`.
    pub fn status_string(&self) -> String {
        format!("({} {})", self.intf1.status, self.intf2.status)
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Python `Link.__str__`: '%s<->%s' % (intf1, intf2)
        write!(f, "{}<->{}", self.intf1, self.intf2)
    }
}

/// `TCLink` — link with TC (netem) interfaces, port of
/// `mininet/link.py::TCLink` (sets `cls1`/`cls2` to `TCIntf`, i.e. applies
/// the link params to both ends).
#[derive(Debug, Clone)]
pub struct TCLink {
    link: Link,
}

impl TCLink {
    /// New TC link with explicit params.
    pub fn new(
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        params: LinkParams,
    ) -> Result<Self> {
        Ok(Self {
            link: Link::new(
                LinkConfig::new(node1, node2)
                    .ports(port1, port2)
                    .kind(LinkKind::Tc)
                    .params(params),
            )?,
        })
    }

    /// New TC link from a spec string, e.g. `"tc,bw=10,delay=5ms,loss=0.1"`.
    pub fn from_spec(
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        spec: &str,
    ) -> Result<Self> {
        Ok(Self {
            link: Link::from_spec(node1, node2, port1, port2, spec)?,
        })
    }

    /// Access the inner link.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Mutable access to the inner link.
    pub fn link_mut(&mut self) -> &mut Link {
        &mut self.link
    }
}

impl std::ops::Deref for TCLink {
    type Target = Link;
    fn deref(&self) -> &Link {
        &self.link
    }
}

impl std::ops::DerefMut for TCLink {
    fn deref_mut(&mut self) -> &mut Link {
        &mut self.link
    }
}

/// `TCULink` — TCLink tuned for UserSwitch (`txo=rxo=False` in Python;
/// the ethtool offload flags are not applied in this slice, TODO).
#[derive(Debug, Clone)]
pub struct TCULink {
    link: Link,
}

impl TCULink {
    /// New TCU link with explicit params.
    pub fn new(
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        params: LinkParams,
    ) -> Result<Self> {
        Ok(Self {
            link: Link::new(
                LinkConfig::new(node1, node2)
                    .ports(port1, port2)
                    .kind(LinkKind::TcU)
                    .params(params),
            )?,
        })
    }

    /// New TCU link from a spec string (e.g. `"tcu,bw=10"`).
    pub fn from_spec(
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        spec: &str,
    ) -> Result<Self> {
        Ok(Self {
            link: Link::from_spec(node1, node2, port1, port2, spec)?,
        })
    }
}

impl std::ops::Deref for TCULink {
    type Target = Link;
    fn deref(&self) -> &Link {
        &self.link
    }
}

impl std::ops::DerefMut for TCULink {
    fn deref_mut(&mut self) -> &mut Link {
        &mut self.link
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn link_construction_defaults() {
        let link = Link::new(LinkConfig::new("h1", "h2").ports(Some(0), Some(1))).unwrap();
        assert_eq!(link.intf1.name, "h1-eth0");
        assert_eq!(link.intf2.name, "h2-eth1");
        assert_eq!(link.port1(), Some(0));
        assert_eq!(link.port2(), Some(1));
        assert_eq!(link.to_string(), "h1-eth0<->h2-eth1");
        assert!(!link.is_attached());
        assert_eq!(link.status(), LinkStatus::Missing);
        assert_eq!(link.status_string(), "(MISSING MISSING)");
    }

    #[test]
    fn link_from_spec() {
        let link = Link::from_spec("h1", "s1", Some(0), Some(2), "tc,bw=10,delay=5ms").unwrap();
        assert_eq!(link.kind, LinkKind::Tc);
        assert_eq!(link.params.bw, Some(10.0));
        assert_eq!(link.intf2.name, "s1-eth2");
        let err = Link::from_spec("h1", "h2", None, None, "bogus,bw=1").unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    #[test]
    fn link_rejects_bad_names() {
        assert!(Link::new(LinkConfig::new("h1", "h2").intf_names("a/b", "h2-eth0")).is_err());
        assert!(Link::new(LinkConfig::new("h1", "h2").intf_names("same0", "same0")).is_err());
    }

    #[test]
    fn tclink_and_tculink_wrappers() {
        let mut tc = TCLink::from_spec("h1", "h2", Some(0), Some(0), "tc,loss=1").unwrap();
        assert_eq!(tc.kind, LinkKind::Tc);
        assert_eq!(tc.params.loss, Some(1.0));
        tc.params.bw = Some(5.0);
        assert_eq!(tc.link().params.bw, Some(5.0));
        let tcu = TCULink::from_spec("h1", "h2", None, None, "tcu,bw=5").unwrap();
        assert_eq!(tcu.kind, LinkKind::TcU);
        assert!(tcu.kind.uses_tc());
        assert!(!LinkKind::Default.uses_tc());
        assert!(!LinkKind::Ovs.uses_tc());
    }

    #[test]
    fn status_aggregation() {
        let mut link = Link::new(LinkConfig::new("h1", "h2")).unwrap();
        link.intf1.status = IntfStatus::Up;
        link.intf2.status = IntfStatus::Up;
        assert_eq!(link.status(), LinkStatus::Up);
        assert_eq!(link.status_string(), "(UP UP)");
        link.intf2.status = IntfStatus::Down;
        assert_eq!(link.status(), LinkStatus::Partial);
        link.intf1.status = IntfStatus::Missing;
        link.intf2.status = IntfStatus::Missing;
        assert_eq!(link.status(), LinkStatus::Missing);
        link.intf1.status = IntfStatus::Down;
        link.intf2.status = IntfStatus::Down;
        assert_eq!(link.status(), LinkStatus::Down);
    }

    #[test]
    fn mbps_conversion_matches_tc_mbit() {
        assert_eq!(mbps_to_bytes_per_sec(10.0), 1_250_000);
        assert_eq!(mbps_to_bytes_per_sec(1000.0), 125_000_000);
    }

    #[test]
    fn set_ip_helper() {
        let mut intf = Intf::new("h1-eth0", "h1", Some(0));
        intf.set_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 24);
        assert_eq!(intf.ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert_eq!(intf.prefix_len, Some(24));
    }

    // ------------------------------------------------------------------
    // Root-gated integration test: full TC link lifecycle across a netns.
    // Must run on a privileged Linux host/CI runner:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1"]
    fn tc_link_lifecycle_across_netns() {
        if !root_tests_enabled() {
            return;
        }
        use mn_netlink::{netns_add, netns_del, setns_name};

        let ns = format!("mnlk{}", std::process::id());
        let _ = netns_del(&ns);
        netns_add(&ns).unwrap();

        // A child process living in the new netns plays the role of "node2"
        // (a mininet host process). SAFETY: child calls setns_name, then a
        // sleep loop, then raw _exit only.
        let holder = unsafe { nix::unistd::fork().unwrap() };
        let holder_pid = match holder {
            nix::unistd::ForkResult::Child => {
                if setns_name(&ns).is_err() {
                    // SAFETY: raw syscall.
                    unsafe { libc::_exit(1) };
                }
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
            }
            nix::unistd::ForkResult::Parent { child } => child,
        };

        let ops = LinkOps::new().unwrap();
        let mut link = TCLink::from_spec(
            "h1",
            "h2",
            Some(0),
            Some(0),
            "tc,bw=10,delay=5ms,jitter=100us,loss=0.1,max_queue_size=200",
        )
        .unwrap();
        link.intf1
            .set_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 24);
        link.intf2
            .set_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 24);
        // Attach: intf2 is moved into the holder's netns (node2 pid).
        link.attach(&ops, None, Some(holder_pid.as_raw() as u32))
            .expect("attach as root");
        assert!(link.is_attached());
        assert_eq!(link.status(), LinkStatus::Up);

        // End 1 is in the root ns: up, address assigned, netem qdisc present.
        let l1 = ops.nl.link_get("h1-eth0").unwrap().expect("h1-eth0 exists");
        assert!(l1.is_up);
        assert_eq!(l1.kind.as_deref(), Some("veth"));
        let addrs = ops.nl.addr_list(Some("h1-eth0")).unwrap();
        assert!(
            addrs
                .iter()
                .any(|a| a.address == IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)) && a.prefix_len == 24),
            "addr missing in {addrs:?}"
        );
        let qdiscs = ops.tc.qdisc_list(&ops.nl, "h1-eth0").unwrap();
        assert!(
            qdiscs
                .iter()
                .any(|q| q.kind == "netem" && q.handle == crate::tc::NETEM_HANDLE),
            "netem qdisc missing in {qdiscs:?}"
        );

        // End 2 moved into the holder's netns.
        assert!(ops.nl.link_index("h2-eth0").unwrap().is_none());

        // Verify inside that netns with a short-lived verifier child.
        // SAFETY: the tokio runtime is built BEFORE the fork — creating a
        // runtime after fork(2) can deadlock on the malloc lock (the exact
        // hazard Netlink::from_runtime's docs warn about; observed as a hang
        // on the first privileged CI run). The child setns's into `ns`, does
        // read-only netlink dumps on the pre-fork runtime, then raw _exit.
        // (`Tc` is a unit struct riding on that same `Netlink`, so it needs
        // no runtime of its own.)
        let rt_nl = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let verifier = unsafe { nix::unistd::fork().unwrap() };
        match verifier {
            nix::unistd::ForkResult::Child => {
                // Never unwind in a forked child: a panic would drop the
                // tokio runtimes post-fork, which can deadlock, leaving the
                // parent blocked forever in waitpid (observed on CI).
                // catch_unwind + ManuallyDrop + raw _exit instead.
                // Each check maps to a distinct exit code so the parent's
                // "verifier child failed: Exited(_, N)" says which step failed.
                let rc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> std::result::Result<u8, u8> {
                        setns_name(&ns).map_err(|_| 2)?;
                        let nl = std::mem::ManuallyDrop::new(
                            mn_netlink::Netlink::from_runtime(rt_nl).map_err(|_| 3)?,
                        );
                        let tc = Tc;
                        let l2 = nl.link_get("h2-eth0").map_err(|_| 5)?.ok_or(6)?;
                        if !l2.is_up {
                            return Err(7);
                        }
                        let addrs = nl.addr_list(Some("h2-eth0")).map_err(|_| 8)?;
                        if !addrs.iter().any(|a| {
                            a.address == IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))
                                && a.prefix_len == 24
                        }) {
                            return Err(9);
                        }
                        let qdiscs = tc.qdisc_list(&nl, "h2-eth0").map_err(|_| 10)?;
                        if !qdiscs.iter().any(|q| q.kind == "netem") {
                            return Err(11);
                        }
                        Ok(0)
                    },
                ));
                let code = match rc {
                    Ok(Ok(c)) => c,
                    Ok(Err(c)) => c,
                    Err(_) => 12,
                };
                // SAFETY: raw syscall only; never returns, so no unwinding.
                unsafe { libc::_exit(code as i32) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("verifier child failed: {other:?}"),
                }
            }
        }

        // Delete from the root ns: deleting end 1 removes the whole pair.
        link.delete(&ops.nl).unwrap();
        assert!(!link.is_attached());
        assert_eq!(link.status(), LinkStatus::Missing);
        assert!(ops.nl.link_index("h1-eth0").unwrap().is_none());

        // SAFETY: kill the holder child, reap it, remove the netns.
        unsafe { libc::kill(holder_pid.as_raw(), libc::SIGKILL) };
        nix::sys::wait::waitpid(holder_pid, None).ok();
        netns_del(&ns).unwrap();
    }
}
