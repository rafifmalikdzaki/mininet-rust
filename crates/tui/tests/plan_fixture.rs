//! Graph-from-plan tests against the captured Python fixture
//! `fixtures/topo_single3.txt` (task 2 baseline), plus JSON export tests.

use std::path::PathBuf;

use mn_net::{Mininet, MininetOptions};
use mn_topo::build_topo;
use mn_tui::{EdgeStatus, GraphView, NodeKind, NodeStatus};

fn fixture() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/topo_single3.txt");
    std::fs::read_to_string(path).expect("fixture exists")
}

/// All single-quoted strings in a line, in order.
fn quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inq = false;
    for ch in s.chars() {
        match ch {
            '\'' => {
                if inq {
                    out.push(std::mem::take(&mut cur));
                }
                inq = !inq;
            }
            _ if inq => cur.push(ch),
            _ => {}
        }
    }
    out
}

/// `nodes(): ['h1', 'h2', 'h3', 's1']` -> ["h1", "h2", "h3", "s1"].
fn fixture_nodes(text: &str) -> Vec<String> {
    let line = text
        .lines()
        .find(|l| l.starts_with("nodes():"))
        .expect("nodes line");
    quoted(line)
}

/// `   ('h1', 's1', 1, {...})` lines -> [("h1", "s1"), ...] (first two
/// quoted strings of each link line).
fn fixture_links(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("('"))
        .map(|l| {
            let q = quoted(l);
            (q[0].clone(), q[1].clone())
        })
        .collect()
}

#[test]
fn graph_from_plan_matches_topo_single3_fixture() {
    let text = fixture();
    let want_nodes = fixture_nodes(&text);
    let want_links = fixture_links(&text);
    assert_eq!(want_nodes, ["h1", "h2", "h3", "s1"]);
    assert_eq!(
        want_links,
        [("h1", "s1"), ("h2", "s1"), ("h3", "s1")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<Vec<_>>()
    );

    let built = build_topo("single,3").unwrap();
    let g = GraphView::from_topo(built.topo(), Some("single,3"));
    let got_nodes: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(got_nodes, want_nodes);
    let got_links: Vec<(String, String)> =
        g.edges.iter().map(|e| (e.a.clone(), e.b.clone())).collect();
    assert_eq!(got_links, want_links);
    // kinds + statuses from the fixture's hosts()/switches() lines
    assert!(text.contains("hosts(): ['h1', 'h2', 'h3']"));
    assert!(text.contains("switches(): ['s1']"));
    for n in &g.nodes {
        let want = if n.id == "s1" {
            NodeKind::Switch
        } else {
            NodeKind::Host
        };
        assert_eq!(n.kind, want, "node {}", n.id);
        assert_eq!(n.status, NodeStatus::Planned);
    }
    // edge labels use the fixture's port numbers: ('h1','s1',...,'port1': 0, 'port2': 1)
    assert_eq!(g.edge("h1", "s1").unwrap().label, "h1-eth0<->s1-eth1");
    assert_eq!(g.edge("h2", "s1").unwrap().label, "h2-eth0<->s1-eth2");
    assert_eq!(g.edge("h3", "s1").unwrap().label, "h3-eth0<->s1-eth3");
    // ... and the fixture-matching intf/port fields sit next to the label
    let e = g.edge("h1", "s1").unwrap();
    assert_eq!(e.intf1.as_deref(), Some("h1-eth0"));
    assert_eq!(e.intf2.as_deref(), Some("s1-eth1"));
    assert_eq!(e.port1, Some(0));
    assert_eq!(e.port2, Some(1));
    assert!(g.edges.iter().all(|e| e.status == EdgeStatus::Planned));
    assert_eq!(g.spec.as_deref(), Some("single,3"));
    assert!(!g.live);
}

fn dry_opts() -> MininetOptions {
    MininetOptions {
        controller: None,
        ..Default::default()
    }
}

#[test]
fn graph_from_dry_run_net_carries_ips_links_and_detail() {
    let net = Mininet::dry_run("single,3", dry_opts()).unwrap();
    let g = GraphView::from_net(&net, false);
    assert_eq!(g.spec.as_deref(), Some("single,3"));
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["h1", "h2", "h3", "s1"]);
    assert_eq!(g.node("h1").unwrap().ip.as_deref(), Some("10.0.0.1"));
    assert_eq!(g.node("h2").unwrap().ip.as_deref(), Some("10.0.0.2"));
    assert_eq!(g.node("h3").unwrap().ip.as_deref(), Some("10.0.0.3"));
    assert_eq!(g.edges.len(), 3);
    assert_eq!(g.edges[0].label, "h1-eth0<->s1-eth1");
    assert!(g.edges.iter().all(|e| e.status == EdgeStatus::Planned));
    let d = g.node("h1").unwrap().detail.as_ref().unwrap();
    assert!(
        d.repr.contains("<Host h1: h1-eth0:10.0.0.1 pid=None>"),
        "repr: {}",
        d.repr
    );
    assert_eq!(d.intfs.len(), 1);
    assert_eq!(d.intfs[0].name, "h1-eth0");
    assert_eq!(d.intfs[0].ip.as_deref(), Some("10.0.0.1"));
    // switch detail includes the lo control intf
    let sd = g.node("s1").unwrap().detail.as_ref().unwrap();
    assert_eq!(sd.intfs[0].name, "lo");
    assert!(sd.repr.contains("<OVSSwitch s1:"));
}

#[test]
fn json_export_parses_and_roundtrips() {
    let net = Mininet::dry_run("single,3", dry_opts()).unwrap();
    let g = GraphView::from_net(&net, false);
    let s = g.to_json();
    let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
    assert_eq!(v["spec"], "single,3");
    assert_eq!(v["live"], false);
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 4);
    let edges = v["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3);
    let s1 = nodes.iter().find(|n| n["id"] == "s1").unwrap();
    assert_eq!(s1["kind"], "switch"); // canonical shape: lowercase, like the fixtures
    assert_eq!(s1["status"], "planned");
    let h1 = nodes.iter().find(|n| n["id"] == "h1").unwrap();
    assert_eq!(h1["kind"], "host");
    assert_eq!(h1["ip"], "10.0.0.1");
    let e = edges
        .iter()
        .find(|e| e["a"] == "h1" && e["b"] == "s1")
        .unwrap();
    assert_eq!(e["status"], "planned");
    assert_eq!(e["label"], "h1-eth0<->s1-eth1");
    assert_eq!(e["intf1"], "h1-eth0");
    assert_eq!(e["intf2"], "s1-eth1");
    assert_eq!(e["port1"], 0);
    assert_eq!(e["port2"], 1);
    // roundtrip
    let back = GraphView::from_json(&s).unwrap();
    assert_eq!(back, g);
}
