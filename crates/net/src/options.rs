//! Constructor options for [`Mininet`](crate::Mininet) — the port of the
//! keyword arguments of `Mininet.__init__` in `mininet/net.py`.
//!
//! Python's class-valued arguments (`switch=`, `host=`, `controller=`,
//! `link=`, `intf=`) are compile-time types in Rust; what remains runtime-
//! configurable is the switch *kind* ([`SwitchKind`](crate::SwitchKind),
//! the `--switch` choice of `bin/mn`), the controller *kind*
//! ([`ControllerKind`], `--controller`), and the link spec string
//! (`--link tc,bw=10,...`).

use mn_topo::Topo;

/// `Mininet(waitConnected=...)`: Python accepts `False`, `True`/`None`
/// (wait indefinitely), or a number of seconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WaitConnected {
    /// `waitConnected=False`: don't wait (`--test none` in `bin/mn`).
    #[default]
    Off,
    /// `waitConnected=True`/`None`: wait indefinitely (`bin/mn --wait`).
    Forever,
    /// `waitConnected=<seconds>` (`bin/mn --twait <s>`).
    Timed(f64),
}

/// The default controller choice (`Mininet(controller=...)` /
/// `bin/mn --controller ...`). Python's `NullController` returns `None`,
/// so a "none" controller is *not* added to the net at all — mirrored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerKind {
    /// `DefaultController` (`--controller default`/`ref`): a `Controller`
    /// node named `c0` with command `controller`, port 6653.
    Default,
    /// `NullController` (`--controller none`): adds no controller node.
    None,
}

/// Port of `Mininet.__init__`'s keyword arguments (see [`crate::Mininet::new`]).
#[derive(Debug, Clone)]
pub struct MininetOptions {
    /// `topo`: topology object to build from (Python `topo=None`).
    pub topo: Option<Topo>,
    /// The spec string the topo was built from (`--topo single,3`), kept
    /// for the dry-run [`Mininet::plan`] output's `spec:` line.
    pub topo_spec: Option<String>,
    /// `type(topo).__name__` of the built topo, for the plan's `class:` line.
    pub topo_class: Option<String>,
    /// `build=True`: build from `topo` immediately in [`Mininet::new`].
    pub build: bool,
    /// `xterms`: spawn xterms per node at build (TODO: term.py is not
    /// ported; stored only).
    pub xterms: bool,
    /// `cleanup`: clean up before creating (stored only; `mininet.clean`
    /// is not ported).
    pub cleanup: bool,
    /// `ipBase='10.0.0.0/8'`: base IP for host address allocation.
    pub ip_base: String,
    /// `inNamespace=False`: switches/controllers in net namespaces
    /// (selects `MininetWithControlNet` in Python — TODO, see crate docs).
    pub in_namespace: bool,
    /// `autoSetMacs=False` (`bin/mn --mac`).
    pub auto_set_macs: bool,
    /// `autoStaticArp=False` (`bin/mn --arp`).
    pub auto_static_arp: bool,
    /// `autoPinCpus=False` (`bin/mn --pin`, needs CPULimitedHost).
    pub auto_pin_cpus: bool,
    /// `listenPort=None` (`bin/mn --listenport`, default 6654 there;
    /// `None` = `--nolistenport`).
    pub listen_port: Option<u16>,
    /// `waitConnected=False`.
    pub wait_connected: WaitConnected,
    /// `link=Link` as a spec string (`--link default` / `--link tc,bw=10`).
    pub link_spec: String,
    /// `switch=OVSSwitch` as a spec string (`--switch default` /
    /// `--switch ovs,failMode=standalone`) — selects the [`NetSwitch`]
    /// subclass in [`Mininet::add_switch`](crate::Mininet::add_switch).
    ///
    /// [`NetSwitch`]: crate::NetSwitch
    pub switch_spec: String,
    /// `controller=DefaultController`: default controller added by
    /// `buildFromTopo` when no controller exists yet; `None` adds none.
    pub controller: Option<ControllerKind>,
}

impl Default for MininetOptions {
    fn default() -> Self {
        // Defaults mirror `Mininet.__init__`'s signature in net.py.
        Self {
            topo: None,
            topo_spec: None,
            topo_class: None,
            build: true,
            xterms: false,
            cleanup: false,
            ip_base: "10.0.0.0/8".to_string(),
            in_namespace: false,
            auto_set_macs: false,
            auto_static_arp: false,
            auto_pin_cpus: false,
            listen_port: None,
            wait_connected: WaitConnected::Off,
            link_spec: "default".to_string(),
            switch_spec: "default".to_string(),
            controller: Some(ControllerKind::Default),
        }
    }
}
