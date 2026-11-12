//! Headless render-to-string golden tests: the exact same `render_frame`
//! code that drives the ratatui UI is rendered into a `StringGrid` and
//! compared against a golden, plus hit-test and color assertions.

use mn_net::{Mininet, MininetOptions};
use mnr_tui::{
    hit_test, node_rect, render_frame, Color, Frame, GraphView, Layout, StringGrid, View,
};

fn dry_graph() -> GraphView {
    let opts = MininetOptions {
        controller: None,
        ..Default::default()
    };
    let net = Mininet::dry_run("single,3", opts).unwrap();
    GraphView::from_net(&net, false)
}

const TITLE: &str = "mn-tui — topo single,3 (dry-run plan)";
const STATUS: &str = "frame 1/1 (headless)";
const FOOTER: &str =
    "q quit | arrows/hjkl pan | +/- zoom | tab cycle | enter detail | r refresh | p pingall";

#[test]
fn render_golden_single3_70x24_selected_detail() {
    let graph = dry_graph();
    let layout = Layout::compute(&graph);
    let view = View::new(70, 24);
    let mut grid = StringGrid::new(70, 24);
    render_frame(
        &mut grid,
        &Frame {
            graph: &graph,
            layout: &layout,
            view: &view,
            selected: Some("h1"),
            show_detail: true,
            title: TITLE,
            status: STATUS,
            footer: FOOTER,
        },
    );
    // MN_TUI_DUMP_GOLDEN=1 cargo test -p mn-tui --test render_golden -- --nocapture
    // prints the raw grid for golden regeneration into tests/golden/.
    if std::env::var("MN_TUI_DUMP_GOLDEN").is_ok() {
        println!("{grid}");
    }
    let expected = include_str!("golden/single3_70x24.txt").trim_end_matches('\n');
    assert_eq!(grid.to_string(), expected);

    // --- color assertions (OMO-style status colors) ---
    // graph area is rows 1..=21 (h-3 = 21 rows), view resized internally
    let gview = {
        let mut v = view.clone();
        v.resize(70, 21);
        v
    };
    // s1 (switch) border is yellow; h2 (unselected host) border is green;
    // h1 (selected) border is white + bold.
    let (x, y, w, h) = node_rect(&graph, &layout, &gview, "s1").unwrap();
    assert_eq!(grid.color_at(x as usize, (y + 1) as usize), Color::Yellow);
    let (x, y, _w, _h) = node_rect(&graph, &layout, &gview, "h2").unwrap();
    assert_eq!(grid.color_at(x as usize, (y + 1) as usize), Color::Green);
    let (x, y, _w, _h) = node_rect(&graph, &layout, &gview, "h1").unwrap();
    let c = grid.cell_at(x as usize, (y + 1) as usize);
    assert_eq!(c.style.fg, Color::White);
    assert!(c.style.bold);
    let _ = (w, h);
    // edges are drawn in green (planned): scan the graph area left of the
    // detail pane for green line glyphs
    let mut green_edge_cells = 0;
    for y in 1..22 {
        for x in 0..35 {
            let c = grid.cell_at(x, y);
            if c.style.fg == Color::Green && "─│/\\•".contains(c.ch) {
                green_edge_cells += 1;
            }
        }
    }
    assert!(
        green_edge_cells >= 3,
        "expected >=3 green edge cells, got {green_edge_cells}"
    );
    // a point on the h1-s1 edge (outside both boxes) hit-tests to no node
    let (ax, ay, aw, ah) = node_rect(&graph, &layout, &gview, "h1").unwrap();
    let (bx, by, bw, bh) = node_rect(&graph, &layout, &gview, "s1").unwrap();
    let mid_x = (ax + aw as i64 / 2 + bx + bw as i64 / 2) / 2;
    let mid_y = (ay + ah as i64 / 2 + by + bh as i64 / 2) / 2;
    if mid_x >= 0 && mid_y >= 0 && (mid_x as usize) < 35 {
        assert_eq!(
            hit_test(&graph, &layout, &gview, mid_x as usize, mid_y as usize),
            None
        );
    }
}

#[test]
fn render_golden_no_selection_panned_zoomed() {
    let graph = dry_graph();
    let layout = Layout::compute(&graph);
    let mut view = View::new(60, 20);
    view.pan_cells(5.0, -3.0);
    view.zoom_in();
    let mut grid = StringGrid::new(60, 20);
    render_frame(
        &mut grid,
        &Frame {
            graph: &graph,
            layout: &layout,
            view: &view,
            selected: None,
            show_detail: false,
            title: TITLE,
            status: "tick 7",
            footer: FOOTER,
        },
    );
    let text = grid.to_string();
    assert!(text.lines().count() == 20);
    assert!(text.contains("tick 7"));
    assert!(text.contains("h1") && text.contains("s1"));
    // after pan+zoom the boxes still hit-test consistently
    let gview = {
        let mut v = view.clone();
        v.resize(60, 17);
        v
    };
    let (x, y, w, h) = node_rect(&graph, &layout, &gview, "s1").unwrap();
    if x >= 0 && y >= 0 && (x + w as i64) < 60 && (y + h as i64) < 17 {
        let cx = (x + w as i64 / 2) as usize;
        let cy = (y + h as i64 / 2) as usize;
        assert_eq!(
            hit_test(&graph, &layout, &gview, cx, cy).as_deref(),
            Some("s1")
        );
    }
}
