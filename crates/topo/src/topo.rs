//! `Topo` — port of `mininet/topo.py`'s `Topo` class: a `MultiGraph` plus
//! host/switch/link option defaults and the legacy port-numbering table.
//!
//! Port numbering mirrors the Python original exactly: a host's first (and
//! usually only) port is `0`, a switch's ports start at `1`; auto-assigned
//! ports are `len(ports[node]) + base` where `base` is 1 for switches and 0
//! for hosts.

use std::collections::HashMap;

use mn_util::{natural, natural_seq};

use crate::graph::{EdgeEntry, EdgeKey, MultiGraph};
use crate::value::{Attrs, Value};

/// Default options popped from the constructor params by Python's
/// `Topo.__init__`: `hopts`/`sopts`/`lopts`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TopoOptions {
    /// Default host options (`hopts`): used by [`Topo::add_host`] when no
    /// per-host opts are given.
    pub hopts: Attrs,
    /// Default switch options (`sopts`): used by [`Topo::add_switch`].
    pub sopts: Attrs,
    /// Default link options (`lopts`): used by [`Topo::add_link`].
    pub lopts: Attrs,
}

/// `ports[src][sport] = (dst, dport)` for one node: insertion-ordered
/// `sport -> (dst, dport)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortMap {
    items: Vec<(i64, (String, i64))>,
}

impl PortMap {
    /// New empty port map.
    pub fn new() -> Self {
        Self::default()
    }

    /// `ports[src][sport]` lookup.
    pub fn get(&self, sport: i64) -> Option<&(String, i64)> {
        self.items.iter().find(|(s, _)| *s == sport).map(|(_, v)| v)
    }

    /// `ports[src][sport] = (dst, dport)`: replaces in place or appends.
    pub fn insert(&mut self, sport: i64, dst: &str, dport: i64) {
        if let Some(slot) = self.items.iter_mut().find(|(s, _)| *s == sport) {
            slot.1 = (dst.to_string(), dport);
        } else {
            self.items.push((sport, (dst.to_string(), dport)));
        }
    }

    /// Number of ports on this node (`len(ports[src])`).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the node has no ports.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate `(sport, (dst, dport))` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (i64, &(String, i64))> {
        self.items.iter().map(|(s, v)| (*s, v))
    }
}

/// `self.ports`: insertion-ordered `node -> PortMap`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ports {
    order: Vec<String>,
    map: HashMap<String, PortMap>,
}

impl Ports {
    /// New empty port table.
    pub fn new() -> Self {
        Self::default()
    }

    /// `ports.setdefault(node, {})`.
    pub fn ensure(&mut self, node: &str) {
        if !self.map.contains_key(node) {
            self.order.push(node.to_string());
            self.map.insert(node.to_string(), PortMap::new());
        }
    }

    /// `len(ports[node])` as an `i64` (0 if the node has no port map yet).
    pub fn len_of(&self, node: &str) -> i64 {
        self.map.get(node).map_or(0, |pm| pm.len() as i64)
    }

    /// `ports[node][sport] = (dst, dport)` (creates the node's map if needed).
    pub fn insert(&mut self, node: &str, sport: i64, dst: &str, dport: i64) {
        self.ensure(node);
        self.map
            .get_mut(node)
            .expect("ensured")
            .insert(sport, dst, dport);
    }

    /// `ports[node]` lookup.
    pub fn get(&self, node: &str) -> Option<&PortMap> {
        self.map.get(node)
    }

    /// Iterate `(node, portmap)` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PortMap)> {
        self.order.iter().map(|n| (n.as_str(), &self.map[n]))
    }

    /// Number of nodes with a port map.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether no node has ports yet.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Result of [`Topo::port`]: Python returns the single `(sport, dport)` tuple
/// when exactly one port on `src` leads to `dst`, otherwise the *list* of all
/// matching `(sport, dport)` pairs (possibly empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortLookup {
    /// Exactly one matching port: `(sport, dport)`.
    Single(i64, i64),
    /// Zero or several matching ports: list of `(sport, dport)`.
    Many(Vec<(i64, i64)>),
}

/// One link as returned by [`Topo::links`]/[`Topo::iter_links`], mirroring
/// the `(node1, node2 [, key [, info]])` tuples of Python's `links()`.
#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    /// `info['node1']` (the first node passed to `addLink`).
    pub node1: &'a str,
    /// `info['node2']`.
    pub node2: &'a str,
    /// Edge key, present iff `with_keys` was requested.
    pub key: Option<&'a EdgeKey>,
    /// Link info dict, present iff `with_info` was requested.
    pub info: Option<&'a Attrs>,
}

/// Port of `Topo` from `mininet/topo.py`: a data-center network
/// representation for structured multi-trees.
///
/// `g` is the backing [`MultiGraph`]; `hopts`/`sopts`/`lopts` are the default
/// node/link options; `ports` is the legacy port table (`ports[src][dst][sport]`
/// in the Python docstring is implemented as `ports[src][sport] = (dst, dport)`).
#[derive(Debug, Clone, Default)]
pub struct Topo {
    /// The backing graph (`self.g`).
    pub g: MultiGraph,
    /// Default host options (`self.hopts`).
    pub hopts: Attrs,
    /// Default switch options (`self.sopts`).
    pub sopts: Attrs,
    /// Default link options (`self.lopts`).
    pub lopts: Attrs,
    /// Port table: `ports[src][sport] = (dst, dport)` (`self.ports`).
    pub ports: Ports,
}

impl Topo {
    /// New empty topo with default options (Python `Topo()`; the base
    /// class's `build()` is a no-op, so this is just [`Topo::with_options`]
    /// with default options).
    pub fn new() -> Self {
        Self::with_options(TopoOptions::default())
    }

    /// New empty topo with the given default host/switch/link options
    /// (Python `Topo(hopts=..., sopts=..., lopts=...)`).
    pub fn with_options(options: TopoOptions) -> Self {
        Self {
            g: MultiGraph::new(),
            hopts: options.hopts,
            sopts: options.sopts,
            lopts: options.lopts,
            ports: Ports::new(),
        }
    }

    /// Port of `addNode(name, **opts)`: add a node to the graph, replacing
    /// any existing attrs for `name`. Returns the name.
    pub fn add_node(&mut self, name: &str, opts: Attrs) -> String {
        self.g.add_node(name, opts);
        name.to_string()
    }

    /// Port of `addHost(name, **opts)`: add a host; when `opts` is empty and
    /// `hopts` is not, the default host options are used. Returns the name.
    pub fn add_host(&mut self, name: &str, mut opts: Attrs) -> String {
        if opts.is_empty() && !self.hopts.is_empty() {
            opts = self.hopts.clone();
        }
        self.add_node(name, opts)
    }

    /// Port of `addSwitch(name, **opts)`: add a node with `isSwitch=True`;
    /// when `opts` is empty and `sopts` is not, the default switch options
    /// are used (they must not contain `isSwitch` — Python would raise
    /// `TypeError` for the duplicate keyword). Returns the name.
    pub fn add_switch(&mut self, name: &str, mut opts: Attrs) -> String {
        if opts.is_empty() && !self.sopts.is_empty() {
            opts = self.sopts.clone();
        }
        let mut attrs = Attrs::new();
        attrs.insert("isSwitch", Value::Bool(true));
        attrs.update(&opts);
        self.add_node(name, attrs)
    }

    /// Port of `addLink(node1, node2, port1=None, port2=None, key=None,
    /// **opts)`: assign ports, store the link info (opts plus
    /// `node1`/`node2`/`port1`/`port2`) and add the edge. Returns the link
    /// key. When `opts` is empty and `lopts` is not, the default link options
    /// are used.
    #[allow(clippy::too_many_arguments)]
    pub fn add_link(
        &mut self,
        node1: &str,
        node2: &str,
        port1: Option<i64>,
        port2: Option<i64>,
        key: Option<EdgeKey>,
        mut opts: Attrs,
    ) -> EdgeKey {
        if opts.is_empty() && !self.lopts.is_empty() {
            opts = self.lopts.clone();
        }
        let (p1, p2) = self.add_port(node1, node2, port1, port2);
        // opts = dict(opts); opts.update(node1=..., node2=..., port1=..., port2=...)
        opts.update_from([
            ("node1".to_string(), Value::Str(node1.to_string())),
            ("node2".to_string(), Value::Str(node2.to_string())),
            ("port1".to_string(), Value::Int(p1)),
            ("port2".to_string(), Value::Int(p2)),
        ]);
        self.g.add_edge(node1, node2, key, opts)
    }

    /// Port of `nodes(sort=True)`: node names, naturally sorted unless
    /// `sort` is false (then in insertion order).
    pub fn nodes(&self, sort: bool) -> Vec<String> {
        let names: Vec<String> = self.g.nodes().into_iter().map(str::to_string).collect();
        if sort {
            Self::sorted(names)
        } else {
            names
        }
    }

    /// Port of `isSwitch(n)`: whether node `n` is a switch. Python raises
    /// `KeyError` for unknown nodes; here unknown nodes are not switches.
    pub fn is_switch(&self, n: &str) -> bool {
        self.g
            .node(n)
            .and_then(|a| a.get("isSwitch"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Port of `switches(sort=True)`.
    pub fn switches(&self, sort: bool) -> Vec<String> {
        self.nodes(sort)
            .into_iter()
            .filter(|n| self.is_switch(n))
            .collect()
    }

    /// Port of `hosts(sort=True)`.
    pub fn hosts(&self, sort: bool) -> Vec<String> {
        self.nodes(sort)
            .into_iter()
            .filter(|n| !self.is_switch(n))
            .collect()
    }

    /// Port of `iterLinks(withKeys=False, withInfo=False)`: iterate links as
    /// [`Link`]s, using the stored `node1`/`node2` info (Python raises
    /// `KeyError` for edges added directly to `g` without link info; here
    /// such edges fall back to the graph's `src`/`dst`).
    pub fn iter_links(&self, with_keys: bool, with_info: bool) -> impl Iterator<Item = Link<'_>> {
        self.g
            .edges_data_keys()
            .into_iter()
            .map(move |(src, dst, key, attrs)| {
                let node1 = attrs.get("node1").and_then(Value::as_str).unwrap_or(src);
                let node2 = attrs.get("node2").and_then(Value::as_str).unwrap_or(dst);
                Link {
                    node1,
                    node2,
                    key: if with_keys { Some(key) } else { None },
                    info: if with_info { Some(attrs) } else { None },
                }
            })
    }

    /// Port of `links(sort=False, withKeys=False, withInfo=False)`.
    ///
    /// With `sort`, links are ordered by `naturalSeq` of the
    /// `(node1, node2)` pair — plus the key when `with_keys` — exactly like
    /// the Python original (stable sort).
    pub fn links(&self, sort: bool, with_keys: bool, with_info: bool) -> Vec<Link<'_>> {
        let mut links: Vec<Link<'_>> = self.iter_links(with_keys, with_info).collect();
        if sort {
            links.sort_by_key(|l| {
                let mut key: Vec<String> = vec![l.node1.to_string(), l.node2.to_string()];
                if with_keys {
                    if let Some(k) = l.key {
                        key.push(k.to_string());
                    }
                }
                natural_seq(key)
            });
        }
        links
    }

    /// Port of `addPort(src, dst, sport=None, dport=None)`: generate the port
    /// mapping for a new edge. Auto ports are `len(ports[node]) + base` with
    /// `base = 1` for switches and `0` for hosts. Returns `(sport, dport)`.
    pub fn add_port(
        &mut self,
        src: &str,
        dst: &str,
        sport: Option<i64>,
        dport: Option<i64>,
    ) -> (i64, i64) {
        // Initialize if necessary
        self.ports.ensure(src);
        self.ports.ensure(dst);
        // New port: number of outlinks + base
        let sport = match sport {
            Some(p) => p,
            None => {
                let base = if self.is_switch(src) { 1 } else { 0 };
                self.ports.len_of(src) + base
            }
        };
        let dport = match dport {
            Some(p) => p,
            None => {
                let base = if self.is_switch(dst) { 1 } else { 0 };
                self.ports.len_of(dst) + base
            }
        };
        self.ports.insert(src, sport, dst, dport);
        self.ports.insert(dst, dport, src, sport);
        (sport, dport)
    }

    /// Port of `port(src, dst)`: `(sport, dport)` on `src` leading to `dst`.
    /// Returns `None` where Python raises `KeyError` (unknown `src`);
    /// [`PortLookup::Many`] carries the 0-or-more matches list otherwise.
    pub fn port(&self, src: &str, dst: &str) -> Option<PortLookup> {
        let pm = self.ports.get(src)?;
        let matches: Vec<(i64, i64)> = pm
            .iter()
            .filter(|(_, (d, _))| d == dst)
            .map(|(s, (_, d))| (s, *d))
            .collect();
        Some(if matches.len() == 1 {
            PortLookup::Single(matches[0].0, matches[0].1)
        } else {
            PortLookup::Many(matches)
        })
    }

    /// Port of `_linkEntry(src, dst, key=None)`: the edge entry and its key
    /// (the smallest key when `key` is `None`). `None` where Python raises
    /// `KeyError`.
    pub fn link_entry(
        &self,
        src: &str,
        dst: &str,
        key: Option<&EdgeKey>,
    ) -> Option<(&EdgeEntry, EdgeKey)> {
        let entry = self.g.get(src, dst)?;
        let key = match key {
            Some(k) => k.clone(),
            None => entry.min_key()?,
        };
        Some((entry, key))
    }

    /// Port of `linkInfo(src, dst, key=None)`: link metadata dict.
    pub fn link_info(&self, src: &str, dst: &str, key: Option<&EdgeKey>) -> Option<&Attrs> {
        let (entry, key) = self.link_entry(src, dst, key)?;
        entry.get(&key)
    }

    /// Port of `setlinkInfo(src, dst, info, key=None)`: replace link metadata.
    /// Returns whether the edge existed.
    pub fn set_link_info(
        &mut self,
        src: &str,
        dst: &str,
        info: Attrs,
        key: Option<&EdgeKey>,
    ) -> bool {
        let key = match key {
            Some(k) => k.clone(),
            None => match self.g.get(src, dst).and_then(EdgeEntry::min_key) {
                Some(k) => k,
                None => return false,
            },
        };
        match self.g.get_mut(src, dst) {
            Some(entry) => {
                entry.set(key, info);
                true
            }
            None => false,
        }
    }

    /// Port of `nodeInfo(name)`: node metadata dict (`None` for unknown nodes,
    /// where Python raises `KeyError`).
    pub fn node_info(&self, name: &str) -> Option<&Attrs> {
        self.g.node(name)
    }

    /// Port of `setNodeInfo(name, info)`: set (or add) node metadata.
    pub fn set_node_info(&mut self, name: &str, info: Attrs) {
        self.g.add_node(name, info);
    }

    /// Port of `convertTo(cls, data=True, keys=True)`: convert the backing
    /// graph to a new [`MultiGraph`].
    pub fn convert_to(&self, data: bool, keys: bool) -> MultiGraph {
        self.g.convert_to(data, keys)
    }

    /// Port of the `sorted` static method: items sorted in natural
    /// (alphabetical) order via `mn_util::natural`.
    pub fn sorted<T: ToString>(items: impl IntoIterator<Item = T>) -> Vec<T> {
        let mut v: Vec<T> = items.into_iter().collect();
        v.sort_by_key(|x| natural(x.to_string()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attrs;

    fn link(t: &mut Topo, n1: &str, n2: &str) -> EdgeKey {
        t.add_link(n1, n2, None, None, None, Attrs::new())
    }

    #[test]
    fn port_numbering_host_zero_switch_one() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h1 = t.add_host("h1", Attrs::new());
        let h2 = t.add_host("h2", Attrs::new());
        assert!(t.is_switch("s1"));
        assert!(!t.is_switch("h1"));
        assert!(
            !t.is_switch("unknown"),
            "unknown node: not a switch (Python KeyError)"
        );
        link(&mut t, &h1, &s);
        link(&mut t, &h2, &s);
        // hosts use port 0, switch ports start at 1
        assert_eq!(t.port("h1", "s1"), Some(PortLookup::Single(0, 1)));
        assert_eq!(t.port("h2", "s1"), Some(PortLookup::Single(0, 2)));
        assert_eq!(t.port("s1", "h1"), Some(PortLookup::Single(1, 0)));
        assert_eq!(t.port("s1", "h2"), Some(PortLookup::Single(2, 0)));
        assert_eq!(t.port("h1", "h2"), Some(PortLookup::Many(vec![])));
        assert_eq!(t.port("nobody", "s1"), None, "Python KeyError -> None");
    }

    #[test]
    fn switch_to_switch_ports() {
        let mut t = Topo::new();
        let s1 = t.add_switch("s1", Attrs::new());
        let s2 = t.add_switch("s2", Attrs::new());
        link(&mut t, &s2, &s1);
        // both switches: base 1; s2 gets port 1, s1 gets port 1
        assert_eq!(t.port("s2", "s1"), Some(PortLookup::Single(1, 1)));
        // a second link s2->s1: parallel edge, port 2 on both
        link(&mut t, &s2, &s1);
        assert_eq!(
            t.port("s2", "s1"),
            Some(PortLookup::Many(vec![(1, 1), (2, 2)]))
        );
    }

    #[test]
    fn add_port_explicit() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h = t.add_host("h1", Attrs::new());
        let (p1, p2) = t.add_port(&h, &s, Some(0), Some(5));
        assert_eq!((p1, p2), (0, 5));
        assert_eq!(t.port(&h, &s), Some(PortLookup::Single(0, 5)));
        assert_eq!(t.port(&s, &h), Some(PortLookup::Single(5, 0)));
        // mixed: explicit sport, auto dport (switch base 1, len(ports[s1])=1)
        let (p1, p2) = t.add_port(&h, &s, Some(3), None);
        assert_eq!(p1, 3);
        assert_eq!(p2, 2, "len(ports[s1])=1 + base 1 = 2");
        assert_eq!(t.port(&h, &s), Some(PortLookup::Many(vec![(0, 5), (3, 2)])));
    }

    #[test]
    fn add_link_stores_info_and_returns_key() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h = t.add_host("h1", Attrs::new());
        let k = t.add_link(&h, &s, None, None, None, attrs!("bw" => 10_i64));
        assert_eq!(k, EdgeKey::Int(1));
        let info = t.link_info(&h, &s, None).expect("info");
        let keys: Vec<&str> = info.iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec!["bw", "node1", "node2", "port1", "port2"],
            "opts first, then link fields"
        );
        assert_eq!(info.get("bw"), Some(&Value::Int(10)));
        assert_eq!(info.get("node1"), Some(&Value::Str("h1".into())));
        assert_eq!(info.get("node2"), Some(&Value::Str("s1".into())));
        assert_eq!(info.get("port1"), Some(&Value::Int(0)));
        assert_eq!(info.get("port2"), Some(&Value::Int(1)));
        // explicit key is used and returned
        let k2 = t.add_link(&h, &s, None, None, Some(EdgeKey::Int(7)), Attrs::new());
        assert_eq!(k2, EdgeKey::Int(7));
        // link_info with explicit key; min key when key=None
        assert_eq!(
            t.link_info(&h, &s, Some(&EdgeKey::Int(7)))
                .unwrap()
                .get("bw"),
            None
        );
        assert_eq!(
            t.link_info(&h, &s, None).unwrap().get("bw"),
            Some(&Value::Int(10))
        );
    }

    #[test]
    fn set_link_info_and_node_info() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h = t.add_host("h1", Attrs::new());
        link(&mut t, &h, &s);
        assert!(t.set_link_info(&h, &s, attrs!("bw" => 100_i64), None));
        assert_eq!(
            t.link_info(&h, &s, None).unwrap().get("bw"),
            Some(&Value::Int(100))
        );
        assert!(!t.set_link_info(&h, "nope", Attrs::new(), None));
        assert_eq!(t.node_info("h1"), Some(&Attrs::new()));
        t.set_node_info("h1", attrs!("ip" => "10.0.0.1"));
        assert_eq!(
            t.node_info("h1").unwrap().get("ip"),
            Some(&Value::Str("10.0.0.1".into()))
        );
        assert_eq!(t.node_info("ghost"), None, "Python KeyError -> None");
    }

    #[test]
    fn hopts_sopts_lopts_defaults() {
        let mut t = Topo::with_options(TopoOptions {
            hopts: attrs!("ip" => "10.0.0.1/8"),
            sopts: attrs!("dpid" => "1"),
            lopts: attrs!("bw" => 10_i64),
        });
        let h = t.add_host("h1", Attrs::new());
        let s = t.add_switch("s1", Attrs::new());
        // explicit opts win over defaults
        let h2 = t.add_host("h2", attrs!("ip" => "10.0.0.2/8"));
        assert_eq!(
            t.node_info(&h).unwrap().get("ip"),
            Some(&Value::Str("10.0.0.1/8".into()))
        );
        assert_eq!(
            t.node_info(&h2).unwrap().get("ip"),
            Some(&Value::Str("10.0.0.2/8".into()))
        );
        // switch: isSwitch first, then sopts
        let sinfo = t.node_info(&s).unwrap();
        let keys: Vec<&str> = sinfo.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["isSwitch", "dpid"]);
        assert_eq!(sinfo.get("isSwitch"), Some(&Value::Bool(true)));
        // link: lopts first, then node1/node2/port1/port2
        t.add_link(&h, &s, None, None, None, Attrs::new());
        let linfo = t.link_info(&h, &s, None).unwrap();
        let lkeys: Vec<&str> = linfo.iter().map(|(k, _)| k).collect();
        assert_eq!(lkeys, vec!["bw", "node1", "node2", "port1", "port2"]);
    }

    #[test]
    fn nodes_hosts_switches_sorting() {
        let mut t = Topo::new();
        for n in ["h10", "h2", "h1", "s2", "s10", "s1"] {
            if n.starts_with('s') {
                t.add_switch(n, Attrs::new());
            } else {
                t.add_host(n, Attrs::new());
            }
        }
        assert_eq!(t.nodes(true), vec!["h1", "h2", "h10", "s1", "s2", "s10"]);
        assert_eq!(t.hosts(true), vec!["h1", "h2", "h10"]);
        assert_eq!(t.switches(true), vec!["s1", "s2", "s10"]);
        assert_eq!(
            t.nodes(false),
            vec!["h10", "h2", "h1", "s2", "s10", "s1"],
            "insertion order when sort=false"
        );
    }

    #[test]
    fn links_sorting_natural_seq() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        for h in ["h10", "h2", "h1"] {
            let h = t.add_host(h, Attrs::new());
            link(&mut t, &h, &s);
        }
        // add a parallel edge h1-s1 with key 2
        let h1 = "h1".to_string();
        t.add_link(&h1, &s, None, None, None, Attrs::new());
        let plain: Vec<(String, String)> = t
            .links(true, false, false)
            .iter()
            .map(|l| (l.node1.to_string(), l.node2.to_string()))
            .collect();
        assert_eq!(
            plain,
            vec![
                ("h1".to_string(), "s1".to_string()),
                ("h1".to_string(), "s1".to_string()),
                ("h2".to_string(), "s1".to_string()),
                ("h10".to_string(), "s1".to_string()),
            ]
        );
        let with_keys: Vec<(String, String, String)> = t
            .links(true, true, false)
            .iter()
            .map(|l| {
                (
                    l.node1.to_string(),
                    l.node2.to_string(),
                    l.key.unwrap().to_string(),
                )
            })
            .collect();
        // sort key is naturalSeq of (node1, node2, key): key "1" < "2"
        assert_eq!(
            with_keys,
            vec![
                ("h1".to_string(), "s1".to_string(), "1".to_string()),
                ("h1".to_string(), "s1".to_string(), "2".to_string()),
                ("h2".to_string(), "s1".to_string(), "1".to_string()),
                ("h10".to_string(), "s1".to_string(), "1".to_string()),
            ]
        );
        // unsorted: graph insertion order
        let unsorted: Vec<(String, String)> = t
            .links(false, false, false)
            .iter()
            .map(|l| (l.node1.to_string(), l.node2.to_string()))
            .collect();
        assert_eq!(
            unsorted,
            vec![
                ("h10".to_string(), "s1".to_string()),
                ("h2".to_string(), "s1".to_string()),
                ("h1".to_string(), "s1".to_string()),
                ("h1".to_string(), "s1".to_string()),
            ]
        );
        // with_info only: 3-tuples (node1, node2, info)
        let infos = t.links(false, false, true);
        assert!(infos.iter().all(|l| l.key.is_none() && l.info.is_some()));
        assert_eq!(infos[0].info.unwrap().get("port2"), Some(&Value::Int(1)));
    }

    #[test]
    fn iter_links_reads_node1_node2_from_info() {
        let mut t = Topo::new();
        // addLink(s2, s1): stored under edge s2->s1 but iterated from s1 (s1 < s2)
        let s1 = t.add_switch("s1", Attrs::new());
        let s2 = t.add_switch("s2", Attrs::new());
        link(&mut t, &s2, &s1);
        let links = t.links(false, true, true);
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].node1, links[0].node2), ("s2", "s1"));
        assert_eq!(links[0].key, Some(&EdgeKey::Int(1)));
        assert_eq!(links[0].info.unwrap().get("port1"), Some(&Value::Int(1)));
        assert_eq!(links[0].info.unwrap().get("port2"), Some(&Value::Int(1)));
    }

    #[test]
    fn sorted_is_natural() {
        let v = Topo::sorted(["a10", "a2", "a1", "b", "10", "9"]);
        assert_eq!(v, vec!["9", "10", "a1", "a2", "a10", "b"]);
    }

    #[test]
    fn convert_to_delegates() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h = t.add_host("h1", Attrs::new());
        link(&mut t, &h, &s);
        let g: MultiGraph = t.convert_to(true, true);
        assert_eq!(g.nodes(), vec!["s1", "h1"]);
        assert_eq!(g.edges_keys(), vec![("h1", "s1", &EdgeKey::Int(1))]);
    }

    #[test]
    fn ports_table_shape() {
        let mut t = Topo::new();
        let s = t.add_switch("s1", Attrs::new());
        let h = t.add_host("h1", Attrs::new());
        link(&mut t, &h, &s);
        // ports: h1 first (addPort called with src=host), then s1
        let order: Vec<&str> = t.ports.iter().map(|(n, _)| n).collect();
        assert_eq!(order, vec!["h1", "s1"]);
        let hmap = t.ports.get("h1").unwrap();
        assert_eq!(hmap.len(), 1);
        assert_eq!(hmap.get(0), Some(&("s1".to_string(), 1)));
        let smap = t.ports.get("s1").unwrap();
        assert_eq!(smap.get(1), Some(&("h1".to_string(), 0)));
        assert_eq!(t.ports.len(), 2);
        assert!(!t.ports.is_empty());
    }
}
