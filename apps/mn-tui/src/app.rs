//! Interactive application state for `mn-tui` (task 13): owns the `Mininet`
//! (when live), the [`GraphView`] snapshot, the layout/view, the selection,
//! and the keybindings. Rendering goes through `mnr_tui::render_frame` into
//! any [`CellBuffer`]; the ratatui loop in `tui.rs` is a thin adapter.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use mn_net::Mininet;
use mnr_tui::{render_frame, CellBuffer, GraphView, Layout, View};

use crate::cli::Args;

/// Footer keybinding line.
pub const FOOTER: &str =
    "q quit | arrows/hjkl pan | +/- or </> zoom | tab cycle | enter detail | r refresh | p pingall";
/// Pan step in screen cells per keypress.
pub const PAN_STEP: f64 = 4.0;
/// Live status refresh interval.
pub const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// The TUI application state.
pub struct App {
    /// The network. In live (root) mode this is the started net; the TUI
    /// owns it and stops it on quit.
    pub net: Mininet,
    /// Whether we started the net (root) — live status + pingall allowed.
    pub live: bool,
    /// Current graph snapshot (rebuilt from `net` on refresh).
    pub graph: GraphView,
    /// Deterministic layered layout of the current graph.
    pub layout: Layout,
    /// Pan/zoom viewport (resized to the frame on every draw).
    pub view: View,
    /// Index into `graph.nodes` of the highlighted node.
    pub selected: usize,
    /// Whether the detail pane is shown for the selected node.
    pub show_detail: bool,
    /// Status line (refresh tick / pingall result / errors).
    pub status: String,
    /// Refresh tick counter.
    pub tick: u64,
}

impl App {
    /// Build the app: dry-run the topo spec; as root, also start the net
    /// (live mode). Non-root gets the read-only dry-run plan.
    pub fn new(args: &Args) -> Result<App, Box<dyn std::error::Error>> {
        let live = mn_net::is_root();
        let mut net = Mininet::dry_run(&args.topo, args.options())?;
        if live {
            net.start()?;
        }
        let graph = GraphView::from_net(&net, live);
        let layout = Layout::compute(&graph);
        let status = if live {
            format!(
                "live net started (root): {} nodes, {} links — refreshing every {:?}",
                graph.nodes.len(),
                graph.edges.len(),
                REFRESH_INTERVAL
            )
        } else {
            "dry-run plan (non-root, read-only) — use --json / --frames N for export".to_string()
        };
        Ok(App {
            net,
            live,
            graph,
            layout,
            view: View::new(80, 24),
            selected: 0,
            show_detail: false,
            status,
            tick: 0,
        })
    }

    /// Title bar text.
    pub fn title(&self, args: &Args) -> String {
        args.title(self.live)
    }

    /// Re-snapshot the graph from the net (live status: link statuses +
    /// node pids), keeping the selection on the same node id.
    pub fn refresh(&mut self) {
        let selected_id = self.graph.nodes.get(self.selected).map(|n| n.id.clone());
        self.graph = GraphView::from_net(&self.net, self.live);
        self.layout = Layout::compute(&self.graph);
        if let Some(id) = selected_id {
            if let Some(i) = self.graph.nodes.iter().position(|n| n.id == id) {
                self.selected = i;
            }
        }
        self.tick += 1;
        let up = self
            .graph
            .nodes
            .iter()
            .filter(|n| n.status == mnr_tui::NodeStatus::Up)
            .count();
        self.status = format!(
            "tick {} — {} nodes ({} up), {} links, spec {:?}",
            self.tick,
            self.graph.nodes.len(),
            up,
            self.graph.edges.len(),
            self.graph.spec
        );
    }

    /// Currently selected node id.
    pub fn selected_id(&self) -> Option<&str> {
        self.graph.nodes.get(self.selected).map(|n| n.id.as_str())
    }

    /// Cycle the selection by `dir` (+1/-1) through the nodes.
    pub fn cycle(&mut self, dir: i64) {
        let n = self.graph.nodes.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as i64;
        self.selected = ((cur + dir).rem_euclid(n as i64)) as usize;
    }

    /// Run pingall on the live net and put the result in the status line.
    pub fn pingall(&mut self) {
        if !self.live {
            self.status =
                "pingall: needs a live net (run as root); dry-run plan is read-only".to_string();
            return;
        }
        match self.net.ping_all(None) {
            Ok(r) => {
                self.status = format!(
                    "pingall: {:.0}% packet loss over {} hosts",
                    r.ploss,
                    self.net.hosts.len()
                );
            }
            Err(e) => {
                self.status = format!("pingall failed: {e}");
            }
        }
    }

    /// Handle one key event; returns `false` when the app should quit.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        if k.kind == KeyEventKind::Release {
            return true;
        }
        match k.code {
            KeyCode::Char('q') => return false,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Up | KeyCode::Char('k') => self.view.pan_cells(0.0, -PAN_STEP),
            KeyCode::Down | KeyCode::Char('j') => self.view.pan_cells(0.0, PAN_STEP),
            KeyCode::Left | KeyCode::Char('h') => self.view.pan_cells(-PAN_STEP, 0.0),
            KeyCode::Right | KeyCode::Char('l') => self.view.pan_cells(PAN_STEP, 0.0),
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('>') => self.view.zoom_in(),
            KeyCode::Char('-') | KeyCode::Char('<') => self.view.zoom_out(),
            KeyCode::Tab => self.cycle(1),
            KeyCode::BackTab => self.cycle(-1),
            KeyCode::Enter => self.show_detail = !self.show_detail,
            KeyCode::Esc => self.show_detail = false,
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('p') => self.pingall(),
            _ => {}
        }
        true
    }

    /// Render one frame into `buf` (the view is resized to the buffer).
    pub fn render_into(&self, buf: &mut impl CellBuffer, title: &str) {
        let mut view = self.view.clone();
        view.resize(buf.width(), buf.height());
        render_frame(
            buf,
            &mnr_tui::Frame {
                graph: &self.graph,
                layout: &self.layout,
                view: &view,
                selected: self.selected_id(),
                show_detail: self.show_detail,
                title,
                status: &self.status,
                footer: FOOTER,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use crossterm::event::KeyEventState;

    fn args() -> Args {
        Args::parse_from(["mn-tui", "--topo", "single,3", "--controller", "none"])
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn app_builds_dry_run_plan_non_root() {
        // sandbox is non-root: live=false, plan statuses
        let args = args();
        let app = App::new(&args).unwrap();
        assert_eq!(app.live, mn_net::is_root());
        assert_eq!(app.graph.nodes.len(), 4);
        assert_eq!(app.graph.edges.len(), 3);
        assert_eq!(app.selected_id(), Some("h1"));
    }

    #[test]
    fn keys_pan_zoom_cycle_and_quit() {
        let args = args();
        let mut app = App::new(&args).unwrap();
        let z0 = app.view.zoom;
        assert!(app.key(key(KeyCode::Char('+'))));
        assert!(app.view.zoom > z0);
        assert!(app.key(key(KeyCode::Char('-'))));
        assert_eq!(app.view.zoom, z0);
        let px = app.view.pan_x;
        assert!(app.key(key(KeyCode::Right)));
        assert!(app.view.pan_x != px);
        assert!(app.key(key(KeyCode::Tab)));
        assert_eq!(app.selected, 1);
        assert!(app.key(key(KeyCode::BackTab)));
        assert_eq!(app.selected, 0);
        assert!(app.key(key(KeyCode::Enter)));
        assert!(app.show_detail);
        assert!(app.key(key(KeyCode::Esc)));
        assert!(!app.show_detail);
        assert!(app.key(key(KeyCode::Char('r'))));
        assert_eq!(app.tick, 1);
        assert!(!app.key(key(KeyCode::Char('q'))));
    }

    #[test]
    fn pingall_on_dry_run_is_a_read_only_notice() {
        let args = args();
        let mut app = App::new(&args).unwrap();
        app.pingall();
        assert!(app.status.contains("read-only"), "status: {}", app.status);
    }

    #[test]
    fn size_parsing() {
        let a = Args::parse_from(["mn-tui", "--size", "100x30"]);
        assert_eq!(a.size(), (100, 30));
        let b = Args::parse_from(["mn-tui", "--size", "bogus"]);
        assert_eq!(b.size(), (80, 24));
    }
}
