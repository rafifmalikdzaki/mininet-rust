//! Shared helpers for the `mn-topo` integration tests: Python-`repr`-compatible
//! rendering of the topology model plus canonical text dumps used both by the
//! fixture characterization test and by the differential test against the
//! original Python implementation.

// Not every integration test uses every helper.
#![allow(dead_code)]

use mn_topo::{Attrs, EdgeKey, Link, MultiGraph, PortLookup, Ports, Topo, Value};

/// Python `repr(str)`: single quotes unless the string contains `'` and not
/// `"` (then double quotes); escapes `\\`, the delimiter, and control chars.
pub fn repr_str(s: &str) -> String {
    let use_double = s.contains('\'') && !s.contains('"');
    let (open, close) = if use_double { ('"', '"') } else { ('\'', '\'') };
    let mut out = String::new();
    out.push(open);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == open => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(close);
    out
}

/// Python `repr(float)` for the finite case: shortest round-trip with a
/// `.0` for integral values (matches CPython for the magnitudes used in
/// tests; CPython switches to `1e+16`-style exponents above 1e16, which the
/// tests avoid).
pub fn repr_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f == f64::INFINITY {
        return "inf".to_string();
    }
    if f == f64::NEG_INFINITY {
        return "-inf".to_string();
    }
    let r = format!("{f:?}");
    if r.contains('.') || r.contains('e') || r.contains("inf") || r.contains("NaN") {
        r
    } else {
        format!("{r}.0")
    }
}

/// Python `repr(value)` for the [`Value`] subset.
pub fn repr_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Int(i) => i.to_string(),
        Value::Float(f) => repr_float(*f),
        Value::Str(s) => repr_str(s),
    }
}

/// Python `repr(dict)` for [`Attrs`] (insertion order, quoted keys).
pub fn repr_attrs(a: &Attrs) -> String {
    let inner: Vec<String> = a
        .iter()
        .map(|(k, v)| format!("{}: {}", repr_str(k), repr_value(v)))
        .collect();
    format!("{{{}}}", inner.join(", "))
}

/// Python `repr(list)`.
pub fn repr_list(parts: Vec<String>) -> String {
    format!("[{}]", parts.join(", "))
}

/// Python `repr(tuple)` (note the single-element `(x,)` form).
pub fn repr_tuple(parts: Vec<String>) -> String {
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

/// Python `repr(edge_key)`: ints bare, strings quoted.
pub fn repr_key(k: &EdgeKey) -> String {
    match k {
        EdgeKey::Int(i) => i.to_string(),
        EdgeKey::Str(s) => repr_str(s),
    }
}

/// Python `repr(topo.ports)`: `{src: {sport: (dst, dport)}}`.
pub fn repr_ports(p: &Ports) -> String {
    let inner: Vec<String> = p
        .iter()
        .map(|(src, pm)| {
            let entries: Vec<String> = pm
                .iter()
                .map(|(sport, (dst, dport))| format!("{sport}: ({}, {dport})", repr_str(dst)))
                .collect();
            format!("{}: {{{}}}", repr_str(src), entries.join(", "))
        })
        .collect();
    format!("{{{}}}", inner.join(", "))
}

/// Python `repr(topo.port(src, dst))`: `(sport, dport)` or a list of tuples.
pub fn repr_port_lookup(p: &PortLookup) -> String {
    match p {
        PortLookup::Single(s, d) => repr_tuple(vec![s.to_string(), d.to_string()]),
        PortLookup::Many(v) => repr_list(
            v.iter()
                .map(|(s, d)| repr_tuple(vec![s.to_string(), d.to_string()]))
                .collect(),
        ),
    }
}

/// repr of one [`Link`] in the requested shape.
pub fn repr_link(l: &Link<'_>, with_keys: bool, with_info: bool) -> String {
    let mut parts = vec![repr_str(l.node1), repr_str(l.node2)];
    if with_keys {
        parts.push(repr_key(l.key.expect("key present when with_keys")));
    }
    if with_info {
        parts.push(repr_attrs(l.info.expect("info present when with_info")));
    }
    repr_tuple(parts)
}

/// Canonical text dump of a topo, byte-identical to what
/// `tests/py_driver.py` produces for the same topology (used by the
/// differential test).
pub fn dump_topo(class_name: &str, t: &Topo, extras: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("class: {class_name}"));
    lines.push(format!(
        "nodes: {}",
        repr_list(t.nodes(true).iter().map(|n| repr_str(n)).collect())
    ));
    lines.push(format!(
        "nodes_unsorted: {}",
        repr_list(t.nodes(false).iter().map(|n| repr_str(n)).collect())
    ));
    lines.push(format!(
        "hosts: {}",
        repr_list(t.hosts(true).iter().map(|n| repr_str(n)).collect())
    ));
    lines.push(format!(
        "switches: {}",
        repr_list(t.switches(true).iter().map(|n| repr_str(n)).collect())
    ));
    for n in t.nodes(false) {
        let info = t.node_info(&n).expect("node exists");
        lines.push(format!("nodeInfo({n}): {}", repr_attrs(info)));
    }
    lines.push("links_ski:".to_string());
    for l in t.links(true, true, true) {
        lines.push(format!("  {}", repr_link(&l, true, true)));
    }
    lines.push("links_unsorted:".to_string());
    for l in t.links(false, true, true) {
        lines.push(format!("  {}", repr_link(&l, true, true)));
    }
    lines.push("links_sort_info:".to_string());
    for l in t.links(true, false, true) {
        lines.push(format!("  {}", repr_link(&l, false, true)));
    }
    lines.push("links_keys:".to_string());
    for l in t.links(false, true, false) {
        lines.push(format!("  {}", repr_link(&l, true, false)));
    }
    lines.push("links_plain:".to_string());
    for l in t.links(false, false, false) {
        lines.push(format!("  {}", repr_link(&l, false, false)));
    }
    lines.push(format!("ports: {}", repr_ports(&t.ports)));
    for l in t.links(true, false, false) {
        let p = t
            .port(l.node1, l.node2)
            .expect("port lookup for a link endpoint");
        lines.push(format!(
            "port({}, {}): {}",
            l.node1,
            l.node2,
            repr_port_lookup(&p)
        ));
    }
    for (k, v) in extras {
        lines.push(format!("{k}: {v}"));
    }
    lines.join("\n")
}

/// Canonical text dump of a raw [`MultiGraph`] plus the keys returned by the
/// `add_edge` calls, byte-identical to `tests/py_driver.py`'s `dump_graph`.
pub fn dump_graph(g: &MultiGraph, keys_returned: &[EdgeKey]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("len: {}", g.len()));
    lines.push(format!(
        "nodes: {}",
        repr_list(g.nodes().iter().map(|n| repr_str(n)).collect())
    ));
    lines.push(format!(
        "nodes_data: {}",
        repr_list(
            g.nodes_data()
                .iter()
                .map(|(n, a)| repr_tuple(vec![repr_str(n), repr_attrs(a)]))
                .collect()
        )
    ));
    lines.push(format!(
        "edges_dk: {}",
        repr_list(
            g.edges_data_keys()
                .iter()
                .map(|(s, d, k, a)| repr_tuple(vec![
                    repr_str(s),
                    repr_str(d),
                    repr_key(k),
                    repr_attrs(a)
                ]))
                .collect()
        )
    ));
    lines.push(format!(
        "edges: {}",
        repr_list(
            g.edges()
                .iter()
                .map(|(s, d)| repr_tuple(vec![repr_str(s), repr_str(d)]))
                .collect()
        )
    ));
    lines.push(format!(
        "edges_k: {}",
        repr_list(
            g.edges_keys()
                .iter()
                .map(|(s, d, k)| repr_tuple(vec![repr_str(s), repr_str(d), repr_key(k)]))
                .collect()
        )
    ));
    lines.push(format!(
        "edges_d: {}",
        repr_list(
            g.edges_data()
                .iter()
                .map(|(s, d, a)| repr_tuple(vec![repr_str(s), repr_str(d), repr_attrs(a)]))
                .collect()
        )
    ));
    lines.push(format!(
        "keys_returned: {}",
        repr_list(keys_returned.iter().map(repr_key).collect())
    ));
    lines.join("\n")
}
