//! `Controller` — stub port of `class Controller( Node )` in
//! `mininet/node.py`.
//!
//! Controllers run in the root namespace by default (`inNamespace=False`)
//! and are not managed processes in this slice: `start` is an explicit
//! TODO (the `controller` binary / OVS controller invocation is a later
//! slice), while lifecycle passthroughs (`stop`/`terminate`) work.

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::ops::{Deref, DerefMut};

use crate::error::{Error, Result};
use crate::node::Node;

/// Default OpenFlow controller port (Python `Controller.port=6653`).
pub const DEFAULT_PORT: u16 = 6653;

/// An OpenFlow controller node. Stub — see module docs.
#[derive(Debug)]
pub struct Controller {
    node: Node,
    /// The controller command (Python `command='controller'`).
    pub command: String,
    /// TCP port the controller listens on.
    pub port: u16,
}

impl Controller {
    /// Port of `Controller.__init__` (model only): root namespace by
    /// default, `controller` command, port 6653.
    pub fn new(
        name: &str,
        command: &str,
        port: u16,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Self {
        let mut node = Node::new(name, in_namespace, params);
        node.set_kind("Controller");
        Self {
            node,
            command: command.to_string(),
            port,
        }
    }

    /// Python default constructor shape: `Controller(name, **params)`.
    pub fn with_params(name: &str, params: BTreeMap<String, String>) -> Self {
        Self::new(name, "controller", DEFAULT_PORT, false, params)
    }

    /// TODO(8b): port `Controller.start` — spawn the controller binary
    /// (`controller`/`ovs-controller`/ryu/...) via the node shell and
    /// wait for the listening port.
    pub fn start(&mut self) -> Result<()> {
        Err(Error::Other(format!(
            "TODO(8b): controller {} startup (command {:?}) not ported yet",
            self.node.name, self.command
        )))
    }

    /// Port of `Controller.stop`: terminate the node.
    pub fn stop(&mut self) -> Result<()> {
        self.node.terminate()
    }

    /// Python `Controller.IP()`: the control-network IP — the `ip` param
    /// if given, else loopback.
    pub fn ip(&self) -> IpAddr {
        self.node
            .params
            .get("ip")
            .and_then(|s| s.split('/').next()?.parse().ok())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]))
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

impl Deref for Controller {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Controller {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_model_defaults_match_python() {
        let mut c = Controller::with_params("c0", BTreeMap::new());
        assert_eq!(c.name, "c0");
        assert!(!c.in_namespace); // Python Controller: inNamespace=False
        assert_eq!(c.command, "controller");
        assert_eq!(c.port, 6653);
        assert_eq!(c.ip(), IpAddr::from([127, 0, 0, 1]));
        assert!(c.start().is_err());
        assert_eq!(c.node().name, "c0");
    }

    #[test]
    fn controller_ip_from_params() {
        let mut params = BTreeMap::new();
        params.insert("ip".to_string(), "10.0.0.99/8".to_string());
        let c = Controller::with_params("c1", params);
        assert_eq!(c.ip(), IpAddr::from([10, 0, 0, 99]));
        assert!(c.to_string().starts_with("<Controller c1: "));
        let mut c = c;
        c.stop().unwrap(); // no-op terminate on unstarted node
        assert!(!c.started());
    }
}
