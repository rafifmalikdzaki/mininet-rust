//! The ratatui + crossterm interactive loop (task 13). This is a thin
//! adapter: all drawing goes through `mn_tui::render_frame` into a
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
use mn_tui::render::{CellBuffer, Color, Style};
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
        if let Some(bg) = style.bg {
            cell.set_bg(map_color(bg));
        }
        if style.bold {
            cell.modifier.insert(Modifier::BOLD);
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
