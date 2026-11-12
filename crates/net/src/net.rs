//! `Mininet` — the orchestrator ported from `class Mininet` in
//! `mininet/net.py`.
//!
//! See the crate docs ([`crate`]) for the ported surface and the deliberate
//! deviations. The short version:
//!
//! - construction and `build()`/`build_from_topo()` are **pure model
//!   operations** — no kernel, no processes, no root — because `mn-node`
//!   defers a node's namespace/shell to [`Node::start`] and `mn-link`
//!   defers veth creation to [`Link::attach`]. This is what makes the
//!   dry-run mode ([`Mininet::dry_run`]/[`Mininet::plan`]) work unprivileged.
//! - [`Mininet::start`] performs all kernel/process side effects in Python's
//!   order: controllers, switches, hosts, links (Python starts hosts at
//!   construction and attaches links in `addLink`; the deferred Rust
//!   lifecycle moves both into `start`, hosts before links so link ends can
//!   be moved into the hosts' namespaces).
//! - [`Mininet::stop`] runs in reverse order with cleanup (link delete,
//!   intf delete, terminate, netns delete) and always runs every step.

use std::collections::{BTreeMap, HashMap};

use mn_link::{intf_name, parse_link_spec, Link, LinkConfig, LinkOps};
use mn_netlink::Netlink;
use mn_node::{Controller, Host, Node, Switch};
use mn_topo::{Attrs, Topo};

use crate::error::{Error, Result};
use crate::map::{NameToNode, NodeId, NodeMap, NodeRef, NodeRefMut};
use crate::options::{ControllerKind, MininetOptions, WaitConnected};

/// Port of `Mininet.randMac()`: a random, non-multicast, locally
/// administered MAC (`macColonHex(rand & 0xfeffffffffff | 0x020000000000)`).
///
/// No `rand` crate in the workspace, so a splitmix64 stream seeded from the
/// clock + pid + a counter is used.
pub fn rand_mac() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos
        ^ ((std::process::id() as u64) << 32)
        ^ CTR
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let val = (z & 0xFEFF_FFFF_FFFF) | 0x0200_0000_0000;
    mn_util::mac_colon_hex(val as i64)
}

/// Convert topo [`Attrs`] to the `BTreeMap<String, String>` params stash
/// used by `mn-node` (`Node.params`), like `Node.__init__(**params)`.
fn attrs_to_params(attrs: &Attrs) -> BTreeMap<String, String> {
    attrs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The network orchestrator: port of `class Mininet` in `mininet/net.py`.
#[derive(Debug)]
pub struct Mininet {
    /// `self.topo`.
    pub topo: Option<Topo>,
    /// The spec string the topo was built from (dry-run plan output).
    pub topo_spec: Option<String>,
    /// `type(topo).__name__` (dry-run plan output).
    pub topo_class: Option<String>,
    /// `self.ipBase` (e.g. `"10.0.0.0/8"`).
    pub ip_base: String,
    /// `self.ipBaseNum` (from `netParse(ipBase)`).
    pub ip_base_num: i64,
    /// `self.prefixLen` (from `netParse(ipBase)`).
    pub prefix_len: i64,
    /// `self.nextIP` — next host IP index for `addHost`.
    next_ip: i64,
    /// `self.inNamespace`.
    pub in_namespace: bool,
    /// `self.xterms` (stored; term spawning is a TODO).
    pub xterms: bool,
    /// `self.cleanup` (stored; `mininet.clean` is not ported).
    pub cleanup: bool,
    /// `self.autoSetMacs`.
    pub auto_set_macs: bool,
    /// `self.autoStaticArp`.
    pub auto_static_arp: bool,
    /// `self.autoPinCpus`.
    pub auto_pin_cpus: bool,
    /// `self.numCores`.
    pub num_cores: usize,
    /// `self.nextCore`.
    next_core: usize,
    /// `self.listenPort` (base port, incremented per root-ns switch).
    pub listen_port: Option<u16>,
    /// `self.waitConn`.
    pub wait_connected: WaitConnected,
    /// `link=` as a spec string (`default` / `tc,bw=10,...`).
    pub link_spec: String,
    /// `controller=` default kind added by `buildFromTopo` when empty.
    pub default_controller: Option<ControllerKind>,
    /// `self.hosts` — as an insertion-ordered map (Python: list + dict).
    pub hosts: NodeMap<Host>,
    /// `self.switches`.
    pub switches: NodeMap<Switch>,
    /// `self.controllers`.
    pub controllers: NodeMap<Controller>,
    /// `self.links` (Python: a plain list).
    pub links: Vec<Link>,
    /// `self.nameToNode`: name -> which map + position.
    name_to_node: NameToNode,
    /// `self.built`.
    pub built: bool,
}

impl Mininet {
    /// Port of `Mininet.__init__`: create the (empty) network object.
    ///
    /// Unlike Python this does **not** call `ensureRoot()` — that check is
    /// deferred to the operations that actually touch the kernel
    /// ([`Mininet::start`]), which is what makes dry-run construction
    /// possible unprivileged. If `opts.build` and `opts.topo` are set,
    /// [`Mininet::build`] runs immediately (Python: `if topo and build`).
    pub fn new(opts: MininetOptions) -> Result<Self> {
        let (ip_base_num, prefix_len) = mn_util::net_parse(&opts.ip_base)
            .ok_or_else(|| Error::Util(format!("could not parse ipBase {:?}", opts.ip_base)))?;
        // Python: hostIP = (0xffffffff >> prefixLen) & ipBaseNum;
        // nextIP = hostIP if hostIP > 0 else 1
        let shift = prefix_len.clamp(0, 63) as u32;
        let host_ip = (0xFFFF_FFFF_i64 >> shift) & ip_base_num;
        let next_ip = if host_ip > 0 { host_ip } else { 1 };
        let num_cores = mn_util::num_cores();
        let mut net = Mininet {
            topo: opts.topo,
            topo_spec: opts.topo_spec,
            topo_class: opts.topo_class,
            ip_base: opts.ip_base,
            ip_base_num,
            prefix_len,
            next_ip,
            in_namespace: opts.in_namespace,
            xterms: opts.xterms,
            cleanup: opts.cleanup,
            auto_set_macs: opts.auto_set_macs,
            auto_static_arp: opts.auto_static_arp,
            auto_pin_cpus: opts.auto_pin_cpus,
            num_cores,
            next_core: 0,
            listen_port: opts.listen_port,
            wait_connected: opts.wait_connected,
            link_spec: opts.link_spec,
            default_controller: opts.controller,
            hosts: NodeMap::new(),
            switches: NodeMap::new(),
            controllers: NodeMap::new(),
            links: Vec::new(),
            name_to_node: NameToNode::new(),
            built: false,
        };
        if net.topo.is_some() && opts.build {
            net.build()?;
        }
        Ok(net)
    }

    /// Dry-run constructor: build the whole topology from a spec string
    /// (`bin/mn --topo single,3` → `"single,3"`) **without touching the
    /// kernel** — pure model build + host IP assignment. The result can be
    /// inspected via [`Mininet::plan`]/[`Mininet::dump`]/[`Mininet::links`]
    /// and is what `apps/mn --dump` prints later.
    pub fn dry_run(spec: &str, mut opts: MininetOptions) -> Result<Self> {
        let built = mn_topo::build_topo(spec)?;
        opts.topo_class = Some(built.class_name().to_string());
        opts.topo_spec = Some(spec.to_string());
        opts.topo = Some(built.topo().clone());
        opts.build = true;
        Mininet::new(opts)
    }

    // ------------------------------------------------------------------
    // addHost / addSwitch / addController / addLink
    // ------------------------------------------------------------------

    /// Port of `Mininet.addHost(name, cls=None, **params)`: default IP
    /// `ipAdd(nextIP, ipBaseNum, prefixLen)/prefixLen` (and MAC when
    /// `autoSetMacs`), caller params win, `nextIP` advances.
    pub fn add_host(&mut self, name: &str, attrs: &Attrs) -> Result<()> {
        let params = attrs_to_params(attrs);
        let in_namespace = attrs
            .get("inNamespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // Node.__init__ default inNamespace=True
        let name = attrs
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();
        let mut host = Host::new(&name, in_namespace, params);
        host.add_host_defaults(
            self.next_ip,
            self.ip_base_num,
            self.prefix_len,
            self.auto_set_macs,
        );
        if self.auto_pin_cpus {
            let core = self.next_core.to_string();
            host.node_mut()
                .params
                .entry("cores".to_string())
                .or_insert(core);
            self.next_core = (self.next_core + 1) % self.num_cores.max(1);
        }
        self.next_ip += 1;
        let idx = self.hosts.insert(&name, host);
        self.name_to_node.insert(&name, NodeId::Host(idx));
        Ok(())
    }

    /// Port of `Mininet.addSwitch(name, cls=None, **params)`: defaults
    /// `listenPort=self.listenPort`, `inNamespace=self.inNamespace`; the
    /// net's `listenPort` increments per root-namespace switch.
    pub fn add_switch(&mut self, name: &str, attrs: &Attrs) -> Result<()> {
        let params = attrs_to_params(attrs);
        let in_namespace = attrs
            .get("inNamespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.in_namespace);
        let name = attrs
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();
        let listen_port = attrs
            .get("listenPort")
            .and_then(|v| v.as_int())
            .and_then(|p| u16::try_from(p).ok())
            .or(self.listen_port);
        let dpid = attrs.get("dpid").and_then(|v| v.as_str());
        let opts = attrs.get("opts").and_then(|v| v.as_str()).unwrap_or("");
        let sw = Switch::new(&name, dpid, opts, listen_port, in_namespace, params)?;
        if !self.in_namespace && self.listen_port.is_some() {
            self.listen_port = self.listen_port.map(|p| p + 1);
        }
        let idx = self.switches.insert(&name, sw);
        self.name_to_node.insert(&name, NodeId::Switch(idx));
        Ok(())
    }

    /// Port of `Mininet.addController(name='c0', controller=None, **params)`.
    /// `ControllerKind::None` mirrors Python's `NullController`, which
    /// returns `None` and is therefore *not* added to the net.
    pub fn add_controller(
        &mut self,
        name: &str,
        kind: ControllerKind,
        attrs: &Attrs,
    ) -> Result<()> {
        if kind == ControllerKind::None {
            return Ok(());
        }
        let params = attrs_to_params(attrs);
        let port = attrs
            .get("port")
            .and_then(|v| v.as_int())
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(mn_node::controller::DEFAULT_PORT);
        let in_namespace = attrs
            .get("inNamespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false); // Controller.__init__ default inNamespace=False
                               // Python Controller.__init__ defaults: command='controller', port=6653.
        let c = Controller::new(name, "controller", port, in_namespace, params);
        let idx = self.controllers.insert(name, c);
        self.name_to_node.insert(name, NodeId::Controller(idx));
        Ok(())
    }

    /// Port of `Mininet.addLink(node1, node2, port1, port2, **params)`:
    /// resolve names, allocate ports (`node.newPort()` when not given),
    /// derive intf names (`node-eth<port>`) and random MACs, build the
    /// [`Link`] model, and register both intfs on their nodes (Python
    /// `Intf.__init__` → `node.addIntf`). Kernel attach is deferred to
    /// [`Mininet::start`].
    pub fn add_link(
        &mut self,
        node1: &str,
        node2: &str,
        port1: Option<u32>,
        port2: Option<u32>,
        extra: &Attrs,
    ) -> Result<()> {
        let id1 = self
            .name_to_node
            .get(node1)
            .ok_or_else(|| Error::InvalidInput(format!("node {node1:?} not in network")))?;
        let id2 = self
            .name_to_node
            .get(node2)
            .ok_or_else(|| Error::InvalidInput(format!("node {node2:?} not in network")))?;
        let p1 = port1.unwrap_or_else(|| self.by_id(id1).node().new_port());
        let p2 = port2.unwrap_or_else(|| self.by_id(id2).node().new_port());
        let name1 = extra
            .get("intfName1")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| intf_name(node1, p1));
        let name2 = extra
            .get("intfName2")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| intf_name(node2, p2));
        let addr1 = extra
            .get("addr1")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(rand_mac);
        let addr2 = extra
            .get("addr2")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(rand_mac);
        // Link class + TC params: base spec plus per-link overrides from the
        // topo link info (later kwargs win, like Python's dict update).
        let mut spec = self.link_spec.clone();
        let mut pairs: Vec<String> = Vec::new();
        for (k, v) in extra.iter() {
            match k {
                "node1" | "node2" | "port1" | "port2" | "intfName1" | "intfName2" | "addr1"
                | "addr2" | "cls" | "port" | "intf" => {}
                _ => pairs.push(format!("{k}={v}")),
            }
        }
        if !pairs.is_empty() {
            spec = format!("{spec},{}", pairs.join(","));
        }
        let link_spec = parse_link_spec(&spec).map_err(Error::Link)?;
        let mut link = Link::new(
            LinkConfig::new(node1, node2)
                .ports(Some(p1), Some(p2))
                .intf_names(name1, name2)
                .kind(link_spec.kind)
                .params(link_spec.params),
        )?;
        link.intf1.mac = Some(mn_netlink::parse_mac(&addr1)?);
        link.intf2.mac = Some(mn_netlink::parse_mac(&addr2)?);
        // Python Intf.__init__ registers the intf on its node.
        let i1 = link.intf1.clone();
        let i2 = link.intf2.clone();
        self.by_id_mut(id1).node_mut().add_intf(i1, Some(p1))?;
        self.by_id_mut(id2).node_mut().add_intf(i2, Some(p2))?;
        self.links.push(link);
        Ok(())
    }

    /// `addLink(**topo_link_info)` — the form `buildFromTopo` uses:
    /// `node1`/`node2`/`port1`/`port2` are read out of the link info dict.
    pub fn add_link_from_info(&mut self, info: &Attrs) -> Result<()> {
        let node1 = info
            .get("node1")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::InvalidInput("link info missing node1".to_string()))?
            .to_string();
        let node2 = info
            .get("node2")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::InvalidInput("link info missing node2".to_string()))?
            .to_string();
        let port1 = info
            .get("port1")
            .and_then(|v| v.as_int())
            .and_then(|p| u32::try_from(p).ok());
        let port2 = info
            .get("port2")
            .and_then(|v| v.as_int())
            .and_then(|p| u32::try_from(p).ok());
        self.add_link(&node1, &node2, port1, port2, info)
    }

    // ------------------------------------------------------------------
    // buildFromTopo / build / configHosts
    // ------------------------------------------------------------------

    /// Port of `Mininet.buildFromTopo(topo)`: default controller (if any),
    /// then hosts, switches, links (topo order, links sorted).
    pub fn build_from_topo(&mut self, topo: &Topo) -> Result<()> {
        eprintln!("*** Creating network");
        if self.controllers.is_empty() {
            if let Some(kind) = self.default_controller {
                // Python: for i, cls in enumerate(classes): addController('c%d' % i)
                self.add_controller("c0", kind, &Attrs::new())?;
                eprintln!("*** Adding controller");
            }
        }
        eprintln!("*** Adding hosts:");
        for host_name in topo.hosts(true) {
            let info = topo.node_info(&host_name).cloned().unwrap_or_default();
            self.add_host(&host_name, &info)?;
            eprint!("{host_name} ");
        }
        eprintln!("\n*** Adding switches:");
        for switch_name in topo.switches(true) {
            let info = topo.node_info(&switch_name).cloned().unwrap_or_default();
            self.add_switch(&switch_name, &info)?;
            eprint!("{switch_name} ");
        }
        eprintln!("\n*** Adding links:");
        for link in topo.links(true, false, true) {
            let info = link
                .info
                .ok_or_else(|| Error::InvalidInput("link info missing".to_string()))?
                .clone();
            self.add_link_from_info(&info)?;
            eprint!("({}, {}) ", link.node1, link.node2);
        }
        eprintln!();
        Ok(())
    }

    /// Port of `Mininet.build()`: build from topo (if any), configure hosts.
    /// Pure model pass until [`Mininet::start`] runs.
    pub fn build(&mut self) -> Result<()> {
        if let Some(topo) = self.topo.clone() {
            self.build_from_topo(&topo)?;
        }
        if self.in_namespace {
            // Python selects MininetWithControlNet here; configureControlNetwork
            // is a documented TODO (see crate docs).
            return Err(Error::Other(
                "inNamespace=True needs MininetWithControlNet (configureControlNetwork) — TODO, not ported".to_string(),
            ));
        }
        eprintln!("*** Configuring hosts");
        self.config_hosts()?;
        if self.auto_static_arp {
            self.static_arp()?;
        }
        self.built = true;
        Ok(())
    }

    /// Port of `Mininet.configHosts()`: `configDefault()` on each host's
    /// default intf (or `config(ip=None, mac=None)` when it has none).
    pub fn config_hosts(&mut self) -> Result<()> {
        let names: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        for name in &names {
            eprint!("{name} ");
            let h = self.hosts.get_mut(name).expect("listed above");
            if h.node().default_intf().is_some() {
                h.config_default()?;
            } else {
                h.node_mut().config(None, None, None, "up")?;
            }
        }
        eprintln!();
        Ok(())
    }

    /// Port of `Mininet.staticArp()`: all-pairs `arp -s` on running hosts.
    /// Non-started hosts (dry-run) are skipped — their shells don't exist.
    pub fn static_arp(&mut self) -> Result<()> {
        let names: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        for src in &names {
            for dst in &names {
                if src == dst {
                    continue;
                }
                let (dst_ip, dst_mac) = {
                    let d = self.hosts.get(dst).expect("listed above");
                    let mac = d.mac(None).map(|m| {
                        format!(
                            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                            m[0], m[1], m[2], m[3], m[4], m[5]
                        )
                    });
                    (d.ip(None).map(|a| a.to_string()), mac)
                };
                let h = self.hosts.get_mut(src).expect("listed above");
                if !h.started() {
                    continue; // dry-run: no shell to run arp in
                }
                if let (Some(ip), Some(mac)) = (dst_ip, dst_mac) {
                    h.set_arp(&ip, &mac)?;
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // start / stop
    // ------------------------------------------------------------------

    /// Port of `Mininet.start()` — controllers, switches, hosts, links, in
    /// that order (Python starts host shells at construction and attaches
    /// links in `addLink`; the deferred Rust lifecycle performs both here,
    /// hosts before links so veth ends can move into host namespaces).
    /// Then host kernel configuration and (optionally) waitConnected.
    pub fn start(&mut self) -> Result<()> {
        if !self.built {
            self.build()?;
        }
        eprintln!("*** Starting controller");
        let c_names: Vec<String> = self.controllers.keys().map(str::to_string).collect();
        for name in &c_names {
            eprint!("{name} ");
            let c = self.controllers.get_mut(name).expect("listed above");
            c.start()?;
        }
        eprintln!();
        eprintln!("*** Starting {} switches", self.switches.len());
        let controllers: Vec<&Controller> = self.controllers.values().collect();
        let s_names: Vec<String> = self.switches.keys().map(str::to_string).collect();
        for name in &s_names {
            eprint!("{name} ");
            let s = self.switches.get_mut(name).expect("listed above");
            s.start(&controllers)?;
        }
        eprintln!();
        eprintln!("*** Starting {} hosts", self.hosts.len());
        let h_names: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        for name in &h_names {
            eprint!("{name} ");
            let h = self.hosts.get_mut(name).expect("listed above");
            h.node_mut().start()?;
        }
        eprintln!();
        eprintln!("*** Starting {} links", self.links.len());
        let ops = LinkOps::new().map_err(Error::Link)?;
        // pid per node name, resolved before the mutable link loop
        let pids: HashMap<String, Option<u32>> = self
            .name_to_node
            .keys()
            .map(|n| {
                let node = self.get(n).expect("name_to_node in sync").node();
                let pid = if node.in_namespace { node.pid() } else { None };
                (n.to_string(), pid)
            })
            .collect();
        for link in &mut self.links {
            let p1 = pids.get(&link.node1).copied().flatten();
            let p2 = pids.get(&link.node2).copied().flatten();
            link.attach(&ops, p1, p2).map_err(Error::Link)?;
            eprint!(".");
        }
        eprintln!();
        // Kernel-side host configuration. Python configures hosts in
        // build() because its shells/veths already exist there; the deferred
        // Rust lifecycle applies the (already model-set) IP/MAC/lo here,
        // after the link ends have been moved into the host namespaces.
        self.config_hosts()?;
        if self.wait_connected != WaitConnected::Off {
            self.wait_connected_switches()?;
        }
        Ok(())
    }

    /// Port of `Mininet.waitConnected`: wait until all switches report
    /// connected (the `Switch::connected` stub is always true today).
    pub fn wait_connected_switches(&mut self) -> Result<()> {
        eprintln!("*** Waiting for switches to connect");
        let timeout = match self.wait_connected {
            WaitConnected::Off => Some(0.0),
            WaitConnected::Forever => None,
            WaitConnected::Timed(t) => Some(t),
        };
        let start = std::time::Instant::now();
        loop {
            let names: Vec<String> = self.switches.keys().map(str::to_string).collect();
            let mut remaining: Vec<String> = Vec::new();
            for name in &names {
                let s = self.switches.get(name).expect("listed above");
                if s.connected() {
                    eprint!("{name} ");
                } else {
                    remaining.push(name.clone());
                }
            }
            if remaining.is_empty() {
                eprintln!();
                return Ok(());
            }
            if let Some(t) = timeout {
                if start.elapsed().as_secs_f64() >= t {
                    eprintln!("\n*** Timed out after {} seconds", t);
                    return Err(Error::Other(format!(
                        "{} switch(es) not connected to a controller",
                        remaining.len()
                    )));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    /// Port of `Mininet.stop()` — reverse of [`Mininet::start`] (links, hosts,
    /// switches, controllers), each with cleanup. Every step runs even if an
    /// earlier one failed; the first error is returned.
    pub fn stop(&mut self) -> Result<()> {
        let mut errs: Vec<Error> = Vec::new();
        eprintln!("*** Stopping {} links", self.links.len());
        let nl = Netlink::new()?;
        for link in &mut self.links {
            eprint!(".");
            if let Err(e) = link.stop(&nl) {
                errs.push(Error::Link(e));
            }
        }
        eprintln!();
        eprintln!("*** Stopping {} hosts", self.hosts.len());
        let h_names: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        for name in &h_names {
            eprint!("{name} ");
            let h = self.hosts.get_mut(name).expect("listed above");
            if let Err(e) = h.stop(true) {
                errs.push(Error::Node(e));
            }
            if let Err(e) = h.terminate() {
                errs.push(Error::Node(e));
            }
        }
        eprintln!();
        eprintln!("*** Stopping {} switches", self.switches.len());
        let s_names: Vec<String> = self.switches.keys().map(str::to_string).collect();
        for name in &s_names {
            eprint!("{name} ");
            let s = self.switches.get_mut(name).expect("listed above");
            s.stop();
            if let Err(e) = s.terminate() {
                errs.push(Error::Node(e));
            }
        }
        eprintln!();
        eprintln!("*** Stopping {} controllers", self.controllers.len());
        let c_names: Vec<String> = self.controllers.keys().map(str::to_string).collect();
        for name in &c_names {
            eprint!("{name} ");
            let c = self.controllers.get_mut(name).expect("listed above");
            if let Err(e) = c.stop() {
                errs.push(Error::Node(e));
            }
        }
        eprintln!("\n*** Done");
        self.built = false;
        if let Some(e) = errs.into_iter().next() {
            return Err(e);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // deletions and lookups
    // ------------------------------------------------------------------

    /// Port of `Mininet.delNode`: stop(deleteIntfs=True) + terminate +
    /// remove from its list and from `nameToNode`.
    pub fn del_node(&mut self, name: &str) -> Result<()> {
        let id = self
            .name_to_node
            .get(name)
            .ok_or_else(|| Error::InvalidInput(format!("node {name:?} not in network")))?;
        match id {
            NodeId::Host(i) => {
                let h = self.hosts.at_mut(i);
                h.stop(true)?;
                h.terminate()?;
                self.hosts.remove(name);
            }
            NodeId::Switch(i) => {
                let s = self.switches.at_mut(i);
                s.stop_with(true);
                s.terminate()?;
                self.switches.remove(name);
            }
            NodeId::Controller(i) => {
                let c = self.controllers.at_mut(i);
                c.stop()?;
                c.terminate()?;
                self.controllers.remove(name);
            }
        }
        self.name_to_node.remove(name);
        Ok(())
    }

    /// Port of `Mininet.delHost`.
    pub fn del_host(&mut self, name: &str) -> Result<()> {
        self.del_node(name)
    }

    /// Port of `Mininet.delSwitch`.
    pub fn del_switch(&mut self, name: &str) -> Result<()> {
        self.del_node(name)
    }

    /// Port of `Mininet.delController`.
    pub fn del_controller(&mut self, name: &str) -> Result<()> {
        self.del_node(name)
    }

    /// Port of `Mininet.linksBetween(node1, node2)`: indices into
    /// [`Mininet::links`] of links connecting the two nodes (either order).
    pub fn links_between(&self, node1: &str, node2: &str) -> Vec<usize> {
        self.links
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                (l.node1 == node1 && l.node2 == node2) || (l.node1 == node2 && l.node2 == node1)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn by_id(&self, id: NodeId) -> NodeRef<'_> {
        match id {
            NodeId::Host(i) => NodeRef::Host(self.hosts.at(i)),
            NodeId::Switch(i) => NodeRef::Switch(self.switches.at(i)),
            NodeId::Controller(i) => NodeRef::Controller(self.controllers.at(i)),
        }
    }

    fn by_id_mut(&mut self, id: NodeId) -> NodeRefMut<'_> {
        match id {
            NodeId::Host(i) => NodeRefMut::Host(self.hosts.at_mut(i)),
            NodeId::Switch(i) => NodeRefMut::Switch(self.switches.at_mut(i)),
            NodeId::Controller(i) => NodeRefMut::Controller(self.controllers.at_mut(i)),
        }
    }

    /// Port of `Mininet.getNodeByName` / `get` (`net[name]`): node(s) with
    /// the given name(s). Returns `None` where Python raises `KeyError`.
    pub fn get(&self, name: &str) -> Option<NodeRef<'_>> {
        self.name_to_node.get(name).map(|id| self.by_id(id))
    }

    /// `getNodeByName(*args)` multi-name form.
    pub fn get_many(&self, names: &[&str]) -> Vec<NodeRef<'_>> {
        names.iter().filter_map(|n| self.get(n)).collect()
    }

    /// `net.host(name)`-style typed lookup.
    pub fn host(&self, name: &str) -> Option<&Host> {
        self.name_to_node.get(name).and_then(|id| match id {
            NodeId::Host(i) => Some(self.hosts.at(i)),
            _ => None,
        })
    }

    /// `net.switch(name)`-style typed lookup.
    pub fn switch(&self, name: &str) -> Option<&Switch> {
        self.name_to_node.get(name).and_then(|id| match id {
            NodeId::Switch(i) => Some(self.switches.at(i)),
            _ => None,
        })
    }

    /// `net.controller(name)`-style typed lookup.
    pub fn controller(&self, name: &str) -> Option<&Controller> {
        self.name_to_node.get(name).and_then(|id| match id {
            NodeId::Controller(i) => Some(self.controllers.at(i)),
            _ => None,
        })
    }

    /// The common [`Node`] view of a named node.
    pub fn node(&self, name: &str) -> Option<&Node> {
        self.get(name).map(|r| r.node())
    }

    /// `nameToNode[name]` raw index.
    pub fn name_to_node(&self, name: &str) -> Option<NodeId> {
        self.name_to_node.get(name)
    }

    /// Mutable variant of [`Mininet::get`].
    pub fn get_mut(&mut self, name: &str) -> Option<NodeRefMut<'_>> {
        self.name_to_node.get(name).map(|id| self.by_id_mut(id))
    }

    /// Mutable typed host lookup.
    pub fn host_mut(&mut self, name: &str) -> Option<&mut Host> {
        match self.name_to_node.get(name) {
            Some(NodeId::Host(i)) => Some(self.hosts.at_mut(i)),
            _ => None,
        }
    }

    /// `name in net` (`__contains__`).
    pub fn contains(&self, name: &str) -> bool {
        self.name_to_node.contains(name)
    }

    /// `__iter__`/`keys()`: node names — hosts, then switches, then
    /// controllers, each in insertion order.
    pub fn keys(&self) -> Vec<String> {
        self.hosts
            .keys()
            .chain(self.switches.keys())
            .chain(self.controllers.keys())
            .map(str::to_string)
            .collect()
    }

    /// `values()`: all nodes in `keys()` order.
    pub fn values(&self) -> Vec<NodeRef<'_>> {
        self.keys()
            .iter()
            .map(|n| self.get(n).expect("keys in sync"))
            .collect()
    }

    /// `__len__`: number of nodes.
    pub fn len(&self) -> usize {
        self.name_to_node.len()
    }

    /// Whether the net has no nodes.
    pub fn is_empty(&self) -> bool {
        self.name_to_node.is_empty()
    }

    /// Iterate node names (`for name in net`).
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.hosts
            .keys()
            .chain(self.switches.keys())
            .chain(self.controllers.keys())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    fn empty() -> Attrs {
        Attrs::new()
    }

    fn opts() -> MininetOptions {
        MininetOptions {
            controller: None,
            ..Default::default()
        }
    }

    #[test]
    fn add_host_assigns_sequential_ips_like_python() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            ..opts()
        })
        .unwrap();
        net.add_host("h1", &empty()).unwrap();
        net.add_host("h2", &empty()).unwrap();
        // explicit ip param wins over the computed default
        let mut attrs = Attrs::new();
        attrs.insert("ip", "10.0.0.99/8");
        net.add_host("h3", &attrs).unwrap();
        net.add_host("h4", &empty()).unwrap();
        assert_eq!(
            net.host("h1")
                .unwrap()
                .node()
                .params
                .get("ip")
                .map(String::as_str),
            Some("10.0.0.1/8")
        );
        assert_eq!(
            net.host("h2")
                .unwrap()
                .node()
                .params
                .get("ip")
                .map(String::as_str),
            Some("10.0.0.2/8")
        );
        assert_eq!(
            net.host("h3")
                .unwrap()
                .node()
                .params
                .get("ip")
                .map(String::as_str),
            Some("10.0.0.99/8")
        );
        // h4 still gets the next sequential IP (nextIP advanced for h3 too)
        assert_eq!(
            net.host("h4")
                .unwrap()
                .node()
                .params
                .get("ip")
                .map(String::as_str),
            Some("10.0.0.4/8")
        );
    }

    #[test]
    fn add_host_auto_set_macs_and_ip_base() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            auto_set_macs: true,
            ip_base: "192.168.0.0/16".to_string(),
            ..opts()
        })
        .unwrap();
        net.add_host("h1", &empty()).unwrap();
        let h = net.host("h1").unwrap();
        assert_eq!(
            h.node().params.get("ip").map(String::as_str),
            Some("192.168.0.1/16")
        );
        assert_eq!(
            h.node().params.get("mac").map(String::as_str),
            Some("00:00:00:00:00:01")
        );
    }

    #[test]
    fn add_switch_increments_listen_port_only_in_root_ns() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            listen_port: Some(6654),
            ..opts()
        })
        .unwrap();
        net.add_switch("s1", &empty()).unwrap();
        net.add_switch("s2", &empty()).unwrap();
        assert_eq!(net.switch("s1").unwrap().listen_port, Some(6654));
        assert_eq!(net.switch("s2").unwrap().listen_port, Some(6655));
        assert_eq!(net.listen_port, Some(6656));
        // explicit listenPort param wins for that switch; net still increments
        let mut attrs = Attrs::new();
        attrs.insert("listenPort", 7000i64);
        net.add_switch("s3", &attrs).unwrap();
        assert_eq!(net.switch("s3").unwrap().listen_port, Some(7000));
        assert_eq!(net.listen_port, Some(6657));
    }

    #[test]
    fn add_controller_none_adds_nothing_default_adds_c0() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            controller: Some(ControllerKind::None),
            ..Default::default()
        })
        .unwrap();
        net.add_controller("c0", ControllerKind::None, &empty())
            .unwrap();
        assert!(net.controllers.is_empty());
        assert!(!net.contains("c0"));
        net.add_controller("c0", ControllerKind::Default, &empty())
            .unwrap();
        let c = net.controller("c0").unwrap();
        assert_eq!(c.command, "controller");
        assert_eq!(c.port, 6653);
        assert!(net.contains("c0"));
    }

    #[test]
    fn add_link_allocates_ports_and_registers_intfs() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            ..opts()
        })
        .unwrap();
        net.add_host("h1", &empty()).unwrap();
        net.add_switch("s1", &empty()).unwrap();
        net.add_link("h1", "s1", None, None, &empty()).unwrap();
        net.add_link("h1", "s1", None, None, &empty()).unwrap();
        assert_eq!(net.links.len(), 2);
        assert_eq!(net.links[0].intf1.name, "h1-eth0");
        assert_eq!(net.links[0].intf2.name, "s1-eth1"); // switches start at port 1
        assert_eq!(net.links[1].intf1.name, "h1-eth1");
        assert_eq!(net.links[1].intf2.name, "s1-eth2");
        // intfs registered on the nodes
        let h1 = net.host("h1").unwrap();
        assert_eq!(h1.intf_names(), vec!["h1-eth0", "h1-eth1"]);
        let s1 = net.switch("s1").unwrap();
        assert_eq!(s1.intf_names(), vec!["s1-eth1", "s1-eth2"]);
        // explicit ports honored
        net.add_link("h1", "s1", Some(5), Some(7), &empty())
            .unwrap();
        assert_eq!(net.links[2].intf1.name, "h1-eth5");
        assert_eq!(net.links[2].intf2.name, "s1-eth7");
        // links_between finds them in either order
        assert_eq!(net.links_between("h1", "s1"), vec![0, 1, 2]);
        assert_eq!(net.links_between("s1", "h1"), vec![0, 1, 2]);
        assert!(net.links_between("h1", "h2").is_empty());
        // unknown node errors
        assert!(net.add_link("h1", "nope", None, None, &empty()).is_err());
    }

    #[test]
    fn lookups_get_host_switch_node_contains_keys_len() {
        let mut net = Mininet::new(MininetOptions {
            build: false,
            ..opts()
        })
        .unwrap();
        net.add_host("h1", &empty()).unwrap();
        net.add_switch("s1", &empty()).unwrap();
        net.add_controller("c0", ControllerKind::Default, &empty())
            .unwrap();
        assert_eq!(net.len(), 3);
        assert!(!net.is_empty());
        assert!(net.contains("h1") && net.contains("s1") && net.contains("c0"));
        assert!(!net.contains("h2"));
        assert_eq!(net.keys(), vec!["h1", "s1", "c0"]);
        assert_eq!(net.get("h1").unwrap().name(), "h1");
        assert_eq!(net.get_many(&["h1", "c0"]).len(), 2);
        assert!(net.get("nope").is_none());
        assert!(net.host("s1").is_none());
        assert!(net.switch("h1").is_none());
        assert_eq!(net.node("c0").unwrap().name, "c0");
        assert_eq!(net.name_to_node("s1"), Some(NodeId::Switch(0)));
        assert_eq!(net.values().len(), 3);
        let names: Vec<&str> = net.iter().collect();
        assert_eq!(names, vec!["h1", "s1", "c0"]);
    }

    #[test]
    fn build_from_topo_single3_dry_run_model() {
        let net = Mininet::dry_run("single,3", opts()).unwrap();
        assert!(net.built);
        assert_eq!(net.hosts.len(), 3);
        assert_eq!(net.switches.len(), 1);
        assert_eq!(net.controllers.len(), 0); // controller: None
        assert_eq!(net.links.len(), 3);
        assert_eq!(net.keys(), vec!["h1", "h2", "h3", "s1"]);
        assert_eq!(
            net.host("h3")
                .unwrap()
                .node()
                .params
                .get("ip")
                .map(String::as_str),
            Some("10.0.0.3/8")
        );
        // host default intf got its IP in the model (configHosts pass)
        assert_eq!(
            net.host("h1").unwrap().ip(None).map(|a| a.to_string()),
            Some("10.0.0.1".to_string())
        );
        // switch has the three link intfs; lo is the control intf
        let s1 = net.switch("s1").unwrap();
        assert_eq!(s1.intf_names(), vec!["s1-eth1", "s1-eth2", "s1-eth3"]);
        assert!(s1.control_intf.is_some());
    }

    #[test]
    fn del_node_removes_everywhere() {
        let mut net = Mininet::dry_run("single,3", opts()).unwrap();
        net.del_node("h2").unwrap();
        assert!(!net.contains("h2"));
        assert_eq!(net.hosts.len(), 2);
        assert_eq!(net.len(), 3);
        assert!(net.del_node("h2").is_err());
    }

    #[test]
    fn stop_on_dry_run_net_is_a_safe_noop_cleanup() {
        // Non-root: stop() must not fail just because nothing was started.
        let mut net = Mininet::dry_run("single,3", opts()).unwrap();
        net.stop().unwrap();
        assert!(!net.built);
    }

    #[test]
    fn rand_mac_is_locally_administered_and_unicast() {
        for _ in 0..64 {
            let mac = rand_mac();
            let bytes: Vec<u8> = mac
                .split(':')
                .map(|p| u8::from_str_radix(p, 16).unwrap())
                .collect();
            assert_eq!(bytes.len(), 6);
            assert_eq!(bytes[0] & 0x01, 0, "multicast bit clear: {mac}");
            assert_eq!(bytes[0] & 0x02, 0x02, "locally administered: {mac}");
        }
        assert_ne!(rand_mac(), rand_mac());
    }

    // ------------------------------------------------------------------
    // Root-gated integration test: real netns hosts + veth link + pingall.
    // Run on a privileged host/CI:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-net -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-net -- --ignored --test-threads=1"]
    fn root_two_hosts_link_pingall_stop() {
        if !root_tests_enabled() {
            return;
        }
        let mut net = Mininet::new(MininetOptions {
            build: false,
            ..opts()
        })
        .unwrap();
        net.add_host("h1", &empty()).unwrap();
        net.add_host("h2", &empty()).unwrap();
        net.add_link("h1", "h2", None, None, &empty()).unwrap();
        net.build().unwrap();
        net.start().unwrap();
        let res = net.ping_all(None).unwrap();
        assert_eq!(res.ploss, 0.0, "ping output: {}", res.output);
        assert!(
            res.output
                .contains("*** Results: 0% dropped (1/1 received)"),
            "output: {}",
            res.output
        );
        // links report OK after attach (Python Intf.status(): exists -> OK)
        let links = net.links();
        assert!(
            links.contains("h1-eth0<->h2-eth0 (OK OK)"),
            "links: {links}"
        );
        // dump carries a real pid now
        let dump = net.dump();
        assert!(
            dump.contains("pid=") && !dump.contains("pid=None"),
            "dump: {dump}"
        );
        assert!(
            dump.contains("<Host h1: h1-eth0:10.0.0.1 pid="),
            "dump: {dump}"
        );
        net.stop().unwrap();
        assert!(!net.built);
    }
}
