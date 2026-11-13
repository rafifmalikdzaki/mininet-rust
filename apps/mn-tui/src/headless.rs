//! Headless export modes of `mn-tui` (task 13): `--json` dumps the
//! [`GraphView`] as JSON; `--frames N` renders N frames through the exact
//! same `render_frame` code into a [`StringGrid`] and prints them to stdout
//! (CI / non-tty). Frames exercise pan/zoom/selection deterministically:
//! frame `i` pans by `3i` cells, zooms on every 3rd frame, and selects node
//! `i % len` with the detail pane on even frames.

use mn_net::Mininet;
use mnr_tui::{render_frame, GraphView, Layout, StringGrid, View};

use crate::app::FOOTER;
use crate::cli::Args;

/// Run a headless mode (`--json` or `--frames N`). As root the net is
/// started first (live graph) and stopped afterwards; non-root exports the
/// dry-run plan.
pub fn run(args: &Args) -> Result<i32, Box<dyn std::error::Error>> {
    let (w, h) = args.size();
    let live = mn_net::is_root();
    let mut net = Mininet::dry_run(&args.topo, args.options())?;
    if live {
        net.start()?;
    }
    let title = args.title(live);
    let result = run_export(args, &net, live, &title, w, h);
    if live {
        net.stop()?;
    }
    result
}

fn run_export(
    args: &Args,
    net: &Mininet,
    live: bool,
    title: &str,
    w: usize,
    h: usize,
) -> Result<i32, Box<dyn std::error::Error>> {
    if args.json {
        let graph = GraphView::from_net(net, live);
        println!("{}", graph.to_json());
        return Ok(0);
    }
    let n = args.frames.unwrap_or(1).max(1);
    for i in 0..n {
        // re-snapshot every frame so live mode shows fresh status
        let graph = GraphView::from_net(net, live);
        let layout = Layout::compute(&graph);
        let mut view = View::new(w, h);
        view.pan_cells(3.0 * i as f64, if i % 2 == 0 { 0.0 } else { -1.0 });
        if i % 3 == 2 {
            view.zoom_in();
        }
        let selected = graph
            .nodes
            .get(i % graph.nodes.len().max(1))
            .map(|n| n.id.clone());
        let show_detail = i % 2 == 0;
        let mut grid = StringGrid::new(w, h);
        let mode = if live { "live" } else { "plan" };
        let status = format!("frame {i_plus}/{n} (headless, {mode} mode)", i_plus = i + 1);
        render_frame(
            &mut grid,
            &mnr_tui::Frame {
                graph: &graph,
                layout: &layout,
                view: &view,
                selected: selected.as_deref(),
                show_detail,
                title,
                status: &status,
                footer: FOOTER,
            },
        );
        println!("=== frame {} ===\n{grid}", i + 1);
    }
    Ok(0)
}
