//! Attribute values and attribute dictionaries.
//!
//! Python's `topo.py` stores arbitrary objects in node/edge attr dicts
//! (`self.node[node] = {...}`, `entry[key] = attr_dict`). The values that
//! actually occur in Mininet topologies are booleans (`isSwitch`), integers
//! (ports, `bw`), floats and strings (`dpid`, `ip`), so this module models
//! them with [`Value`] and keeps the dicts insertion-ordered like CPython
//! dicts in [`Attrs`].

use std::fmt;

/// A single attribute value. Mirrors the JSON-ish subset of Python objects
/// used by `topo.py`/`topolib.py` node and link options.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Python `bool` (e.g. `isSwitch=True`).
    Bool(bool),
    /// Python `int` (e.g. `port1=0`).
    Int(i64),
    /// Python `float`.
    Float(f64),
    /// Python `str` (e.g. `dpid='101'`).
    Str(String),
}

impl Value {
    /// Borrow as `bool` if this is a [`Value::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Borrow as `i64` if this is a [`Value::Int`].
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Borrow as `f64` if this is a [`Value::Float`].
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Borrow as `&str` if this is a [`Value::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

/// Python `str(value)`: used by `naturalSeq` when sorting link tuples that
/// contain edge keys.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{}", if *b { "True" } else { "False" }),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x:?}"),
            Value::Str(s) => write!(f, "{s}"),
        }
    }
}

/// An insertion-ordered string-keyed attribute dict, mirroring a CPython
/// `dict` as used by `MultiGraph.add_node`/`add_edge` (`attr_dict.update(...)`,
/// `attr_dict[key] = value` keeps the original position of an existing key).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attrs {
    items: Vec<(String, Value)>,
}

impl Attrs {
    /// New empty dict (`{}`).
    pub fn new() -> Self {
        Self::default()
    }

    /// `d[key]` lookup.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `d[key] = value`: replaces the value in place (keeping the key's
    /// original position) or appends a new key. Returns the old value.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        let key = key.into();
        if let Some(slot) = self.items.iter_mut().find(|(k, _)| *k == key) {
            Some(std::mem::replace(&mut slot.1, value.into()))
        } else {
            self.items.push((key, value.into()));
            None
        }
    }

    /// `d.update(other)`: insert/replace every entry of `other` in order.
    pub fn update(&mut self, other: &Attrs) {
        for (k, v) in &other.items {
            self.insert(k.clone(), v.clone());
        }
    }

    /// `d.update(iterable_of_pairs)`.
    pub fn update_from(&mut self, iter: impl IntoIterator<Item = (String, Value)>) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }

    /// `key in d`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.items.iter().any(|(k, _)| k == key)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the dict is empty (Python truthiness: `not opts`).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate `(key, value)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.items.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl FromIterator<(String, Value)> for Attrs {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let mut a = Attrs::new();
        a.update_from(iter);
        a
    }
}

impl<'a> IntoIterator for &'a Attrs {
    type Item = (&'a str, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (String, Value)>,
        fn(&'a (String, Value)) -> (&'a str, &'a Value),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl IntoIterator for Attrs {
    type Item = (String, Value);
    type IntoIter = std::vec::IntoIter<(String, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

/// Build an [`Attrs`] dict literal, e.g. `attrs!("isSwitch" => true)`.
///
/// Keys accept anything `Into<String>`; values anything `Into<Value>`
/// (`bool`, `i64`, `f64`, `&str`, `String`).
#[macro_export]
macro_rules! attrs {
    ($($key:expr => $value:expr),* $(,)?) => {
        $crate::Attrs::from_iter([$(($key.to_string(), $crate::Value::from($value))),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_conversions() {
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(3_i64), Value::Int(3));
        assert_eq!(Value::from(2.5_f64), Value::Float(2.5));
        assert_eq!(Value::from("x"), Value::Str("x".to_string()));
        assert_eq!(Value::from("x".to_string()), Value::Str("x".to_string()));
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Int(7).as_int(), Some(7));
        assert_eq!(Value::Float(1.5).as_float(), Some(1.5));
        assert_eq!(Value::Str("s".into()).as_str(), Some("s"));
        assert_eq!(Value::Int(1).as_str(), None);
    }

    #[test]
    fn value_display_is_python_str() {
        assert_eq!(Value::Bool(true).to_string(), "True");
        assert_eq!(Value::Bool(false).to_string(), "False");
        assert_eq!(Value::Int(-5).to_string(), "-5");
        assert_eq!(Value::Str("h1".into()).to_string(), "h1");
    }

    #[test]
    fn attrs_insert_keeps_position_and_replaces() {
        let mut a = Attrs::new();
        a.insert("b", 1_i64);
        a.insert("a", 2_i64);
        assert_eq!(a.insert("b", 3_i64), Some(Value::Int(1)));
        let keys: Vec<&str> = a.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["b", "a"], "existing key keeps its position");
        assert_eq!(a.get("b"), Some(&Value::Int(3)));
        assert_eq!(a.len(), 2);
        assert!(!a.is_empty());
        assert!(a.contains_key("a"));
        assert!(!a.contains_key("z"));
    }

    #[test]
    fn attrs_update_semantics() {
        // Python: attr_dict.update(attrs) — new keys appended, existing
        // keys replaced in place.
        let mut a: Attrs = [
            ("x".to_string(), Value::Int(1)),
            ("y".to_string(), Value::Int(2)),
        ]
        .into_iter()
        .collect();
        let b: Attrs = [
            ("y".to_string(), Value::Int(20)),
            ("z".to_string(), Value::Bool(true)),
        ]
        .into_iter()
        .collect();
        a.update(&b);
        let got: Vec<(String, Value)> = a.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("x".to_string(), Value::Int(1)),
                ("y".to_string(), Value::Int(20)),
                ("z".to_string(), Value::Bool(true)),
            ]
        );
    }

    #[test]
    fn attrs_update_from_pairs() {
        let mut a = Attrs::new();
        a.update_from([
            ("node1".to_string(), Value::from("h1")),
            ("port1".to_string(), Value::from(0_i64)),
        ]);
        assert_eq!(a.get("node1"), Some(&Value::Str("h1".to_string())));
        assert_eq!(a.get("port1"), Some(&Value::Int(0)));
    }

    #[test]
    fn attrs_macro() {
        let a = attrs!("isSwitch" => true, "dpid" => "101");
        assert_eq!(a.get("isSwitch"), Some(&Value::Bool(true)));
        assert_eq!(a.get("dpid"), Some(&Value::Str("101".to_string())));
        assert_eq!(a.len(), 2);
        let empty = attrs!();
        assert!(empty.is_empty());
    }

    #[test]
    fn attrs_from_iter_str_keys() {
        let a: Attrs = [("k".to_string(), Value::from(1_i64))]
            .into_iter()
            .collect();
        assert_eq!(a.get("k"), Some(&Value::Int(1)));
    }
}
