//! The graph model: [`GraphView`], a serializable snapshot of the emulated
//! network as nodes (boxes) and edges (links), built either from a dry-run
//! topology plan ([`GraphView::from_topo`]) or from a live `mn-net`
//! [`Mininet`] ([`GraphView::from_net`]).
//!
//! The JSON shape is the **canonical graph shape** — the same one the
//! `mn-tui --json` export emits and the MCP server (`crates/mcp`) reuses
//! for its `topo_plan`/`graph`/`net_build` payloads (see
//! `crates/mcp/src/graph.rs`). Enum values serialize lowercase, matching
//! the `fixtures/topo_*.txt` characterization outputs, and edges keep
//! the fixture-matching `intf1`/`intf2`/`port1`/`port2` fields next to
//! the `label`:
//!
//! ```json
//! { "spec": "single,3", "live": false,
//!   "nodes": [ { "id": "h1", "label": "h1", "kind": "host",
//!                "ip": "10.0.0.1", "status": "planned", "detail": { ... } } ],
//!   "edges": [ { "a": "h1", "b": "s1", "status": "planned",
//!                "label": "h1-eth0<->s1-eth1",
//!                "intf1": "h1-eth0", "intf2": "s1-eth1",
//!                "port1": 0, "port2": 1 } ] }
//! ```
//!
//! (`detail` is omitted entirely when `None`.)

use mn_link::LinkStatus;
use mn_net::{link_str, repr_node, Mininet, NodeRef};
use mn_topo::Topo;
use serde::{Deserialize, Serialize};

/// Role of a node in the graph (Python `isSwitch` / node class).
/// Serializes lowercase (`host`/`switch`/`controller`, matching the
/// fixtures and the MCP graph payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// A host (`h1`, ...).
    Host,
    /// A switch (`s1`, ...) — rendered with a yellow box.
    Switch,
    /// A controller (`c0`, ...) — rendered with a cyan box.
    Controller,
}

impl NodeKind {
    /// Lowercase name used in the box status line and JSON-adjacent text.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Host => "host",
            NodeKind::Switch => "switch",
            NodeKind::Controller => "controller",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Liveness of a node: `Up` when its process pid exists (live net), `Down`
/// when a live net has no pid for it, `Missing` is unused for nodes today
/// (kept for symmetry with edges), `Planned` in dry-run mode.
/// Serializes lowercase (`up`/`down`/`missing`/`planned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    /// Process running (pid known) — green.
    Up,
    /// Live net, but no pid — red.
    Down,
    /// Not present in the kernel — red.
    Missing,
    /// Dry-run plan, not started — green.
    Planned,
}

impl NodeStatus {
    /// Lowercase name for the box status line.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeStatus::Up => "up",
            NodeStatus::Down => "down",
            NodeStatus::Missing => "missing",
            NodeStatus::Planned => "planned",
        }
    }
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Liveness of an edge (a link's two intf ends).
/// Serializes lowercase (`up`/`down`/`missing`/`planned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeStatus {
    /// Both ends up — green.
    Up,
    /// Ends exist but down (or mixed) — red.
    Down,
    /// An end is missing — red.
    Missing,
    /// Dry-run plan, link not attached — green.
    Planned,
}

impl EdgeStatus {
    /// Lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeStatus::Up => "up",
            EdgeStatus::Down => "down",
            EdgeStatus::Missing => "missing",
            EdgeStatus::Planned => "planned",
        }
    }
}

impl std::fmt::Display for EdgeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One interface of a node, for the detail pane (`Intf.name/ip/status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntfInfo {
    /// Interface name (e.g. `h1-eth0`).
    pub name: String,
    /// Assigned IP, if any.
    pub ip: Option<String>,
    /// Kernel state string (`UP`/`DOWN`/`MISSING`).
    pub status: String,
}

/// Extra per-node data shown in the detail pane (Enter in the TUI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    /// Python-style `repr(node)` (`<Host h1: h1-eth0:10.0.0.1 pid=None> `).
    pub repr: String,
    /// Shell/holder pid when running.
    pub pid: Option<u32>,
    /// Interfaces in port order (switches include the `lo` control intf).
    pub intfs: Vec<IntfInfo>,
    /// Distinct assigned IPs, in interface order.
    pub ips: Vec<String>,
}

/// One node box of the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GNode {
    /// Node name (`h1`, `s1`, `c0`).
    pub id: String,
    /// Label drawn in the box (same as `id` today).
    pub label: String,
    /// Host / switch / controller.
    pub kind: NodeKind,
    /// Primary IP (default intf for hosts, control IP for controllers).
    pub ip: Option<String>,
    /// Liveness.
    pub status: NodeStatus,
    /// Detail-pane payload (present when built from a `Mininet`).
    /// Omitted from the JSON entirely when `None` (a plain `default`
    /// alone would still serialize `null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<NodeDetail>,
}

/// One edge of the graph (a mininet link between nodes `a` and `b`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GEdge {
    /// First endpoint node id.
    pub a: String,
    /// Second endpoint node id.
    pub b: String,
    /// Liveness of the link.
    pub status: EdgeStatus,
    /// `intf1<->intf2` label (e.g. `h1-eth0<->s1-eth1`).
    pub label: String,
    /// First endpoint's interface name (`h1-eth0`) — the fixture-matching
    /// field of the topo `links(withInfo=True)` / MCP graph output.
    /// Present when built from a topo/net with port info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intf1: Option<String>,
    /// Second endpoint's interface name (`s1-eth1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intf2: Option<String>,
    /// First endpoint's port number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port1: Option<u32>,
    /// Second endpoint's port number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port2: Option<u32>,
}

/// The whole graph view: a snapshot of the emulated network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphView {
    /// The topo spec the graph was built from (`single,3`), if any.
    #[serde(default)]
    pub spec: Option<String>,
    /// Whether this snapshot came from a started (live) net.
    pub live: bool,
    /// Nodes, in stable (net insertion) order.
    pub nodes: Vec<GNode>,
    /// Edges (links), in net order.
    pub edges: Vec<GEdge>,
}

impl GraphView {
    /// Build the graph from a topology plan (`build_topo(spec)` result)
    /// without any `Mininet`: every node/edge is `Planned`, IPs come from
    /// the topo's `ip` node attr when present, and edges carry the
    /// fixture-matching `intf1`/`intf2`/`port1`/`port2` from the link info
    /// (the same fields the MCP server's graph payload exposes).
    pub fn from_topo(topo: &Topo, spec: Option<&str>) -> GraphView {
        let mut nodes = Vec::new();
        for name in topo.nodes(true) {
            let kind = if topo.is_switch(&name) {
                NodeKind::Switch
            } else {
                NodeKind::Host
            };
            let ip = topo
                .node_info(&name)
                .and_then(|info| info.get("ip"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            nodes.push(GNode {
                id: name.clone(),
                label: name,
                kind,
                ip,
                status: NodeStatus::Planned,
                detail: None,
            });
        }
        let mut edges = Vec::new();
        for link in topo.links(true, false, true) {
            let (label, intf1, intf2, port1, port2) = match link.info {
                Some(info) => {
                    let p1 = info.get("port1").and_then(|v| v.as_int()).unwrap_or(0);
                    let p2 = info.get("port2").and_then(|v| v.as_int()).unwrap_or(0);
                    // same intf-name derivation as mn-net's link_str
                    (
                        link_str(link.node1, link.node2, info),
                        Some(format!("{}-eth{p1}", link.node1)),
                        Some(format!("{}-eth{p2}", link.node2)),
                        u32::try_from(p1).ok(),
                        u32::try_from(p2).ok(),
                    )
                }
                None => (
                    format!("{}<->{}", link.node1, link.node2),
                    None,
                    None,
                    None,
                    None,
                ),
            };
            edges.push(GEdge {
                a: link.node1.to_string(),
                b: link.node2.to_string(),
                status: EdgeStatus::Planned,
                label,
                intf1,
                intf2,
                port1,
                port2,
            });
        }
        GraphView {
            spec: spec.map(str::to_string),
            live: false,
            nodes,
            edges,
        }
    }

    /// Build the graph from a `mn-net` [`Mininet`] — a dry-run plan
    /// (`live=false`: nodes without pids are `Planned`, unattached links
    /// `Planned`, host IPs come from the net's address assignment) or a
    /// started net (`live=true`: node status from pids, link status from
    /// the link/intf model — the same data `net.links()` prints).
    pub fn from_net(net: &Mininet, live: bool) -> GraphView {
        let mut nodes = Vec::new();
        for name in net.keys() {
            let nref = net.get(&name).expect("keys in sync");
            let node = nref.node();
            let kind = match nref {
                NodeRef::Host(_) => NodeKind::Host,
                NodeRef::Switch(_) => NodeKind::Switch,
                NodeRef::Controller(_) => NodeKind::Controller,
            };
            let pid = node.pid();
            let status = if pid.is_some() {
                NodeStatus::Up
            } else if live {
                NodeStatus::Down
            } else {
                NodeStatus::Planned
            };
            let ip = match nref {
                NodeRef::Controller(c) => Some(c.ip().to_string()),
                _ => node.ip(None).map(|a| a.to_string()),
            };
            let mut intfs = Vec::new();
            if let NodeRef::Switch(s) = nref {
                if let Some(lo) = &s.control_intf {
                    intfs.push(IntfInfo {
                        name: lo.name.clone(),
                        ip: lo.ip.map(|a| a.to_string()),
                        status: lo.status.to_string(),
                    });
                }
            }
            for i in node.intf_list() {
                intfs.push(IntfInfo {
                    name: i.name.clone(),
                    ip: i.ip.map(|a| a.to_string()),
                    status: i.status.to_string(),
                });
            }
            let mut ips: Vec<String> = Vec::new();
            for i in &intfs {
                if let Some(ip) = &i.ip {
                    if !ips.contains(ip) {
                        ips.push(ip.clone());
                    }
                }
            }
            let detail = NodeDetail {
                repr: repr_node(nref),
                pid,
                intfs,
                ips,
            };
            nodes.push(GNode {
                id: name.clone(),
                label: name,
                kind,
                ip,
                status,
                detail: Some(detail),
            });
        }
        let mut edges = Vec::new();
        for link in &net.links {
            let status = if !link.is_attached() {
                if live {
                    EdgeStatus::Missing
                } else {
                    EdgeStatus::Planned
                }
            } else {
                match link.status() {
                    LinkStatus::Up => EdgeStatus::Up,
                    LinkStatus::Down => EdgeStatus::Down,
                    LinkStatus::Missing => EdgeStatus::Missing,
                    LinkStatus::Partial => EdgeStatus::Down,
                }
            };
            edges.push(GEdge {
                a: link.node1.clone(),
                b: link.node2.clone(),
                status,
                label: format!("{}<->{}", link.intf1.name, link.intf2.name),
                intf1: Some(link.intf1.name.clone()),
                intf2: Some(link.intf2.name.clone()),
                port1: link.intf1.port,
                port2: link.intf2.port,
            });
        }
        GraphView {
            spec: net.topo_spec.clone(),
            live,
            nodes,
            edges,
        }
    }

    /// Look up a node by id.
    pub fn node(&self, id: &str) -> Option<&GNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Look up an edge by endpoint ids (either direction).
    pub fn edge(&self, a: &str, b: &str) -> Option<&GEdge> {
        self.edges.iter().find(|e| {
            (e.a.as_str(), e.b.as_str()) == (a, b) || (e.a.as_str(), e.b.as_str()) == (b, a)
        })
    }

    /// Serialize as pretty JSON (the `--json` payload).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("GraphView is plain JSON data")
    }

    /// Parse a JSON payload produced by [`GraphView::to_json`].
    pub fn from_json(s: &str) -> Result<GraphView, serde_json::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mn_net::MininetOptions;

    fn dry_opts() -> MininetOptions {
        MininetOptions {
            controller: None,
            ..Default::default()
        }
    }

    #[test]
    fn from_topo_single3_shape() {
        let built = mn_topo::build_topo("single,3").unwrap();
        let g = GraphView::from_topo(built.topo(), Some("single,3"));
        assert_eq!(g.spec.as_deref(), Some("single,3"));
        assert!(!g.live);
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["h1", "h2", "h3", "s1"]);
        assert_eq!(g.nodes[3].kind, NodeKind::Switch);
        assert!(g.nodes[..3].iter().all(|n| n.kind == NodeKind::Host));
        assert!(g.nodes.iter().all(|n| n.status == NodeStatus::Planned));
        assert_eq!(g.edges.len(), 3);
        let e = g.edge("h1", "s1").unwrap();
        assert_eq!(e.label, "h1-eth0<->s1-eth1");
        // fixture-matching intf/port fields are populated from the link info
        assert_eq!(e.intf1.as_deref(), Some("h1-eth0"));
        assert_eq!(e.intf2.as_deref(), Some("s1-eth1"));
        assert_eq!(e.port1, Some(0));
        assert_eq!(e.port2, Some(1));
        assert_eq!(g.edge("s1", "h3").unwrap().a, "h3"); // undirected lookup
    }

    #[test]
    fn from_net_dry_run_has_ips_detail_and_planned_status() {
        let net = Mininet::dry_run("single,3", dry_opts()).unwrap();
        let g = GraphView::from_net(&net, false);
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.node("h1").unwrap().ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(g.node("h3").unwrap().ip.as_deref(), Some("10.0.0.3"));
        let d = g.node("h1").unwrap().detail.as_ref().unwrap();
        assert!(d.repr.contains("<Host h1: h1-eth0:10.0.0.1 pid=None>"));
        assert_eq!(d.pid, None);
        assert_eq!(d.intfs.len(), 1);
        assert_eq!(d.intfs[0].name, "h1-eth0");
        assert_eq!(d.ips, vec!["10.0.0.1".to_string()]);
        assert_eq!(g.edges.len(), 3);
        assert!(g.edges.iter().all(|e| e.status == EdgeStatus::Planned));
        assert_eq!(g.edges[0].label, "h1-eth0<->s1-eth1");
        assert_eq!(g.edges[0].intf1.as_deref(), Some("h1-eth0"));
        assert_eq!(g.edges[0].port2, Some(1));
    }

    #[test]
    fn json_roundtrip_preserves_graph() {
        let net = Mininet::dry_run("single,3", dry_opts()).unwrap();
        let g = GraphView::from_net(&net, false);
        let s = g.to_json();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["nodes"].as_array().unwrap().len(), 4);
        assert_eq!(v["edges"].as_array().unwrap().len(), 3);
        assert_eq!(v["spec"], "single,3");
        // canonical shape: lowercase enum casing (matches the fixtures)
        assert_eq!(v["nodes"][3]["kind"], "switch");
        assert_eq!(v["nodes"][0]["kind"], "host");
        assert_eq!(v["nodes"][0]["status"], "planned");
        // fixture-matching edge fields ride along with the label
        assert_eq!(v["edges"][0]["label"], "h1-eth0<->s1-eth1");
        assert_eq!(v["edges"][0]["intf1"], "h1-eth0");
        assert_eq!(v["edges"][0]["intf2"], "s1-eth1");
        assert_eq!(v["edges"][0]["port1"], 0);
        assert_eq!(v["edges"][0]["port2"], 1);
        // from_net always fills detail, so it is present here
        assert!(v["nodes"][0].get("detail").is_some());
        let back = GraphView::from_json(&s).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn detail_is_omitted_from_json_when_none() {
        let built = mn_topo::build_topo("single,3").unwrap();
        let g = GraphView::from_topo(built.topo(), Some("single,3"));
        let s = g.to_json();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        // #[serde(default)] alone would serialize "detail": null; the
        // skip_serializing_if keeps the key out entirely ...
        for n in v["nodes"].as_array().unwrap() {
            assert!(n.get("detail").is_none(), "node: {n}");
        }
        // ... and the JSON still round-trips (default fills None back in)
        let back = GraphView::from_json(&s).unwrap();
        assert_eq!(back, g);
        assert!(back.nodes.iter().all(|n| n.detail.is_none()));
        // from_topo edges still carry the fixture-matching intf/port fields
        assert_eq!(v["edges"][0]["intf1"], "h1-eth0");
        assert_eq!(v["edges"][0]["port2"], 1);
    }
}
