//! `Host` — port of `class Host( Node ): "A host is simply a Node"`, plus
//! the automatic IP/MAC assignment hooks that `Mininet.addHost` applies in
//! `mininet/net.py`, and the `CPULimitedHost` stub.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, DerefMut};

use mn_topo::Attrs;

use crate::error::Result;
use crate::node::Node;

/// A virtual host: simply a [`Node`] (Python `class Host(Node): pass`).
///
/// Adds the `net.py addHost` defaulting hooks on top: `ip`/`mac` params
/// derived from a host index via `mn_util::ip_add_with` /
/// `mn_util::mac_colon_hex`.
#[derive(Debug)]
pub struct Host {
    node: Node,
}

impl Host {
    /// New host (Python `Host(name, inNamespace=True, **params)`).
    pub fn new(name: &str, in_namespace: bool, params: BTreeMap<String, String>) -> Self {
        let mut node = Node::new(name, in_namespace, params);
        node.set_kind("Host");
        Self { node }
    }

    /// New host with `inNamespace=True` (Python's default).
    pub fn with_params(name: &str, params: BTreeMap<String, String>) -> Self {
        Self::new(name, true, params)
    }

    /// New host from topology node attrs (`buildFromTopo` passes
    /// `topo.nodeInfo(name)` as `**params`).
    pub fn from_topo_params(name: &str, attrs: &Attrs) -> Self {
        let mut host = Self::new(name, true, BTreeMap::new());
        host.node = Node::with_topo_params(name, true, attrs);
        host.node.set_kind("Host");
        host
    }

    /// Port of `Mininet.addHost`'s defaulting (net.py):
    /// `ip = ipAdd(nextIP, ipBaseNum, prefixLen) + '/prefixLen'` and, when
    /// `autoSetMacs`, `mac = macColonHex(nextIP)`.
    ///
    /// Stores the resulting `ip`/`mac` entries in `params` **without
    /// clobbering caller-supplied values** — Python does
    /// `defaults.update(params)`, so `params` wins; here that is
    /// `entry().or_insert_with()` semantics. Returns the computed
    /// `"ip/prefix"` string, if any.
    pub fn add_host_defaults(
        &mut self,
        next_ip: i64,
        ip_base_num: i64,
        prefix_len: i64,
        auto_set_macs: bool,
    ) -> Option<String> {
        let ip = mn_util::ip_add_with(next_ip, prefix_len, ip_base_num);
        if let Some(ip) = &ip {
            self.node
                .params
                .entry("ip".to_string())
                .or_insert_with(|| format!("{ip}/{prefix_len}"));
        }
        if auto_set_macs {
            self.node
                .params
                .entry("mac".to_string())
                .or_insert_with(|| mn_util::mac_colon_hex(next_ip));
        }
        ip.map(|ip| format!("{ip}/{prefix_len}"))
    }

    /// Assign an IP from a host index (the `ipAdd` hook): sets
    /// `params["ip"] = "<ip>/<prefix>"` and returns it.
    pub fn assign_ip(&mut self, index: i64, prefix_len: i64, ip_base_num: i64) -> Option<String> {
        let ip = mn_util::ip_add_with(index, prefix_len, ip_base_num)?;
        let entry = format!("{ip}/{prefix_len}");
        self.node.params.insert("ip".to_string(), entry.clone());
        Some(entry)
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

impl Deref for Host {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Host {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

/// Stub for Python's `CPULimitedHost` (cgroup CPU bandwidth limiting).
///
/// TODO(8b/Phase 3): port `initCgroups`/`cgroupSet`/`setCPUFrac`/`setCPUs`
/// — the cgroup v1/v2 plumbing (`cgcreate`/`cgclassify`/`cgset`, RT
/// `chrt`) is out of scope for task 8a; the methods below return explicit
/// TODO errors instead of silently doing nothing.
#[derive(Debug)]
pub struct CPULimitedHost {
    /// The wrapped host.
    pub host: Host,
    /// Scheduler: `"cfs"` (default) or `"rt"`.
    pub sched: String,
    /// CFS period in microseconds (Python `period_us`, default 100000).
    pub period_us: u64,
}

impl CPULimitedHost {
    /// New CPU-limited host stub.
    pub fn new(name: &str, sched: &str, params: BTreeMap<String, String>) -> Self {
        let period_us = params
            .get("period_us")
            .and_then(|v| v.parse().ok())
            .unwrap_or(100000);
        Self {
            host: Host::with_params(name, params),
            sched: sched.to_string(),
            period_us,
        }
    }

    /// TODO: port `CPULimitedHost.setCPUFrac` (cgroup quota/period +
    /// `chrt` for rt).
    pub fn set_cpu_frac(&mut self, _f: f64, _sched: Option<&str>) -> Result<()> {
        Err(crate::Error::Other(format!(
            "TODO: CPULimitedHost cgroup CPU limiting (sched={}) not ported yet",
            self.sched
        )))
    }

    /// TODO: port `CPULimitedHost.setCPUs` (cpuset cgroup).
    pub fn set_cpus(&mut self, _cores: &str) -> Result<()> {
        Err(crate::Error::Other(
            "TODO: CPULimitedHost cpuset limiting not ported yet".to_string(),
        ))
    }

    /// TODO: port `CPULimitedHost.config` (Node.config + cgroup setup).
    pub fn config(&mut self, _cpu: f64, _cores: Option<&str>) -> Result<BTreeMap<String, String>> {
        Err(crate::Error::Other(
            "TODO: CPULimitedHost.config not ported yet".to_string(),
        ))
    }
}

impl Deref for CPULimitedHost {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.host
    }
}

impl DerefMut for CPULimitedHost {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mn_link::Intf;
    use std::net::IpAddr;

    #[test]
    fn host_is_a_node_with_host_kind() {
        let h = Host::with_params("h1", BTreeMap::new());
        assert!(h.in_namespace); // Python default inNamespace=True
        assert_eq!(h.name, "h1");
        assert_eq!(h.to_string(), "<Host h1:  pid=None> ");
    }

    #[test]
    fn add_host_defaults_matches_net_py() {
        // net.py: ip = ipAdd(nextIP, ipBaseNum=0x0a000000, prefixLen=8) + '/8'
        //         mac = macColonHex(nextIP)  (autoSetMacs)
        let mut h = Host::with_params("h1", BTreeMap::new());
        let ip = h.add_host_defaults(
            1,
            mn_util::DEFAULT_IP_BASE_NUM,
            mn_util::DEFAULT_PREFIX_LEN,
            true,
        );
        assert_eq!(ip.as_deref(), Some("10.0.0.1/8"));
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert_eq!(
            h.params.get("mac").map(String::as_str),
            Some("00:00:00:00:00:01")
        );
        let mut h2 = Host::with_params("h2", BTreeMap::new());
        let ip = h2.add_host_defaults(300, mn_util::DEFAULT_IP_BASE_NUM, 8, false);
        assert_eq!(ip.as_deref(), Some("10.0.1.44/8"));
        assert!(!h2.params.contains_key("mac"));
        // Out-of-range index: Python asserts; mn-util returns None.
        let mut h3 = Host::with_params("h3", BTreeMap::new());
        assert!(h3
            .add_host_defaults(0x1000000, mn_util::DEFAULT_IP_BASE_NUM, 8, false)
            .is_none());
    }

    #[test]
    fn add_host_defaults_does_not_clobber_caller_params() {
        // Python net.py: defaults.update(params) — caller-supplied ip/mac win.
        let mut h = Host::with_params(
            "h1",
            [
                ("ip".to_string(), "192.168.7.7/24".to_string()),
                ("mac".to_string(), "02:00:00:00:00:99".to_string()),
            ]
            .into_iter()
            .collect(),
        );
        let ip = h.add_host_defaults(1, mn_util::DEFAULT_IP_BASE_NUM, 8, true);
        // The computed default is still returned ...
        assert_eq!(ip.as_deref(), Some("10.0.0.1/8"));
        // ... but params keep the caller-supplied values.
        assert_eq!(
            h.params.get("ip").map(String::as_str),
            Some("192.168.7.7/24")
        );
        assert_eq!(
            h.params.get("mac").map(String::as_str),
            Some("02:00:00:00:00:99")
        );
        // Partial override: only ip supplied, mac still gets the default.
        let mut h2 = Host::with_params(
            "h2",
            [("ip".to_string(), "172.16.0.5/12".to_string())]
                .into_iter()
                .collect(),
        );
        h2.add_host_defaults(2, mn_util::DEFAULT_IP_BASE_NUM, 8, true);
        assert_eq!(
            h2.params.get("ip").map(String::as_str),
            Some("172.16.0.5/12")
        );
        assert_eq!(
            h2.params.get("mac").map(String::as_str),
            Some("00:00:00:00:00:02")
        );
    }

    #[test]
    fn assign_ip_uses_ip_add_hook() {
        let mut h = Host::with_params("h5", BTreeMap::new());
        assert_eq!(
            h.assign_ip(5, 8, mn_util::DEFAULT_IP_BASE_NUM).as_deref(),
            Some("10.0.0.5/8")
        );
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.5/8"));
    }

    #[test]
    fn host_config_default_applies_params_to_default_intf() {
        let mut h = Host::with_params(
            "h1",
            [("ip".to_string(), "10.0.0.1/8".to_string())]
                .into_iter()
                .collect(),
        );
        h.add_intf(Intf::new("h1-eth0", "h1", Some(0)), Some(0))
            .unwrap();
        let r = h.config_default().unwrap();
        assert_eq!(r.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert_eq!(h.ip(None), Some(IpAddr::from([10, 0, 0, 1])));
        assert_eq!(h.default_intf().unwrap().name, "h1-eth0");
    }

    #[test]
    fn host_from_topo_params() {
        let mut attrs = Attrs::new();
        attrs.insert("ip", "10.0.0.9/8");
        let h = Host::from_topo_params("h9", &attrs);
        assert_eq!(h.name, "h9");
        assert!(h.in_namespace);
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.9/8"));
        assert!(h.to_string().starts_with("<Host h9: "));
    }

    #[test]
    fn cpu_limited_host_stub_returns_todo_errors() {
        let mut h = CPULimitedHost::new("h1", "cfs", BTreeMap::new());
        assert_eq!(h.period_us, 100000);
        assert!(h.set_cpu_frac(0.5, None).is_err());
        assert!(h.set_cpus("0-1").is_err());
        assert!(h.config(-1.0, None).is_err());
        let err = h.set_cpu_frac(0.5, None).unwrap_err().to_string();
        assert!(err.contains("TODO"), "got {err}");
    }
}
