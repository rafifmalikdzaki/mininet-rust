//! Deterministic layered layout + pan/zoom view math + hit-testing.
//!
//! The layout is a multi-source BFS layering: switches and controllers are
//! the roots (layer 0), their neighbors land on layer 1, and so on. Within
//! a layer, nodes are natural-sorted (`h2` before `h10`) and centered
//! around x=0, so the placement is a pure function of the [`GraphView`] —
//! same graph in, same coordinates out, every time (asserted by tests).
//!
//! Coordinates are "world" units: 1 x-unit renders as 1 terminal column,
//! 1 y-unit as `1 / CHAR_ASPECT` rows (terminal cells are ~twice as tall
//! as wide, so `CHAR_ASPECT = 0.5` makes the graph look square-ish).

use std::collections::{HashMap, VecDeque};

use mn_util::natural;

use crate::model::{GNode, GraphView, NodeKind};

/// Horizontal gap between node centers, in world x-units (columns at zoom 1).
pub const H_GAP: f64 = 24.0;
/// Vertical gap between layers, in world y-units.
pub const V_GAP: f64 = 12.0;
/// Terminal cell aspect ratio (height/width ≈ 0.5): world y is scaled by
/// this when mapping to screen rows.
pub const CHAR_ASPECT: f64 = 0.5;
/// Height of a node box in screen rows (top border, 3 content rows, bottom).
pub const BOX_HEIGHT: usize = 5;
/// Minimum zoom factor.
pub const MIN_ZOOM: f64 = 0.25;
/// Maximum zoom factor.
pub const MAX_ZOOM: f64 = 4.0;
/// Zoom step for +/- (multiplicative).
pub const ZOOM_STEP: f64 = 1.25;

/// Computed node positions in world coordinates, parallel to
/// `GraphView::nodes`.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Node ids, same order as `graph.nodes`.
    pub ids: Vec<String>,
    /// World `(x, y)` per node, parallel to `ids`.
    pub pos: Vec<(f64, f64)>,
}

impl Layout {
    /// Compute the layered layout for `graph` (deterministic).
    pub fn compute(graph: &GraphView) -> Layout {
        let n = graph.nodes.len();
        let ids: Vec<String> = graph.nodes.iter().map(|node| node.id.clone()).collect();
        let index: HashMap<&str, usize> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &graph.edges {
            if let (Some(&a), Some(&b)) = (index.get(e.a.as_str()), index.get(e.b.as_str())) {
                if a != b {
                    adj[a].push(b);
                    adj[b].push(a);
                }
            }
        }
        // Roots: switches + controllers, natural-sorted for determinism.
        let mut roots: Vec<usize> = (0..n)
            .filter(|&i| graph.nodes[i].kind != NodeKind::Host)
            .collect();
        roots.sort_by_key(|&i| natural(&ids[i]));
        if roots.is_empty() && n > 0 {
            // No switch/controller (e.g. `minimal`): the first node is the root.
            roots.push(0);
        }
        // Multi-source BFS; neighbors visited in natural order.
        let mut layer = vec![usize::MAX; n];
        let mut queue: VecDeque<usize> = VecDeque::new();
        for &r in &roots {
            layer[r] = 0;
            queue.push_back(r);
        }
        while let Some(u) = queue.pop_front() {
            let mut nbrs = adj[u].clone();
            nbrs.sort_by_key(|&v| natural(&ids[v]));
            for v in nbrs {
                if layer[v] == usize::MAX {
                    layer[v] = layer[u] + 1;
                    queue.push_back(v);
                }
            }
        }
        // Unreachable nodes (isolated) get their own layer below the rest.
        let max_layer = layer.iter().copied().filter(|&l| l != usize::MAX).max();
        let next = max_layer.map_or(0, |m| m + 1);
        for l in layer.iter_mut() {
            if *l == usize::MAX {
                *l = next;
            }
        }
        // Group by layer; center each layer around x=0, natural-sorted.
        let mut by_layer: Vec<Vec<usize>> = vec![Vec::new(); next + 1];
        for (i, &l) in layer.iter().enumerate() {
            by_layer[l].push(i);
        }
        let mut pos = vec![(0.0, 0.0); n];
        for (l, members) in by_layer.iter().enumerate() {
            if members.is_empty() {
                continue;
            }
            let mut members = members.clone();
            members.sort_by_key(|&i| natural(&ids[i]));
            let count = members.len();
            for (k, &i) in members.iter().enumerate() {
                let x = (k as f64 - (count - 1) as f64 / 2.0) * H_GAP;
                pos[i] = (x, l as f64 * V_GAP);
            }
        }
        Layout { ids, pos }
    }

    /// World position of node `id`, if present.
    pub fn get(&self, id: &str) -> Option<(f64, f64)> {
        self.ids.iter().position(|i| i == id).map(|i| self.pos[i])
    }
}

/// Pan/zoom viewport over the world. `width`/`height` are the size of the
/// *frame* the view is drawn into (render_frame carves out title/status/
/// footer rows internally).
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// World point at the center of the frame.
    pub pan_x: f64,
    pub pan_y: f64,
    /// Zoom factor (1.0 = 1 world x-unit per column).
    pub zoom: f64,
    /// Frame width in cells.
    pub width: usize,
    /// Frame height in rows.
    pub height: usize,
}

impl View {
    /// A default view centered on the world origin at zoom 1.
    pub fn new(width: usize, height: usize) -> Self {
        View {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
            width,
            height,
        }
    }

    /// Resize the frame (keeps pan/zoom).
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    /// Center the view on a world point.
    pub fn center_on(&mut self, wx: f64, wy: f64) {
        self.pan_x = wx;
        self.pan_y = wy;
    }

    /// World → screen (cell) coordinates.
    pub fn to_screen(&self, wx: f64, wy: f64) -> (f64, f64) {
        (
            (wx - self.pan_x) * self.zoom + self.width as f64 / 2.0,
            (wy - self.pan_y) * self.zoom * CHAR_ASPECT + self.height as f64 / 2.0,
        )
    }

    /// Screen (cell) → world coordinates (inverse of [`View::to_screen`]).
    pub fn to_world(&self, sx: f64, sy: f64) -> (f64, f64) {
        (
            (sx - self.width as f64 / 2.0) / self.zoom + self.pan_x,
            (sy - self.height as f64 / 2.0) / (self.zoom * CHAR_ASPECT) + self.pan_y,
        )
    }

    /// Pan by `dx`/`dy` *screen cells* (content moves with the drag).
    pub fn pan_cells(&mut self, dx: f64, dy: f64) {
        self.pan_x -= dx / self.zoom;
        self.pan_y -= dy / (self.zoom * CHAR_ASPECT);
    }

    /// Zoom in one step (clamped to [`MAX_ZOOM`]).
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * ZOOM_STEP).min(MAX_ZOOM);
    }

    /// Zoom out one step (clamped to [`MIN_ZOOM`]).
    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / ZOOM_STEP).max(MIN_ZOOM);
    }
}

/// Width (in cells) of a node's box: widest content line plus 1 cell of
/// padding on each side (the border occupies the outermost columns).
pub fn node_box_width(node: &GNode) -> usize {
    let status_line = format!("{} {}", node.kind, node.status);
    let ip = node.ip.as_deref().unwrap_or("-");
    node.label.len().max(ip.len()).max(status_line.len()) + 2
}

/// Screen-space rectangle `(x, y, w, h)` of a node's box, relative to the
/// graph area (the caller adds the area offset). `x`/`y` may be negative
/// when the box is partially off-screen.
pub fn node_rect(
    graph: &GraphView,
    layout: &Layout,
    view: &View,
    id: &str,
) -> Option<(i64, i64, usize, usize)> {
    let node = graph.node(id)?;
    let (wx, wy) = layout.get(id)?;
    let (sx, sy) = view.to_screen(wx, wy);
    let w = node_box_width(node);
    let h = BOX_HEIGHT;
    Some((
        sx.round() as i64 - (w as i64 / 2),
        sy.round() as i64 - (h as i64 / 2),
        w,
        h,
    ))
}

/// Hit-test a screen cell (graph-area coordinates): returns the topmost
/// node whose box contains the cell (nodes are drawn in graph order, so
/// the last match wins), or `None`.
pub fn hit_test(
    graph: &GraphView,
    layout: &Layout,
    view: &View,
    x: usize,
    y: usize,
) -> Option<String> {
    let (x, y) = (x as i64, y as i64);
    for node in graph.nodes.iter().rev() {
        if let Some((rx, ry, w, h)) = node_rect(graph, layout, view, &node.id) {
            if x >= rx && x < rx + w as i64 && y >= ry && y < ry + h as i64 {
                return Some(node.id.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeStatus, GEdge, GNode, NodeStatus};

    fn node(id: &str, kind: NodeKind) -> GNode {
        GNode {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            ip: None,
            status: NodeStatus::Planned,
            detail: None,
        }
    }

    fn edge(a: &str, b: &str) -> GEdge {
        GEdge {
            a: a.to_string(),
            b: b.to_string(),
            status: EdgeStatus::Planned,
            label: format!("{a}<->{b}"),
            intf1: None,
            intf2: None,
            port1: None,
            port2: None,
        }
    }

    fn single3() -> GraphView {
        let nodes = vec![
            node("h1", NodeKind::Host),
            node("h2", NodeKind::Host),
            node("h3", NodeKind::Host),
            node("s1", NodeKind::Switch),
        ];
        let edges = vec![edge("h1", "s1"), edge("h2", "s1"), edge("h3", "s1")];
        GraphView {
            spec: Some("single,3".to_string()),
            live: false,
            nodes,
            edges,
        }
    }

    #[test]
    fn layout_single3_is_layered_and_centered() {
        let g = single3();
        let l = Layout::compute(&g);
        // s1 is the only root: layer 0, centered at origin.
        assert_eq!(l.get("s1"), Some((0.0, 0.0)));
        // hosts on layer 1, natural-sorted and centered: h1, h2, h3.
        assert_eq!(l.get("h1"), Some((-H_GAP, V_GAP)));
        assert_eq!(l.get("h2"), Some((0.0, V_GAP)));
        assert_eq!(l.get("h3"), Some((H_GAP, V_GAP)));
    }

    #[test]
    fn layout_is_deterministic() {
        let g = single3();
        assert_eq!(Layout::compute(&g), Layout::compute(&g));
        // also for a bigger, asymmetric topo: linear,4 (h1-s1-h2-s2-h3-s3-h4)
        let built = mn_topo::build_topo("linear,4").unwrap();
        let g2 = GraphView::from_topo(built.topo(), Some("linear,4"));
        assert_eq!(Layout::compute(&g2), Layout::compute(&g2));
        // switches are the roots (layer 0), hosts spread over layers 1..
        let l2 = Layout::compute(&g2);
        assert_eq!(l2.get("s1").unwrap().1, 0.0);
        assert_eq!(l2.get("s3").unwrap().1, 0.0);
        assert!(l2.get("h2").unwrap().1 > 0.0);
    }

    #[test]
    fn view_screen_world_roundtrip() {
        let mut v = View::new(80, 24);
        v.pan_cells(4.0, -2.0);
        v.zoom_in();
        let (sx, sy) = v.to_screen(3.0, 7.0);
        let (wx, wy) = v.to_world(sx, sy);
        assert!((wx - 3.0).abs() < 1e-9);
        assert!((wy - 7.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_clamps() {
        let mut v = View::new(80, 24);
        for _ in 0..100 {
            v.zoom_in();
        }
        assert_eq!(v.zoom, MAX_ZOOM);
        for _ in 0..200 {
            v.zoom_out();
        }
        assert_eq!(v.zoom, MIN_ZOOM);
    }

    #[test]
    fn hit_test_finds_box_and_misses_empty() {
        let g = single3();
        let l = Layout::compute(&g);
        let v = View::new(60, 18);
        // s1 box center hits s1.
        let (x, y, w, h) = node_rect(&g, &l, &v, "s1").unwrap();
        let cx = (x + w as i64 / 2) as usize;
        let cy = (y + h as i64 / 2) as usize;
        assert_eq!(hit_test(&g, &l, &v, cx, cy).as_deref(), Some("s1"));
        // top-left corner is empty.
        assert_eq!(hit_test(&g, &l, &v, 0, 0), None);
        // after panning, the hit test follows the box.
        let mut v2 = v.clone();
        v2.pan_cells(6.0, 3.0);
        let (x, y, w, h) = node_rect(&g, &l, &v2, "s1").unwrap();
        let cx = (x + w as i64 / 2) as usize;
        let cy = (y + h as i64 / 2) as usize;
        assert_eq!(hit_test(&g, &l, &v2, cx, cy).as_deref(), Some("s1"));
        // after zooming, same.
        let mut v3 = v.clone();
        v3.zoom_in();
        v3.zoom_in();
        let (x, y, w, h) = node_rect(&g, &l, &v3, "h2").unwrap();
        let cx = (x + w as i64 / 2) as usize;
        let cy = (y + h as i64 / 2) as usize;
        assert_eq!(hit_test(&g, &l, &v3, cx, cy).as_deref(), Some("h2"));
    }

    #[test]
    fn isolated_nodes_get_own_layer() {
        let g = GraphView {
            spec: None,
            live: false,
            nodes: vec![
                node("h1", NodeKind::Host),
                node("h2", NodeKind::Host),
                node("h3", NodeKind::Host),
            ],
            edges: vec![edge("h1", "h2")],
        };
        let l = Layout::compute(&g);
        // no switch/controller: h1 is the root (layer 0), h2 layer 1, h3 isolated -> layer 2.
        assert_eq!(l.get("h1").unwrap().1, 0.0);
        assert_eq!(l.get("h2").unwrap().1, V_GAP);
        assert_eq!(l.get("h3").unwrap().1, 2.0 * V_GAP);
    }
}
