//! `MultiGraph` — port of `mininet/topo.py`'s `MultiGraph`, the
//! networkx.MultiGraph replacement that backs every `Topo`.
//!
//! Semantics mirrored from the Python original:
//!
//! - `node` is an insertion-ordered `dict[node, attr_dict]`;
//! - `edge[src][dst]` and `edge[dst][src]` are the *same* dict
//!   (`entry = self.edge[dst][src] = self.edge[src][dst]`), mapping
//!   `key -> attr_dict`;
//! - `add_edge` with `key=None` picks `max([0] + int keys) + 1`;
//! - `edges_iter` iterates `edge` in insertion order but skips `src > dst`
//!   (each undirected edge is reported once, from the lexically smaller end).
//!
//! Deviation: Python raises `KeyError`/`TypeError` on bad lookups; this port
//! returns `None` (the convention used throughout `mn-util`).

use std::collections::HashMap;
use std::fmt;

use crate::value::Attrs;

/// An edge key: Python allows any hashable; the keys that occur in Mininet
/// are ints (auto-assigned or explicit) and strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKey {
    /// Integer key (auto-assigned keys are always ints).
    Int(i64),
    /// String key (e.g. `addLink(..., key='uplink')`).
    Str(String),
}

impl EdgeKey {
    /// The int value if this is an [`EdgeKey::Int`].
    pub fn as_int(&self) -> Option<i64> {
        match self {
            EdgeKey::Int(i) => Some(*i),
            EdgeKey::Str(_) => None,
        }
    }
}

/// Python `str(key)`: used by `naturalSeq` when sorting links with keys.
impl fmt::Display for EdgeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeKey::Int(i) => write!(f, "{i}"),
            EdgeKey::Str(s) => write!(f, "{s}"),
        }
    }
}

/// The shared `edge[src][dst] == edge[dst][src]` dict: an insertion-ordered
/// `key -> attr_dict` map.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdgeEntry {
    items: Vec<(EdgeKey, Attrs)>,
}

impl EdgeEntry {
    /// New empty entry (`{}`).
    pub fn new() -> Self {
        Self::default()
    }

    /// `entry[key]` lookup.
    pub fn get(&self, key: &EdgeKey) -> Option<&Attrs> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, a)| a)
    }

    /// `entry[key] = attrs`: replaces in place (keeping position) or appends.
    pub fn set(&mut self, key: EdgeKey, attrs: Attrs) {
        if let Some(slot) = self.items.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = attrs;
        } else {
            self.items.push((key, attrs));
        }
    }

    /// Python `min(entry)`: the smallest key. With homogeneous keys this is
    /// exact; mixed int/str keys raise `TypeError` in Python 3, here the
    /// derived total order (all `Int` keys sort before all `Str` keys) is
    /// used instead.
    pub fn min_key(&self) -> Option<EdgeKey> {
        self.items.iter().map(|(k, _)| k.clone()).min()
    }

    /// Number of parallel edges between the two endpoints.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no parallel edges.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate `(key, attrs)` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&EdgeKey, &Attrs)> {
        self.items.iter().map(|(k, a)| (k, a))
    }
}

/// Destination map for one `src`: insertion-ordered `dst -> entry` where the
/// entry is a shared index into the graph's entry slab.
#[derive(Debug, Clone, Default, PartialEq)]
struct DstMap {
    order: Vec<String>,
    index: HashMap<String, usize>,
}

/// Port of `MultiGraph` from `mininet/topo.py`: tracks nodes and edges.
///
/// Nodes and edges keep insertion order (like CPython dicts), which
/// `edges_iter` relies on; `edge[src][dst]` and `edge[dst][src]` alias one
/// shared [`EdgeEntry`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MultiGraph {
    node_order: Vec<String>,
    node: HashMap<String, Attrs>,
    edge_order: Vec<String>,
    edge: HashMap<String, DstMap>,
    entries: Vec<EdgeEntry>,
}

impl MultiGraph {
    /// New empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `add_node(node, attr_dict=None, **attrs)`. The Python version
    /// first does `attr_dict.update(attrs)` on the passed dict; here the
    /// caller merges with [`Attrs::update`] and passes the result. Re-adding
    /// an existing node replaces its attrs but keeps its position.
    pub fn add_node(&mut self, node: impl Into<String>, attrs: Attrs) {
        let node = node.into();
        if !self.node.contains_key(&node) {
            self.node_order.push(node.clone());
        }
        self.node.insert(node, attrs);
    }

    /// Port of `add_edge(src, dst, key=None, attr_dict=None, **attrs)`.
    /// Auto-assigns `key = max([0] + int keys of entry) + 1` when `key` is
    /// `None` and returns the key used.
    pub fn add_edge(
        &mut self,
        src: &str,
        dst: &str,
        key: Option<EdgeKey>,
        attrs: Attrs,
    ) -> EdgeKey {
        // self.node.setdefault(src, {}); self.node.setdefault(dst, {})
        for n in [src, dst] {
            if !self.node.contains_key(n) {
                self.node_order.push(n.to_string());
                self.node.insert(n.to_string(), Attrs::default());
            }
        }
        // self.edge.setdefault(src, {}); self.edge.setdefault(dst, {})
        for n in [src, dst] {
            if !self.edge.contains_key(n) {
                self.edge_order.push(n.to_string());
                self.edge.insert(n.to_string(), DstMap::default());
            }
        }
        // self.edge[src].setdefault(dst, {}) -> shared entry index
        let entry_idx = {
            let dm = self.edge.get_mut(src).expect("src edge map inserted above");
            if let Some(&i) = dm.index.get(dst) {
                i
            } else {
                let i = self.entries.len();
                self.entries.push(EdgeEntry::new());
                dm.order.push(dst.to_string());
                dm.index.insert(dst.to_string(), i);
                i
            }
        };
        // entry = self.edge[dst][src] = self.edge[src][dst]
        {
            let dm = self.edge.get_mut(dst).expect("dst edge map inserted above");
            match dm.index.get(src) {
                Some(&i) => debug_assert_eq!(i, entry_idx, "edge ends must alias"),
                None => {
                    dm.order.push(src.to_string());
                    dm.index.insert(src.to_string(), entry_idx);
                }
            }
        }
        let entry = &mut self.entries[entry_idx];
        // If no key, pick next ordinal number among the *int* keys
        let key = match key {
            Some(k) => k,
            None => {
                let max_int = entry
                    .iter()
                    .filter_map(|(k, _)| k.as_int())
                    .max()
                    .unwrap_or(0);
                EdgeKey::Int(max_int + 1)
            }
        };
        entry.set(key.clone(), attrs);
        key
    }

    /// Port of `nodes(data=False)`: node names in insertion order.
    pub fn nodes(&self) -> Vec<&str> {
        self.node_order.iter().map(String::as_str).collect()
    }

    /// Port of `nodes(data=True)`: `(node, attrs)` pairs in insertion order.
    pub fn nodes_data(&self) -> Vec<(&str, &Attrs)> {
        self.node_order
            .iter()
            .map(|n| (n.as_str(), &self.node[n]))
            .collect()
    }

    /// Port of `self.node[node]` (attr dict of one node).
    pub fn node(&self, node: &str) -> Option<&Attrs> {
        self.node.get(node)
    }

    /// Port of `self.edge[node]`: `(dst, entry)` pairs in insertion order.
    pub fn links_from(&self, node: &str) -> Option<Vec<(&str, &EdgeEntry)>> {
        let dm = self.edge.get(node)?;
        Some(
            dm.order
                .iter()
                .map(|d| (d.as_str(), &self.entries[dm.index[d]]))
                .collect(),
        )
    }

    /// Port of `self.edge[src][dst]`.
    pub fn get(&self, src: &str, dst: &str) -> Option<&EdgeEntry> {
        let dm = self.edge.get(src)?;
        dm.index.get(dst).map(|&i| &self.entries[i])
    }

    /// Mutable access to `self.edge[src][dst]` (for `setlinkInfo`).
    pub fn get_mut(&mut self, src: &str, dst: &str) -> Option<&mut EdgeEntry> {
        let dm = self.edge.get_mut(src)?;
        let i = *dm.index.get(dst)?;
        Some(&mut self.entries[i])
    }

    /// Core edge iterator, port of `edges_iter(data=True, keys=True)`:
    /// yields `(src, dst, key, attrs)` for each parallel edge, skipping the
    /// `src > dst` duplicates (each edge reported from its lexically smaller
    /// endpoint), in graph insertion order.
    pub fn iter_edges(&self) -> impl Iterator<Item = (&str, &str, &EdgeKey, &Attrs)> {
        self.edges_data_keys().into_iter()
    }

    /// Port of `edges()`: `(src, dst)` pairs.
    pub fn edges(&self) -> Vec<(&str, &str)> {
        self.edges_data_keys()
            .into_iter()
            .map(|(s, d, _, _)| (s, d))
            .collect()
    }

    /// Port of `edges(keys=True)`: `(src, dst, key)` triples.
    pub fn edges_keys(&self) -> Vec<(&str, &str, &EdgeKey)> {
        self.edges_data_keys()
            .into_iter()
            .map(|(s, d, k, _)| (s, d, k))
            .collect()
    }

    /// Port of `edges(data=True)`: `(src, dst, attrs)` triples.
    pub fn edges_data(&self) -> Vec<(&str, &str, &Attrs)> {
        self.edges_data_keys()
            .into_iter()
            .map(|(s, d, _, a)| (s, d, a))
            .collect()
    }

    /// Port of `edges(data=True, keys=True)`: full `(src, dst, key, attrs)`.
    pub fn edges_data_keys(&self) -> Vec<(&str, &str, &EdgeKey, &Attrs)> {
        let mut out = Vec::new();
        for src in &self.edge_order {
            let dm = &self.edge[src];
            for dst in &dm.order {
                if src.as_str() > dst.as_str() {
                    // Skip duplicate edges
                    continue;
                }
                let entry = &self.entries[dm.index[dst]];
                for (k, a) in entry.iter() {
                    out.push((src.as_str(), dst.as_str(), k, a));
                }
            }
        }
        out
    }

    /// Port of `__len__`: the number of nodes.
    pub fn len(&self) -> usize {
        self.node.len()
    }

    /// Whether the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.node.is_empty()
    }

    /// Port of `convertTo(cls, data=False, keys=False)`: build a new
    /// [`MultiGraph`] from this one.
    ///
    /// `data`/`keys` select the edge tuple shape exactly like the Python
    /// original: the key is kept iff `keys`, and the attrs are kept iff
    /// `data || keys` (otherwise the new edge gets empty attrs and, when
    /// `keys` is false, an auto-assigned key).
    pub fn convert_to(&self, data: bool, keys: bool) -> MultiGraph {
        let mut g = MultiGraph::new();
        for (n, a) in self.nodes_data() {
            g.add_node(n, a.clone());
        }
        let with_data = data || keys;
        for (src, dst, key, attrs) in self.edges_data_keys() {
            g.add_edge(
                src,
                dst,
                if keys { Some(key.clone()) } else { None },
                if with_data {
                    attrs.clone()
                } else {
                    Attrs::new()
                },
            );
        }
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;

    fn attrs_of(pairs: &[(&str, i64)]) -> Attrs {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::from(*v)))
            .collect()
    }

    #[test]
    fn add_node_keeps_order_and_replaces() {
        let mut g = MultiGraph::new();
        g.add_node("b", attrs_of(&[("x", 1)]));
        g.add_node("a", Attrs::new());
        g.add_node("b", attrs_of(&[("y", 2)])); // replaces attrs, keeps position
        assert_eq!(g.nodes(), vec!["b", "a"]);
        assert_eq!(g.node("b"), Some(&attrs_of(&[("y", 2)])));
        assert_eq!(g.node("missing"), None);
        assert_eq!(g.len(), 2);
        assert!(!g.is_empty());
    }

    #[test]
    fn add_edge_auto_keys_are_ordinals() {
        let mut g = MultiGraph::new();
        let k1 = g.add_edge("a", "b", None, Attrs::new());
        let k2 = g.add_edge("a", "b", None, Attrs::new());
        let k3 = g.add_edge("b", "a", None, Attrs::new()); // same entry as a-b
        assert_eq!(k1, EdgeKey::Int(1));
        assert_eq!(k2, EdgeKey::Int(2));
        assert_eq!(k3, EdgeKey::Int(3));
        // add_edge also creates the nodes (setdefault)
        assert_eq!(g.nodes(), vec!["a", "b"]);
        assert_eq!(g.len(), 2);
        let entry = g.get("a", "b").expect("entry");
        assert_eq!(entry.len(), 3);
        // edge[b][a] aliases edge[a][b]
        assert_eq!(g.get("b", "a"), Some(entry));
    }

    #[test]
    fn add_edge_key_auto_with_existing_int_key() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", Some(EdgeKey::Int(5)), Attrs::new());
        let k = g.add_edge("a", "b", None, Attrs::new());
        assert_eq!(k, EdgeKey::Int(6), "max([0, 5]) + 1");
    }

    #[test]
    fn add_edge_key_auto_with_only_string_keys() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", Some(EdgeKey::Str("x".into())), Attrs::new());
        let k = g.add_edge("a", "b", None, Attrs::new());
        assert_eq!(k, EdgeKey::Int(1), "no int keys -> max([0]) + 1 = 1");
    }

    #[test]
    fn add_edge_explicit_key_replaces_attrs_in_place() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", Some(EdgeKey::Int(1)), attrs_of(&[("p", 1)]));
        g.add_edge("a", "b", Some(EdgeKey::Int(1)), attrs_of(&[("q", 2)]));
        let entry = g.get("a", "b").expect("entry");
        assert_eq!(entry.len(), 1, "same key replaces, not appends");
        assert_eq!(entry.get(&EdgeKey::Int(1)), Some(&attrs_of(&[("q", 2)])));
        assert_eq!(entry.iter().next().unwrap().0, &EdgeKey::Int(1));
    }

    #[test]
    fn add_edge_attr_merge_via_update() {
        // Python add_edge(src, dst, key, attr_dict, **attrs) mutates
        // attr_dict with attrs before storing; caller merges via update().
        let mut g = MultiGraph::new();
        let mut attr = attrs_of(&[("p", 1)]);
        attr.update(&attrs_of(&[("q", 2)]));
        g.add_edge("a", "b", None, attr);
        let entry = g.get("a", "b").expect("entry");
        let stored = entry.get(&EdgeKey::Int(1)).expect("attrs");
        let keys: Vec<&str> = stored.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["p", "q"]);
    }

    #[test]
    fn edges_iter_skips_src_greater_dst() {
        let mut g = MultiGraph::new();
        g.add_edge("s1", "h1", None, attrs_of(&[("w", 1)])); // stored from s1
        g.add_edge("h2", "s1", None, attrs_of(&[("w", 2)]));
        // edge dict order: s1, h1, h2. From s1: dsts h1 (s1>h1 -> skip), h2 (skip).
        // From h1: dst s1 -> h1>s1? no -> yield. From h2: dst s1 -> yield.
        let edges = g.edges_data_keys();
        let pairs: Vec<(&str, &str)> = edges.iter().map(|(s, d, _, _)| (*s, *d)).collect();
        assert_eq!(pairs, vec![("h1", "s1"), ("h2", "s1")]);
        assert_eq!(edges[0].2, &EdgeKey::Int(1));
        assert_eq!(edges[0].3.get("w"), Some(&Value::from(1_i64)));
    }

    #[test]
    fn edges_variants() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", None, attrs_of(&[("w", 9)]));
        g.add_edge("a", "b", None, Attrs::new());
        assert_eq!(g.edges(), vec![("a", "b"), ("a", "b")]);
        assert_eq!(
            g.edges_keys(),
            vec![("a", "b", &EdgeKey::Int(1)), ("a", "b", &EdgeKey::Int(2))]
        );
        assert_eq!(g.edges_data().len(), 2);
        assert_eq!(g.edges_data()[0].2.get("w"), Some(&Value::from(9_i64)));
        assert_eq!(g.edges_data()[1].2.len(), 0);
        assert_eq!(g.iter_edges().count(), 2);
    }

    #[test]
    fn self_loop() {
        let mut g = MultiGraph::new();
        let k = g.add_edge("a", "a", None, Attrs::new());
        assert_eq!(k, EdgeKey::Int(1));
        assert_eq!(g.edges(), vec![("a", "a")]);
        assert_eq!(g.get("a", "a").expect("entry").len(), 1);
        assert_eq!(g.links_from("a").expect("map").len(), 1);
    }

    #[test]
    fn links_from_and_get_mut() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", None, attrs_of(&[("w", 1)]));
        g.add_edge("a", "c", None, Attrs::new());
        let links = g.links_from("a").expect("a");
        let dsts: Vec<&str> = links.iter().map(|(d, _)| *d).collect();
        assert_eq!(dsts, vec!["b", "c"]);
        assert!(g.links_from("zz").is_none());
        let entry = g.get_mut("a", "b").expect("entry");
        entry.set(EdgeKey::Int(1), attrs_of(&[("w", 42)]));
        assert_eq!(
            g.get("a", "b")
                .expect("entry")
                .get(&EdgeKey::Int(1))
                .unwrap()
                .get("w"),
            Some(&Value::from(42_i64))
        );
        assert!(g.get_mut("a", "zz").is_none());
    }

    #[test]
    fn min_key_and_entry_iter() {
        let mut g = MultiGraph::new();
        g.add_edge("a", "b", Some(EdgeKey::Int(3)), Attrs::new());
        g.add_edge("a", "b", Some(EdgeKey::Int(1)), Attrs::new());
        g.add_edge("a", "b", Some(EdgeKey::Int(2)), Attrs::new());
        let entry = g.get("a", "b").expect("entry");
        assert_eq!(entry.min_key(), Some(EdgeKey::Int(1)));
        let keys: Vec<EdgeKey> = entry.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys,
            vec![EdgeKey::Int(3), EdgeKey::Int(1), EdgeKey::Int(2)]
        );
    }

    #[test]
    fn convert_to_roundtrip() {
        let mut g = MultiGraph::new();
        g.add_node("a", attrs_of(&[("x", 1)]));
        g.add_node("b", Attrs::new());
        g.add_edge("a", "b", None, attrs_of(&[("w", 5)]));
        g.add_edge("a", "b", Some(EdgeKey::Int(9)), attrs_of(&[("w", 6)]));

        let h: MultiGraph = g.convert_to(true, true);
        assert_eq!(h.nodes(), vec!["a", "b"]);
        assert_eq!(h.node("a"), Some(&attrs_of(&[("x", 1)])));
        assert_eq!(h.edges_keys(), g.edges_keys());
        assert_eq!(h.get("a", "b"), g.get("a", "b"));

        let h2: MultiGraph = g.convert_to(false, false);
        assert_eq!(h2.edges(), vec![("a", "b"), ("a", "b")]);
        // keys=False -> auto keys reassigned 1, 2 in iteration order
        assert_eq!(
            h2.edges_keys(),
            vec![("a", "b", &EdgeKey::Int(1)), ("a", "b", &EdgeKey::Int(2))]
        );
        assert_eq!(h2.get("a", "b").expect("e").len(), 2);

        let h3: MultiGraph = g.convert_to(true, false);
        assert_eq!(h3.edges_data().len(), 2);
        assert_eq!(h3.edges_keys().len(), 2);
    }

    #[test]
    fn edge_key_display() {
        assert_eq!(EdgeKey::Int(12).to_string(), "12");
        assert_eq!(EdgeKey::Str("k".into()).to_string(), "k");
    }
}
