//! OVS link/interface stubs — **out of scope** for the netlink port.
//!
//! TODO(port): `OVSIntf`/`OVSLink` of `mininet/link.py`. Per the migration
//! plan (`tasks/plan.md`, risk table), Open vSwitch control stays behind
//! `ovs-vsctl` subprocess calls initially — the FFI surface is too large to
//! port now. Python behaviour recorded here for the later slice:
//!
//! - `OVSIntf` is always up and refuses any `ifconfig` other than `up`
//!   (`OVSIntf.ifconfig` raises on anything else).
//! - `OVSLink` between two `OVSSwitch`es is a *patch link*: the switch
//!   creates it, so `OVSLink.makeIntfPair` returns `(None, None)`.
//!   Non-patch OVS links (host <-> switch) are plain veth pairs and can
//!   reuse [`crate::Link`].
//! - Python warns that no more than ~64 OVS patch links should be used in
//!   a row.

use crate::intf::intf_name;

/// Patch interface on an OVSSwitch. Stub — see module docs.
#[derive(Debug, Clone)]
pub struct OVSIntf {
    /// Interface name.
    pub name: String,
    /// Owning switch name.
    pub node: String,
    /// Port number on the switch.
    pub port: Option<u32>,
}

impl OVSIntf {
    /// New OVS interface stub.
    pub fn new(name: impl Into<String>, node: impl Into<String>, port: Option<u32>) -> Self {
        Self {
            name: name.into(),
            node: node.into(),
            port,
        }
    }

    /// Python `OVSIntf.ifconfig("up")` is a no-op: an OVSIntf is always up.
    pub fn is_up(&self) -> bool {
        true
    }
}

/// OVS link. Stub — see module docs.
#[derive(Debug, Clone)]
pub struct OVSLink {
    /// First node name.
    pub node1: String,
    /// Second node name.
    pub node2: String,
    /// True when both ends are OVSSwitches (patch link created by the
    /// switch via `ovs-vsctl`, so no veth pair is made).
    pub is_patch_link: bool,
}

impl OVSLink {
    /// New OVS link stub. `is_patch_link` mirrors Python's
    /// `isinstance(node1, OVSSwitch) and isinstance(node2, OVSSwitch)`.
    pub fn new(node1: &str, node2: &str, is_patch_link: bool) -> Self {
        Self {
            node1: node1.to_string(),
            node2: node2.to_string(),
            is_patch_link,
        }
    }

    /// Python `OVSLink.makeIntfPair`: patch links return `(None, None)`
    /// (delegated to the switch); other OVS links make a normal veth pair.
    pub fn make_intf_pair(&self, port1: u32, port2: u32) -> Option<(String, String)> {
        if self.is_patch_link {
            None
        } else {
            Some((intf_name(&self.node1, port1), intf_name(&self.node2, port2)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ovs_intf_is_always_up() {
        let intf = OVSIntf::new("s1-eth1", "s1", Some(1));
        assert!(intf.is_up());
    }

    #[test]
    fn patch_links_have_no_intf_pair() {
        let patch = OVSLink::new("s1", "s2", true);
        assert!(patch.make_intf_pair(1, 1).is_none());
        let normal = OVSLink::new("h1", "s1", false);
        assert_eq!(
            normal.make_intf_pair(0, 1),
            Some(("h1-eth0".to_string(), "s1-eth1".to_string()))
        );
    }
}
