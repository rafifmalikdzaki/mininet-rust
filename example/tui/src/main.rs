use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, IsTerminal, Read, Seek, SeekFrom, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

const REFRESH: Duration = Duration::from_secs(2);
const STALE_SECS: u64 = 30;
const HELP: &str = "1–4 / Tab   Select view\n\
↑/↓, j/k    Scroll content\n\
PgUp/PgDn   Scroll by page\n\
←/→, h/l    Select step attempt in Logs\n\
r           Refresh now\n\
p           Pause / resume automatic refresh\n\
? or Esc    Close help\n\
q           Quit (does not stop the experiment)\n\
\n\
A start without finish is not proof the runner is alive.\n\
Exporter network counters are not per-node; probes are.";

#[derive(Default)]
struct Snapshot {
    scenario: Option<Value>,
    events: Vec<Value>,
    telemetry: Vec<Value>,
    telemetry_error: Option<String>,
    errors: Vec<String>,
    event_age: Option<u64>,
    metrics: Vec<String>,
    metric_status: String,
}

struct App {
    run: PathBuf,
    prometheus: Option<String>,
    snapshot: Snapshot,
    offsets: Offsets,
    tab: usize,
    selection: usize,
    scroll: usize,
    help: bool,
    paused: bool,
}

fn read_json(path: &Path, errors: &mut Vec<String>) -> Option<Value> {
    match fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(v) => Some(v),
            Err(e) => {
                errors.push(format!("{}: invalid JSON: {e}", path.display()));
                None
            }
        },
        Err(e) => {
            errors.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// Bytes, complete lines, and parse errors already loaded, so refreshes parse
/// only new lines and keep file-wide error line numbers and messages.
#[derive(Default)]
struct Offsets {
    events: u64,
    telemetry: u64,
    events_lines: u64,
    telemetry_lines: u64,
    events_errors: Vec<String>,
    telemetry_errors: Vec<String>,
}

/// Read JSONL from the start. Complete lines are parsed; a valid final object
/// without a trailing newline is accepted and consumed. An invalid trailing
/// fragment is left for the next read. Returns values, consumed bytes, lines.
fn read_jsonl(
    path: &Path,
    start_line: u64,
    errors: &mut Vec<String>,
) -> io::Result<(Vec<Value>, u64, u64)> {
    let data = fs::read_to_string(path)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (complete, tail) = match data.rfind('\n') {
        Some(end) => (&data[..=end], &data[end + 1..]),
        None => ("", data.as_str()),
    };
    let mut values = Vec::new();
    for (i, line) in complete.split('\n').enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(v) => values.push(v),
            Err(e) => errors.push(format!("{name} line {}: {e}", start_line + i as u64 + 1)),
        }
    }
    let lines = complete.bytes().filter(|b| *b == b'\n').count() as u64;
    let mut consumed = complete.len() as u64;
    if !tail.trim().is_empty() {
        if let Ok(value) = serde_json::from_str(tail) {
            values.push(value);
            consumed = data.len() as u64;
            // The tail has no terminator yet, so it is not counted as a line.
            // The newline appended later increments the count exactly once.
        }
    }
    Ok((values, consumed, lines))
}

/// Read only lines appended after `offset`. Returns `None` when the file shrank
/// (the caller must reload it), and otherwise the new values, offset, and
/// number of complete lines consumed.
fn append_jsonl(
    path: &Path,
    offset: u64,
    start_line: u64,
    errors: &mut Vec<String>,
) -> io::Result<Option<(Vec<Value>, u64, u64)>> {
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() < offset {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    let Some(end) = data.iter().rposition(|b| *b == b'\n') else {
        return Ok(Some((Vec::new(), offset, 0)));
    };
    let complete = String::from_utf8_lossy(&data[..=end]);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut values = Vec::new();
    for (i, line) in complete.split('\n').enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(v) => values.push(v),
            Err(e) => errors.push(format!("{name} line {}: {e}", start_line + i as u64 + 1)),
        }
    }
    let lines = complete.bytes().filter(|b| *b == b'\n').count() as u64;
    Ok(Some((values, offset + end as u64 + 1, lines)))
}

fn update_metrics(s: &mut Snapshot, prometheus: Option<&str>) {
    s.metrics.clear();
    match prometheus {
        None => s.metric_status = "DISCONNECTED · no Prometheus endpoint configured".into(),
        Some(addr) => match query_metrics(addr) {
            Ok(metrics) if metrics.is_empty() => {
                s.metric_status = "MISSING / STALE · no exporter network counters returned".into()
            }
            Ok(metrics) => {
                s.metric_status =
                    "CONNECTED · exporter network counters (sample age unknown)".into();
                s.metrics = metrics;
            }
            Err(e) => s.metric_status = format!("DISCONNECTED · Prometheus: {e}"),
        },
    }
}

fn event_age(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.elapsed().ok())
        .map(|d| d.as_secs())
}

fn load(run: &Path, prometheus: Option<&str>) -> (Snapshot, Offsets) {
    let mut s = Snapshot::default();
    let mut offsets = Offsets::default();
    s.scenario = read_json(&run.join("scenario.json"), &mut s.errors);
    let path = run.join("events.jsonl");
    match read_jsonl(&path, 0, &mut offsets.events_errors) {
        Ok((events, offset, lines)) => {
            s.event_age = event_age(&path);
            s.events = events;
            offsets.events = offset;
            offsets.events_lines = lines;
        }
        Err(e) => s.errors.push(format!("{}: {e}", path.display())),
    }
    s.errors.extend(offsets.events_errors.iter().cloned());
    let path = run.join("telemetry.jsonl");
    match read_jsonl(&path, 0, &mut offsets.telemetry_errors) {
        Ok((telemetry, offset, lines)) => {
            s.telemetry = telemetry;
            offsets.telemetry = offset;
            offsets.telemetry_lines = lines;
        }
        Err(e) => s.telemetry_error = Some(format!("{}: {e}", path.display())),
    }
    s.errors.extend(offsets.telemetry_errors.iter().cloned());
    update_metrics(&mut s, prometheus);
    (s, offsets)
}

/// Refresh in place, parsing only lines appended since the previous refresh.
fn refresh(s: &mut Snapshot, offsets: &mut Offsets, run: &Path, prometheus: Option<&str>) {
    let mut errors = Vec::new();
    s.telemetry_error = None;
    s.scenario = read_json(&run.join("scenario.json"), &mut errors);
    let path = run.join("events.jsonl");
    match append_jsonl(
        &path,
        offsets.events,
        offsets.events_lines,
        &mut offsets.events_errors,
    ) {
        Ok(Some((events, offset, lines))) => {
            s.events.extend(events);
            offsets.events = offset;
            offsets.events_lines += lines;
            s.event_age = event_age(&path);
        }
        Ok(None) => {
            offsets.events_errors.clear();
            match read_jsonl(&path, 0, &mut offsets.events_errors) {
                Ok((events, offset, lines)) => {
                    s.events = events;
                    offsets.events = offset;
                    offsets.events_lines = lines;
                    s.event_age = event_age(&path);
                }
                Err(e) => {
                    s.events.clear();
                    s.event_age = None;
                    offsets.events = 0;
                    offsets.events_lines = 0;
                    offsets.events_errors.clear();
                    errors.push(format!("{}: {e}", path.display()));
                }
            }
        }
        Err(e) => {
            s.events.clear();
            s.event_age = None;
            offsets.events = 0;
            offsets.events_lines = 0;
            offsets.events_errors.clear();
            errors.push(format!("{}: {e}", path.display()));
        }
    }
    let path = run.join("telemetry.jsonl");
    match append_jsonl(
        &path,
        offsets.telemetry,
        offsets.telemetry_lines,
        &mut offsets.telemetry_errors,
    ) {
        Ok(Some((telemetry, offset, lines))) => {
            s.telemetry.extend(telemetry);
            offsets.telemetry = offset;
            offsets.telemetry_lines += lines;
        }
        Ok(None) => {
            offsets.telemetry_errors.clear();
            match read_jsonl(&path, 0, &mut offsets.telemetry_errors) {
                Ok((telemetry, offset, lines)) => {
                    s.telemetry = telemetry;
                    offsets.telemetry = offset;
                    offsets.telemetry_lines = lines;
                }
                Err(e) => {
                    s.telemetry.clear();
                    offsets.telemetry = 0;
                    offsets.telemetry_lines = 0;
                    offsets.telemetry_errors.clear();
                    s.telemetry_error = Some(format!("{}: {e}", path.display()));
                }
            }
        }
        Err(e) => {
            s.telemetry.clear();
            offsets.telemetry = 0;
            offsets.telemetry_lines = 0;
            offsets.telemetry_errors.clear();
            s.telemetry_error = Some(format!("{}: {e}", path.display()));
        }
    }
    errors.extend(offsets.events_errors.iter().cloned());
    errors.extend(offsets.telemetry_errors.iter().cloned());
    s.errors = errors;
    update_metrics(s, prometheus);
}

// Query only an explicitly supplied local HTTP endpoint. The exporter runs in
// its own network namespace, so counters are not per-emulated-node measurements.
fn query_metrics(addr: &str) -> Result<Vec<String>, String> {
    let addr = addr
        .strip_prefix("http://")
        .unwrap_or(addr)
        .trim_end_matches('/');
    if !(addr.starts_with("127.0.0.1:") || addr.starts_with("localhost:")) || addr.contains('/') {
        return Err("use localhost:PORT or 127.0.0.1:PORT".into());
    }
    let mut stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    write!(stream, "GET /api/v1/query?query=node_network_receive_bytes_total%7Bjob%3D%22research-host%22%7D HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| e.to_string())?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or("invalid HTTP response")?;
    if !headers.lines().next().unwrap_or("").contains(" 200 ") {
        return Err(headers.lines().next().unwrap_or("HTTP error").into());
    }
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if v["status"] != "success" {
        return Err(format!("query failed: {}", v["error"]));
    }
    let results = v["data"]["result"]
        .as_array()
        .ok_or("missing query results")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    Ok(results
        .iter()
        .map(|row| {
            let device = row["metric"]["device"].as_str().unwrap_or("unknown device");
            let instance = row["metric"]["instance"]
                .as_str()
                .unwrap_or("unknown instance");
            let value = row["value"][1].as_str().unwrap_or("missing");
            let age = row["value"][0].as_f64().map(|t| (now - t).max(0.0) as u64);
            let freshness = match age {
                Some(a) => format!("query evaluated {a}s ago; sample age unknown"),
                None => "query timestamp missing".into(),
            };
            format!("{instance} / {device}: {value} bytes received · {freshness}")
        })
        .collect())
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "missing".into(),
        Some(x) => x.to_string(),
    }
}
fn timestamp(v: &Value) -> String {
    text(v, "timestamp_ms")
}
fn event_line(v: &Value) -> String {
    match v["event"].as_str().unwrap_or("unknown") {
        "start" => format!("{}  START   {}", timestamp(v), text(v, "scenario")),
        "step" => format!(
            "{}  STEP {}  {}  exit={}  {}ms  attempt {}  {}",
            timestamp(v),
            text(v, "index"),
            text(v, "node"),
            text(v, "exit_code"),
            text(v, "duration_ms"),
            text(v, "attempt"),
            text(v, "command")
        ),
        "finish" => format!(
            "{}  FINISH  success={}  error={}",
            timestamp(v),
            text(v, "success"),
            text(v, "error")
        ),
        _ => format!("{}  {}", timestamp(v), v),
    }
}
fn state(s: &Snapshot) -> (&'static str, Color) {
    if let Some(finish) = s.events.iter().rev().find(|e| e["event"] == "finish") {
        return if finish["success"] == true {
            ("COMPLETED", Color::Green)
        } else {
            ("FAILED", Color::Red)
        };
    }
    if s.events.iter().any(|e| e["event"] == "start") {
        if s.event_age.unwrap_or(u64::MAX) > STALE_SECS {
            ("STALE / STATUS UNKNOWN", Color::Yellow)
        } else {
            ("STARTED / NO FINISH", Color::Cyan)
        }
    } else {
        ("DISCONNECTED / NO START", Color::Red)
    }
}
fn block(title: &str, active: bool) -> Block<'_> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if active { Color::Cyan } else { Color::DarkGray }))
}
fn lines_paragraph(lines: Vec<String>, title: &str, scroll: usize, active: bool) -> Paragraph<'_> {
    Paragraph::new(lines.join("\n"))
        .block(block(title, active))
        .scroll((scroll.min(u16::MAX as usize) as u16, 0))
        .wrap(Wrap { trim: false })
}
fn node_lines(s: &Snapshot) -> Vec<String> {
    let mut nodes = BTreeMap::<String, (String, usize, usize, u64)>::new();
    if let Some(items) = s.scenario.as_ref().and_then(|v| v["nodes"].as_array()) {
        for n in items {
            nodes.insert(text(n, "name"), (text(n, "kind"), 0, 0, 0));
        }
    }
    for e in s.events.iter().filter(|e| e["event"] == "step") {
        let item = nodes
            .entry(text(e, "node"))
            .or_insert(("unknown".into(), 0, 0, 0));
        item.1 += 1;
        if e["success"] == false {
            item.2 += 1;
        }
        item.3 += e["duration_ms"].as_u64().unwrap_or(0);
    }
    if nodes.is_empty() {
        nodes.extend(
            s.telemetry
                .iter()
                .map(|sample| (text(sample, "node"), ("unknown".into(), 0, 0, 0))),
        );
    }
    let mut out = vec!["NODE                 KIND        ATTEMPTS   FAILED   DURATION".into()];
    out.extend(
        nodes
            .into_iter()
            .map(|(name, (kind, count, failed, duration))| {
                format!("{name:<20} {kind:<11} {count:<10} {failed:<8} {duration} ms")
            }),
    );
    out.push(String::new());
    out.push("Attempt counts and duration are derived from runner step events.".into());
    out.extend(probe_lines(s, None));
    out
}
fn probe_lines(s: &Snapshot, filter_node: Option<&str>) -> Vec<String> {
    let mut out = vec!["Latest per-node probes · startup, step attempts and hold sampling".into()];
    let mut latest = BTreeMap::new();
    for sample in &s.telemetry {
        if filter_node.is_some_and(|node| sample["node"].as_str() != Some(node)) {
            continue;
        }
        latest.insert((text(sample, "node"), text(sample, "probe")), sample);
    }
    if let Some(error) = &s.telemetry_error {
        out.push(format!("MISSING / DISCONNECTED · {error}"));
    }
    if latest.is_empty() {
        out.push("MISSING · no complete telemetry samples".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    for ((node, probe), sample) in latest {
        let freshness = match sample["timestamp_ms"].as_u64() {
            Some(ms) if u128::from(ms) > now => "TIMESTAMP IN FUTURE".to_string(),
            Some(ms) => {
                let age = (now - u128::from(ms)) / 1000;
                if age > u128::from(STALE_SECS) {
                    format!("STALE · {age}s ago")
                } else {
                    format!("{age}s ago")
                }
            }
            None => "TIMESTAMP MISSING".to_string(),
        };
        out.push(format!(
            "\n── {node} / {probe} · {freshness} · exit {} ──",
            text(sample, "exit_code")
        ));
        out.push(format!("$ {}", text(sample, "command")));
        match sample["stdout"].as_str() {
            Some(stdout) if !stdout.is_empty() => out.extend(stdout.lines().map(str::to_owned)),
            Some(_) => out.push("(stdout empty)".into()),
            None => out.push("MISSING · stdout".into()),
        }
        match sample["stderr"].as_str() {
            Some(stderr) if !stderr.is_empty() => {
                out.push("stderr:".into());
                out.extend(stderr.lines().map(str::to_owned));
            }
            None => out.push("MISSING · stderr".into()),
            _ => {}
        }
    }
    out
}
fn steps(s: &Snapshot) -> Vec<&Value> {
    s.events.iter().filter(|e| e["event"] == "step").collect()
}
fn overview(s: &Snapshot, run: &Path) -> Vec<String> {
    let scenario = s.scenario.as_ref();
    let events = steps(s);
    let failures = events.iter().filter(|e| e["success"] == false).count();
    let mut out = vec![
        format!(
            "SCENARIO    {}",
            scenario
                .map(|v| text(v, "name"))
                .unwrap_or_else(|| "MISSING".into())
        ),
        format!("RUN         {}", run.display()),
        format!("STATE       {}", state(s).0),
        format!(
            "EVENT AGE   {}",
            s.event_age
                .map(|a| format!("{a}s since file update"))
                .unwrap_or_else(|| "MISSING".into())
        ),
        format!(
            "ATTEMPTS    {} recorded · {} failed",
            events.len(),
            failures
        ),
        format!(
            "TOPOLOGY    {} nodes · {} links",
            scenario
                .and_then(|v| v["nodes"].as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "missing".into()),
            scenario
                .and_then(|v| v["links"].as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "missing".into())
        ),
        String::new(),
        format!("METRICS     {}", s.metric_status),
    ];
    out.extend(s.metrics.iter().cloned());
    out.push(String::new());
    out.push(format!(
        "NODE PROBES {} recorded · see Nodes for latest interface and routing output",
        s.telemetry.len()
    ));
    if !s.errors.is_empty() {
        out.push(String::new());
        out.push("READ ERRORS".into());
        out.extend(s.errors.iter().cloned());
    }
    out
}
fn frr_log_lines(run: &Path, scenario: &str, node: &str) -> Vec<String> {
    let expected = format!("mnr-{scenario}-{node}");
    let frr_root = run.join("frr");
    let mut out = Vec::new();
    match fs::read_dir(&frr_root) {
        Ok(entries) => {
            let mut directories: Vec<_> = entries
                .filter_map(Result::ok)
                .filter(|e| e.file_name().as_os_str() == std::ffi::OsStr::new(&expected))
                .collect();
            if let Some(dir) = directories.pop() {
                match fs::read_dir(dir.path()) {
                    Ok(files) => {
                        let mut paths: Vec<_> = files
                            .filter_map(Result::ok)
                            .map(|e| e.path())
                            .filter(|p| p.extension().is_some_and(|ext| ext == "log"))
                            .collect();
                        paths.sort();
                        if paths.is_empty() {
                            out.push(
                                "\nFRR logs: MISSING · no copied log files for this node".into(),
                            );
                        }
                        for path in paths {
                            out.push(format!("\n── FRR log · {} ──", path.display()));
                            match fs::read_to_string(&path) {
                                Ok(data) if data.is_empty() => out.push("(empty)".into()),
                                Ok(data) => out.extend(data.lines().map(str::to_owned)),
                                Err(e) => out.push(format!("ERROR · {e}")),
                            }
                        }
                    }
                    Err(e) => out.push(format!("\nFRR logs: ERROR · {e}")),
                }
            } else {
                out.push("\nFRR logs: MISSING · no copied logs for this node".into());
            }
        }
        Err(e) => out.push(format!("\nFRR logs: MISSING / ERROR · {e}")),
    }
    out
}

fn logs(app: &App) -> Vec<String> {
    let steps = steps(&app.snapshot);
    if steps.is_empty() {
        return vec!["MISSING · no recorded step output yet".into()];
    }
    let selected = app.selection.min(steps.len() - 1);
    let step = steps[selected];
    let Some(index) = step["index"].as_u64() else {
        return vec!["ERROR · step event has no valid index".into()];
    };
    let Some(attempt) = step["attempt"].as_u64().filter(|n| *n > 0) else {
        return vec!["ERROR · step event has no valid attempt number".into()];
    };
    let mut out = vec![format!(
        "ATTEMPT {} of {} · {}",
        selected + 1,
        steps.len(),
        event_line(step)
    )];
    for suffix in ["stdout", "stderr", "json"] {
        let path = app
            .run
            .join("steps")
            .join(format!("{index:04}-attempt-{attempt:04}.{suffix}"));
        out.push(format!("\n── {suffix} · {} ──", path.display()));
        match fs::read_to_string(&path) {
            Ok(data) if !data.is_empty() => out.extend(data.lines().map(str::to_owned)),
            Ok(_) => out.push("(empty)".into()),
            Err(e) => out.push(format!("MISSING / ERROR · {e}")),
        }
    }
    let scenario = app
        .snapshot
        .scenario
        .as_ref()
        .map(|s| text(s, "name"))
        .unwrap_or_default();
    out.extend(frr_log_lines(
        &app.run,
        &scenario,
        text(step, "node").as_str(),
    ));
    out.push("\nLatest probes for this node (not necessarily from the selected attempt):".into());
    out.extend(probe_lines(&app.snapshot, step["node"].as_str()));
    out
}
fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("Terminal too small · resize (minimum 30 × 8)\nq quit"),
            area,
        );
        return;
    }
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    let (status, color) = state(&app.snapshot);
    let name = app
        .snapshot
        .scenario
        .as_ref()
        .map(|v| text(v, "name"))
        .unwrap_or_else(|| "NO SCENARIO".into());
    let header = Line::from(vec![
        Span::styled(
            " RESEARCH / MONITOR ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {name}  ")),
        Span::styled(
            status,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(if app.paused { "  PAUSED" } else { "" }),
    ]);
    frame.render_widget(
        Paragraph::new(header).block(block("Experiment", false)),
        areas[0],
    );
    let names = ["1 Overview", "2 Nodes", "3 Events", "4 Logs"];
    let tabs = Line::from(
        names
            .iter()
            .enumerate()
            .flat_map(|(i, n)| {
                [
                    Span::styled(
                        format!(" {n} "),
                        if i == app.tab {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                    Span::raw(" "),
                ]
            })
            .collect::<Vec<_>>(),
    );
    frame.render_widget(Paragraph::new(tabs).block(block("Views", false)), areas[1]);
    let (title, data) = match app.tab {
        0 => ("Run & telemetry", overview(&app.snapshot, &app.run)),
        1 => ("Node activity", node_lines(&app.snapshot)),
        2 => (
            "Event stream · epoch milliseconds",
            if app.snapshot.events.is_empty() {
                vec!["MISSING · no complete events recorded".into()]
            } else {
                app.snapshot.events.iter().map(event_line).collect()
            },
        ),
        _ => ("Step output · ←/→ select attempt", logs(app)),
    };
    frame.render_widget(lines_paragraph(data, title, app.scroll, true), areas[2]);
    frame.render_widget(
        Paragraph::new(" Tab views  ↑/↓ scroll  ←/→ step  p pause  r refresh  ? help  q quit")
            .style(Style::default().fg(Color::Gray)),
        areas[3],
    );
    if app.help {
        let width = area.width.saturating_sub(4).min(70);
        let height = area.height.saturating_sub(2).min(13);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(HELP)
                .block(block("Keyboard help", true))
                .wrap(Wrap { trim: false }),
            rect,
        );
    }
}
fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let mut run = None;
    let mut prometheus = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("Usage: research-monitor RUN_DIRECTORY [--prometheus localhost:9090]\nRead-only local experiment monitor. Press ? for keys.");
                return Ok(());
            }
            "--prometheus" => {
                prometheus = Some(args.next().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--prometheus requires localhost:PORT",
                    )
                })?);
            }
            _ if arg.starts_with('-') || run.is_some() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unexpected argument: {arg}"),
                ))
            }
            _ => run = Some(PathBuf::from(arg)),
        }
    }
    let run = run.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "supply a run directory; use --help",
        )
    })?;
    let (snapshot, offsets) = load(&run, prometheus.as_deref());
    let mut app = App {
        run,
        prometheus,
        snapshot,
        offsets,
        tab: 0,
        selection: 0,
        scroll: 0,
        help: false,
        paused: false,
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "research-monitor needs an interactive terminal",
        ));
    }
    let mut terminal = ratatui::init();
    let result = (|| -> io::Result<()> {
        let mut last_refresh = std::time::Instant::now();
        loop {
            terminal.draw(|f| draw(f, &app))?;
            if event::poll(Duration::from_millis(150))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if app.help {
                        match key.code {
                            KeyCode::Char('?') | KeyCode::Esc => app.help = false,
                            KeyCode::Char('q') => break,
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('?') => app.help = true,
                        KeyCode::Char('p') => app.paused = !app.paused,
                        KeyCode::Char('r') => {
                            refresh(
                                &mut app.snapshot,
                                &mut app.offsets,
                                &app.run,
                                app.prometheus.as_deref(),
                            );
                            last_refresh = std::time::Instant::now();
                        }
                        KeyCode::Tab => {
                            app.tab = (app.tab + 1) % 4;
                            app.scroll = 0;
                        }
                        KeyCode::BackTab => {
                            app.tab = (app.tab + 3) % 4;
                            app.scroll = 0;
                        }
                        KeyCode::Char(c @ '1'..='4') => {
                            app.tab = c as usize - '1' as usize;
                            app.scroll = 0;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.scroll = app.scroll.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.scroll = app.scroll.saturating_add(1)
                        }
                        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
                        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
                        KeyCode::Left | KeyCode::Char('h') if app.tab == 3 => {
                            app.selection = app.selection.saturating_sub(1);
                            app.scroll = 0;
                        }
                        KeyCode::Right | KeyCode::Char('l') if app.tab == 3 => {
                            app.selection = (app.selection + 1)
                                .min(steps(&app.snapshot).len().saturating_sub(1));
                            app.scroll = 0;
                        }
                        _ => {}
                    }
                }
            }
            if !app.paused && last_refresh.elapsed() >= REFRESH {
                refresh(
                    &mut app.snapshot,
                    &mut app.offsets,
                    &app.run,
                    app.prometheus.as_deref(),
                );
                last_refresh = std::time::Instant::now();
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jsonl_reader_skips_partial_tail_and_reports_bad_line() {
        let path = std::env::temp_dir().join(format!(
            "research-monitor-jsonl-{}.jsonl",
            std::process::id()
        ));
        fs::write(
            &path,
            "{\"event\":\"start\"}\nnot json\n{\"event\":\"partial\"",
        )
        .unwrap();
        let mut errors = Vec::new();
        let (values, _, _) = read_jsonl(&path, 0, &mut errors).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(values.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("line 2"), "{errors:?}");
    }

    #[test]
    fn jsonl_reader_accepts_valid_final_line_without_newline() {
        let path = std::env::temp_dir().join(format!(
            "research-monitor-no-newline-{}.jsonl",
            std::process::id()
        ));
        let data = "{\"event\":\"start\"}";
        fs::write(&path, data).unwrap();
        let mut errors = Vec::new();
        let (values, consumed, lines) = read_jsonl(&path, 0, &mut errors).unwrap();
        let _ = fs::remove_file(&path);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(values.len(), 1);
        assert_eq!(consumed, data.len() as u64);
        assert_eq!(lines, 0, "unterminated tail is not yet a counted line");
    }

    #[test]
    fn refresh_counts_the_terminating_newline_once() {
        let dir = std::env::temp_dir().join(format!(
            "research-monitor-newline-count-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scenario.json"), "{}").unwrap();
        fs::write(dir.join("events.jsonl"), "{\"a\":1}").unwrap();
        fs::write(dir.join("telemetry.jsonl"), "{}\n").unwrap();
        let (mut snapshot, mut offsets) = load(&dir, None);
        let append = |text: &str| {
            fs::OpenOptions::new()
                .append(true)
                .open(dir.join("events.jsonl"))
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        };
        append("\n");
        refresh(&mut snapshot, &mut offsets, &dir, None);
        append("not json\n");
        refresh(&mut snapshot, &mut offsets, &dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("events.jsonl line 2")),
            "{:?}",
            snapshot.errors
        );
    }

    #[test]
    fn truncation_drops_errors_from_the_previous_file() {
        let dir = std::env::temp_dir().join(format!(
            "research-monitor-truncate-errors-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scenario.json"), "{}").unwrap();
        fs::write(dir.join("events.jsonl"), "not json\n{\"event\":\"a\"}\n").unwrap();
        fs::write(dir.join("telemetry.jsonl"), "not json\n{\"probe\":\"a\"}\n").unwrap();
        let (mut snapshot, mut offsets) = load(&dir, None);
        fs::write(dir.join("events.jsonl"), "{\"event\":\"clean\"}\n").unwrap();
        fs::write(dir.join("telemetry.jsonl"), "{\"probe\":\"clean\"}\n").unwrap();
        refresh(&mut snapshot, &mut offsets, &dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0]["event"], "clean");
        assert_eq!(snapshot.telemetry.len(), 1);
        assert!(
            !snapshot
                .errors
                .iter()
                .any(|e| e.contains("events.jsonl") || e.contains("telemetry.jsonl")),
            "{:?}",
            snapshot.errors
        );
    }

    #[test]
    fn load_and_refresh_report_parse_errors_under_each_file() {
        let dir = std::env::temp_dir().join(format!(
            "research-monitor-load-parse-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scenario.json"), "{}").unwrap();
        fs::write(
            dir.join("events.jsonl"),
            "{\"event\":\"start\"}\nnot json\n",
        )
        .unwrap();
        fs::write(dir.join("telemetry.jsonl"), "not json\n").unwrap();
        let (mut snapshot, mut offsets) = load(&dir, None);
        refresh(&mut snapshot, &mut offsets, &dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(snapshot.events.len(), 1);
        assert!(snapshot.telemetry.is_empty());
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("events.jsonl line 2")),
            "{:?}",
            snapshot.errors
        );
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("telemetry.jsonl line 1")),
            "{:?}",
            snapshot.errors
        );
    }

    #[test]
    fn load_routes_missing_file_errors_to_the_matching_channel() {
        let dir = std::env::temp_dir().join(format!(
            "research-monitor-load-missing-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let (snapshot, _) = load(&dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(snapshot.event_age, None);
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("events.jsonl") && e.contains("No such file")),
            "{:?}",
            snapshot.errors
        );
        assert!(
            snapshot
                .telemetry_error
                .as_deref()
                .is_some_and(|e| e.contains("telemetry.jsonl") && e.contains("No such file")),
            "{:?}",
            snapshot.telemetry_error
        );
    }

    #[test]
    fn refresh_appends_new_lines_and_reloads_after_truncation() {
        let dir =
            std::env::temp_dir().join(format!("research-monitor-refresh-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scenario.json"), "{}").unwrap();
        fs::write(dir.join("events.jsonl"), "{\"event\":\"start\"}\n").unwrap();
        fs::write(dir.join("telemetry.jsonl"), "{\"probe\":\"a\"}\n").unwrap();
        let (mut snapshot, mut offsets) = load(&dir, None);
        let append = |file: &str, text: &str| {
            let mut f = fs::OpenOptions::new()
                .append(true)
                .open(dir.join(file))
                .unwrap();
            f.write_all(text.as_bytes()).unwrap();
        };

        append("events.jsonl", "{\"event\":\"step\"}\n");
        append("telemetry.jsonl", "{\"probe\":\"b\"}\n");
        refresh(&mut snapshot, &mut offsets, &dir, None);
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.telemetry.len(), 2);

        append("events.jsonl", "{\"event\":\"partial\"");
        refresh(&mut snapshot, &mut offsets, &dir, None);
        assert_eq!(snapshot.events.len(), 2);

        append("events.jsonl", "}\n");
        refresh(&mut snapshot, &mut offsets, &dir, None);
        assert_eq!(snapshot.events.len(), 3);

        fs::write(dir.join("events.jsonl"), "{\"event\":\"fresh\"}\n").unwrap();
        refresh(&mut snapshot, &mut offsets, &dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0]["event"], "fresh");
    }

    #[test]
    fn refresh_reports_absolute_parse_error_lines() {
        let dir = std::env::temp_dir().join(format!(
            "research-monitor-line-numbers-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scenario.json"), "{}").unwrap();
        fs::write(dir.join("events.jsonl"), "{}\n{}\n").unwrap();
        fs::write(dir.join("telemetry.jsonl"), "{}\n").unwrap();
        let (mut snapshot, mut offsets) = load(&dir, None);
        fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(dir.join("telemetry.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        refresh(&mut snapshot, &mut offsets, &dir, None);
        let _ = fs::remove_dir_all(&dir);
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("events.jsonl line 3")),
            "{:?}",
            snapshot.errors
        );
        assert!(
            snapshot
                .errors
                .iter()
                .any(|e| e.contains("telemetry.jsonl line 2")),
            "{:?}",
            snapshot.errors
        );
    }

    #[test]
    fn unfinished_old_run_is_not_claimed_alive() {
        let snapshot = Snapshot {
            events: vec![json!({"event":"start", "scenario":"test"})],
            event_age: Some(31),
            ..Snapshot::default()
        };
        assert_eq!(state(&snapshot).0, "STALE / STATUS UNKNOWN");
    }

    #[test]
    fn finish_reports_failure() {
        let snapshot = Snapshot {
            events: vec![
                json!({"event":"start"}),
                json!({"event":"finish","success":false}),
            ],
            ..Snapshot::default()
        };
        assert_eq!(state(&snapshot).0, "FAILED");
    }

    #[test]
    fn nodes_include_recorded_attempts_without_invented_protocol_stats() {
        let snapshot = Snapshot {
            scenario: Some(json!({"nodes":[{"name":"r1","kind":"host"}]})),
            events: vec![json!({"event":"step","node":"r1","success":false,"duration_ms":12})],
            ..Snapshot::default()
        };
        let output = node_lines(&snapshot).join("\n");
        assert!(output.contains("r1"));
        assert!(output.contains("12 ms"));
        assert!(output.contains("MISSING · no complete telemetry samples"));
    }

    #[test]
    fn probes_show_latest_values_and_errors_for_selected_node() {
        let snapshot = Snapshot {
            telemetry: vec![
                json!({"node":"r1","probe":"ip_link","timestamp_ms":1,"exit_code":0,"command":"ip -s link","stdout":"old counter","stderr":""}),
                json!({"node":"r1","probe":"ip_link","timestamp_ms":2,"exit_code":1,"command":"ip -s link","stdout":"new counter","stderr":"probe failed"}),
                json!({"node":"r2","probe":"ip_link","timestamp_ms":2,"exit_code":0,"command":"ip -s link","stdout":"other node","stderr":""}),
            ],
            ..Snapshot::default()
        };
        let output = probe_lines(&snapshot, Some("r1")).join("\n");
        assert!(output.contains("new counter"));
        assert!(output.contains("STALE"));
        assert!(output.contains("probe failed"));
        assert!(!output.contains("old counter"));
        assert!(!output.contains("other node"));
    }
}
