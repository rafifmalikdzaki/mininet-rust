//! `Switch` — stub port of `class Switch( Node )` in `mininet/node.py`.
//!
//! Only the model is ported (dpid derivation, port base 1, control intf
//! for root-ns switches). The OpenFlow/OVS startup (`OVSSwitch.start`
//! etc.) is out of scope for task 8a and returns explicit TODO errors.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, DerefMut};

use mn_link::Intf;

use crate::controller::Controller;
use crate::error::{Error, Result};
use crate::node::Node;

/// Datapath ID length in hex digits (`Switch.dpidLen`).
pub const DPID_LEN: usize = 16;

/// Port of `Switch.defaultDpid`: a given dpid is validated (hex, ≤ 16
/// digits, colons stripped); otherwise the hex of the **first digit run**
/// in the switch name is used (`s1 -> 1`, `s1a2 -> 1`), zero-padded to
/// [`DPID_LEN`]. Python: `nums = re.findall(r'\d+', name);
/// dpid = hex(int(nums[0]))[2:]`.
pub fn default_dpid(name: &str, dpid: Option<&str>) -> Result<String> {
    if let Some(dpid) = dpid {
        let cleaned: String = dpid.chars().filter(|c| *c != ':').collect();
        if cleaned.is_empty()
            || cleaned.len() > DPID_LEN
            || u128::from_str_radix(&cleaned, 16).is_err()
        {
            return Err(Error::InvalidInput(format!(
                "invalid dpid {dpid:?}: must be 1..={DPID_LEN} hex digits"
            )));
        }
        return Ok(format!("{:0>width$}", cleaned, width = DPID_LEN));
    }
    // First run of ASCII digits (re.findall(r'\d+', name)[0]), not the
    // concatenation of all digits: "s1a2" must yield 1, not 12.
    let mut first_run: Option<String> = None;
    let mut cur = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            first_run = Some(std::mem::take(&mut cur));
            break;
        }
    }
    let digits = first_run.or(if cur.is_empty() { None } else { Some(cur) });
    let digits = digits.ok_or_else(|| {
        Error::InvalidInput(format!(
            "Unable to derive default datapath ID for {name:?} - please either \
             specify a dpid or use a canonical switch name such as s23."
        ))
    })?;
    let n = digits
        .parse::<u128>()
        .map_err(|_| Error::InvalidInput(format!("switch name number too large: {name:?}")))?;
    Ok(format!("{:0>width$}", format!("{n:x}"), width = DPID_LEN))
}

/// A switch node: a [`Node`] with a datapath ID. Stub — see module docs.
#[derive(Debug)]
pub struct Switch {
    node: Node,
    /// Datapath ID, 16 hex digits (`Switch.dpid`).
    pub dpid: String,
    /// Extra switch options (`Switch.opts`).
    pub opts: String,
    /// Port to listen on for dpctl connections (`Switch.listenPort`).
    pub listen_port: Option<u16>,
    /// Control interface (`lo`) for root-namespace switches.
    pub control_intf: Option<Intf>,
}

impl Switch {
    /// Port of `Switch.__init__` (model only).
    pub fn new(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut node = Node::new(name, in_namespace, params);
        node.port_base = 1; // Switch.portBase: switches start with port 1
        node.set_kind("Switch");
        let dpid = default_dpid(name, dpid)?;
        let control_intf = if !in_namespace {
            Some(Intf::new("lo", name, Some(0)))
        } else {
            None
        };
        Ok(Self {
            node,
            dpid,
            opts: opts.to_string(),
            listen_port,
            control_intf,
        })
    }

    /// TODO(8b): port `Switch.start(controllers)` — OVSSwitch/OVSBridge
    /// startup stays on `ovs-vsctl` subprocesses per the migration plan.
    pub fn start(&mut self, _controllers: &[&Controller]) -> Result<()> {
        Err(Error::Other(format!(
            "TODO(8b): {} switch startup (OVS/OpenFlow control plane) not ported yet",
            self.node.name
        )))
    }

    /// Port of `Switch.stop`: only deletes interfaces (the datapath kill
    /// is part of the 8b TODO). Python's default is `deleteIntfs=True`.
    pub fn stop(&mut self) {
        self.stop_with(true);
    }

    /// Port of `Switch.stop(deleteIntfs)`: delete interfaces when asked.
    pub fn stop_with(&mut self, delete_intfs: bool) {
        if delete_intfs {
            self.node.delete_intfs(true);
        }
    }

    /// Port of `Switch.defaultIntf`: the control interface (`lo`) when the
    /// switch runs in the root namespace, else the node's default
    /// interface — Python: `return self.controlIntf or
    /// Node.defaultIntf(self)`.
    pub fn default_intf(&self) -> Option<&Intf> {
        self.control_intf
            .as_ref()
            .or_else(|| self.node.default_intf())
    }

    /// Python `Switch.connected()` assumes connected by default.
    pub fn connected(&self) -> bool {
        true
    }

    /// The inner node.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Mutable access to the inner node.
    pub fn node_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl Deref for Switch {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Switch {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Switch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dpid_from_name_like_python() {
        // Python: nums = re.findall(r'\d+', name); dpid = hex(int(nums[0]))[2:]
        assert_eq!(default_dpid("s1", None).unwrap(), "0000000000000001");
        assert_eq!(default_dpid("s23", None).unwrap(), "0000000000000017");
        assert_eq!(default_dpid("s10", None).unwrap(), "000000000000000a");
        // The FIRST digit group wins, not the concatenation of all digits:
        // "s1a2" -> 1 (Python re.findall(r'\d+')[0] == '1'), not 12.
        assert_eq!(default_dpid("s1a2", None).unwrap(), "0000000000000001");
        assert_eq!(default_dpid("s10x20", None).unwrap(), "000000000000000a");
        assert_eq!(default_dpid("a2b3c", None).unwrap(), "0000000000000002");
        assert_eq!(default_dpid("s007", None).unwrap(), "0000000000000007");
        assert!(default_dpid("switch", None).is_err());
        // A '-' is not a digit, so the first digit run of "s-1" is "1".
        assert_eq!(default_dpid("s-1", None).unwrap(), "0000000000000001");
    }

    #[test]
    fn default_dpid_validates_explicit_dpid() {
        assert_eq!(
            default_dpid("s1", Some("00:00:00:00:00:00:00:2a")).unwrap(),
            "000000000000002a"
        );
        assert_eq!(default_dpid("s1", Some("2a")).unwrap(), "000000000000002a");
        assert!(default_dpid("s1", Some("not-hex")).is_err());
        assert!(default_dpid("s1", Some(&"f".repeat(17))).is_err());
        assert!(default_dpid("s1", Some("")).is_err());
    }

    #[test]
    fn switch_model_port_base_and_control_intf() {
        let s = Switch::new("s1", None, "", None, false, BTreeMap::new()).unwrap();
        assert_eq!(s.port_base, 1);
        assert_eq!(s.dpid, "0000000000000001");
        assert!(s.control_intf.is_some());
        assert_eq!(s.control_intf.as_ref().unwrap().name, "lo");
        assert_eq!(s.new_port(), 1);
        // Switch.defaultIntf: control intf (lo) for root-ns switches even
        // when the node has real interfaces on other ports.
        let mut s_root = Switch::new("s1", None, "", None, false, BTreeMap::new()).unwrap();
        s_root
            .node_mut()
            .add_intf(Intf::new("s1-eth1", "s1", Some(1)), Some(1))
            .unwrap();
        assert_eq!(s_root.default_intf().unwrap().name, "lo");
        assert_eq!(s_root.node().default_intf().unwrap().name, "s1-eth1");
        let mut s2 = Switch::new("s2", None, "", None, true, BTreeMap::new()).unwrap();
        assert!(s2.control_intf.is_none());
        // ... and for namespaced switches it falls back to the node default.
        s2.node_mut()
            .add_intf(Intf::new("s2-eth1", "s2", Some(1)), Some(1))
            .unwrap();
        assert_eq!(s2.default_intf().unwrap().name, "s2-eth1");
        assert!(s2.start(&[]).is_err()); // TODO stub
        let err = s2.start(&[]).unwrap_err().to_string();
        assert!(err.contains("TODO"), "got {err}");
        assert!(s2.connected());
        s2.stop_with(true);
        s2.stop(); // Python default: deleteIntfs=True
        assert_eq!(s2.to_string(), "<Switch s2: s2-eth1:None pid=None> ");
    }
}
