//! Terminal-free renderer: everything is drawn into a generic
//! [`CellBuffer`], so the same code drives the ratatui UI in `apps/mn-tui`
//! and the [`StringGrid`] used by headless tests and `--frames N`.
//!
//! Frame layout (OMO-style: the graph IS the UI):
//!
//! ```text
//! ┌─ row 0: title bar (topo spec + mode) ─────────────────────────┐
//! │  rows 1..h-2: the graph — node boxes + unicode edges,           │
//! │               with the detail pane floating on the right       │
//! │               when a node is selected (Enter)                  │
//! ├─ row h-2: status line (refresh tick / pingall result / errors) ┤
//! └─ row h-1: footer with keybindings ─────────────────────────────┘
//! ```

use std::fmt;

use crate::layout::{node_rect, Layout, View};
use crate::model::{EdgeStatus, GNode, GraphView, NodeKind, NodeStatus};

/// Terminal-agnostic color (mapped to ratatui/crossterm colors by the app).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// Terminal default.
    #[default]
    Default,
    /// Black.
    Black,
    /// Red — missing/down.
    Red,
    /// Green — up/ok/planned.
    Green,
    /// Yellow — switches (and the status line).
    Yellow,
    /// Blue.
    Blue,
    /// Magenta.
    Magenta,
    /// Cyan — controllers (and the detail pane).
    Cyan,
    /// Gray — secondary text, selected background.
    Gray,
    /// White — labels, selected border.
    White,
}

/// Cell style: foreground color, optional background, bold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    /// Foreground color.
    pub fg: Color,
    /// Optional background color.
    pub bg: Option<Color>,
    /// Bold modifier.
    pub bold: bool,
}

impl Style {
    /// New style with the given foreground color.
    pub fn new(fg: Color) -> Style {
        Style {
            fg,
            bg: None,
            bold: false,
        }
    }

    /// Builder: bold.
    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    /// Builder: background color.
    pub fn bg(mut self, bg: Color) -> Style {
        self.bg = Some(bg);
        self
    }
}

/// One cell of a [`CellBuffer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Grapheme (kept as a single char; all our glyphs are single-width).
    pub ch: char,
    /// Cell style.
    pub style: Style,
}

/// A drawable cell buffer. Implementations: [`StringGrid`] (tests,
/// `--frames`) and the ratatui adapter in `apps/mn-tui`.
pub trait CellBuffer {
    /// Buffer width in cells.
    fn width(&self) -> usize;
    /// Buffer height in rows.
    fn height(&self) -> usize;
    /// Set one cell (implementations must bounds-check).
    fn set(&mut self, x: usize, y: usize, ch: char, style: Style);

    /// Draw a string starting at `(x, y)` (clipped to the buffer width).
    fn set_str(&mut self, x: usize, y: usize, s: &str, style: Style) {
        for (i, ch) in s.chars().enumerate() {
            if x + i >= self.width() {
                break;
            }
            self.set(x + i, y, ch, style);
        }
    }

    /// Fill a whole row with a (padded/clipped to width) string.
    fn fill_row(&mut self, y: usize, s: &str, style: Style) {
        if y >= self.height() {
            return;
        }
        let w = self.width();
        let fitted: String = s.chars().take(w).collect();
        self.set_str(0, y, &fitted, style);
        for x in fitted.chars().count()..w {
            self.set(x, y, ' ', style);
        }
    }

    /// Clear the buffer to blank cells with the default style.
    fn clear(&mut self) {
        let (w, h) = (self.width(), self.height());
        for y in 0..h {
            for x in 0..w {
                self.set(x, y, ' ', Style::default());
            }
        }
    }
}

/// A [`CellBuffer`] backed by a string grid — used by tests and by
/// `mn-tui --frames N` to render headless frames to a text log.
#[derive(Debug, Clone, PartialEq)]
pub struct StringGrid {
    w: usize,
    h: usize,
    cells: Vec<Cell>,
}

impl StringGrid {
    /// New blank grid of `w` x `h` cells.
    pub fn new(w: usize, h: usize) -> StringGrid {
        StringGrid {
            w,
            h,
            cells: vec![
                Cell {
                    ch: ' ',
                    style: Style::default(),
                };
                w * h
            ],
        }
    }

    /// The cell at `(x, y)`.
    pub fn cell_at(&self, x: usize, y: usize) -> &Cell {
        &self.cells[y * self.w + x]
    }

    /// Foreground color at `(x, y)`.
    pub fn color_at(&self, x: usize, y: usize) -> Color {
        self.cell_at(x, y).style.fg
    }

    /// One row as a string (no trailing newline).
    pub fn row(&self, y: usize) -> String {
        self.cells[y * self.w..(y + 1) * self.w]
            .iter()
            .map(|c| c.ch)
            .collect()
    }
}

impl CellBuffer for StringGrid {
    fn width(&self) -> usize {
        self.w
    }

    fn height(&self) -> usize {
        self.h
    }

    fn set(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if x < self.w && y < self.h {
            self.cells[y * self.w + x] = Cell { ch, style };
        }
    }
}

impl fmt::Display for StringGrid {
    /// The grid as text: `h` rows joined by `\n` (no trailing newline).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for y in 0..self.h {
            if y > 0 {
                f.write_str("\n")?;
            }
            for c in &self.cells[y * self.w..(y + 1) * self.w] {
                write!(f, "{}", c.ch)?;
            }
        }
        Ok(())
    }
}

/// Node status color: green = up/planned (ok), red = down/missing.
pub fn status_color(s: NodeStatus) -> Color {
    match s {
        NodeStatus::Up | NodeStatus::Planned => Color::Green,
        NodeStatus::Down | NodeStatus::Missing => Color::Red,
    }
}

/// Edge status color: green = up/planned, red = down/missing.
pub fn edge_color(s: EdgeStatus) -> Color {
    match s {
        EdgeStatus::Up | EdgeStatus::Planned => Color::Green,
        EdgeStatus::Down | EdgeStatus::Missing => Color::Red,
    }
}

/// Node box border color: yellow = switch, cyan = controller,
/// status color = host.
pub fn node_border_color(node: &GNode) -> Color {
    match node.kind {
        NodeKind::Switch => Color::Yellow,
        NodeKind::Controller => Color::Cyan,
        NodeKind::Host => status_color(node.status),
    }
}

/// Truncate/pad `s` to exactly `w` chars.
fn fit(s: &str, w: usize) -> String {
    let mut out: String = s.chars().take(w).collect();
    while out.chars().count() < w {
        out.push(' ');
    }
    out
}

/// A drawing pen: a buffer plus the graph-area offset `(ox, oy)` and area
/// size `(aw, ah)`; all coordinates are area-relative and clipped.
struct Pen<'a> {
    buf: &'a mut dyn CellBuffer,
    ox: usize,
    oy: usize,
    aw: usize,
    ah: usize,
}

impl Pen<'_> {
    /// Set one cell at area coords `(x, y)`, clipped to the area.
    fn put(&mut self, x: i64, y: i64, ch: char, style: Style) {
        if x >= 0 && y >= 0 && x < self.aw as i64 && y < self.ah as i64 {
            self.buf
                .set(self.ox + x as usize, self.oy + y as usize, ch, style);
        }
    }

    /// Draw one bordered box at area-relative rect `rect = (rx, ry, w, h)`
    /// with the given content lines (clipped to the box and the area). The
    /// interior is filled opaquely so floating panes cover what is beneath.
    fn draw_box(
        &mut self,
        rect: (i64, i64, usize, usize),
        border: Style,
        content_bg: Option<Color>,
        lines: &[String],
    ) {
        let (rx, ry, w, h) = rect;
        if w < 2 || h < 2 {
            return;
        }
        // horizontal + vertical borders
        for x in 0..w as i64 {
            self.put(rx + x, ry, '─', border);
            self.put(rx + x, ry + h as i64 - 1, '─', border);
        }
        for y in 0..h as i64 {
            self.put(rx, ry + y, '│', border);
            self.put(rx + w as i64 - 1, ry + y, '│', border);
        }
        // corners
        self.put(rx, ry, '┌', border);
        self.put(rx + w as i64 - 1, ry, '┐', border);
        self.put(rx, ry + h as i64 - 1, '└', border);
        self.put(rx + w as i64 - 1, ry + h as i64 - 1, '┘', border);
        // opaque interior
        let fill_style = Style {
            fg: Color::Default,
            bg: content_bg,
            bold: false,
        };
        for y in 1..h as i64 - 1 {
            for x in 1..w as i64 - 1 {
                self.put(rx + x, ry + y, ' ', fill_style);
            }
        }
        // content
        let inner_w = w - 2;
        for (i, line) in lines.iter().enumerate() {
            if i + 2 >= h {
                break;
            }
            let y = ry + 1 + i as i64;
            let style = Style {
                fg: border.fg,
                bg: content_bg,
                bold: border.bold,
            };
            let fitted = fit(line, inner_w);
            for (k, ch) in fitted.chars().enumerate() {
                self.put(rx + 1 + k as i64, y, ch, style);
            }
        }
    }
}

/// Bresenham line points with a per-step glyph: horizontal `─`, vertical
/// `│`, diagonals `\` and `/`.
fn line_points(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<(i64, i64, char)> {
    let mut pts: Vec<(i64, i64, char)> = Vec::new();
    let dx = (x1 - x0).abs();
    let sx: i64 = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy: i64 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        let ch = match pts.last() {
            None => '•',
            Some(&(px, py, _)) => match (x - px, y - py) {
                (1, 0) | (-1, 0) => '─',
                (0, 1) | (0, -1) => '│',
                (1, 1) | (-1, -1) => '\\',
                _ => '/',
            },
        };
        pts.push((x, y, ch));
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    pts
}

/// Draw the graph (edges first, then node boxes on top) into the buffer at
/// area offset `(ox, oy)`. `view` must be sized to the graph area.
pub fn render_graph(
    buf: &mut impl CellBuffer,
    ox: usize,
    oy: usize,
    view: &View,
    graph: &GraphView,
    layout: &Layout,
    selected: Option<&str>,
) {
    let rects: Vec<Option<(i64, i64, usize, usize)>> = graph
        .nodes
        .iter()
        .map(|n| node_rect(graph, layout, view, &n.id))
        .collect();
    let rect_of = |id: &str| -> Option<(i64, i64, usize, usize)> {
        graph
            .nodes
            .iter()
            .position(|n| n.id == id)
            .and_then(|i| rects[i])
    };
    let inside_node = |px: i64, py: i64| -> bool {
        rects
            .iter()
            .flatten()
            .any(|&(rx, ry, w, h)| px >= rx && px < rx + w as i64 && py >= ry && py < ry + h as i64)
    };
    let mut pen = Pen {
        buf,
        ox,
        oy,
        aw: view.width,
        ah: view.height,
    };
    // edges
    for e in &graph.edges {
        let (Some((ax, ay, aw, ah)), Some((bx, by, bw, bh))) = (rect_of(&e.a), rect_of(&e.b))
        else {
            continue;
        };
        let x0 = ax + aw as i64 / 2;
        let y0 = ay + ah as i64 / 2;
        let x1 = bx + bw as i64 / 2;
        let y1 = by + bh as i64 / 2;
        let style = Style::new(edge_color(e.status));
        for (px, py, ch) in line_points(x0, y0, x1, y1) {
            if !inside_node(px, py) {
                pen.put(px, py, ch, style);
            }
        }
    }
    // nodes (boxes overwrite edge ends)
    for (node, rect) in graph.nodes.iter().zip(&rects) {
        let Some((rx, ry, w, h)) = rect else {
            continue;
        };
        let is_selected = selected == Some(node.id.as_str());
        let border = if is_selected {
            Style::new(Color::White).bold()
        } else {
            Style::new(node_border_color(node))
        };
        let content_bg = if is_selected { Some(Color::Gray) } else { None };
        let status_line = format!("{} {}", node.kind, node.status);
        let lines = vec![
            node.label.clone(),
            node.ip.clone().unwrap_or_else(|| "-".to_string()),
            status_line,
        ];
        pen.draw_box((*rx, *ry, *w, *h), border, content_bg, &lines);
    }
}

/// Detail-pane content lines for a node (repr, intfs, ips, pid).
pub fn detail_lines(node: &GNode) -> Vec<String> {
    let mut lines = vec![
        format!(" {} ", node.id),
        format!("kind:   {}", node.kind),
        format!("status: {}", node.status),
        format!("ip:     {}", node.ip.as_deref().unwrap_or("-")),
    ];
    match &node.detail {
        Some(d) => {
            lines.push(format!(
                "pid:    {}",
                d.pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "None".to_string())
            ));
            lines.push(format!("repr: {}", d.repr.trim_end()));
            lines.push("intfs:".to_string());
            for i in &d.intfs {
                lines.push(format!(
                    "  {} {} {}",
                    i.name,
                    i.ip.as_deref().unwrap_or("-"),
                    i.status
                ));
            }
            if !d.ips.is_empty() {
                lines.push(format!("ips: {}", d.ips.join(", ")));
            }
        }
        None => {
            lines.push("pid:    -".to_string());
            lines.push("repr:   -".to_string());
        }
    }
    lines
}

/// Draw the detail pane floating on the right side of the graph area.
pub fn render_detail(
    buf: &mut impl CellBuffer,
    ox: usize,
    oy: usize,
    w: usize,
    h: usize,
    graph: &GraphView,
    selected: &str,
) {
    let Some(node) = graph.node(selected) else {
        return;
    };
    if w < 8 || h < 3 {
        return;
    }
    let pw = (w / 2).clamp(10, 44).min(w);
    let px = (w - pw) as i64;
    let lines = detail_lines(node);
    let mut pen = Pen {
        buf,
        ox,
        oy,
        aw: w,
        ah: h,
    };
    pen.draw_box((px, 0, pw, h), Style::new(Color::Cyan), None, &lines);
}

/// Everything needed to draw one frame (see [`render_frame`]).
#[derive(Debug, Clone)]
pub struct Frame<'a> {
    /// The graph snapshot.
    pub graph: &'a GraphView,
    /// The computed layout of `graph`.
    pub layout: &'a Layout,
    /// Pan/zoom viewport (sized to the full frame; the graph area is
    /// derived internally).
    pub view: &'a View,
    /// Selected node id (white bold border + detail pane).
    pub selected: Option<&'a str>,
    /// Whether to draw the detail pane for `selected`.
    pub show_detail: bool,
    /// Title bar text (row 0).
    pub title: &'a str,
    /// Status line text (row h-2).
    pub status: &'a str,
    /// Footer keybinding text (row h-1).
    pub footer: &'a str,
}

/// Render a whole frame: title bar (row 0), graph area (rows 1..h-2, with
/// the detail pane when `show_detail`), status line (row h-2), footer
/// (row h-1). See [`Frame`] for the parameters.
pub fn render_frame(buf: &mut impl CellBuffer, frame: &Frame<'_>) {
    buf.clear();
    let w = buf.width();
    let h = buf.height();
    if w == 0 || h == 0 {
        return;
    }
    // title bar
    buf.fill_row(
        0,
        &format!(" {} ", frame.title),
        Style::new(Color::White).bg(Color::Blue).bold(),
    );
    // footer
    if h >= 2 {
        buf.fill_row(
            h - 1,
            &format!(" {} ", frame.footer),
            Style::new(Color::Black).bg(Color::Gray),
        );
    }
    // status line
    if h >= 3 {
        buf.fill_row(
            h - 2,
            &format!(" {} ", frame.status),
            Style::new(Color::Yellow),
        );
    }
    // graph area: rows 1..h-2
    let gh = h.saturating_sub(3);
    if gh >= 1 {
        let mut gview = frame.view.clone();
        gview.resize(w, gh);
        render_graph(buf, 0, 1, &gview, frame.graph, frame.layout, frame.selected);
        if frame.show_detail {
            if let Some(id) = frame.selected {
                render_detail(buf, 0, 1, w, gh, frame.graph, id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeStatus, GEdge, NodeKind, NodeStatus};

    fn node(id: &str, kind: NodeKind, ip: Option<&str>, status: NodeStatus) -> GNode {
        GNode {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            ip: ip.map(str::to_string),
            status,
            detail: None,
        }
    }

    #[test]
    fn string_grid_set_str_and_display() {
        let mut g = StringGrid::new(10, 3);
        g.set_str(2, 1, "hello world", Style::new(Color::Red));
        assert_eq!(g.row(1), "  hello wo");
        g.set(0, 0, 'x', Style::new(Color::Green).bold());
        assert_eq!(g.cell_at(0, 0).ch, 'x');
        assert_eq!(g.color_at(0, 0), Color::Green);
        assert!(g.cell_at(0, 0).style.bold);
        // bounds-checked
        g.set(100, 100, 'y', Style::default());
        assert_eq!(g.to_string().lines().count(), 3);
    }

    #[test]
    fn line_points_glyphs() {
        let pts = line_points(0, 0, 3, 0);
        assert_eq!(
            pts.iter().map(|p| p.2).collect::<Vec<_>>(),
            ['•', '─', '─', '─']
        );
        let pts = line_points(0, 0, 0, 2);
        assert_eq!(pts.iter().map(|p| p.2).collect::<Vec<_>>(), ['•', '│', '│']);
        let pts = line_points(0, 0, 2, 2);
        assert_eq!(
            pts.iter().map(|p| p.2).collect::<Vec<_>>(),
            ['•', '\\', '\\']
        );
        let pts = line_points(0, 2, 2, 0);
        assert_eq!(pts.iter().map(|p| p.2).collect::<Vec<_>>(), ['•', '/', '/']);
    }

    #[test]
    fn frame_renders_title_status_footer() {
        let graph = GraphView {
            spec: Some("minimal".to_string()),
            live: false,
            nodes: vec![
                node("h1", NodeKind::Host, Some("10.0.0.1"), NodeStatus::Planned),
                node("h2", NodeKind::Host, Some("10.0.0.2"), NodeStatus::Planned),
            ],
            edges: vec![GEdge {
                a: "h1".to_string(),
                b: "h2".to_string(),
                status: EdgeStatus::Planned,
                label: "h1-eth0<->h2-eth0".to_string(),
                intf1: Some("h1-eth0".to_string()),
                intf2: Some("h2-eth0".to_string()),
                port1: Some(0),
                port2: Some(0),
            }],
        };
        let layout = Layout::compute(&graph);
        let view = View::new(50, 22);
        let mut grid = StringGrid::new(50, 22);
        render_frame(
            &mut grid,
            &Frame {
                graph: &graph,
                layout: &layout,
                view: &view,
                selected: Some("h1"),
                show_detail: true,
                title: "mn-tui — topo minimal (dry-run plan)",
                status: "status: ok",
                footer: "q quit",
            },
        );
        let text = grid.to_string();
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows.len(), 22);
        assert!(rows[0].starts_with(" mn-tui — topo minimal (dry-run plan)"));
        assert!(rows[20].starts_with(" status: ok"));
        assert!(rows[21].starts_with(" q quit"));
        // node boxes present
        assert!(text.contains('┌') && text.contains('┐'));
        assert!(text.contains("h1") && text.contains("h2"));
        assert!(text.contains("10.0.0.1"));
        // edge between the two boxes (no switch -> h1 is root at layer 0,
        // h2 at layer 1 -> vertical edge '│')
        assert!(text.contains('│'));
        // selected node h1 has a white bold border
        let l = Layout::compute(&graph);
        let v = View::new(50, 19); // graph area height = 22-3
        let (x, y, _w, _h) = node_rect(&graph, &l, &v, "h1").unwrap();
        assert_eq!(grid.color_at(x as usize, (y + 1) as usize), Color::White);
        // detail pane on the right shows the repr-less fallback
        assert!(text.contains("kind:   host"));
        assert!(text.contains("status: planned"));
    }
}
