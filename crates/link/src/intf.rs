//! Interface model — the Rust port of `Intf` in `mininet/link.py`.
//!
//! Python's `Intf` is behavioural (it runs `ifconfig`/`ip link` inside its
//! owning node via `node.cmd`). Here the interface is a data object; the
//! kernel operations it needs (set up, assign address, delete) are applied
//! by [`crate::Link`] through [`mn_netlink::Netlink`] while both veth ends
//! are still in the current namespace (an interface's up/address/qdisc
//! state travels with it when it is moved into a node namespace).

use std::fmt;
use std::net::IpAddr;

/// Canonical interface name for port `n` of `node` — the port of
/// `Link.intfName`: `node.name + '-eth' + str(n)` (e.g. `h1-eth0`).
pub fn intf_name(node: &str, n: u32) -> String {
    format!("{node}-eth{n}")
}

/// State of an interface, from `RTM_GETLINK` (Python `Intf.status` /
/// `Intf.isUp`: `"OK"`/`"MISSING"` plus `"UP"` in `ip link show` output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntfStatus {
    /// Interface exists and `IFF_UP` is set.
    Up,
    /// Interface exists but is down.
    Down,
    /// Interface does not exist (in the queried namespace).
    Missing,
}

impl fmt::Display for IntfStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IntfStatus::Up => "UP",
            IntfStatus::Down => "DOWN",
            IntfStatus::Missing => "MISSING",
        })
    }
}

/// Basic interface object. Port of `mininet/link.py::Intf` (fields only;
/// the behaviour lives in [`crate::Link`]).
#[derive(Debug, Clone)]
pub struct Intf {
    /// Interface name (e.g. `h1-eth0`).
    pub name: String,
    /// Name of the owning node (where this intf most likely lives).
    pub node: String,
    /// Port number on the node (OpenFlow port if the node is a switch).
    pub port: Option<u32>,
    /// MAC address if one was assigned.
    pub mac: Option<[u8; 6]>,
    /// IP address if one was assigned.
    pub ip: Option<IpAddr>,
    /// Prefix length for `ip` (Python `Intf.prefixLen`).
    pub prefix_len: Option<u8>,
    /// Last known kernel state.
    pub status: IntfStatus,
}

impl Intf {
    /// New interface object (not yet attached to the kernel).
    pub fn new(name: impl Into<String>, node: impl Into<String>, port: Option<u32>) -> Self {
        Self {
            name: name.into(),
            node: node.into(),
            port,
            mac: None,
            ip: None,
            prefix_len: None,
            status: IntfStatus::Missing,
        }
    }

    /// Python `Intf.isUp()` (without setUp): whether the interface is up.
    pub fn is_up(&self) -> bool {
        self.status == IntfStatus::Up
    }

    /// Set the IP address/prefix (Python `Intf.setIP`, minus the ifconfig
    /// call — [`crate::Link::attach`] applies it via `addr_add`).
    pub fn set_ip(&mut self, ip: IpAddr, prefix_len: u8) {
        self.ip = Some(ip);
        self.prefix_len = Some(prefix_len);
    }
}

/// Set an IP/prefix on an interface (convenience free function, Python
/// `Intf.setIP`).
pub fn set_ip(intf: &mut Intf, ip: IpAddr, prefix_len: u8) {
    intf.set_ip(ip, prefix_len);
}

impl fmt::Display for Intf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intf_name_matches_python_link_intfname() {
        // Python: node.name + '-eth' + repr(n)
        assert_eq!(intf_name("h1", 0), "h1-eth0");
        assert_eq!(intf_name("s10", 23), "s10-eth23");
    }

    #[test]
    fn intf_status_display() {
        assert_eq!(IntfStatus::Up.to_string(), "UP");
        assert_eq!(IntfStatus::Down.to_string(), "DOWN");
        assert_eq!(IntfStatus::Missing.to_string(), "MISSING");
        let intf = Intf::new("h1-eth0", "h1", Some(0));
        assert_eq!(intf.to_string(), "h1-eth0");
        assert!(!intf.is_up());
    }
}
