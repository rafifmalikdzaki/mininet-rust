//! The `--switch` class seam: which `mn-node` switch subclass a net's
//! switches are.
//!
//! Python's `bin/mn` maps the `--switch` spec through the `SWITCHES` dict
//! (`default|ovs|ovsk -> OVSSwitch`, `ovsbr -> OVSBridge`, `user ->
//! UserSwitch`, `ivs -> IVSSwitch`, `lxbr -> LinuxBridge`) via `customClass`
//! and passes the class to `Mininet(switch=...)`; `addSwitch` then
//! instantiates it. Rust cannot store a class in a field, so [`SwitchKind`]
//! is the runtime enum of that dict and [`NetSwitch`] is the enum of
//! instantiated switch objects stored in
//! [`Mininet::switches`](crate::Mininet::switches) (previously the base
//! [`Switch`], whose `start()` always errored — the PE-review seam: any
//! switch-bearing topo failed at `start()`).
//!
//! With [`NetSwitch`], `Mininet::start` dispatches to the real subclass
//! `start()` (`OVSSwitch` runs its `ovs-vsctl` argv, `UserSwitch` launches
//! `ofdatapath`/`ofprotocol`, ...), so switch-bearing topos start on root
//! hosts. `lxbr` (`LinuxBridge`, `mininet/nodelib.py`) is ported in
//! `mn-node::nodelib`; wiring the seam to it is a follow-up — it is modeled as the base [`Switch`] with kind `LinuxBridge`
//! so dry-run/dump still work, and `start()` fails with a clear error.
//!
//! Deviations from CPython (documented):
//! - `batch` is not defaulted to `True` the way `buildFromTopo` does
//!   (`params.setdefault('batch', True)` for classes with `batchStartup`);
//!   commands run immediately in `start()` instead of being queued and
//!   drained by `batchStartup` — the same `ovs-vsctl` argv either way.
//! - Boolean kwargs (`stp`, `inband`, `batch`, `verbose`) are parsed
//!   properly (`true/1/yes`); Python's `customClass` would pass the raw
//!   string, where even `'false'` is truthy.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use mn_node::{Controller, IVSSwitch, Node, OVSBridge, OVSSwitch, Switch, UserSwitch};

use crate::error::{Error, Result};

/// `bin/mn`'s `SWITCHES` dict keys, as a runtime enum — the `--switch`
/// spec's class choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchKind {
    /// `default|ovs|ovsk` → [`OVSSwitch`].
    Ovs,
    /// `ovsbr` → [`OVSBridge`] (standalone fail mode).
    Bridge,
    /// `user` → [`UserSwitch`] (OpenFlow reference user switch).
    User,
    /// `ivs` → [`IVSSwitch`].
    Ivs,
    /// `lxbr` → `LinuxBridge` — ported in `mn-node::nodelib`, seam wiring is a
    /// base [`Switch`], `start()` errors clearly.
    LxBr,
}

impl SwitchKind {
    /// Parse the class name of a `--switch` spec (`default`, `ovs`, ...).
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "default" | "ovs" | "ovsk" => Ok(Self::Ovs),
            "ovsbr" => Ok(Self::Bridge),
            "user" => Ok(Self::User),
            "ivs" => Ok(Self::Ivs),
            "lxbr" => Ok(Self::LxBr),
            other => Err(Error::InvalidInput(format!(
                "unknown switch class {other:?} (expected one of: \
                 default|ivs|lxbr|ovs|ovsbr|ovsk|user)"
            ))),
        }
    }

    /// The Python class name (`type(switch).__name__`, as `repr` shows it).
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Ovs => "OVSSwitch",
            Self::Bridge => "OVSBridge",
            Self::User => "UserSwitch",
            Self::Ivs => "IVSSwitch",
            Self::LxBr => "LinuxBridge",
        }
    }
}

/// Parse a boolean spec kwarg (`stp=true`, ...). Python would pass the raw
/// string through (any non-empty string is truthy); we parse it properly —
/// documented deviation, see module docs.
fn kw_bool(value: Option<&str>, default: bool) -> bool {
    match value {
        None => default,
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
    }
}

/// A switch stored in [`Mininet::switches`](crate::Mininet::switches): the
/// `mn-node` subclass selected by the `--switch` spec. Derefs to the base
/// [`Switch`], so all node/switch model access (`intf_names`,
/// `control_intf`, `params`, ...) works uniformly; lifecycle methods
/// (`start`/`stop`/`connected`/`terminate`) dispatch to the subclass.
#[derive(Debug)]
pub enum NetSwitch {
    /// `OVSSwitch` (`--switch default|ovs|ovsk`).
    Ovs(OVSSwitch),
    /// `OVSBridge` (`--switch ovsbr`).
    Bridge(OVSBridge),
    /// `UserSwitch` (`--switch user`).
    User(UserSwitch),
    /// `IVSSwitch` (`--switch ivs`).
    Ivs(IVSSwitch),
    /// `LinuxBridge` (`--switch lxbr`) — ported in mn-node::nodelib; the base
    /// [`Switch`] stands in for the model and `start()` errors.
    LxBr(Switch),
}

impl NetSwitch {
    /// Build a switch from a `--switch` spec string
    /// (`name[,k=v...]`, e.g. `default`, `ovs,failMode=standalone`) plus the
    /// node attrs `addSwitch` extracted (`dpid`, `opts`, `listenPort`, ...).
    ///
    /// Mirrors `customClass(SWITCHES, spec)` + `cls(name, **params)`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_spec(
        spec: &str,
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut parts = spec.split(',');
        let kind_name = parts.next().unwrap_or("default");
        let kwargs: Vec<(&str, &str)> = parts.filter_map(|p| p.split_once('=')).collect();
        let kw = |key: &str| kwargs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        let kind = SwitchKind::from_name(kind_name)?;
        match kind {
            SwitchKind::Ovs => Ok(Self::Ovs(OVSSwitch::new(
                name,
                dpid,
                opts,
                listen_port,
                kw("failMode").unwrap_or("secure"),
                kw("datapath").unwrap_or("kernel"),
                kw_bool(kw("inband"), false),
                kw("protocols"),
                kw("reconnectms").and_then(|v| v.parse().ok()),
                kw_bool(kw("stp"), false),
                kw_bool(kw("batch"), false),
                in_namespace,
                params,
            )?)),
            SwitchKind::Bridge => Ok(Self::Bridge(OVSBridge::new(
                name,
                dpid,
                opts,
                listen_port,
                kw("datapath").unwrap_or("kernel"),
                kw_bool(kw("inband"), false),
                kw("protocols"),
                kw("reconnectms").and_then(|v| v.parse().ok()),
                kw_bool(kw("stp"), false),
                kw_bool(kw("batch"), false),
                in_namespace,
                params,
            )?)),
            SwitchKind::User => Ok(Self::User(UserSwitch::new(
                name,
                kw("dpopts"),
                dpid,
                opts,
                listen_port,
                in_namespace,
                params,
            )?)),
            SwitchKind::Ivs => Ok(Self::Ivs(IVSSwitch::new(
                name,
                kw_bool(kw("verbose"), false),
                dpid,
                opts,
                listen_port,
                in_namespace,
                params,
            )?)),
            SwitchKind::LxBr => {
                // LinuxBridge is ported in mn-node::nodelib; wiring the seam
                // to it is a follow-up. Keep the base-Switch model so
                // dry-run/dump work, and fail clearly at start().
                let mut sw = Switch::new(name, dpid, opts, listen_port, in_namespace, params)?;
                sw.node_mut().set_kind("LinuxBridge");
                Ok(Self::LxBr(sw))
            }
        }
    }

    /// The base [`Switch`] view (model access: intfs, params, ...).
    pub fn as_switch(&self) -> &Switch {
        match self {
            Self::Ovs(s) => s.switch(),
            Self::Bridge(s) => s.ovs().switch(),
            Self::User(s) => s.switch(),
            Self::Ivs(s) => s.switch(),
            Self::LxBr(s) => s,
        }
    }

    /// Mutable base [`Switch`] view.
    pub fn as_switch_mut(&mut self) -> &mut Switch {
        match self {
            Self::Ovs(s) => s.switch_mut(),
            Self::Bridge(s) => s.ovs_mut().switch_mut(),
            Self::User(s) => s.switch_mut(),
            Self::Ivs(s) => s.switch_mut(),
            Self::LxBr(s) => s,
        }
    }

    /// The common [`Node`] view.
    pub fn node(&self) -> &Node {
        self.as_switch().node()
    }

    /// Mutable common [`Node`] view.
    pub fn node_mut(&mut self) -> &mut Node {
        self.as_switch_mut().node_mut()
    }

    /// The switch kind selected by the `--switch` spec.
    pub fn kind(&self) -> SwitchKind {
        match self {
            Self::Ovs(_) => SwitchKind::Ovs,
            Self::Bridge(_) => SwitchKind::Bridge,
            Self::User(_) => SwitchKind::User,
            Self::Ivs(_) => SwitchKind::Ivs,
            Self::LxBr(_) => SwitchKind::LxBr,
        }
    }

    /// Port of `Switch.start(controllers)` with subclass dispatch — this is
    /// what makes `Mininet::start` work on switch-bearing topos (root).
    pub fn start(&mut self, controllers: &[&Controller]) -> Result<()> {
        match self {
            Self::Ovs(s) => Ok(s.start(controllers)?),
            Self::Bridge(s) => Ok(s.start(controllers)?),
            Self::User(s) => Ok(s.start(controllers)?),
            Self::Ivs(s) => Ok(s.start(controllers)?),
            Self::LxBr(s) => Err(Error::Other(format!(
                "{}: switch class 'lxbr' (LinuxBridge, mininet/nodelib.py) is not \
                 ported in mn-node yet; use --switch ovs (default)",
                s.node().name
            ))),
        }
    }

    /// Port of `Switch.stop(deleteIntfs)` with subclass dispatch. On a
    /// not-yet-started node only the model is cleared (Python's `cmd()` on
    /// a dead shell just warns), so dry-run `stop()` stays safe non-root.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        if !self.node().started() {
            self.as_switch_mut().stop_with(delete_intfs);
            return Ok(());
        }
        match self {
            Self::Ovs(s) => Ok(s.stop(delete_intfs)?),
            Self::Bridge(s) => Ok(s.ovs_mut().stop(delete_intfs)?),
            Self::User(s) => Ok(s.stop(delete_intfs)?),
            Self::Ivs(s) => Ok(s.stop(delete_intfs)?),
            Self::LxBr(s) => {
                s.stop_with(delete_intfs);
                Ok(())
            }
        }
    }

    /// Port of `Switch.connected()` with subclass dispatch.
    pub fn connected(&mut self) -> bool {
        match self {
            Self::Ovs(s) => s.connected(),
            Self::Bridge(s) => s.connected(),
            Self::User(s) => s.connected(),
            // Base Switch/IVSSwitch assume connected (Python default).
            Self::Ivs(s) => s.connected(),
            Self::LxBr(s) => s.connected(),
        }
    }

    /// Port of `Node.terminate` (SIGHUP the shell's process group).
    pub fn terminate(&mut self) -> Result<()> {
        Ok(self.node_mut().terminate()?)
    }
}

impl Deref for NetSwitch {
    type Target = Switch;
    fn deref(&self) -> &Switch {
        self.as_switch()
    }
}

impl DerefMut for NetSwitch {
    fn deref_mut(&mut self) -> &mut Switch {
        self.as_switch_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn switch_kind_from_name_matches_bin_mn_switches_dict() {
        assert_eq!(SwitchKind::from_name("default").unwrap(), SwitchKind::Ovs);
        assert_eq!(SwitchKind::from_name("ovs").unwrap(), SwitchKind::Ovs);
        assert_eq!(SwitchKind::from_name("ovsk").unwrap(), SwitchKind::Ovs);
        assert_eq!(SwitchKind::from_name("ovsbr").unwrap(), SwitchKind::Bridge);
        assert_eq!(SwitchKind::from_name("user").unwrap(), SwitchKind::User);
        assert_eq!(SwitchKind::from_name("ivs").unwrap(), SwitchKind::Ivs);
        assert_eq!(SwitchKind::from_name("lxbr").unwrap(), SwitchKind::LxBr);
        assert!(SwitchKind::from_name("bogus").is_err());
        assert_eq!(SwitchKind::Ovs.class_name(), "OVSSwitch");
        assert_eq!(SwitchKind::Bridge.class_name(), "OVSBridge");
        assert_eq!(SwitchKind::User.class_name(), "UserSwitch");
        assert_eq!(SwitchKind::Ivs.class_name(), "IVSSwitch");
        assert_eq!(SwitchKind::LxBr.class_name(), "LinuxBridge");
    }

    #[test]
    fn default_spec_builds_ovs_switch_with_python_defaults() {
        let sw =
            NetSwitch::from_spec("default", "s1", None, "", Some(6654), false, params()).unwrap();
        assert_eq!(sw.kind(), SwitchKind::Ovs);
        // repr class name comes from Node::kind()
        assert_eq!(sw.node().kind(), "OVSSwitch");
        assert_eq!(sw.dpid, "0000000000000001");
        assert_eq!(sw.listen_port, Some(6654));
        assert!(sw.control_intf.is_some());
        // the seam: the stored value IS the OVSSwitch, so start() dispatches
        // to OVSSwitch::start (never the base Switch's "no start" error).
        assert!(matches!(sw, NetSwitch::Ovs(_)));
        let mut sw = sw;
        if let Err(e) = sw.start(&[]) {
            assert!(
                !e.to_string().contains("base Switch has no start"),
                "start must dispatch to OVSSwitch, got: {e}"
            );
        }
    }

    #[test]
    fn ovs_spec_kwargs_map_to_constructor_params() {
        let sw = NetSwitch::from_spec(
            "ovs,failMode=standalone,datapath=user,stp=true",
            "s2",
            None,
            "",
            None,
            false,
            params(),
        )
        .unwrap();
        match &sw {
            NetSwitch::Ovs(o) => {
                assert_eq!(o.fail_mode, "standalone");
                assert_eq!(o.datapath, "user");
                assert!(o.stp);
            }
            _ => panic!("expected Ovs"),
        }
        // invalid failMode is rejected (OVSSwitch::new validates)
        assert!(
            NetSwitch::from_spec("ovs,failMode=bogus", "s2", None, "", None, false, params())
                .is_err()
        );
    }

    #[test]
    fn user_ivs_ovsbr_lxbr_specs_build_their_classes() {
        let user = NetSwitch::from_spec("user", "s1", None, "", None, false, params()).unwrap();
        assert_eq!(user.node().kind(), "UserSwitch");
        assert_eq!(user.dpid.len(), 12); // UserSwitch.dpidLen

        let ivs = NetSwitch::from_spec("ivs,verbose=true", "s1", None, "", None, false, params())
            .unwrap();
        assert_eq!(ivs.node().kind(), "IVSSwitch");

        let br = NetSwitch::from_spec("ovsbr", "s1", None, "", None, false, params()).unwrap();
        assert_eq!(br.node().kind(), "OVSBridge");

        let lx = NetSwitch::from_spec("lxbr", "s1", None, "", None, false, params()).unwrap();
        assert_eq!(lx.node().kind(), "LinuxBridge");
        let mut lx = lx;
        let err = lx.start(&[]).unwrap_err().to_string();
        assert!(err.contains("not ported"), "got: {err}");
    }

    #[test]
    fn stop_on_unstarted_switch_is_a_model_only_noop() {
        // Dry-run safety: stop() on a never-started switch must not run
        // ovs-vsctl or touch the kernel.
        let mut sw =
            NetSwitch::from_spec("default", "s1", None, "", None, false, params()).unwrap();
        sw.node_mut()
            .add_intf(mn_link::Intf::new("s1-eth1", "s1", Some(1)), Some(1))
            .unwrap();
        // must not run ovs-vsctl or touch the kernel: model-only cleanup
        sw.stop(true).unwrap();
        let _ = sw.connected(); // dispatch callable without a started shell
    }
}
