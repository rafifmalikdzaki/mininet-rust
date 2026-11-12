//! `mn-tui` — the terminal-free core of the OMO-style live graph TUI
//! (task 13). OMO (Oh My OpenAgent, omo.dev) renders every workflow run as a
//! real dependency graph in its UI; `mn-tui` does the same for the emulated
//! network: nodes as boxes, links as edges, live status colors.
//!
//! This crate deliberately has **no terminal dependencies**: all rendering
//! goes through the generic [`render::CellBuffer`] trait, so the exact same
//! draw code runs on a ratatui buffer in `apps/mn-tui` and on a
//! [`render::StringGrid`] in headless tests / `--frames N` CI logs.
//!
//! # Layout
//!
//! - [`model`]: [`GraphView`] / [`GNode`] / [`GEdge`] — the serializable
//!   graph snapshot built from a dry-run plan ([`GraphView::from_topo`] /
//!   [`GraphView::from_net`] with `live=false`) or from a live `mn-net`
//!   [`Mininet`] (`live=true`: link status from the link/intf model, node
//!   status from pids). `GraphView` serializes to JSON (`--json`, and the
//!   `topo_plan`/`graph` payloads of the MCP server later).
//! - [`layout`]: deterministic layered layout (multi-source BFS from
//!   switches/controllers, hosts around them, natural-sorted within a
//!   layer) plus the pan/zoom [`View`] transform and hit-testing math.
//! - [`render`]: [`CellBuffer`], [`StringGrid`], and the OMO-style frame
//!   renderer (title bar, node boxes, unicode edges, detail pane, status
//!   line, footer).
//!
//! # Status colors (OMO-style)
//!
//! - nodes: green = up/ok, red = missing/down, yellow = switch,
//!   cyan = controller, white = selected.
//! - edges: green = up/planned, red = down/missing.

pub mod layout;
pub mod model;
pub mod render;

pub use layout::{
    hit_test, node_rect, Layout, View, BOX_HEIGHT, CHAR_ASPECT, H_GAP, MAX_ZOOM, MIN_ZOOM, V_GAP,
    ZOOM_STEP,
};
pub use model::{EdgeStatus, GEdge, GNode, GraphView, IntfInfo, NodeDetail, NodeKind, NodeStatus};
pub use render::{
    detail_lines, render_detail, render_frame, render_graph, status_color, Cell, CellBuffer, Color,
    Frame, StringGrid, Style,
};
