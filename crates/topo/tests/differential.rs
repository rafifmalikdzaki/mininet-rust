//! Differential test: run the ORIGINAL `mininet/topo.py` + `mininet/topolib.py`
//! under `python3` (the repo's `mininet` package, driven by `tests/py_driver.py`)
//! on generated topos and raw `MultiGraph` op sequences, and assert that the
//! Rust port in `mn-topo` produces byte-identical canonical dumps.
//!
//! Protocol: cases are sent to the driver as JSON on stdin; the driver returns
//! a JSON array of `{"ok": "<dump>"}` / `{"error": "<ExceptionClass>"}`. The
//! Rust side renders the same dump (tests/common/mod.rs) and the same Python
//! exception class (`TopoError::python_exception`), and the two are compared.

mod common;

use common::{dump_graph, dump_topo};
use mn_topo::{
    build_topo, Attrs, BuiltTopo, EdgeKey, LinearTopo, MinimalTopo, MultiGraph,
    SingleSwitchReversedTopo, SingleSwitchTopo, TopoOptions, TorusTopo, TreeTopo, Value,
};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Minimal JSON (std only): enough for the case/result protocol.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get<'a>(&'a self, key: &str) -> Option<&'a Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("expected string, got {self:?}"),
        }
    }
}

fn s(v: &str) -> Json {
    Json::Str(v.to_string())
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn ser_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn ser(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Json::Float(f) => {
            let _ = write!(out, "{f:?}");
        }
        Json::Str(x) => ser_str(x, out),
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                ser(item, out);
            }
            out.push(']');
        }
        Json::Obj(kv) => {
            out.push('{');
            for (i, (k, val)) in kv.iter().enumerate() {
                if i > 0 {
                    out.push(',')
                }
                ser_str(k, out);
                out.push(':');
                ser(val, out);
            }
            out.push('}');
        }
    }
}

fn skip_ws(s: &str, pos: &mut usize) {
    while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

fn parse_string(s: &str, pos: &mut usize) -> Option<String> {
    if !s[*pos..].starts_with('"') {
        return None;
    }
    *pos += 1;
    let mut out = String::new();
    loop {
        let c = s[*pos..].chars().next()?;
        *pos += c.len_utf8();
        match c {
            '"' => return Some(out),
            '\\' => {
                let e = s[*pos..].chars().next()?;
                *pos += e.len_utf8();
                match e {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex1 = s.get(*pos..*pos + 4)?;
                        let mut cp = u32::from_str_radix(hex1, 16).ok()?;
                        *pos += 4;
                        if (0xd800..0xdc00).contains(&cp) {
                            let hex2 = s.get(*pos + 2..*pos + 6)?;
                            let lo = u32::from_str_radix(hex2, 16).ok()?;
                            *pos += 6;
                            cp = 0x1_0000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                        }
                        out.push(char::from_u32(cp)?);
                    }
                    _ => return None,
                }
            }
            c => out.push(c),
        }
    }
}

fn parse_number(s: &str, pos: &mut usize) -> Option<Json> {
    let start = *pos;
    if s[*pos..].starts_with('-') {
        *pos += 1;
    }
    let mut is_float = false;
    while *pos < s.len() {
        match s.as_bytes()[*pos] {
            b'0'..=b'9' => *pos += 1,
            b'.' | b'e' | b'E' => {
                is_float = true;
                *pos += 1;
            }
            b'+' | b'-' if matches!(s.as_bytes()[*pos - 1], b'e' | b'E') => *pos += 1,
            _ => break,
        }
    }
    let text = &s[start..*pos];
    if is_float {
        text.parse::<f64>().ok().map(Json::Float)
    } else {
        text.parse::<i64>().ok().map(Json::Int)
    }
}

fn parse_value(s: &str, pos: &mut usize) -> Option<Json> {
    skip_ws(s, pos);
    let c = s[*pos..].chars().next()?;
    match c {
        'n' => {
            s[*pos..].starts_with("null").then(|| *pos += 4)?;
            Some(Json::Null)
        }
        't' => {
            s[*pos..].starts_with("true").then(|| *pos += 4)?;
            Some(Json::Bool(true))
        }
        'f' => {
            s[*pos..].starts_with("false").then(|| *pos += 5)?;
            Some(Json::Bool(false))
        }
        '"' => parse_string(s, pos).map(Json::Str),
        '[' => {
            *pos += 1;
            let mut items = Vec::new();
            skip_ws(s, pos);
            if s[*pos..].starts_with(']') {
                *pos += 1;
                return Some(Json::Arr(items));
            }
            loop {
                items.push(parse_value(s, pos)?);
                skip_ws(s, pos);
                match s[*pos..].chars().next()? {
                    ',' => *pos += 1,
                    ']' => {
                        *pos += 1;
                        return Some(Json::Arr(items));
                    }
                    _ => return None,
                }
            }
        }
        '{' => {
            *pos += 1;
            let mut kv = Vec::new();
            skip_ws(s, pos);
            if s[*pos..].starts_with('}') {
                *pos += 1;
                return Some(Json::Obj(kv));
            }
            loop {
                skip_ws(s, pos);
                let key = parse_string(s, pos)?;
                skip_ws(s, pos);
                if !s[*pos..].starts_with(':') {
                    return None;
                }
                *pos += 1;
                kv.push((key, parse_value(s, pos)?));
                skip_ws(s, pos);
                match s[*pos..].chars().next()? {
                    ',' => *pos += 1,
                    '}' => {
                        *pos += 1;
                        return Some(Json::Obj(kv));
                    }
                    _ => return None,
                }
            }
        }
        '-' => parse_number(s, pos),
        _ if c.is_ascii_digit() => parse_number(s, pos),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Case construction
// ---------------------------------------------------------------------------

fn topo_spec_cases() -> Vec<Json> {
    let specs = [
        // fixtures
        "single,3",
        "linear,4",
        "tree,2,2",
        // defaults
        "minimal",
        "single",
        "reversed",
        "linear",
        "tree",
        // edge cases
        "single,1",
        "linear,1",
        "tree,1,1",
        "tree,3,4",
        // varying fanout/depth/counts (natural-sort stress: >= 10 nodes)
        "single,10",
        "single,12",
        "reversed,1",
        "reversed,4",
        "reversed,10",
        "linear,1,3",
        "linear,3,2",
        "linear,10",
        "linear,10,2",
        "tree,1,3",
        "tree,2,3",
        "tree,3,1",
        "tree,3,3",
        "tree,4,3",
        "torus,3,3",
        "torus,3,4",
        "torus,4,3,2",
        "torus,3,3,2",
        "torus,5,4,1",
        // error cases
        "torus,2,2",
        "torus,2,3",
        "torus,3",
        "torus",
        "bogus,1",
        "bogus",
        "single,3,foo=1", // swallowed by build(**_opts)
        "linear,2,foo=1",
        "tree,2,2,foo=1",   // TypeError: TreeTopo.build takes no kwargs
        "minimal,1",        // TypeError: too many args
        "single,2.5",       // TypeError: non-int k
        "single,3,hopts=1", // TypeError: scalar hopts popped by __init__
        "tree,1,2,3",       // TypeError: too many args
    ];
    specs
        .iter()
        .map(|spec| obj(vec![("fn", s("topo")), ("spec", s(spec))]))
        .collect()
}

fn make_cases() -> Vec<Json> {
    vec![
        // hopts/sopts/lopts applied through the constructor params
        obj(vec![
            ("fn", s("make")),
            ("topo", s("single")),
            ("args", Json::Arr(vec![Json::Int(4)])),
            (
                "kwargs",
                obj(vec![
                    ("hopts", obj(vec![("ip", s("10.0.0.1/8"))])),
                    ("sopts", obj(vec![("dpid", s("abc"))])),
                    (
                        "lopts",
                        obj(vec![("bw", Json::Int(10)), ("delay", s("5ms"))]),
                    ),
                ]),
            ),
        ]),
        // extra kwargs swallowed by build(**_opts) of single/linear
        obj(vec![
            ("fn", s("make")),
            ("topo", s("single")),
            ("args", Json::Arr(vec![Json::Int(3)])),
            ("kwargs", obj(vec![("foo", Json::Int(1))])),
        ]),
        obj(vec![
            ("fn", s("make")),
            ("topo", s("linear")),
            ("args", Json::Arr(vec![Json::Int(3), Json::Int(2)])),
            (
                "kwargs",
                obj(vec![("lopts", obj(vec![("bw", Json::Int(100))]))]),
            ),
        ]),
        // tree/reversed take no kwargs -> TypeError
        obj(vec![
            ("fn", s("make")),
            ("topo", s("tree")),
            ("args", Json::Arr(vec![Json::Int(2), Json::Int(2)])),
            ("kwargs", obj(vec![("foo", Json::Int(1))])),
        ]),
        // scalar hopts -> TypeError when unpacked as node opts
        obj(vec![
            ("fn", s("make")),
            ("topo", s("single")),
            ("args", Json::Arr(vec![Json::Int(2)])),
            ("kwargs", obj(vec![("hopts", Json::Int(5))])),
        ]),
        // non-int arg -> TypeError in irange
        obj(vec![
            ("fn", s("make")),
            ("topo", s("single")),
            ("args", Json::Arr(vec![Json::Float(2.5)])),
            ("kwargs", Json::Obj(vec![])),
        ]),
        // unknown name -> Exception
        obj(vec![
            ("fn", s("make")),
            ("topo", s("nope")),
            ("args", Json::Arr(vec![])),
            ("kwargs", Json::Obj(vec![])),
        ]),
    ]
}

fn graph_cases() -> Vec<Json> {
    let node = |n: &str, attrs: Json, extra: Json| {
        obj(vec![
            ("op", s("add_node")),
            ("node", s(n)),
            ("attrs", attrs),
            ("extra", extra),
        ])
    };
    let edge = |src: &str, dst: &str, key: Json, attrs: Json, extra: Json| {
        obj(vec![
            ("op", s("add_edge")),
            ("src", s(src)),
            ("dst", s(dst)),
            ("key", key),
            ("attrs", attrs),
            ("extra", extra),
        ])
    };
    let empty_obj = Json::Obj(vec![]);
    vec![obj(vec![
        ("fn", s("graph")),
        (
            "ops",
            Json::Arr(vec![
                // attr_dict + **attrs merge; re-adding a node replaces attrs
                node(
                    "a",
                    obj(vec![("x", Json::Int(1))]),
                    obj(vec![("y", Json::Int(2))]),
                ),
                node("b", Json::Null, empty_obj.clone()),
                node("a", obj(vec![("z", Json::Bool(true))]), empty_obj.clone()),
                // auto keys 1, 2, then reversed (b,a) shares the entry -> 3
                edge(
                    "a",
                    "b",
                    Json::Null,
                    obj(vec![("p", Json::Int(1))]),
                    obj(vec![("q", Json::Int(2))]),
                ),
                edge("a", "b", Json::Null, empty_obj.clone(), empty_obj.clone()),
                edge("b", "a", Json::Null, empty_obj.clone(), empty_obj.clone()),
                // explicit int key 7, then auto -> max(7)+1 = 8
                edge("a", "b", Json::Int(7), empty_obj.clone(), empty_obj.clone()),
                edge("a", "b", Json::Null, empty_obj.clone(), empty_obj.clone()),
                // string key, then auto (int keys only: 1,2,3,7,8 -> 9)
                edge("a", "b", s("k"), empty_obj.clone(), empty_obj.clone()),
                edge("a", "b", Json::Null, empty_obj.clone(), empty_obj.clone()),
                // explicit key re-used replaces attrs in place
                edge(
                    "a",
                    "b",
                    Json::Int(7),
                    obj(vec![("r", Json::Int(9))]),
                    empty_obj.clone(),
                ),
                // self loop
                edge("c", "c", Json::Null, empty_obj.clone(), empty_obj.clone()),
                // only string keys -> auto int key 1
                edge("x", "y", s("only"), empty_obj.clone(), empty_obj.clone()),
                edge("x", "y", Json::Null, empty_obj.clone(), empty_obj.clone()),
                // 'z' > 'a'/'b' so edges from z are skipped in edges_iter
                edge("z", "a", Json::Null, empty_obj.clone(), empty_obj.clone()),
            ]),
        ),
    ])]
}

// ---------------------------------------------------------------------------
// Rust-side case execution
// ---------------------------------------------------------------------------

fn value_from_json(v: &Json) -> Value {
    match v {
        Json::Null => Value::Str(String::new()),
        Json::Bool(b) => Value::Bool(*b),
        Json::Int(i) => Value::Int(*i),
        Json::Float(f) => Value::Float(*f),
        Json::Str(s) => Value::Str(s.clone()),
        other => panic!("non-scalar attr value {other:?}"),
    }
}

/// attrs dict from a JSON value: null/missing -> empty; object -> Attrs;
/// anything else -> Err (Python would fail unpacking/updating with it).
fn attrs_from_json(v: Option<&Json>) -> Result<Attrs, ()> {
    match v {
        None | Some(Json::Null) => Ok(Attrs::new()),
        Some(Json::Obj(kv)) => Ok(kv
            .iter()
            .map(|(k, val)| (k.clone(), value_from_json(val)))
            .collect()),
        Some(_) => Err(()),
    }
}

fn edge_key_from_json(v: Option<&Json>) -> Option<EdgeKey> {
    match v {
        None | Some(Json::Null) => None,
        Some(Json::Int(i)) => Some(EdgeKey::Int(*i)),
        Some(Json::Str(s)) => Some(EdgeKey::Str(s.clone())),
        Some(other) => panic!("bad edge key {other:?}"),
    }
}

fn dump_builtin(b: &BuiltTopo) -> String {
    let extras: Vec<(&'static str, String)> = match b {
        BuiltTopo::Minimal(t) => vec![("k", t.inner.k.to_string())],
        BuiltTopo::SingleSwitch(t) => vec![("k", t.k.to_string())],
        BuiltTopo::SingleSwitchReversed(t) => vec![("k", t.k.to_string())],
        BuiltTopo::Linear(t) => vec![("k", t.k.to_string()), ("n", t.n.to_string())],
        BuiltTopo::Tree(t) => vec![
            ("hostNum", t.host_num.to_string()),
            ("switchNum", t.switch_num.to_string()),
        ],
        BuiltTopo::Torus(_) => vec![],
    };
    dump_topo(b.class_name(), b.topo(), &extras)
}

fn make_topo(case: &Json) -> Result<BuiltTopo, &'static str> {
    let name = case.get("topo").map(Json::as_str).unwrap_or("");
    let args = match case.get("args") {
        Some(Json::Arr(a)) => a.clone(),
        _ => vec![],
    };
    let kwargs = case.get("kwargs").cloned().unwrap_or(Json::Obj(vec![]));
    let accepts_kwargs = matches!(name, "single" | "linear");
    let mut options = TopoOptions::default();
    if let Json::Obj(kv) = &kwargs {
        for (k, v) in kv {
            match k.as_str() {
                "hopts" | "sopts" | "lopts" => {
                    let a = attrs_from_json(Some(v)).map_err(|_| "TypeError")?;
                    match k.as_str() {
                        "hopts" => options.hopts = a,
                        "sopts" => options.sopts = a,
                        _ => options.lopts = a,
                    }
                }
                _ if !accepts_kwargs => return Err("TypeError"),
                _ => {}
            }
        }
    }
    let arg = |i: usize| -> Result<i64, &'static str> {
        match args.get(i) {
            Some(Json::Int(v)) => Ok(*v),
            _ => Err("TypeError"),
        }
    };
    let opt = |i: usize, default: i64| -> Result<i64, &'static str> {
        match args.get(i) {
            None => Ok(default),
            Some(Json::Int(v)) => Ok(*v),
            Some(_) => Err("TypeError"),
        }
    };
    match name {
        "minimal" => {
            if !args.is_empty() {
                return Err("TypeError");
            }
            Ok(BuiltTopo::Minimal(MinimalTopo::with_options(options)))
        }
        "single" => {
            if args.len() > 1 {
                return Err("TypeError");
            }
            Ok(BuiltTopo::SingleSwitch(SingleSwitchTopo::with_options(
                opt(0, 2)?,
                options,
            )))
        }
        "reversed" => {
            if args.len() > 1 {
                return Err("TypeError");
            }
            Ok(BuiltTopo::SingleSwitchReversed(
                SingleSwitchReversedTopo::with_options(opt(0, 2)?, options),
            ))
        }
        "linear" => {
            if args.len() > 2 {
                return Err("TypeError");
            }
            Ok(BuiltTopo::Linear(LinearTopo::with_options(
                opt(0, 2)?,
                opt(1, 1)?,
                options,
            )))
        }
        "tree" => {
            if args.len() > 2 {
                return Err("TypeError");
            }
            Ok(BuiltTopo::Tree(TreeTopo::with_options(
                opt(0, 1)?,
                opt(1, 2)?,
                options,
            )))
        }
        "torus" => {
            if args.len() > 3 {
                return Err("TypeError");
            }
            let x = arg(0)?;
            let y = arg(1)?;
            let n = opt(2, 1)?;
            TorusTopo::with_options(x, y, n, options)
                .map(BuiltTopo::Torus)
                .map_err(|e| e.python_exception())
        }
        _ => Err("Exception"),
    }
}

fn run_graph_ops(ops: &[Json]) -> Result<(MultiGraph, Vec<EdgeKey>), &'static str> {
    let mut g = MultiGraph::new();
    let mut keys = Vec::new();
    for op in ops {
        let opname = op.get("op").map(Json::as_str).unwrap_or("");
        let mut attrs = attrs_from_json(op.get("attrs")).map_err(|_| "TypeError")?;
        if let Some(extra) = op.get("extra") {
            let extra = attrs_from_json(Some(extra)).map_err(|_| "TypeError")?;
            attrs.update(&extra);
        }
        match opname {
            "add_node" => {
                let node = op.get("node").map(Json::as_str).unwrap_or("");
                g.add_node(node, attrs);
            }
            "add_edge" => {
                let src = op.get("src").map(Json::as_str).unwrap_or("");
                let dst = op.get("dst").map(Json::as_str).unwrap_or("");
                let key = edge_key_from_json(op.get("key"));
                keys.push(g.add_edge(src, dst, key, attrs));
            }
            _ => return Err("Exception"),
        }
    }
    Ok((g, keys))
}

fn rust_case(case: &Json) -> Result<String, &'static str> {
    match case.get("fn").map(Json::as_str) {
        Some("topo") => {
            let spec = case.get("spec").map(Json::as_str).unwrap_or("");
            match build_topo(spec) {
                Ok(b) => Ok(dump_builtin(&b)),
                Err(e) => Err(e.python_exception()),
            }
        }
        Some("make") => make_topo(case).map(|b| dump_builtin(&b)),
        Some("graph") => {
            let ops = match case.get("ops") {
                Some(Json::Arr(ops)) => ops.clone(),
                _ => vec![],
            };
            let (g, keys) = run_graph_ops(&ops)?;
            Ok(dump_graph(&g, &keys))
        }
        _ => Err("Exception"),
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn differential_against_python_topo() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let driver = manifest.join("tests").join("py_driver.py");

    let mut cases = topo_spec_cases();
    cases.extend(make_cases());
    cases.extend(graph_cases());

    // Serialize cases as JSON on stdin for the python driver.
    let mut input = String::from("[");
    for (i, case) in cases.iter().enumerate() {
        if i > 0 {
            input.push(',');
        }
        ser(case, &mut input);
    }
    input.push_str("]\n");

    let mut child = Command::new("python3")
        .arg(&driver)
        .current_dir(&repo_root)
        .env("PYTHONPATH", repo_root.join("legacy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn python3 (is python3 on PATH?)");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write cases to python stdin");
    let output = child.wait_with_output().expect("wait for python3");
    assert!(
        output.status.success(),
        "python driver failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8(output.stdout).expect("driver output is utf-8");
    let mut pos = 0;
    let parsed = parse_value(&stdout, &mut pos).expect("parse driver JSON output");
    let results = match parsed {
        Json::Arr(items) => items,
        other => panic!("expected JSON array from driver, got {other:?}"),
    };
    assert_eq!(
        results.len(),
        cases.len(),
        "driver returned wrong number of results"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (case, py) in cases.iter().zip(results.iter()) {
        let rust = rust_case(case);
        let py_ok = py.get("ok").map(|v| v.as_str().to_string());
        let py_err = py.get("error").map(|v| v.as_str().to_string());
        let matches = match (&rust, py_ok.as_deref(), py_err.as_deref()) {
            (Ok(dump), Some(py_dump), None) => dump.as_str() == py_dump,
            (Err(cls), None, Some(py_cls)) => *cls == py_cls,
            _ => false,
        };
        if !matches {
            let case_json = {
                let mut s = String::new();
                ser(case, &mut s);
                s
            };
            mismatches.push(format!(
                "case={case_json}\n  python: {}\n  rust:   {}",
                match (&py_ok, &py_err) {
                    (Some(d), _) => format!("ok:\n{d}"),
                    (None, Some(e)) => format!("error: {e}"),
                    _ => "???".to_string(),
                },
                match &rust {
                    Ok(d) => format!("ok:\n{d}"),
                    Err(e) => format!("error: {e}"),
                }
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} differential cases MISMATCH:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n----\n")
    );
    println!("differential: all {} cases match python", cases.len());
}
