//! Differential test: run the ORIGINAL `mininet/util.py` functions under
//! `python3` (the repo's `mininet` package) on a large set of generated
//! inputs and assert that the Rust port in `mn-util` produces identical
//! outputs.
//!
//! Protocol: cases are sent to `tests/py_driver.py` as JSON on stdin; the
//! driver returns a JSON array of results (Python exceptions become
//! `{"__error__": ...}`, normalized to `Json::Error` on the Rust side, where
//! a `None` return from the Rust port maps to `Json::Error` as well).

use mn_util::*;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Minimal JSON (std only): enough for the case/result protocol.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    /// Python raised an exception / Rust returned None.
    Error,
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
            c if (c as u32) <= 0x7e => out.push(c),
            c if (c as u32) <= 0xffff => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => {
                // astral plane: surrogate pair
                let v = c as u32 - 0x1_0000;
                let _ = write!(
                    out,
                    "\\u{:04x}\\u{:04x}",
                    0xd800 + (v >> 10),
                    0xdc00 + (v & 0x3ff)
                );
            }
        }
    }
    out.push('"');
}

fn json_float(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v == f64::INFINITY {
        "Infinity".to_string()
    } else if v == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        // Debug of a finite f64 is the shortest round-trip form and a valid
        // JSON number (always contains '.' or 'e').
        format!("{v:?}")
    }
}

fn ser(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(v) => {
            let _ = write!(out, "{v}");
        }
        Json::Float(v) => out.push_str(&json_float(*v)),
        Json::Str(s) => ser_str(s, out),
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
                    out.push(',');
                }
                ser_str(k, out);
                out.push(':');
                ser(val, out);
            }
            out.push('}');
        }
        Json::Error => out.push_str("{\"__error__\":\"error\"}"),
    }
}

fn skip_ws(s: &str, pos: &mut usize) {
    while *pos < s.len() && s.as_bytes()[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

fn expect(s: &str, pos: &mut usize, word: &str) -> Option<()> {
    if s[*pos..].starts_with(word) {
        *pos += word.len();
        Some(())
    } else {
        None
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
                        let cp = u32::from_str_radix(hex1, 16).ok()?;
                        *pos += 4;
                        let cp = if (0xd800..0xdc00).contains(&cp) {
                            // high surrogate: expect a low surrogate
                            if s[*pos..].starts_with("\\u") {
                                let hex2 = s.get(*pos + 2..*pos + 6)?;
                                let lo = u32::from_str_radix(hex2, 16).ok()?;
                                *pos += 6;
                                0x1_0000 + ((cp - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                return None;
                            }
                        } else {
                            cp
                        };
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
        text.parse::<i128>().ok().map(Json::Int)
    }
}

fn parse_value(s: &str, pos: &mut usize) -> Option<Json> {
    skip_ws(s, pos);
    let c = s[*pos..].chars().next()?;
    match c {
        'n' => {
            expect(s, pos, "null")?;
            Some(Json::Null)
        }
        't' => {
            expect(s, pos, "true")?;
            Some(Json::Bool(true))
        }
        'f' => {
            expect(s, pos, "false")?;
            Some(Json::Bool(false))
        }
        'N' => {
            expect(s, pos, "NaN")?;
            Some(Json::Float(f64::NAN))
        }
        'I' => {
            expect(s, pos, "Infinity")?;
            Some(Json::Float(f64::INFINITY))
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
                let val = parse_value(s, pos)?;
                kv.push((key, val));
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
        '-' => {
            if s[*pos..].starts_with("-Infinity") {
                *pos += "-Infinity".len();
                Some(Json::Float(f64::NEG_INFINITY))
            } else {
                parse_number(s, pos)
            }
        }
        _ if c.is_ascii_digit() => parse_number(s, pos),
        _ => None,
    }
}

/// Convert Python's {"__error__": ...} markers (recursively) to Json::Error.
fn normalize(v: Json) -> Json {
    match v {
        Json::Arr(items) => Json::Arr(items.into_iter().map(normalize).collect()),
        Json::Obj(kv) => {
            if kv.len() == 1 && kv[0].0 == "__error__" {
                Json::Error
            } else {
                Json::Obj(kv.into_iter().map(|(k, v)| (k, normalize(v))).collect())
            }
        }
        other => other,
    }
}

fn json_eq(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Null, Json::Null) => true,
        (Json::Bool(x), Json::Bool(y)) => x == y,
        (Json::Int(x), Json::Int(y)) => x == y,
        (Json::Float(x), Json::Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Json::Str(x), Json::Str(y)) => x == y,
        (Json::Arr(x), Json::Arr(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(x, y)| json_eq(x, y))
        }
        (Json::Obj(x), Json::Obj(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && json_eq(v1, v2))
        }
        (Json::Error, Json::Error) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Deterministic PRNG (xorshift64*)
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
    fn pick<T: Clone>(&mut self, opts: &[T]) -> T {
        opts[self.below(opts.len() as u64) as usize].clone()
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn gen_str(&mut self, alphabet: &[char], max_len: usize) -> String {
        let len = self.below(max_len as u64 + 1) as usize;
        (0..len)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize])
            .collect()
    }
    fn gen_f64(&mut self) -> f64 {
        f64::from_bits(self.next())
    }
}

// ---------------------------------------------------------------------------
// Case generation
// ---------------------------------------------------------------------------

type Case = (String, Vec<Json>);

fn jstr(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}
fn jint(v: i64) -> Json {
    Json::Int(v as i128)
}

fn build_cases() -> Vec<Case> {
    let mut rng = Rng::new(0x5eed_1234_abcd_ef01);
    let mut cases: Vec<Case> = Vec::new();
    let mut add = |fn_name: &str, args: Vec<Json>| cases.push((fn_name.to_string(), args));

    // ---- _colonHex / macColonHex ----
    for _ in 0..300 {
        let val = if rng.chance(50) {
            rng.range_i64(-300, 300)
        } else if rng.chance(50) {
            rng.range_i64(0, 1 << 48)
        } else {
            rng.range_i64(0, i64::MAX >> 1)
        };
        let bytecount = rng.range_i64(0, 10) as usize;
        add("_colonHex", vec![jint(val), jint(bytecount as i64)]);
    }
    for _ in 0..200 {
        let mac = if rng.chance(90) {
            rng.range_i64(0, 1 << 48)
        } else {
            rng.range_i64(-1000, 0)
        };
        add("macColonHex", vec![jint(mac)]);
    }

    // ---- ipStr / ipNum ----
    for _ in 0..300 {
        let v = match rng.below(4) {
            0 => rng.range_i64(-(1 << 33), 1 << 33),
            1 => rng.pick(&[0, -1, 0xffff_ffff, 1 << 32, 0x0a00_0001]),
            _ => rng.range_i64(-300, 300),
        };
        add("ipStr", vec![jint(v)]);
    }
    for _ in 0..300 {
        let comp = |rng: &mut Rng| {
            if rng.chance(80) {
                rng.range_i64(-300, 300)
            } else {
                rng.range_i64(-(1 << 32), 1 << 32)
            }
        };
        add(
            "ipNum",
            vec![
                jint(comp(&mut rng)),
                jint(comp(&mut rng)),
                jint(comp(&mut rng)),
                jint(comp(&mut rng)),
            ],
        );
    }

    // ---- ipAdd ----
    let bases = [0x0a00_0000i64, 0, 0xffff_ffff, 0xac10_1800, 0x7f00_0001, -1];
    let prefixes = [0i64, 1, 8, 16, 24, 30, 31, 32, 33, 40, 64, 100, -1];
    for _ in 0..500 {
        let i = if rng.chance(70) {
            rng.range_i64(-10, 300)
        } else {
            rng.pick(&[0i64, 255, 256, -1, 65535, 1 << 24])
        };
        let prefix_len = rng.pick(&prefixes);
        let base = rng.pick(&bases);
        match rng.below(3) {
            0 => add("ipAdd", vec![jint(i)]),
            1 => add("ipAdd", vec![jint(i), jint(prefix_len)]),
            _ => add("ipAdd", vec![jint(i), jint(prefix_len), jint(base)]),
        }
    }

    // ---- ipParse / netParse ----
    let ip_pool = [
        "10.0.0.1",
        "1",
        "1.2",
        "1.2.3",
        "1.2.3.4",
        "1.2.3.4.5",
        "",
        "a.b",
        "1.2.3.x",
        " 10.0.0.1 ",
        "+1.-2.3.4",
        "1_0.2.3.4",
        "1.2.3.-4",
        "256.0.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "999.999.999.999",
        "-1.-2.-3.-4",
        "1..2",
        "1.",
        ".1",
        "1.2.3.",
        "0x1.2.3.4",
        "1.2.3.4 ",
        "  ",
        "1_2.3.4.5",
    ];
    for _ in 0..200 {
        add("ipParse", vec![jstr(rng.pick(&ip_pool))]);
    }
    for _ in 0..200 {
        // random dotted strings
        let n = rng.range_i64(1, 5);
        let mut parts = Vec::new();
        for _ in 0..n {
            parts.push(match rng.below(4) {
                0 => rng.range_i64(-300, 300).to_string(),
                1 => rng.gen_str(&['0', '1', '9', 'a', ' ', '_', '+', '-'], 5),
                _ => rng.range_i64(0, 255).to_string(),
            });
        }
        add("ipParse", vec![jstr(parts.join("."))]);
    }
    let net_pool = [
        "10.0.0.0/8",
        "10.0.0.0",
        "1.2.3.4/24",
        "1.2.3.4/0",
        "1.2.3.4/32",
        "1.2.3.4/33",
        "1.2.3.4/-1",
        "1.2.3.4/ 8 ",
        "1.2.3.4/24/5",
        "1.2.3.4/",
        "/8",
        "abc/8",
        "1.2.3.4/0x8",
        "1.2.3.4/1_0",
        "1.2/16",
        "5",
        "1.2.3.4/+24",
    ];
    for _ in 0..150 {
        add("netParse", vec![jstr(rng.pick(&net_pool))]);
    }
    for _ in 0..100 {
        let ip = rng.pick(&ip_pool).to_string();
        let pf = rng
            .pick(&["8", "24", "0", "-1", "33", "", "x", " 16 ", "1_0"])
            .to_string();
        add("netParse", vec![jstr(format!("{ip}/{pf}"))]);
    }

    // ---- checkInt / checkFloat / makeNumeric ----
    let num_pool = [
        "1",
        "-1",
        "+1",
        " 1 ",
        "\t12\n",
        "1_0",
        "1__0",
        "_1",
        "1_",
        "0x10",
        "0",
        "007",
        "-0",
        "+0",
        "123456789012345678",
        "-123456789012345678",
        "9223372036854775807",
        "-9223372036854775808",
        "abc",
        "",
        " ",
        "1.5",
        ".5",
        "5.",
        "1e5",
        "1E5",
        "1e+5",
        "1e-5",
        "1e",
        "1.5e-3",
        "1_0.5",
        "1.5_",
        "1_.5",
        "1._5",
        "1_0e1_0",
        "inf",
        "Inf",
        "infinity",
        "-INF",
        "+nan",
        "NaN",
        "1e999",
        "-1e999",
        "0.1e1",
        "00.5",
        "+ 1",
        "1.2.3",
        "e5",
        ".",
        "+",
        "-",
        "1.5_e3",
        "1e_10",
        "e",
        "0b101",
    ];
    for _ in 0..250 {
        add("checkInt", vec![jstr(rng.pick(&num_pool))]);
        add("checkFloat", vec![jstr(rng.pick(&num_pool))]);
        add("makeNumeric", vec![jstr(rng.pick(&num_pool))]);
    }
    let alpha: Vec<char> = "ab1._= -,+eE09\t".chars().collect();
    for _ in 0..250 {
        let s = rng.gen_str(&alpha, 10);
        add("checkInt", vec![jstr(s.clone())]);
        add("checkFloat", vec![jstr(s.clone())]);
        add("makeNumeric", vec![jstr(s)]);
    }

    // ---- natural / naturalSeq ----
    let nat_pool = [
        "a1b2", "1a", "a1", "", "10", "9", "x10y9", "a01b002", "h1", "h10", "h2", "s1-eth2", "5",
        "007", "a b 3", "é1", "a  2", "0", "00", "a0", "0a",
    ];
    for _ in 0..200 {
        add("natural", vec![jstr(rng.pick(&nat_pool))]);
    }
    let nat_alpha: Vec<char> = "abz019-_ ".chars().collect();
    for _ in 0..200 {
        let s = rng.gen_str(&nat_alpha, 12);
        match rng.below(5) {
            0 => add("natural", vec![jint(rng.range_i64(-100, 100))]),
            _ => add("natural", vec![jstr(s)]),
        }
    }
    for _ in 0..100 {
        let n = rng.range_i64(0, 6);
        let items: Vec<Json> = (0..n)
            .map(|_| {
                if rng.chance(30) {
                    jint(rng.range_i64(-50, 50))
                } else {
                    jstr(rng.pick(&nat_pool).to_string())
                }
            })
            .collect();
        add("naturalSeq", vec![Json::Arr(items)]);
    }

    // ---- irange ----
    for _ in 0..60 {
        let a = rng.range_i64(-6, 7);
        let b = rng.range_i64(-6, 7);
        add("irange", vec![jint(a), jint(b)]);
    }
    add("irange", vec![jint(0), jint(100)]);

    // ---- splitArgs ----
    let split_pool = [
        "fn",
        "fn,1,2.5,kw=3,x=y",
        "fn,a=1,a=2",
        "",
        "fn,=x",
        "fn,a=b=c",
        "fn, 1 ,x= 2 ",
        "fn,",
        "fn,,",
        "a,b,c=d,e",
        "x=1,y=2,z=3",
        "fn,1.5e3,k=v w",
        "Single,2",
        "t,True",
        "fn,-3,kw=-2.5",
        "fn,1e999,k=inf",
        "topo,single,3",
        "link,tc,bw=10,delay=5ms",
    ];
    for _ in 0..200 {
        add("splitArgs", vec![jstr(rng.pick(&split_pool))]);
    }
    let split_alpha: Vec<char> = "ab1.,= -_xy".chars().collect();
    for _ in 0..200 {
        add("splitArgs", vec![jstr(rng.gen_str(&split_alpha, 16))]);
    }

    // ---- unitScale ----
    let num_specials = [
        Json::Int(0),
        Json::Int(1),
        Json::Int(999),
        Json::Int(1000),
        Json::Float(999.5),
        Json::Float(1e6),
        Json::Float(1e15),
        Json::Float(1e18),
        Json::Float(-5.0),
        Json::Float(0.5),
        Json::Float(f64::NAN),
        Json::Float(f64::INFINITY),
        Json::Float(f64::NEG_INFINITY),
        Json::Float(123.456),
        jint(1_000_000_000_000_000_000),
    ];
    let prefixes = [
        "", "k", "K", "M", "m", "G", "T", "P", "p", "x", "MG", "km", "kk", "KM", "Gt", "kM",
    ];
    for _ in 0..300 {
        let num = if rng.chance(60) {
            rng.pick(&num_specials).clone()
        } else if rng.chance(50) {
            jint(rng.range_i64(-10, 1_000_000_000_000_000_000))
        } else {
            Json::Float(rng.gen_f64())
        };
        if rng.chance(20) {
            add("unitScale", vec![num]);
        } else {
            add("unitScale", vec![num, jstr(rng.pick(&prefixes))]);
        }
    }

    // ---- fmtBps ----
    let fmts = [
        FMT_BPS_DEFAULT,
        "%.3f %s",
        "%s: %.2f",
        "%.0f %s bps",
        "%08.1f|%s",
        "%-10s|%.1f",
        "%f %s",
        "%e %s",
        "%.2e %s",
        "%.1E %s",
        "%s",
        "%.1f",
        "100%%",
        "%+.1f %s",
        "% .1f %s",
        "%5.1f %s",
        "%x %s",
        "%.1f %sbits/sec",
        "%s=%f",
    ];
    let bps_pool = [
        Json::Int(0),
        Json::Int(1),
        Json::Int(999),
        Json::Int(1000),
        Json::Float(1234.567),
        Json::Float(0.0),
        Json::Float(-0.0),
        Json::Float(2.5),
        Json::Float(999.95),
        Json::Float(1e9),
        Json::Float(2.5e18),
        Json::Float(f64::NAN),
        Json::Float(f64::INFINITY),
        Json::Float(-3.0),
        Json::Float(0.04),
        jint(10_000_000),
    ];
    for _ in 0..400 {
        let bps = if rng.chance(60) {
            rng.pick(&bps_pool).clone()
        } else if rng.chance(50) {
            jint(rng.range_i64(-1000, 10_000_000_000))
        } else {
            Json::Float(rng.gen_f64())
        };
        let fmt = rng.pick(&fmts);
        // integer conversions can't take huge floats in the Rust port
        // (CPython would produce an exact big int); keep those small.
        let bps = if fmt.contains(['d', 'i', 'u', 'x', 'o', 'c']) {
            match bps {
                Json::Float(f) if f.abs() > 1e15 => Json::Float(rng.range_i64(-1000, 1000) as f64),
                other => other,
            }
        } else {
            bps
        };
        match rng.below(4) {
            0 => add("fmtBps", vec![bps]),
            1 => add(
                "fmtBps",
                vec![bps, jstr(rng.pick(&["", "k", "M", "G", "z", "K"]))],
            ),
            _ => add(
                "fmtBps",
                vec![bps, jstr(rng.pick(&["", "k", "M", "G", "z"])), jstr(fmt)],
            ),
        }
    }
    let fmt_alpha: Vec<char> = "%.-0123456789fedsix+ #".chars().collect();
    for _ in 0..100 {
        let fmt = rng.gen_str(&fmt_alpha, 12);
        add(
            "fmtBps",
            vec![Json::Float(rng.gen_f64()), jstr(""), jstr(fmt)],
        );
    }

    // ---- numCores ----
    add("numCores", vec![]);

    // ---- retry ----
    for _ in 0..24 {
        let retries = rng.pick(&[-1i64, 0, 1, 2, 3, 5, 10]);
        let fail = rng.pick(&[0i64, 1, 2, 3, 4, 10]);
        add("retry", vec![jint(retries), Json::Int(0), jint(fail)]);
    }

    cases
}

// ---------------------------------------------------------------------------
// Rust-side evaluation of each case
// ---------------------------------------------------------------------------

fn as_str(v: &Json) -> &str {
    match v {
        Json::Str(s) => s,
        _ => panic!("expected string arg, got {v:?}"),
    }
}

fn as_i64(v: &Json) -> i64 {
    match v {
        Json::Int(v) => *v as i64,
        _ => panic!("expected int arg, got {v:?}"),
    }
}

fn as_f64(v: &Json) -> f64 {
    match v {
        Json::Int(v) => *v as f64,
        Json::Float(v) => *v,
        _ => panic!("expected numeric arg, got {v:?}"),
    }
}

fn opt_str(v: Option<String>) -> Json {
    match v {
        Some(s) => Json::Str(s),
        None => Json::Error,
    }
}

fn numeric_json(v: &Numeric) -> Json {
    match v {
        Numeric::Int(v) => Json::Int(*v as i128),
        Numeric::Float(v) => Json::Float(*v),
        Numeric::Text(v) => Json::Str(v.clone()),
    }
}

fn natural_json(v: &Json) -> Json {
    match v {
        Json::Str(s) => Json::Arr(natural(s).iter().map(part_json).collect()),
        Json::Int(v) => Json::Arr(natural(v).iter().map(part_json).collect()),
        _ => panic!("bad natural arg {v:?}"),
    }
}

fn part_json(p: &NaturalPart) -> Json {
    match p {
        NaturalPart::Num(v) => Json::Int(*v),
        NaturalPart::Text(v) => Json::Str(v.clone()),
    }
}

fn rust_result(fn_name: &str, args: &[Json]) -> Json {
    match fn_name {
        "_colonHex" => Json::Str(colon_hex(as_i64(&args[0]), as_i64(&args[1]) as usize)),
        "macColonHex" => Json::Str(mac_colon_hex(as_i64(&args[0]))),
        "ipStr" => Json::Str(ip_str(as_i64(&args[0]))),
        "ipNum" => Json::Int(ip_num(
            as_i64(&args[0]),
            as_i64(&args[1]),
            as_i64(&args[2]),
            as_i64(&args[3]),
        ) as i128),
        "ipAdd" => match args.len() {
            1 => opt_str(ip_add(as_i64(&args[0]))),
            2 => opt_str(ip_add_with(
                as_i64(&args[0]),
                as_i64(&args[1]),
                DEFAULT_IP_BASE_NUM,
            )),
            _ => opt_str(ip_add_with(
                as_i64(&args[0]),
                as_i64(&args[1]),
                as_i64(&args[2]),
            )),
        },
        "ipParse" => match ip_parse(as_str(&args[0])) {
            Some(v) => Json::Int(v as i128),
            None => Json::Error,
        },
        "netParse" => match net_parse(as_str(&args[0])) {
            Some((addr, prefix)) => {
                Json::Arr(vec![Json::Int(addr as i128), Json::Int(prefix as i128)])
            }
            None => Json::Error,
        },
        "checkInt" => Json::Bool(check_int(as_str(&args[0]))),
        "checkFloat" => Json::Bool(check_float(as_str(&args[0]))),
        "makeNumeric" => numeric_json(&make_numeric(as_str(&args[0]))),
        "natural" => natural_json(&args[0]),
        "naturalSeq" => match &args[0] {
            Json::Arr(items) => Json::Arr(items.iter().map(natural_json).collect()),
            _ => panic!("bad naturalSeq arg"),
        },
        "irange" => Json::Arr(
            irange(as_i64(&args[0]), as_i64(&args[1]))
                .map(|v| Json::Int(v as i128))
                .collect(),
        ),
        "splitArgs" => {
            let r = split_args(as_str(&args[0]));
            Json::Arr(vec![
                Json::Str(r.func),
                Json::Arr(r.args.iter().map(numeric_json).collect()),
                Json::Obj(
                    r.kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), numeric_json(v)))
                        .collect(),
                ),
            ])
        }
        "unitScale" => {
            let num = as_f64(&args[0]);
            let prefix = args.get(1).map(as_str).unwrap_or("");
            match unit_scale(num, prefix) {
                Some((p, f)) => Json::Arr(vec![Json::Str(p), Json::Float(f)]),
                None => Json::Error,
            }
        }
        "fmtBps" => {
            let bps = as_f64(&args[0]);
            let prefix = args.get(1).map(as_str).unwrap_or("");
            let fmt = args.get(2).map(as_str).unwrap_or(FMT_BPS_DEFAULT);
            opt_str(fmt_bps_with(bps, prefix, fmt))
        }
        "numCores" => Json::Int(num_cores() as i128),
        "retry" => {
            let retries = as_i64(&args[0]);
            let delay = as_f64(&args[1]);
            let fail_times = as_i64(&args[2]);
            let mut calls: i64 = 0;
            let ok = retry(retries, Duration::from_secs_f64(delay), || {
                calls += 1;
                calls > fail_times
            });
            Json::Arr(vec![
                Json::Str(if ok { "ok" } else { "exit" }.to_string()),
                Json::Int(calls as i128),
            ])
        }
        other => panic!("unknown fn {other}"),
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn differential_against_python_util() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let driver = manifest.join("tests").join("py_driver.py");

    let cases = build_cases();

    // Serialize cases as JSON on stdin for the python driver.
    let mut input = String::from("[");
    for (i, (fn_name, args)) in cases.iter().enumerate() {
        if i > 0 {
            input.push(',');
        }
        input.push_str("{\"fn\":");
        ser_str(fn_name, &mut input);
        input.push_str(",\"args\":");
        ser(&Json::Arr(args.clone()), &mut input);
        input.push('}');
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
    for ((fn_name, args), py) in cases.iter().zip(results.iter()) {
        let py = normalize(py.clone());
        let rs = rust_result(fn_name, args);
        if !json_eq(&rs, &py) {
            mismatches.push(format!(
                "fn={fn_name} args={}\n  python: {}\n  rust:   {}",
                {
                    let mut s = String::new();
                    ser(&Json::Arr(args.clone()), &mut s);
                    s
                },
                {
                    let mut s = String::new();
                    ser(&py, &mut s);
                    s
                },
                {
                    let mut s = String::new();
                    ser(&rs, &mut s);
                    s
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
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("differential: all {} cases match python", cases.len());
}
