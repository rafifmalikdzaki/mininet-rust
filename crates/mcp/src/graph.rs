//! Graph JSON builders shared by the `topo_plan` / `graph` / `net_build`
//! tools.
//!
//! The shape (matching the `fixtures/topo_*.txt` characterization outputs):
//!
//! ```json
//! {
//!   "spec": "single,3", "class": "SingleSwitchTopo",
//!   "running": false, "built": true,
//!   "nodes": [{"name": "h1", "kind": "host", "ip": "10.0.0.1",
//!              "pid": null, "status": "down"}, ...],
//!   "edges": [{"a": "h1", "b": "s1", "intf1": "h1-eth0", "intf2": "s1-eth1",
//!              "port1": 0, "port2": 1, "status": ["MISSING", "MISSING"]}, ...]
//! }
//! ```

use mn_net::{Mininet, MininetOptions};
use serde_json::{json, Value};

/// Node kind tag for the graph JSON (`host`/`switch`/`controller`).
fn kind_of(net: &Mininet, name: &str) -> &'static str {
    if net.hosts.contains(name) {
        "host"
    } else if net.switches.contains(name) {
        "switch"
    } else if net.controllers.contains(name) {
        "controller"
    } else {
        "node"
    }
}

/// One node entry: name/kind/ip from the model, pid + status live.
fn node_json(net: &Mininet, name: &str) -> Value {
    let node = net.get(name).expect("keys() and nameToNode in sync").node();
    let pid = node.pid();
    let status = if pid.is_some() { "running" } else { "down" };
    json!({
        "name": name,
        "kind": kind_of(net, name),
        "ip": node.ip(None).map(|a| a.to_string()),
        "pid": pid,
        "status": status,
    })
}

/// The full graph JSON for a built [`Mininet`] model. `running` is the
/// session's started flag (live kernel state only exists after
/// [`Mininet::start`], which needs root).
pub fn net_graph(net: &Mininet, running: bool) -> Value {
    let nodes: Vec<Value> = net.keys().iter().map(|n| node_json(net, n)).collect();
    let edges: Vec<Value> = net
        .links
        .iter()
        .map(|l| {
            json!({
                "a": l.node1,
                "b": l.node2,
                "intf1": l.intf1.name,
                "intf2": l.intf2.name,
                "port1": l.intf1.port,
                "port2": l.intf2.port,
                "status": [l.intf1.status.to_string(), l.intf2.status.to_string()],
            })
        })
        .collect();
    json!({
        "spec": net.topo_spec.as_deref(),
        "class": net.topo_class.as_deref(),
        "running": running,
        "built": net.built,
        "nodes": nodes,
        "edges": edges,
    })
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

    #[test]
    fn plan_graph_single3_matches_fixture_structure() {
        let g = plan_graph("single,3").unwrap();
        assert_eq!(g["spec"], "single,3");
        assert_eq!(g["class"], "SingleSwitchTopo");
        assert_eq!(g["planned"], true);
        let nodes: Vec<(String, String, Value)> = g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["name"].as_str().unwrap().to_string(),
                    n["kind"].as_str().unwrap().to_string(),
                    n["ip"].clone(),
                )
            })
            .collect();
        assert_eq!(
            nodes,
            vec![
                ("h1".to_string(), "host".to_string(), json!("10.0.0.1")),
                ("h2".to_string(), "host".to_string(), json!("10.0.0.2")),
                ("h3".to_string(), "host".to_string(), json!("10.0.0.3")),
                ("s1".to_string(), "switch".to_string(), Value::Null),
            ]
        );
        let edges: Vec<(String, String, i64, i64)> = g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["a"].as_str().unwrap().to_string(),
                    e["b"].as_str().unwrap().to_string(),
                    e["port1"].as_i64().unwrap(),
                    e["port2"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            edges,
            vec![
                ("h1".to_string(), "s1".to_string(), 0, 1),
                ("h2".to_string(), "s1".to_string(), 0, 2),
                ("h3".to_string(), "s1".to_string(), 0, 3),
            ]
        );
    }

    #[test]
    fn plan_graph_bad_spec_is_err() {
        assert!(plan_graph("bogus,1")
            .unwrap_err()
            .contains("Invalid topo name"));
    }
}
