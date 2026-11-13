//! The ratatui + crossterm interactive loop (task 13). This is a thin
//! adapter: all drawing goes through `mnr_tui::render_frame` into a
//! [`CellBuffer`] implemented over the ratatui frame buffer, so the headless
//! `--frames` output and the live UI are pixel-identical.
//!
//! Keys: `q` quit (stops the net if we started it), arrows/hjkl pan,
//! `+`/`-` (or `>`/`<`) zoom, tab cycle nodes, enter toggles the detail pane,
//! `r` refresh, `p` pingall (status line). In live (root) mode the graph is
//! re-snapshotted every [`REFRESH_INTERVAL`].

use std::io::stdout;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use mnr_tui::render::{CellBuffer, Color, Style};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color as RatColor, Modifier};
use ratatui::Terminal;

use crate::app::{App, REFRESH_INTERVAL};
use crate::cli::Args;

/// Run the interactive TUI. Returns the process exit code. If we started
/// the net (root), it is stopped on exit — including after `q`.
pub fn run(args: &Args) -> i32 {
    let mut app = match App::new(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mn-tui: {e}");
            return 1;
        }
    };
    ratatui::init();
    let code = event_loop(&mut app, args);
    ratatui::restore();
    if app.live {
        if let Err(e) = app.net.stop() {
            eprintln!("mn-tui: failed to stop net: {e}");
        }
    }
    code
}

fn event_loop(app: &mut App, args: &Args) -> i32 {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("mn-tui: cannot open terminal: {e}");
            return 1;
        }
    };
    let mut last_refresh = Instant::now();
    loop {
        let title = app.title(args);
        if terminal
            .draw(|f| {
                let area = f.area();
                app.view.resize(area.width as usize, area.height as usize);
                let mut buf = RatBuf {
                    buf: f.buffer_mut(),
                    area,
                };
                app.render_into(&mut buf, &title);
            })
            .is_err()
        {
            break;
        }
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) => {
                    if !app.key(k) {
                        break;
                    }
                }
                Ok(Event::Resize(..)) => {}
                Ok(_) => {}
                Err(_) => {}
            }
        }
        if app.live && last_refresh.elapsed() >= REFRESH_INTERVAL {
            app.refresh();
            last_refresh = Instant::now();
        }
    }
    0
}

/// [`CellBuffer`] adapter over a ratatui frame buffer region.
struct RatBuf<'a> {
    buf: &'a mut Buffer,
    area: Rect,
}

impl CellBuffer for RatBuf<'_> {
    fn width(&self) -> usize {
        self.area.width as usize
    }

    fn height(&self) -> usize {
        self.area.height as usize
    }

    fn set(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        let Some(cell) = self
            .buf
            .cell_mut((self.area.x + x as u16, self.area.y + y as u16))
        else {
            return;
        };
        cell.set_symbol(&ch.to_string());
        cell.set_fg(map_color(style.fg));
        // bg and modifier must be written on EVERY set, not only when
        // set: ratatui reuses the frame buffer between frames and
        // `render_frame` starts with `clear()`, which redraws every cell
        // with `Style::default()` (bg None, bold false). Skipping the
        // writes here would leave the previous frame's bg/BOLD stuck on
        // the cell. `StringGrid` gets this for free by replacing the
        // whole cell.
        cell.set_bg(map_color(style.bg.unwrap_or(Color::Default)));
        if style.bold {
            cell.modifier.insert(Modifier::BOLD);
        } else {
            cell.modifier.remove(Modifier::BOLD);
        }
    }
}

fn map_color(c: Color) -> RatColor {
    match c {
        Color::Default => RatColor::Reset,
        Color::Black => RatColor::Black,
        Color::Red => RatColor::Red,
        Color::Green => RatColor::Green,
        Color::Yellow => RatColor::Yellow,
        Color::Blue => RatColor::Blue,
        Color::Magenta => RatColor::Magenta,
        Color::Cyan => RatColor::Cyan,
        Color::Gray => RatColor::Gray,
        Color::White => RatColor::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnr_tui::{render_frame, Frame, GraphView, Layout, StringGrid, View};

    fn dry_graph() -> GraphView {
        let net = mn_net::Mininet::dry_run(
            "single,3",
            mn_net::MininetOptions {
                controller: None,
                ..Default::default()
            },
        )
        .unwrap();
        GraphView::from_net(&net, false)
    }

    fn frame<'a>(
        graph: &'a GraphView,
        layout: &'a Layout,
        view: &'a View,
        selected: Option<&'a str>,
        show_detail: bool,
        status: &'a str,
    ) -> Frame<'a> {
        Frame {
            graph,
            layout,
            view,
            selected,
            show_detail,
            title: "test title",
            status,
            footer: "test footer",
        }
    }

    /// Direct check of the fix: after `set()` with bg+BOLD, `clear()` (the
    /// default trait method `render_frame` calls each frame) must reset
    /// symbol, fg, bg AND the BOLD modifier on the ratatui cell.
    #[test]
    fn set_then_clear_resets_bg_and_bold() {
        let area = Rect::new(0, 0, 3, 1);
        let mut rbuf = Buffer::empty(area);
        {
            let mut rb = RatBuf {
                buf: &mut rbuf,
                area,
            };
            rb.set(
                1,
                0,
                'x',
                Style {
                    fg: Color::White,
                    bg: Some(Color::Red),
                    bold: true,
                },
            );
        }
        {
            let mut rb = RatBuf {
                buf: &mut rbuf,
                area,
            };
            rb.clear();
        }
        let c = rbuf.cell((1, 0)).expect("in area");
        assert_eq!(c.symbol(), " ");
        assert_eq!(c.fg, RatColor::Reset);
        assert_eq!(c.bg, RatColor::Reset);
        assert!(!c.modifier.contains(Modifier::BOLD));
    }

    /// Headless-vs-ratbuf consistency: render two frames (the first with
    /// a selection + detail pane, the second without) into the SAME
    /// ratatui buffer, then compare every cell (symbol, fg, bg, bold)
    /// against a `StringGrid` rendered with only the second frame. With
    /// conditional bg/modifier writes the stale styled cells of frame A
    /// would leak into frame B and this comparison would fail.
    #[test]
    fn ratbuf_matches_string_grid_cell_for_cell_across_frames() {
        let graph = dry_graph();
        let layout = Layout::compute(&graph);
        let (w, h) = (44usize, 14usize);
        let view = View::new(w, h);
        let area = Rect::new(0, 0, w as u16, h as u16);
        let mut rbuf = Buffer::empty(area);
        {
            let mut rb = RatBuf {
                buf: &mut rbuf,
                area,
            };
            render_frame(
                &mut rb,
                &frame(&graph, &layout, &view, Some("h1"), true, "A"),
            );
        }
        {
            let mut rb = RatBuf {
                buf: &mut rbuf,
                area,
            };
            render_frame(&mut rb, &frame(&graph, &layout, &view, None, false, "B"));
        }
        let mut grid = StringGrid::new(w, h);
        render_frame(&mut grid, &frame(&graph, &layout, &view, None, false, "B"));
        for y in 0..h {
            for x in 0..w {
                let want = grid.cell_at(x, y);
                let got = rbuf.cell((x as u16, y as u16)).expect("in area");
                assert_eq!(got.symbol(), want.ch.to_string(), "symbol at ({x},{y})");
                assert_eq!(got.fg, map_color(want.style.fg), "fg at ({x},{y})");
                assert_eq!(
                    got.bg,
                    map_color(want.style.bg.unwrap_or(Color::Default)),
                    "bg at ({x},{y})"
                );
                assert_eq!(
                    got.modifier.contains(Modifier::BOLD),
                    want.style.bold,
                    "bold at ({x},{y})"
                );
            }
        }
    }
}
