//! Characterization tests: rebuild the topos of the checked-in baseline
//! fixtures (`fixtures/topo_single3.txt`, `fixtures/topo_linear4.txt`,
//! `fixtures/topo_tree.txt`, captured from the original Python Mininet via
//! `buildTopo` on this branch) with the Rust port and assert byte-identical
//! output of the exact dump the fixtures were captured with:
//!
//! ```text
//! buildTopo arg: <spec>
//! topo class: <ClassName>
//! nodes(): [...]          hosts(): [...]          switches(): [...]
//! nodeInfo(<n>): {...}    (for each node, natural-sorted)
//! links(sort=True, withKeys=True, withInfo=True): ...
//! ports: {...}
//! port(<src>, <dst>): (...)  (for each link, sorted)
//! ```

mod common;

use common::{repr_attrs, repr_link, repr_list, repr_port_lookup, repr_ports, repr_str};
use mn_topo::build_topo;
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Render exactly what the fixture capture script printed for `spec`.
fn render(spec: &str) -> String {
    let built = build_topo(spec).expect("build_topo succeeds");
    let t = built.topo();
    let mut out = String::new();
    out.push_str(&format!("buildTopo arg: {spec}\n"));
    out.push_str(&format!("topo class: {}\n", built.class_name()));
    out.push_str(&format!(
        "nodes(): {}\n",
        repr_list(t.nodes(true).iter().map(|n| repr_str(n)).collect())
    ));
    out.push_str(&format!(
        "hosts(): {}\n",
        repr_list(t.hosts(true).iter().map(|n| repr_str(n)).collect())
    ));
    out.push_str(&format!(
        "switches(): {}\n",
        repr_list(t.switches(true).iter().map(|n| repr_str(n)).collect())
    ));
    for n in t.nodes(true) {
        let info = t.node_info(&n).expect("node exists");
        out.push_str(&format!("nodeInfo({n}): {}\n", repr_attrs(info)));
    }
    out.push_str("links(sort=True, withKeys=True, withInfo=True):\n");
    for link in t.links(true, true, true) {
        // print( "  ", link ) -> three spaces before the tuple repr
        out.push_str(&format!("   {}\n", repr_link(&link, true, true)));
    }
    out.push_str(&format!("ports: {}\n", repr_ports(&t.ports)));
    for link in t.links(true, false, false) {
        let p = t
            .port(link.node1, link.node2)
            .expect("port lookup for link endpoint");
        out.push_str(&format!(
            "port({}, {}): {}\n",
            link.node1,
            link.node2,
            repr_port_lookup(&p)
        ));
    }
    out
}

#[test]
fn topo_single3_matches_fixture() {
    assert_eq!(render("single,3"), fixture("topo_single3.txt"));
}

#[test]
fn topo_linear4_matches_fixture() {
    assert_eq!(render("linear,4"), fixture("topo_linear4.txt"));
}

#[test]
fn topo_tree_matches_fixture() {
    assert_eq!(render("tree,2,2"), fixture("topo_tree.txt"));
}

#[test]
fn fixture_render_is_deterministic() {
    // rendering twice gives identical output (guards against HashMap
    // iteration leaking into the dump)
    assert_eq!(render("tree,3,4"), render("tree,3,4"));
    assert_eq!(render("linear,10,2"), render("linear,10,2"));
}
