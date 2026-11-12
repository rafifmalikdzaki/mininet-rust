//! Route operations over rtnetlink — the Rust equivalent of
//! `ip route add default via <gw>` in `mininet/node.py` (`setDefaultRoute`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::StreamExt;
use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteMessage};
use netlink_packet_route::AddressFamily;
use rtnetlink::RouteMessageBuilder;

use crate::{Netlink, Result};

/// Summary of a route entry (from `RTM_GETROUTE` dumps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInfo {
    /// Destination prefix length (0 = default route).
    pub destination_prefix_length: u8,
    /// Address family of the route.
    pub address_family: AddressFamily,
    /// Gateway (`via`), if any.
    pub gateway: Option<IpAddr>,
    /// Output interface index, if any.
    pub oif: Option<u32>,
}

impl RouteInfo {
    fn from_message(msg: &RouteMessage) -> Self {
        let gateway = msg.attributes.iter().find_map(|a| match a {
            RouteAttribute::Gateway(RouteAddress::Inet(v4)) => Some(IpAddr::V4(*v4)),
            RouteAttribute::Gateway(RouteAddress::Inet6(v6)) => Some(IpAddr::V6(*v6)),
            _ => None,
        });
        let oif = msg.attributes.iter().find_map(|a| match a {
            RouteAttribute::Oif(idx) => Some(*idx),
            _ => None,
        });
        Self {
            destination_prefix_length: msg.header.destination_prefix_length,
            address_family: msg.header.address_family,
            gateway,
            oif,
        }
    }
}

impl Netlink {
    /// Dump the main routing table (equivalent of `ip route show`).
    /// Works unprivileged.
    pub fn route_list(&self) -> Result<Vec<RouteInfo>> {
        self.run(async {
            let mut stream = self.handle.route().get(RouteMessage::default()).execute();
            let mut out = Vec::new();
            while let Some(msg) = stream.next().await {
                out.push(RouteInfo::from_message(&msg?));
            }
            Ok(out)
        })
    }

    /// Add a default IPv4 route via `gateway`, optionally bound to an output
    /// interface index (equivalent of `ip route add default via <gw> [dev <oif>]`).
    pub fn route_add_default_v4(&self, gateway: Ipv4Addr, oif: Option<u32>) -> Result<()> {
        self.run(async {
            let mut builder = RouteMessageBuilder::<Ipv4Addr>::new().gateway(gateway);
            if let Some(idx) = oif {
                builder = builder.output_interface(idx);
            }
            self.handle.route().add(builder.build()).execute().await
        })?;
        Ok(())
    }

    /// Add a default IPv6 route via `gateway`, optionally bound to an output
    /// interface index (equivalent of `ip -6 route add default via <gw>`).
    pub fn route_add_default_v6(&self, gateway: Ipv6Addr, oif: Option<u32>) -> Result<()> {
        self.run(async {
            let mut builder = RouteMessageBuilder::<Ipv6Addr>::new().gateway(gateway);
            if let Some(idx) = oif {
                builder = builder.output_interface(idx);
            }
            self.handle.route().add(builder.build()).execute().await
        })?;
        Ok(())
    }

    /// Add a default route for either address family.
    pub fn route_add_default(&self, gateway: IpAddr, oif: Option<u32>) -> Result<()> {
        match gateway {
            IpAddr::V4(v4) => self.route_add_default_v4(v4, oif),
            IpAddr::V6(v6) => self.route_add_default_v6(v6, oif),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    // ------------------------------------------------------------------
    // Root-gated test: default routes inside a fresh netns (never touches
    // the host's routing table). Must run on a privileged Linux host/CI:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn route_add_default_in_netns() {
        if !root_tests_enabled() {
            return;
        }
        use crate::ns::{netns_add, netns_del, setns_name};
        let ns = format!("mnrt{}", std::process::id());
        let _ = netns_del(&ns);
        netns_add(&ns).unwrap();

        // SAFETY: child changes its own netns, runs netlink ops, exits via
        // raw _exit. Run with --test-threads=1 (see ignore message).
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                let rc = (|| -> Result<()> {
                    setns_name(&ns)?;
                    let nl = Netlink::new()?;
                    nl.veth_create("ra", "rb", None, None)?;
                    nl.link_set_up("ra", true)?;
                    nl.link_set_up("rb", true)?;
                    nl.addr_add_ipv4("ra", Ipv4Addr::new(10, 9, 0, 1), 24)?;
                    nl.addr_add_ipv6("ra", "fd00::1".parse().unwrap(), 64)?;
                    let idx = nl.link_index("ra")?.unwrap();
                    nl.route_add_default_v4(Ipv4Addr::new(10, 9, 0, 254), Some(idx))?;
                    nl.route_add_default_v6("fd00::fe".parse().unwrap(), Some(idx))?;
                    // Verify via RTM_GETROUTE dump that both defaults exist.
                    let routes = nl.route_list()?;
                    assert!(
                        routes.iter().any(|r| r.destination_prefix_length == 0
                            && r.address_family == AddressFamily::Inet
                            && r.gateway == Some(IpAddr::V4(Ipv4Addr::new(10, 9, 0, 254)))),
                        "default v4 route missing in {routes:?}"
                    );
                    assert!(
                        routes.iter().any(|r| r.destination_prefix_length == 0
                            && r.address_family == AddressFamily::Inet6),
                        "default v6 route missing in {routes:?}"
                    );
                    Ok(())
                })();
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(if rc.is_ok() { 0 } else { 1 }) };
            }
            nix::unistd::ForkResult::Parent { child: pid } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(pid, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("route child failed: {other:?}"),
                }
            }
        }
        netns_del(&ns).unwrap();
    }
}
