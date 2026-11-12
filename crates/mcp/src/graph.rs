//! Graph JSON builders shared by the `topo_plan` / `graph` / `net_build`
//! tools.
//!
//! The canonical graph shape is [`mn_tui::GraphView`] — the same type
//! `mn-tui --json` emits (lowercase `kind`/`status` values matching the
//! `fixtures/topo_*.txt` characterization outputs, edges carrying both
//! the `label` and the fixture-matching `intf1`/`intf2`/`port1`/`port2`
//! fields):
//!
//! ```json
//! {
//!   "spec": "single,3", "class": "SingleSwitchTopo",
//!   "running": false, "built": true, "live": false,
//!   "nodes": [{"id": "h1", "label": "h1", "kind": "host", "ip": "10.0.0.1",
//!              "status": "planned", "detail": {"repr": "...", "pid": null,
//!              "intfs": [...], "ips": [...]}}, ...],
//!   "edges": [{"a": "h1", "b": "s1", "status": "planned",
//!              "label": "h1-eth0<->s1-eth1",
//!              "intf1": "h1-eth0", "intf2": "s1-eth1",
//!              "port1": 0, "port2": 1}, ...]
//! }
//! ```
//!
//! On top of the [`GraphView`] fields, the session-level `class` /
//! `running` / `built` (and `planned` for `topo_plan`) are added.

use mn_net::{Mininet, MininetOptions};
use mn_tui::GraphView;
use serde_json::Value;

/// The full graph JSON for a built [`Mininet`] model: the canonical
/// [`GraphView`] snapshot plus the session-level `class`/`running`/
/// `built` fields. `running` is the session's started flag and doubles
/// as `live` (live kernel state only exists after [`Mininet::start`],
/// which needs root).
pub fn net_graph(net: &Mininet, running: bool) -> Value {
    let graph = GraphView::from_net(net, running);
    let mut v = serde_json::to_value(&graph).expect("GraphView is plain JSON data");
    v["class"] = match &net.topo_class {
        Some(c) => Value::String(c.clone()),
        None => Value::Null,
    };
    v["running"] = Value::Bool(running);
    v["built"] = Value::Bool(net.built);
    v
}

/// `topo_plan`: dry-run graph JSON for a built-in topo spec (`single,3`,
/// ...), built **without touching the kernel** and without the default
/// controller, so the graph is exactly the topo's nodes/edges (matching
/// `fixtures/topo_single3.txt`). Non-root safe.
pub fn plan_graph(spec: &str) -> Result<Value, String> {
    let opts = MininetOptions {
        controller: None,
        ..Default::default()
    };
    let net = Mininet::dry_run(spec, opts).map_err(|e| e.to_string())?;
    let mut g = net_graph(&net, false);
    g["planned"] = Value::Bool(true);
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_graph_single3_matches_fixture_structure() {
        let g = plan_graph("single,3").unwrap();
        assert_eq!(g["spec"], "single,3");
        assert_eq!(g["class"], "SingleSwitchTopo");
        assert_eq!(g["planned"], true);
        assert_eq!(g["running"], false);
        assert_eq!(g["built"], true);
        assert_eq!(g["live"], false);
        // canonical GraphView shape: nodes keyed by "id", lowercase kinds
        let nodes: Vec<(String, String, String, Value)> = g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["id"].as_str().unwrap().to_string(),
                    n["kind"].as_str().unwrap().to_string(),
                    n["status"].as_str().unwrap().to_string(),
                    n["ip"].clone(),
                )
            })
            .collect();
        assert_eq!(
            nodes,
            vec![
                (
                    "h1".to_string(),
                    "host".to_string(),
                    "planned".to_string(),
                    json!("10.0.0.1")
                ),
                (
                    "h2".to_string(),
                    "host".to_string(),
                    "planned".to_string(),
                    json!("10.0.0.2")
                ),
                (
                    "h3".to_string(),
                    "host".to_string(),
                    "planned".to_string(),
                    json!("10.0.0.3")
                ),
                (
                    "s1".to_string(),
                    "switch".to_string(),
                    "planned".to_string(),
                    Value::Null
                ),
            ]
        );
        // edges: a/b + fixture-matching intf/port fields + label + status
        let edges: Vec<(String, String, String, String, i64, i64)> = g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["a"].as_str().unwrap().to_string(),
                    e["b"].as_str().unwrap().to_string(),
                    e["intf1"].as_str().unwrap().to_string(),
                    e["intf2"].as_str().unwrap().to_string(),
                    e["port1"].as_i64().unwrap(),
                    e["port2"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            edges,
            vec![
                (
                    "h1".to_string(),
                    "s1".to_string(),
                    "h1-eth0".to_string(),
                    "s1-eth1".to_string(),
                    0,
                    1
                ),
                (
                    "h2".to_string(),
                    "s1".to_string(),
                    "h2-eth0".to_string(),
                    "s1-eth2".to_string(),
                    0,
                    2
                ),
                (
                    "h3".to_string(),
                    "s1".to_string(),
                    "h3-eth0".to_string(),
                    "s1-eth3".to_string(),
                    0,
                    3
                ),
            ]
        );
        let e0 = &g["edges"][0];
        assert_eq!(e0["label"], "h1-eth0<->s1-eth1");
        assert_eq!(e0["status"], "planned");
    }

    #[test]
    fn plan_graph_bad_spec_is_err() {
        assert!(plan_graph("bogus,1")
            .unwrap_err()
            .contains("Invalid topo name"));
    }
}
