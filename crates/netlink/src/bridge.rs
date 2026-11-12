//! Linux-bridge link operations over rtnetlink — the netlink form of the
//! `brctl addbr/addif/delif/stp/setbridgeprio` and `ifconfig <dev> up|down`
//! shell-outs in `mininet/nodelib.py`'s `LinuxBridge` (ported in
//! `mn-node::nodelib`).
//!
//! Two halves:
//!
//! - **Pure message builders** ([`bridge_add_msg`], [`enslave_msg`],
//!   [`disenslave_msg`], [`link_up_msg`]) plus dump helpers
//!   ([`msg_ifname`], [`msg_bridge_data`], [`enslaved_port_states`],
//!   [`any_port_forwarding`]) — unit-tested, no kernel access;
//! - [`BridgeNl`], a synchronous rtnetlink facade (same shape as
//!   [`Netlink`](crate::Netlink): an internal current-thread tokio runtime
//!   driving the connection) for the real link ops, and
//!   [`run_bridge_in_netns`] to run them inside another process's netns
//!   (fork + `setns(2)`, like [`run_in_netns`](crate::run_in_netns)).
//!
//! Opening the socket and dumping links needs no privileges; creating
//! bridges/enslaving/ deleting links requires root / `CAP_SYS_ADMIN`.

use std::ffi::CString;

use futures::StreamExt;
use netlink_packet_route::link::{
    BridgePortState, BridgeStpState, InfoBridge, InfoBridgePort, InfoData, InfoPortData,
    LinkAttribute, LinkInfo as LinkInfoAttr, LinkMessage,
};
use rtnetlink::{LinkBridge, LinkMessageBuilder, LinkUnspec};

use crate::{check_fork_safety, setns_path_raw, validate_ifname, Error, Result};

// Re-export of the raw packet types so dependents (e.g. `mn-node::nodelib`)
// can name them without a direct `netlink-packet-route` dependency.
pub use netlink_packet_route::link as link_packet;

// ---------------------------------------------------------------------
// BridgeNl — minimal sync rtnetlink facade for Linux-bridge link ops
// ---------------------------------------------------------------------

/// Synchronous rtnetlink facade for the bridge link operations
/// (`type bridge` create, `IFLA_MASTER` enslave, up/down, delete).
///
/// Same shape as [`Netlink`](crate::Netlink) (an internal current-thread
/// tokio runtime driving the rtnetlink connection).
#[derive(Debug)]
pub struct BridgeNl {
    rt: tokio::runtime::Runtime,
    handle: rtnetlink::Handle,
}

impl BridgeNl {
    /// Open a `NETLINK_ROUTE` socket and spawn the connection driver on an
    /// internal current-thread tokio runtime. Opening the socket needs no
    /// privileges.
    pub fn new() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Self::from_runtime(rt)
    }

    /// Build a facade on an existing current-thread runtime. Fork children
    /// must not *create* a runtime after fork(2), so the runtime is built
    /// before the fork and moved in — see [`run_bridge_in_netns`].
    pub fn from_runtime(rt: tokio::runtime::Runtime) -> Result<Self> {
        let handle = rt.block_on(async {
            let (connection, handle, _messages) = rtnetlink::new_connection()?;
            tokio::spawn(connection);
            Ok::<rtnetlink::Handle, std::io::Error>(handle)
        })?;
        Ok(Self { rt, handle })
    }

    fn run<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }

    /// Dump all links (`RTM_GETLINK`, like `ip link show`). Works
    /// unprivileged.
    pub fn link_msgs(&self) -> Result<Vec<LinkMessage>> {
        Ok(self.run(async {
            let mut stream = self.handle.link().get().execute();
            let mut out = Vec::new();
            while let Some(msg) = stream.next().await {
                out.push(msg?);
            }
            Ok::<Vec<LinkMessage>, rtnetlink::Error>(out)
        })?)
    }

    /// Kernel index of interface `name`, or `None` (`ip link show dev
    /// <name>`; we dump and filter in userspace like the rest of this
    /// crate).
    pub fn link_index(&self, name: &str) -> Result<Option<u32>> {
        Ok(self
            .link_msgs()?
            .into_iter()
            .find(|m| msg_ifname(m) == Some(name))
            .map(|m| m.header.index))
    }

    /// `RTM_NEWLINK` create — `handle.link().add(msg)`.
    pub fn link_add(&self, msg: LinkMessage) -> Result<()> {
        self.run(async { self.handle.link().add(msg).execute().await })?;
        Ok(())
    }

    /// `RTM_NEWLINK` modify — `handle.link().set(msg)` (up/down, master).
    pub fn link_set(&self, msg: LinkMessage) -> Result<()> {
        self.run(async { self.handle.link().set(msg).execute().await })?;
        Ok(())
    }

    /// `RTM_DELLINK` by index — `handle.link().del(idx)`.
    pub fn link_del(&self, index: u32) -> Result<()> {
        self.run(async { self.handle.link().del(index).execute().await })?;
        Ok(())
    }
}

/// Run `f` with a fresh [`BridgeNl`] **inside the netns of `pid`** — the
/// bridge-op equivalent of [`run_in_netns`](crate::run_in_netns): forks a
/// child that `setns(2)`es into `/proc/<pid>/ns/net`, opens the netlink
/// socket on a pre-fork runtime, runs `f`, and exits 0/1; the parent
/// `waitpid(2)`s. Requires root / `CAP_SYS_ADMIN`.
pub fn run_bridge_in_netns<F>(pid: u32, f: F) -> Result<()>
where
    F: FnOnce(&BridgeNl) -> Result<()>,
{
    check_fork_safety()?;
    let c_path = CString::new(format!("/proc/{pid}/ns/net"))
        .map_err(|_| Error::InvalidInput(format!("bad ns path for pid {pid}")))?;
    // The runtime is built pre-fork and moved into the child.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // SAFETY: the child branch calls only setns_path_raw (open+setns+close
    // on the pre-allocated CString), BridgeNl::from_runtime (socket setup
    // on the pre-fork runtime), f, and libc::_exit — no allocation after
    // fork(2); check_fork_safety gates the single-threaded precondition.
    // The parent branch only calls waitpid.
    let child = unsafe { nix::unistd::fork()? };
    match child {
        nix::unistd::ForkResult::Child => {
            let rc = (|| -> Result<i32> {
                setns_path_raw(&c_path)?;
                let nl = BridgeNl::from_runtime(rt)?;
                f(&nl)?;
                Ok(0)
            })();
            let code = rc.unwrap_or(1);
            // SAFETY: raw syscall only.
            unsafe { libc::_exit(code) }
        }
        nix::unistd::ForkResult::Parent { child } => {
            use nix::sys::wait::{waitpid, WaitStatus};
            match waitpid(child, None)? {
                WaitStatus::Exited(_, 0) => Ok(()),
                WaitStatus::Exited(_, code) => Err(Error::Other(format!(
                    "bridge netlink op inside netns of pid {pid} failed (exit code {code})"
                ))),
                other => Err(Error::Other(format!(
                    "bridge netlink child for pid {pid} ended unexpectedly: {other:?}"
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------
// Pure rtnetlink message builders (unit-tested; no kernel access)
// ---------------------------------------------------------------------

/// Interface name of a link message (`IFLA_IFNAME`).
pub fn msg_ifname(msg: &LinkMessage) -> Option<&str> {
    msg.attributes.iter().find_map(|a| match a {
        LinkAttribute::IfName(n) => Some(n.as_str()),
        _ => None,
    })
}

/// The `InfoBridge` attribute list of a link message (`IFLA_INFO_DATA`
/// under `type bridge`), if any.
pub fn msg_bridge_data(msg: &LinkMessage) -> Option<&Vec<InfoBridge>> {
    msg.attributes.iter().find_map(|a| match a {
        LinkAttribute::LinkInfo(infos) => infos.iter().find_map(|i| match i {
            LinkInfoAttr::Data(InfoData::Bridge(b)) => Some(b),
            _ => None,
        }),
        _ => None,
    })
}

/// `RTM_NEWLINK` creating a Linux bridge named `name` — the netlink form
/// of `brctl addbr <name>` (`ip link add name <name> type bridge`).
///
/// The bridge is created **down**, like `brctl addbr`;
/// the caller brings it up at the end (`ifconfig <br> up`,
/// [`link_up_msg`]). With `stp`, the message also carries
/// `IFLA_BR_STP_STATE=1` (kernel STP, `brctl stp <br> on`) and
/// `IFLA_BR_PRIORITY` (`brctl setbridgeprio`).
pub fn bridge_add_msg(name: &str, stp: bool, prio: u16) -> Result<LinkMessage> {
    validate_ifname(name)?;
    let builder = LinkMessageBuilder::<LinkBridge>::new(name);
    let builder = if stp {
        builder.stp_state(BridgeStpState::KernelStp).priority(prio)
    } else {
        builder
    };
    Ok(builder.build())
}

/// `RTM_NEWLINK` bringing link `index` up/down — `ifconfig <dev> up|down`.
pub fn link_up_msg(index: u32, up: bool) -> LinkMessage {
    let builder = LinkUnspec::new_with_index(index);
    if up {
        builder.up().build()
    } else {
        builder.down().build()
    }
}

/// `RTM_NEWLINK` enslaving port `port` to bridge `br` — `brctl addif <br>
/// <port>` == `ip link set <port> master <br>` (`IFLA_MASTER`).
pub fn enslave_msg(port: u32, br: u32) -> LinkMessage {
    LinkUnspec::new_with_index(port).controller(br).build()
}

/// `RTM_NEWLINK` releasing port `port` from its master — `brctl delif`
/// (`IFLA_MASTER = 0`, `ip link set <port> nomaster`).
pub fn disenslave_msg(port: u32) -> LinkMessage {
    LinkUnspec::new_with_index(port).nocontroller().build()
}

/// STP port states of all links enslaved to bridge `br_idx` in a link dump
/// (links with `IFLA_MASTER == br_idx` and bridge port data). This is the
/// netlink equivalent of the per-port states in `brctl showstp <br>`.
pub fn enslaved_port_states(msgs: &[LinkMessage], br_idx: u32) -> Vec<BridgePortState> {
    let mut states = Vec::new();
    for msg in msgs {
        let master = msg.attributes.iter().find_map(|a| match a {
            LinkAttribute::Controller(idx) => Some(*idx),
            _ => None,
        });
        if master != Some(br_idx) {
            continue;
        }
        for attr in &msg.attributes {
            if let LinkAttribute::LinkInfo(infos) = attr {
                for info in infos {
                    if let LinkInfoAttr::PortData(InfoPortData::BridgePort(ports)) = info {
                        for p in ports {
                            if let InfoBridgePort::State(s) = p {
                                states.push(*s);
                            }
                        }
                    }
                }
            }
        }
    }
    states
}

/// Python's `'forwarding' in self.cmd('brctl showstp', self)`: any enslaved
/// port in the forwarding state.
pub fn any_port_forwarding(states: &[BridgePortState]) -> bool {
    states.contains(&BridgePortState::Forwarding)
}
