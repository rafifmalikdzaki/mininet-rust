//! Insertion-ordered node maps — the Rust stand-in for the CPython `dict`s
//! `Mininet.hosts`/`switches`/`controllers` were lists *and* `nameToNode`
//! was a dict in `mininet/net.py`.
//!
//! Python stores the same node object in both the list and `nameToNode`;
//! Rust cannot alias like that, so [`Mininet`](crate::Mininet) keeps one
//! [`NodeMap`] per node kind (the source of truth, insertion-ordered like a
//! CPython dict) plus a [`NameToNode`] index that maps a name to a
//! [`NodeId`] (which map + position). Borrowed views over the three maps are
//! exposed as [`NodeRef`] / [`NodeRefMut`].

use std::collections::HashMap;

use mn_node::{Controller, Host, Node, Switch};

/// Insertion-ordered `name -> T` map (a CPython `dict` with `list` order).
#[derive(Debug, Clone)]
pub struct NodeMap<T> {
    order: Vec<String>,
    index: HashMap<String, usize>,
    items: Vec<T>,
}

impl<T> Default for NodeMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> NodeMap<T> {
    /// New empty map.
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            index: HashMap::new(),
            items: Vec::new(),
        }
    }

    /// `map[name] = item`: replaces in place (keeping position) or appends.
    /// Returns the item's position (the [`NodeId`] payload).
    pub fn insert(&mut self, name: &str, item: T) -> usize {
        if let Some(&i) = self.index.get(name) {
            self.items[i] = item;
            i
        } else {
            let i = self.items.len();
            self.order.push(name.to_string());
            self.index.insert(name.to_string(), i);
            self.items.push(item);
            i
        }
    }

    /// `map.get(name)`.
    pub fn get(&self, name: &str) -> Option<&T> {
        self.index.get(name).map(|&i| &self.items[i])
    }

    /// `map.get_mut(name)`.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut T> {
        match self.index.get(name) {
            Some(&i) => Some(&mut self.items[i]),
            None => None,
        }
    }

    /// `del map[name]`; returns the removed item.
    pub fn remove(&mut self, name: &str) -> Option<T> {
        let i = self.index.remove(name)?;
        self.order.remove(i);
        // Rebuild the index: positions shifted by one after `i`.
        let item = self.items.remove(i);
        for (pos, n) in self.order.iter().enumerate() {
            if pos >= i {
                self.index.insert(n.clone(), pos);
            }
        }
        Some(item)
    }

    /// `name in map`.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// `len(map)`.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Position of `name` (payload of the corresponding [`NodeId`]).
    pub fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// Item at a position returned by [`NodeMap::insert`]/[`NodeMap::position`].
    pub fn at(&self, i: usize) -> &T {
        &self.items[i]
    }

    /// Mutable item at a position.
    pub fn at_mut(&mut self, i: usize) -> &mut T {
        &mut self.items[i]
    }

    /// `map.keys()` in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    /// `map.values()` in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    /// Mutable `map.values()` in insertion order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }

    /// `map.items()` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.order
            .iter()
            .map(move |n| (n.as_str(), &self.items[self.index[n]]))
    }
}

/// Which map a [`NameToNode`] entry points at, plus the position in it —
/// the Rust replacement for Python storing the node object itself in
/// `nameToNode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// Index into [`Mininet::hosts`](crate::Mininet::hosts).
    Host(usize),
    /// Index into [`Mininet::switches`](crate::Mininet::switches).
    Switch(usize),
    /// Index into [`Mininet::controllers`](crate::Mininet::controllers).
    Controller(usize),
}

/// `Mininet.nameToNode`: name -> [`NodeId`], insertion-ordered.
#[derive(Debug, Clone, Default)]
pub struct NameToNode {
    order: Vec<String>,
    index: HashMap<String, NodeId>,
}

impl NameToNode {
    /// New empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// `nameToNode[name] = id` (replace keeps position, like a dict).
    pub fn insert(&mut self, name: &str, id: NodeId) {
        if !self.index.contains_key(name) {
            self.order.push(name.to_string());
        }
        self.index.insert(name.to_string(), id);
    }

    /// `nameToNode.get(name)`.
    pub fn get(&self, name: &str) -> Option<NodeId> {
        self.index.get(name).copied()
    }

    /// `nameToNode.pop(name)`.
    pub fn remove(&mut self, name: &str) -> Option<NodeId> {
        let id = self.index.remove(name)?;
        self.order.retain(|n| n != name);
        Some(id)
    }

    /// `name in nameToNode`.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// `len(nameToNode)`.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Node names in insertion order (`for name in self` iterates
    /// hosts, switches, controllers in Python; [`Mininet::keys`] provides
    /// that order — this is the raw map order).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }
}

/// Borrowed view of a node in any of the three maps — the Rust equivalent of
/// a Python `Node` object reference fetched from `nameToNode`.
#[derive(Debug, Clone, Copy)]
pub enum NodeRef<'a> {
    /// A host.
    Host(&'a Host),
    /// A switch.
    Switch(&'a Switch),
    /// A controller.
    Controller(&'a Controller),
}

impl<'a> NodeRef<'a> {
    /// The common [`Node`] view (`node.name`, `node.cmd`, ...).
    pub fn node(&self) -> &'a Node {
        match self {
            NodeRef::Host(h) => h.node(),
            NodeRef::Switch(s) => s.node(),
            NodeRef::Controller(c) => c.node(),
        }
    }

    /// Node name.
    pub fn name(&self) -> &'a str {
        &self.node().name
    }
}

/// Mutably borrowed view of a node in any of the three maps.
pub enum NodeRefMut<'a> {
    /// A host.
    Host(&'a mut Host),
    /// A switch.
    Switch(&'a mut Switch),
    /// A controller.
    Controller(&'a mut Controller),
}

impl<'a> NodeRefMut<'a> {
    /// The common [`Node`] view, mutably.
    pub fn node_mut(&mut self) -> &mut Node {
        match self {
            NodeRefMut::Host(h) => h.node_mut(),
            NodeRefMut::Switch(s) => s.node_mut(),
            NodeRefMut::Controller(c) => c.node_mut(),
        }
    }

    /// Node name.
    pub fn name(&self) -> &str {
        match self {
            NodeRefMut::Host(h) => &h.node().name,
            NodeRefMut::Switch(s) => &s.node().name,
            NodeRefMut::Controller(c) => &c.node().name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_map_insert_get_remove_keeps_insertion_order() {
        let mut m: NodeMap<u32> = NodeMap::new();
        assert_eq!(m.insert("h2", 2), 0);
        assert_eq!(m.insert("h1", 1), 1);
        assert_eq!(m.insert("h10", 10), 2);
        let keys: Vec<&str> = m.keys().collect();
        assert_eq!(keys, vec!["h2", "h1", "h10"]);
        // replace keeps position
        assert_eq!(m.insert("h1", 100), 1);
        assert_eq!(m.get("h1"), Some(&100));
        assert_eq!(m.len(), 3);
        assert!(!m.is_empty());
        assert_eq!(m.remove("h2"), Some(2));
        assert_eq!(m.get("h2"), None);
        assert!(!m.contains("h2"));
        let keys: Vec<&str> = m.keys().collect();
        assert_eq!(keys, vec!["h1", "h10"]);
        assert_eq!(m.at(1), &10);
        assert_eq!(m.position("h10"), Some(1));
    }

    #[test]
    fn name_to_node_roundtrip() {
        let mut n = NameToNode::new();
        n.insert("h1", NodeId::Host(0));
        n.insert("s1", NodeId::Switch(0));
        n.insert("h1", NodeId::Host(3)); // replace keeps position
        assert_eq!(n.get("h1"), Some(NodeId::Host(3)));
        assert_eq!(n.get("s1"), Some(NodeId::Switch(0)));
        assert_eq!(n.get("c0"), None);
        assert!(n.contains("s1"));
        assert_eq!(n.len(), 2);
        assert!(!n.is_empty());
        let keys: Vec<&str> = n.keys().collect();
        assert_eq!(keys, vec!["h1", "s1"]);
        assert_eq!(n.remove("h1"), Some(NodeId::Host(3)));
        assert_eq!(n.len(), 1);
    }
}
