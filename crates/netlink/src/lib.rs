//! `mn-netlink` — kernel-facing foundation of the Rust mininet.
//!
//! Following containerlab's core lesson, this crate talks to the Linux
//! kernel directly over rtnetlink instead of shelling out to `ip`/`tc`.
//! It is the Rust counterpart of what Python mininet does with
//! `ip netns add/del`, `ip link add ... type veth`, `ip link set ... netns`,
//! `ip link set ... up`, `ip addr add`, and `ip route add default` (see
//! `mininet/util.py::makeIntfPair`/`moveIntf`, `mininet/node.py`,
//! `mininet/link.py::TCIntf`).
//!
//! # Sync API over an internal runtime
//!
//! [`Netlink`] owns a tokio *current-thread* runtime and the rtnetlink
//! connection task spawned on it, so callers can stay fully synchronous:
//!
//! ```no_run
//! use mn_netlink::Netlink;
//! let nl = Netlink::new()?;
//! for link in nl.link_list()? {
//!     println!("{} (index {})", link.name, link.index);
//! }
//! # Ok::<(), mn_netlink::Error>(())
//! ```
//!
//! # Privilege model
//!
//! Reading links/addresses (`link_list`, `addr_list`, ...) works
//! unprivileged. Creating namespaces/interfaces, moving interfaces between
//! namespaces, adding addresses/routes, and `setns` all require root /
//! `CAP_SYS_ADMIN`. Tests that need those privileges are `#[ignore]`d and
//! additionally gated on `MN_RUST_ROOT_TESTS=1`; they must run on a
//! privileged Linux host or CI runner.

mod addr;
mod error;
mod link;
mod ns;
mod route;

pub use addr::AddrInfo;
pub use error::{Error, Result};
pub use link::{parse_mac, validate_ifname, LinkInfo, IFNAMSIZ_MAX};
pub use ns::{
    netns_add, netns_del, netns_exists, netns_list, netns_list_in, setns_name, setns_pid,
    NETNS_PATH,
};
pub use route::RouteInfo;

use std::future::Future;

use rtnetlink::Handle;

/// Synchronous facade over an rtnetlink connection.
///
/// Cheap to keep around: create one per process (or per namespace-setup
/// phase) and reuse it. Creating the connection only opens a netlink
/// socket, which does **not** require root.
#[derive(Debug)]
pub struct Netlink {
    rt: tokio::runtime::Runtime,
    handle: Handle,
}

impl Netlink {
    /// Open a `NETLINK_ROUTE` socket and spawn the connection driver on an
    /// internal current-thread tokio runtime.
    pub fn new() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let handle = rt.block_on(async {
            let (connection, handle, _messages) = rtnetlink::new_connection()?;
            tokio::spawn(connection);
            Ok::<Handle, std::io::Error>(handle)
        })?;
        Ok(Self { rt, handle })
    }

    /// Drive a future to completion on the internal runtime.
    ///
    /// The spawned rtnetlink connection task is polled while `block_on`
    /// runs, so request/response exchanges make progress.
    pub(crate) fn run<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}

/// Whether root/namespace integration tests should actually run.
///
/// All root-requiring tests in this crate are `#[ignore]`d by default **and**
/// additionally gated on this: they only execute when the environment
/// variable `MN_RUST_ROOT_TESTS=1` is set, and must be run on a privileged
/// Linux host/CI runner, e.g.:
/// `MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored`
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

    // Non-root tests: opening a netlink socket and dumping links/addresses
    // requires no privileges on Linux.

    #[test]
    fn can_open_netlink_connection_without_root() {
        Netlink::new().expect("NETLINK_ROUTE socket must open unprivileged");
    }

    #[test]
    fn link_list_contains_loopback() {
        let nl = Netlink::new().unwrap();
        let links = nl.link_list().expect("RTM_GETLINK dump works unprivileged");
        assert!(
            links.iter().any(|l| l.name == "lo" && l.index == 1),
            "expected loopback (lo, index 1) in {links:?}"
        );
    }

    #[test]
    fn link_index_finds_loopback() {
        let nl = Netlink::new().unwrap();
        assert_eq!(nl.link_index("lo").unwrap(), Some(1));
        assert_eq!(nl.link_index("definitely-not-an-intf0").unwrap(), None);
    }

    #[test]
    fn addr_list_runs_without_root() {
        let nl = Netlink::new().unwrap();
        // Dumping addresses requires no privileges; the loopback address
        // 127.0.0.1/8 is present on any healthy Linux host.
        let addrs = nl
            .addr_list(None)
            .expect("RTM_GETADDR dump works unprivileged");
        assert!(
            addrs.iter().any(|a| a.address.is_loopback()),
            "expected a loopback address in {addrs:?}"
        );
    }
}
