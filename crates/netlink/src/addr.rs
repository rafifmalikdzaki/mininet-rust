//! Address operations over rtnetlink — the Rust equivalent of
//! `ip addr add <ip>/<prefix> dev <intf>` in `mininet/node.py` (`Intf.config`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures::StreamExt;
use netlink_packet_route::address::AddressAttribute;

use crate::{Error, Netlink, Result};

/// Summary of an address assigned to an interface (from `RTM_GETADDR`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrInfo {
    /// Interface index the address is on.
    pub index: u32,
    /// The address.
    pub address: IpAddr,
    /// Prefix length.
    pub prefix_len: u8,
}

/// Validate a prefix length for the given address family.
pub fn validate_prefix_len(addr: IpAddr, prefix_len: u8) -> Result<()> {
    let max = match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    if prefix_len > max {
        return Err(Error::InvalidInput(format!(
            "prefix length {prefix_len} too large for {addr} (max {max})"
        )));
    }
    Ok(())
}

impl Netlink {
    /// Dump addresses (equivalent of `ip addr show`), optionally filtered to
    /// one interface. Works unprivileged.
    pub fn addr_list(&self, ifname: Option<&str>) -> Result<Vec<AddrInfo>> {
        self.run(async {
            let mut req = self.handle.address().get();
            if let Some(name) = ifname {
                let idx = self
                    .link_index(name)?
                    .ok_or_else(|| Error::Other(format!("no such interface: {name:?}")))?;
                req = req.set_link_index_filter(idx);
            }
            let mut stream = req.execute();
            let mut out = Vec::new();
            while let Some(msg) = stream.next().await {
                let msg = msg?;
                let address = msg.attributes.iter().find_map(|a| match a {
                    AddressAttribute::Address(addr) => Some(*addr),
                    _ => None,
                });
                if let Some(address) = address {
                    out.push(AddrInfo {
                        index: msg.header.index,
                        address,
                        prefix_len: msg.header.prefix_len,
                    });
                }
            }
            Ok(out)
        })
    }

    /// Add an address with prefix to an interface (equivalent of
    /// `ip addr add <addr>/<prefix_len> dev <ifname>`).
    pub fn addr_add(&self, ifname: &str, addr: IpAddr, prefix_len: u8) -> Result<()> {
        validate_prefix_len(addr, prefix_len)?;
        let idx = self
            .link_index(ifname)?
            .ok_or_else(|| Error::Other(format!("no such interface: {ifname:?}")))?;
        self.run(async {
            self.handle
                .address()
                .add(idx, addr, prefix_len)
                .execute()
                .await
        })?;
        Ok(())
    }

    /// Add an IPv4 address with prefix (convenience wrapper).
    pub fn addr_add_ipv4(&self, ifname: &str, addr: Ipv4Addr, prefix_len: u8) -> Result<()> {
        self.addr_add(ifname, IpAddr::V4(addr), prefix_len)
    }

    /// Add an IPv6 address with prefix (convenience wrapper).
    pub fn addr_add_ipv6(&self, ifname: &str, addr: Ipv6Addr, prefix_len: u8) -> Result<()> {
        self.addr_add(ifname, IpAddr::V6(addr), prefix_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    #[test]
    fn validate_prefix_len_bounds() {
        let v4 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert!(validate_prefix_len(v4, 0).is_ok());
        assert!(validate_prefix_len(v4, 32).is_ok());
        assert!(validate_prefix_len(v4, 33).is_err());
        assert!(validate_prefix_len(v6, 128).is_ok());
        assert!(validate_prefix_len(v6, 129).is_err());
    }

    #[test]
    fn addr_add_rejects_bad_prefix_before_touching_kernel() {
        let nl = Netlink::new().unwrap();
        let v4 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(matches!(
            nl.addr_add("lo", v4, 33),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn addr_add_unknown_interface_reports_not_found() {
        let nl = Netlink::new().unwrap();
        let v4 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        // Unprivileged add would fail with EPERM anyway; the missing link
        // must be reported first.
        assert!(matches!(
            nl.addr_add("no-such-intf0", v4, 24),
            Err(Error::Other(_))
        ));
    }

    // ------------------------------------------------------------------
    // Root-gated test: assign IPv4/IPv6 addresses inside a fresh netns.
    // Must run on a privileged Linux host/CI runner:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn addr_add_ipv4_and_ipv6_in_netns() {
        if !root_tests_enabled() {
            return;
        }
        use crate::ns::{netns_add, netns_del, setns_name};
        let ns = format!("mnrt{}", std::process::id());
        let _ = netns_del(&ns);
        netns_add(&ns).unwrap();

        // SAFETY: child changes its own netns, runs netlink ops, then exits
        // via raw _exit. Run with --test-threads=1 (see ignore message).
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                let rc = (|| -> Result<()> {
                    setns_name(&ns)?;
                    let nl = Netlink::new()?;
                    nl.veth_create("va", "vb", None, None)?;
                    nl.link_set_up("va", true)?;
                    nl.link_set_up("vb", true)?;
                    nl.addr_add_ipv4("va", Ipv4Addr::new(10, 0, 0, 1), 24)?;
                    nl.addr_add_ipv4("vb", Ipv4Addr::new(10, 0, 0, 2), 24)?;
                    nl.addr_add_ipv6("va", "fd00::1".parse().unwrap(), 64)?;
                    let addrs = nl.addr_list(Some("va"))?;
                    assert!(
                        addrs
                            .iter()
                            .any(|a| a.address == IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))
                                && a.prefix_len == 24),
                        "v4 addr missing in {addrs:?}"
                    );
                    assert!(
                        addrs
                            .iter()
                            .any(|a| a.address.is_ipv6() && a.prefix_len == 64),
                        "v6 addr missing in {addrs:?}"
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
                    other => panic!("addr child failed: {other:?}"),
                }
            }
        }
        netns_del(&ns).unwrap();
    }
}
