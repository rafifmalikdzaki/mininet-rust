//! Observability: `dump()` / `links()` / `plan()` in the **exact** formats of
//! `mininet/cli.py`'s `do_dump`/`do_links` and the `=== topo ===` section of
//! `scripts/diff_topology.sh`'s Python reference driver — this is what
//! `scripts/diff_topology.sh` diffs.
//!
//! Formats (from the Python original):
//!
//! - `do_dump`: `output('%s\n' % repr(node))` for `node in mn.values()`, and
//!   `Node.__repr__` is `'<%s %s: %s pid=%s> ' % (class, name, intfs, pid)`
//!   with `intfs = ','.join('%s:%s' % (i.name, i.IP()))` (note the trailing
//!   space) and `class = self.__class__.__name__` — taken from
//!   [`Node::kind`](mn_node::Node::kind) here, so `--switch default` shows
//!   `OVSSwitch`. `Controller.__repr__` is
//!   `'<%s %s: %s:%s pid=%s> ' % (class, name, IP(), port, pid)`.
//! - `do_links`: `output(link, link.status(), '\n')` with
//!   `Link.__str__` = `'intf1<->intf2'` and `Link.status()` =
//!   `"(%s %s)" % (intf1.status(), intf2.status())` where `Intf.status()`
//!   is `"OK"` if the interface exists (regardless of up/down) else
//!   `"MISSING"`.
//! - topo section (diff_topology.sh py driver): `spec:`/`class:`/`nodes:`/
//!   `hosts:`/`switches:`/`links:` lines; `links:` is the naturally sorted
//!   `src-eth<port1><->dst-eth<port2>` strings.

use mn_link::{Intf, IntfStatus};
use mn_node::Node;
use mn_topo::Topo;

use crate::map::NodeRef;
use crate::Mininet;

/// Python `Intf.status()`: `"OK"` if the interface exists, else `"MISSING"`.
/// (Python only checks presence in `ip link show`, so a down-but-present
/// interface is still `"OK"`.)
fn py_intf_status(intf: &Intf) -> &'static str {
    match intf.status {
        IntfStatus::Missing => "MISSING",
        IntfStatus::Up | IntfStatus::Down => "OK",
    }
}

/// Python `Intf.IP()` rendering: the bare address, or `"None"`.
fn py_intf_ip(intf: &Intf) -> String {
    intf.ip
        .map(|a| a.to_string())
        .unwrap_or_else(|| "None".to_string())
}

fn pid_repr(node: &Node) -> String {
    node.pid()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "None".to_string())
}

/// `','.join('%s:%s' % (i.name, i.IP()))` over `node.intfList()`, with the
/// root-namespace switch's `lo` control interface first (Python
/// `Switch.__init__` registers `Intf('lo', port=0)`, whose IP is
/// 127.0.0.1).
fn intfs_repr(node: &Node, control_intf: Option<&Intf>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(lo) = control_intf {
        let ip = if lo.name == "lo" {
            "127.0.0.1".to_string() // Python Intf.__init__: lo -> 127.0.0.1
        } else {
            py_intf_ip(lo)
        };
        parts.push(format!("{}:{ip}", lo.name));
    }
    for i in node.intf_list() {
        parts.push(format!("{}:{}", i.name, py_intf_ip(i)));
    }
    parts.join(",")
}

/// Python `repr(node)` for any node kind (`Node.__repr__` /
/// `Controller.__repr__`), trailing space included. The class name is
/// `Node::kind()` — i.e. `self.__class__.__name__` in Python — so a switch
/// built from `--switch default` prints `<OVSSwitch s1: ...>`, exactly like
/// Python's `repr(OVSSwitch(...))`.
pub fn repr_node(node: NodeRef<'_>) -> String {
    match node {
        NodeRef::Host(h) => {
            let n = h.node();
            format!(
                "<{} {}: {} pid={}> ",
                n.kind(),
                n.name,
                intfs_repr(n, None),
                pid_repr(n)
            )
        }
        NodeRef::Switch(s) => {
            let n = s.node();
            format!(
                "<{} {}: {} pid={}> ",
                n.kind(),
                n.name,
                intfs_repr(n, s.control_intf.as_ref()),
                pid_repr(n)
            )
        }
        NodeRef::Controller(c) => {
            let n = c.node();
            format!(
                "<{} {}: {}:{} pid={}> ",
                n.kind(),
                n.name,
                c.ip(),
                c.port,
                pid_repr(n)
            )
        }
    }
}

impl Mininet {
    /// Port of `cli.py do_dump`: `repr(node)` per node in `values()` order
    /// (hosts, switches, controllers — insertion order), one per line.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for name in self.keys() {
            let node = self.get(&name).expect("keys in sync");
            out.push_str(&repr_node(node));
            out.push('\n');
        }
        out
    }

    /// Port of `cli.py do_links`: `'<link> (<status1> <status2>)'` per link.
    pub fn links(&self) -> String {
        let mut out = String::new();
        for link in &self.links {
            out.push_str(&format!(
                "{}<->{} ({} {})\n",
                link.intf1.name,
                link.intf2.name,
                py_intf_status(&link.intf1),
                py_intf_status(&link.intf2)
            ));
        }
        out
    }

    /// The `=== topo ===` section of the dry-run plan (the same lines
    /// `scripts/diff_topology.sh`'s Python driver prints), if a topo is set.
    pub fn topo_summary(&self) -> Option<String> {
        let topo = self.topo.as_ref()?;
        Some(topo_section(
            topo,
            self.topo_spec.as_deref(),
            self.topo_class.as_deref(),
        ))
    }

    /// Dry-run plan: the whole planned topology — `=== topo ===`,
    /// `=== dump ===`, `=== links ===` sections — printed **without touching
    /// the kernel** (this is the `--dump` path of `apps/mn` later).
    pub fn plan(&self) -> String {
        let mut out = String::new();
        if let Some(section) = self.topo_summary() {
            out.push_str(&section);
        }
        out.push_str("=== dump ===\n");
        out.push_str(&self.dump());
        out.push_str("=== links ===\n");
        out.push_str(&self.links());
        out
    }

    /// Print the dry-run [`Mininet::plan`] to stdout.
    pub fn print_plan(&self) {
        print!("{}", self.plan());
    }
}

/// Render the `=== topo ===` section for a [`Topo`] (used by
/// [`Mininet::topo_summary`]; also the reference for the differential
/// driver).
pub fn topo_section(topo: &Topo, spec: Option<&str>, class: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("=== topo ===\n");
    if let Some(spec) = spec {
        out.push_str(&format!("spec: {spec}\n"));
    }
    if let Some(class) = class {
        out.push_str(&format!("class: {class}\n"));
    }
    out.push_str(&format!("nodes: {}\n", topo.nodes(true).join(" ")));
    out.push_str(&format!("hosts: {}\n", topo.hosts(true).join(" ")));
    out.push_str(&format!("switches: {}\n", topo.switches(true).join(" ")));
    let mut links: Vec<String> = topo
        .links(true, false, true)
        .iter()
        .map(|l| link_str(l.node1, l.node2, l.info.expect("with_info")))
        .collect();
    links = Topo::sorted(links);
    out.push_str(&format!("links: {}\n", links.join(" ")));
    out
}

/// `'<src>-eth<port1><-><dst>-eth<port2>'` from a topo link's info dict.
pub fn link_str(src: &str, dst: &str, info: &mn_topo::Attrs) -> String {
    let p1 = info.get("port1").and_then(|v| v.as_int()).unwrap_or(0);
    let p2 = info.get("port2").and_then(|v| v.as_int()).unwrap_or(0);
    format!("{src}-eth{p1}<->{dst}-eth{p2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::MininetOptions;

    fn opts() -> MininetOptions {
        MininetOptions {
            controller: None,
            ..Default::default()
        }
    }

    #[test]
    fn dump_single3_exact_python_format() {
        let net = Mininet::dry_run("single,3", opts()).unwrap();
        let expected = "\
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<Host h3: h3-eth0:10.0.0.3 pid=None> \n\
<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None,s1-eth3:None pid=None> \n";
        assert_eq!(net.dump(), expected);
    }

    #[test]
    fn links_single3_exact_python_format() {
        let net = Mininet::dry_run("single,3", opts()).unwrap();
        let expected = "\
h1-eth0<->s1-eth1 (MISSING MISSING)\n\
h2-eth0<->s1-eth2 (MISSING MISSING)\n\
h3-eth0<->s1-eth3 (MISSING MISSING)\n";
        assert_eq!(net.links(), expected);
    }

    #[test]
    fn plan_single3_sections() {
        let net = Mininet::dry_run("single,3", opts()).unwrap();
        let expected = "\
=== topo ===
spec: single,3
class: SingleSwitchTopo
nodes: h1 h2 h3 s1
hosts: h1 h2 h3
switches: s1
links: h1-eth0<->s1-eth1 h2-eth0<->s1-eth2 h3-eth0<->s1-eth3
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<Host h3: h3-eth0:10.0.0.3 pid=None> \n\
<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None,s1-eth3:None pid=None> \n\
=== links ===
h1-eth0<->s1-eth1 (MISSING MISSING)\n\
h2-eth0<->s1-eth2 (MISSING MISSING)\n\
h3-eth0<->s1-eth3 (MISSING MISSING)\n";
        assert_eq!(net.plan(), expected);
    }

    #[test]
    fn py_intf_status_maps_like_python() {
        let mut i = Intf::new("h1-eth0", "h1", Some(0));
        assert_eq!(py_intf_status(&i), "MISSING");
        i.status = IntfStatus::Up;
        assert_eq!(py_intf_status(&i), "OK");
        i.status = IntfStatus::Down;
        assert_eq!(py_intf_status(&i), "OK"); // Python only checks existence
    }
}
