//! Unit scaling and iperf-style rate formatting, ported from `unitScale`
//! and `fmtBps` of `mininet/util.py`.
//!
//! `fmtBps` applies a Python `%`-format string to `(bps, prefix)`, so this
//! module also contains a small [`percent_format`] engine that mirrors
//! CPython's `%` formatting for the conversions used in practice
//! (`d i u f e E s x X o c` and `%%`, with flags/width/precision, including
//! `*` forms). Unsupported conversions (e.g. `%g`, `%b`) return `None`,
//! mirroring Python's `ValueError`.

/// Default `fmt` of Python `fmtBps`.
pub const FMT_BPS_DEFAULT: &str = "%.1f %sbits/sec";

const SCALE: &str = "kMGTP";

/// Port of `unitScale(num, prefix='')`: returns `(prefix, factor)`.
///
/// With a non-empty `prefix`, the factor for that prefix is returned and
/// the original (case-preserved) prefix string is echoed back. With an
/// empty prefix, the largest `kMGTP` factor not exceeding `num` is chosen.
/// Returns `None` where Python raises `ValueError` (unknown prefix).
pub fn unit_scale(num: f64, prefix: &str) -> Option<(String, f64)> {
    if !prefix.is_empty() {
        let pos = SCALE.to_lowercase().find(&prefix.to_lowercase())?;
        let factor = 10f64.powi(3 * (pos as i32 + 1));
        return Some((prefix.to_string(), factor));
    }
    let mut out_prefix = String::new();
    let mut factor = 1f64;
    for (i, c) in SCALE.chars().enumerate() {
        let f = 10f64.powi(3 * (i as i32 + 1));
        if num < f {
            break;
        }
        out_prefix = c.to_string();
        factor = f;
    }
    Some((out_prefix, factor))
}

/// Port of `fmtBps(bps)` with default `prefix=''` and default `fmt`.
pub fn fmt_bps(bps: f64) -> Option<String> {
    fmt_bps_with(bps, "", FMT_BPS_DEFAULT)
}

/// Port of `fmtBps(bps, prefix='', fmt='%.1f %sbits/sec')`.
///
/// Returns `None` where Python raises (`ValueError` for an unknown prefix,
/// `TypeError`/`ValueError` for a bad `fmt` or argument mismatch).
pub fn fmt_bps_with(bps: f64, prefix: &str, fmt: &str) -> Option<String> {
    let (prefix, factor) = unit_scale(bps, prefix)?;
    let bps = bps / factor;
    percent_format(fmt, &[PercentValue::Float(bps), PercentValue::Str(prefix)])
}

/// A value usable with [`percent_format`] (mirrors the values `fmtBps`
/// passes to `fmt % (bps, prefix)`).
#[derive(Debug, Clone, PartialEq)]
pub enum PercentValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

impl PercentValue {
    /// `int` value for `%d`/`%i`/`%u`/`%x`/`%o`/`%c`. Like Python, finite
    /// floats are truncated toward zero; strings and non-finite floats are
    /// rejected (Python: `TypeError`/`OverflowError`).
    fn as_i64(&self) -> Option<i64> {
        match self {
            PercentValue::Int(v) => Some(*v),
            PercentValue::Float(f) if f.is_finite() => Some(*f as i64),
            _ => None,
        }
    }

    /// `float` value for `%f`/`%e`.
    fn as_f64(&self) -> Option<f64> {
        match self {
            PercentValue::Float(f) => Some(*f),
            PercentValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// `str(value)` for `%s`.
    fn to_python_str(&self) -> String {
        match self {
            PercentValue::Int(v) => v.to_string(),
            PercentValue::Float(f) => python_float_repr(*f),
            PercentValue::Str(s) => s.clone(),
            PercentValue::Bool(b) => {
                if *b {
                    "True".to_string()
                } else {
                    "False".to_string()
                }
            }
            PercentValue::Null => "None".to_string(),
        }
    }
}

/// `repr()`/`str()` of a Python float: shortest round-trip digits, positional
/// notation for decimal exponents in `[-4, 16)`, scientific otherwise with
/// a signed, at-least-two-digit exponent (`1e+16`, `1e-05`).
pub fn python_float_repr(v: f64) -> String {
    if v.is_nan() {
        // CPython prints 'nan' regardless of sign bit
        return "nan".to_string();
    }
    let s = format!("{v:?}");
    match s.find(['e', 'E']) {
        None => s,
        Some(pos) => {
            let exp: i32 = s[pos + 1..].parse().unwrap_or(0);
            format!(
                "{}e{}{:02}",
                &s[..pos],
                if exp < 0 { '-' } else { '+' },
                exp.abs()
            )
        }
    }
}

#[derive(Default)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alt: bool,
    width: Option<usize>,
    prec: Option<usize>,
}

/// A small engine for CPython's `fmt % args` operator.
///
/// Supported conversions: `d i u f F e E s x X o c` and bare `%%`, with the
/// flags `- + space 0 #`, numeric or `*` width/precision. Returns `None`
/// where CPython raises (`ValueError` for an incomplete/unsupported format,
/// `TypeError` for argument-count mismatches or wrong argument types).
pub fn percent_format(fmt: &str, args: &[PercentValue]) -> Option<String> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut arg_idx = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            return None; // ValueError: incomplete format
        }
        if chars[i] == '%' {
            out.push('%');
            i += 1;
            continue;
        }
        let mut spec = Spec::default();
        while i < chars.len() {
            match chars[i] {
                '-' => spec.left = true,
                '+' => spec.plus = true,
                ' ' => spec.space = true,
                '0' => spec.zero = true,
                '#' => spec.alt = true,
                _ => break,
            }
            i += 1;
        }
        if i < chars.len() && chars[i] == '*' {
            i += 1;
            match args.get(arg_idx) {
                Some(PercentValue::Int(w)) if *w >= 0 => {
                    spec.width = Some(*w as usize);
                    arg_idx += 1;
                }
                _ => return None,
            }
        } else {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i > start {
                spec.width = Some(chars[start..i].iter().collect::<String>().parse().ok()?);
            }
        }
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            if i < chars.len() && chars[i] == '*' {
                i += 1;
                match args.get(arg_idx) {
                    Some(PercentValue::Int(p)) if *p >= 0 => {
                        spec.prec = Some(*p as usize);
                        arg_idx += 1;
                    }
                    _ => return None,
                }
            } else {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let p: usize = if i > start {
                    chars[start..i].iter().collect::<String>().parse().ok()?
                } else {
                    0
                };
                spec.prec = Some(p);
            }
        }
        if i >= chars.len() {
            return None;
        }
        let conv = chars[i];
        i += 1;
        if conv == '%' {
            // '%' with flags/width is a ValueError in CPython
            return None;
        }
        let arg = args.get(arg_idx)?; // TypeError: not enough arguments
        arg_idx += 1;
        let core = match conv {
            'd' | 'i' | 'u' => {
                let v = arg.as_i64()?;
                if v < 0 {
                    format!("{v}")
                } else {
                    apply_sign(format!("{v}"), &spec)
                }
            }
            'f' | 'F' => {
                let v = arg.as_f64()?;
                let p = spec.prec.unwrap_or(6);
                // Rust's {:.p} rounds half-to-even on the exact binary
                // value, exactly like CPython's %.pf. Rust prints "NaN"
                // where CPython prints "nan".
                apply_sign(lower_nan(format!("{v:.p$}")), &spec)
            }
            'e' | 'E' => {
                let v = arg.as_f64()?;
                let p = spec.prec.unwrap_or(6);
                let upper = conv == 'E';
                // CPython: %e prints 'nan'/'inf', %E prints 'NAN'/'INF'.
                let mut core = if upper {
                    format!("{v:.p$E}").replace("NaN", "NAN")
                } else {
                    lower_nan(format!("{v:.p$e}"))
                };
                if spec.alt && p == 0 {
                    if let Some(pos) = core.find(['e', 'E']) {
                        core.insert(pos, '.');
                    }
                }
                fix_exponent(&mut core, upper);
                apply_sign(core, &spec)
            }
            's' => arg.to_python_str(),
            'x' | 'X' | 'o' => {
                let v = arg.as_i64()?;
                let neg = v < 0;
                let mag = v.unsigned_abs();
                let digits = match conv {
                    'x' => format!("{mag:x}"),
                    'X' => format!("{mag:X}"),
                    _ => format!("{mag:o}"),
                };
                let prefix = match conv {
                    'x' if spec.alt => "0x",
                    'X' if spec.alt => "0X",
                    'o' if spec.alt => "0o",
                    _ => "",
                };
                let core = format!("{prefix}{digits}");
                if neg {
                    format!("-{core}")
                } else {
                    apply_sign(core, &spec)
                }
            }
            'c' => match arg {
                PercentValue::Int(v) => char::from_u32(*v as u32)?.to_string(),
                PercentValue::Str(s) if s.chars().count() == 1 => s.clone(),
                _ => return None,
            },
            _ => return None, // ValueError: unsupported format character
        };
        // CPython ignores the '0' flag for %s and %c (pads with spaces)
        let zero = spec.zero && !matches!(conv, 's' | 'c');
        out.push_str(&pad(&core, &spec, zero));
    }
    if arg_idx != args.len() {
        return None; // TypeError: not all arguments converted
    }
    Some(out)
}

/// CPython prints `nan`; Rust's float formatting prints `NaN`.
fn lower_nan(core: String) -> String {
    core.replace("NaN", "nan")
}

/// Prepend `+`/space sign flag unless the core is already negative.
fn apply_sign(mut core: String, spec: &Spec) -> String {
    if core.starts_with('-') {
        core
    } else if spec.plus {
        core.insert(0, '+');
        core
    } else if spec.space {
        core.insert(0, ' ');
        core
    } else {
        core
    }
}

/// Rewrite Rust's `1.2e4` exponent as CPython's `1.2e+04`.
fn fix_exponent(core: &mut String, upper: bool) {
    let e = if upper { 'E' } else { 'e' };
    if let Some(pos) = core.find(e) {
        let exp: i32 = core[pos + 1..].parse().unwrap_or(0);
        let mantissa = core[..pos].to_string();
        *core = format!(
            "{mantissa}{e}{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        );
    }
}

/// Apply width and alignment; `zero` pads with '0' after any sign.
fn pad(core: &str, spec: &Spec, zero: bool) -> String {
    let width = spec.width.unwrap_or(0);
    let len = core.chars().count();
    if len >= width {
        return core.to_string();
    }
    let n = width - len;
    if zero && !spec.left {
        let (sign, rest) = if let Some(r) = core.strip_prefix('-') {
            ("-", r)
        } else if let Some(r) = core.strip_prefix('+') {
            ("+", r)
        } else if let Some(r) = core.strip_prefix(' ') {
            (" ", r)
        } else {
            ("", core)
        };
        format!("{sign}{}{rest}", "0".repeat(n))
    } else if spec.left {
        format!("{core}{}", " ".repeat(n))
    } else {
        format!("{}{core}", " ".repeat(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f64) -> PercentValue {
        PercentValue::Float(v)
    }

    #[test]
    fn unit_scale_cases() {
        assert_eq!(unit_scale(999.0, ""), Some(("".to_string(), 1.0)));
        assert_eq!(unit_scale(1000.0, ""), Some(("k".to_string(), 1000.0)));
        assert_eq!(unit_scale(1234.5, ""), Some(("k".to_string(), 1000.0)));
        assert_eq!(unit_scale(1e18, ""), Some(("P".to_string(), 1e15)));
        assert_eq!(unit_scale(1e19, ""), Some(("P".to_string(), 1e15)));
        assert_eq!(unit_scale(-5.0, ""), Some(("".to_string(), 1.0)));
        assert_eq!(unit_scale(5.0, "K"), Some(("K".to_string(), 1000.0)));
        assert_eq!(unit_scale(5.0, "MG"), Some(("MG".to_string(), 1e6)));
        assert_eq!(unit_scale(5.0, "m"), Some(("m".to_string(), 1e6)));
        assert_eq!(unit_scale(5.0, "p"), Some(("p".to_string(), 1e15)));
        assert_eq!(unit_scale(f64::NAN, ""), Some(("P".to_string(), 1e15)));
        assert_eq!(unit_scale(5.0, "x"), None);
        assert_eq!(unit_scale(5.0, "kk"), None);
    }

    #[test]
    fn fmt_bps_cases() {
        assert_eq!(fmt_bps(1234.567).as_deref(), Some("1.2 kbits/sec"));
        assert_eq!(fmt_bps(0.0).as_deref(), Some("0.0 bits/sec"));
        assert_eq!(fmt_bps(-0.0).as_deref(), Some("-0.0 bits/sec"));
        assert_eq!(fmt_bps(1e9).as_deref(), Some("1.0 Gbits/sec"));
        assert_eq!(fmt_bps(2.5e18).as_deref(), Some("2500.0 Pbits/sec"));
        assert_eq!(
            fmt_bps_with(1234.567, "M", FMT_BPS_DEFAULT).as_deref(),
            Some("0.0 Mbits/sec")
        );
        assert_eq!(
            fmt_bps_with(1234.567, "", "%.3f %s").as_deref(),
            Some("1.235 k")
        );
        assert_eq!(fmt_bps(f64::NAN).as_deref(), Some("nan Pbits/sec"));
        assert_eq!(fmt_bps(f64::INFINITY).as_deref(), Some("inf Pbits/sec"));
        assert_eq!(fmt_bps_with(1234.567, "z", FMT_BPS_DEFAULT), None);
    }

    #[test]
    fn float_repr_cases() {
        assert_eq!(python_float_repr(1.5), "1.5");
        assert_eq!(python_float_repr(100.0), "100.0");
        assert_eq!(python_float_repr(1e16), "1e+16");
        assert_eq!(python_float_repr(1.5e16), "1.5e+16");
        assert_eq!(python_float_repr(1e15), "1000000000000000.0");
        assert_eq!(python_float_repr(1e-5), "1e-05");
        assert_eq!(python_float_repr(1e-4), "0.0001");
        assert_eq!(python_float_repr(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(python_float_repr(f64::NAN), "nan");
        assert_eq!(python_float_repr(f64::INFINITY), "inf");
        assert_eq!(python_float_repr(f64::NEG_INFINITY), "-inf");
        assert_eq!(python_float_repr(-0.0), "-0.0");
        assert_eq!(python_float_repr(5e-324), "5e-324");
        assert_eq!(python_float_repr(1.5e300), "1.5e+300");
    }

    #[test]
    fn percent_format_cases() {
        let s = |v: &str| PercentValue::Str(v.to_string());
        assert_eq!(
            percent_format("%.1f %sbits/sec", &[f(1234.567), s("k")]).as_deref(),
            Some("1234.6 kbits/sec")
        );
        assert_eq!(percent_format("%.1f", &[f(0.25)]).as_deref(), Some("0.2"));
        assert_eq!(percent_format("%.0f", &[f(2.5)]).as_deref(), Some("2"));
        assert_eq!(percent_format("%.1f", &[f(-0.04)]).as_deref(), Some("-0.0"));
        assert_eq!(
            percent_format("%08.1f", &[f(-1.23456)]).as_deref(),
            Some("-00001.2")
        );
        assert_eq!(
            percent_format("%-08.1f|", &[f(1.23456)]).as_deref(),
            Some("1.2     |")
        );
        assert_eq!(
            percent_format("%05.1f", &[f(1.23456)]).as_deref(),
            Some("001.2")
        );
        assert_eq!(
            percent_format("%-8s|", &[s("ab")]).as_deref(),
            Some("ab      |")
        );
        assert_eq!(
            percent_format("%08s|", &[s("ab")]).as_deref(),
            Some("      ab|")
        );
        assert_eq!(
            percent_format("%+d", &[PercentValue::Int(5)]).as_deref(),
            Some("+5")
        );
        assert_eq!(
            percent_format("% d", &[PercentValue::Int(5)]).as_deref(),
            Some(" 5")
        );
        assert_eq!(
            percent_format("%05d|", &[PercentValue::Int(-42)]).as_deref(),
            Some("-0042|")
        );
        assert_eq!(percent_format("%d", &[f(1.9)]).as_deref(), Some("1"));
        assert_eq!(percent_format("%d", &[f(-1.9)]).as_deref(), Some("-1"));
        assert_eq!(
            percent_format("%u", &[PercentValue::Int(-5)]).as_deref(),
            Some("-5")
        );
        assert_eq!(
            percent_format("%.1e", &[f(123.456)]).as_deref(),
            Some("1.2e+02")
        );
        assert_eq!(
            percent_format("%e", &[f(0.0)]).as_deref(),
            Some("0.000000e+00")
        );
        assert_eq!(
            percent_format("%.0e", &[f(12345.0)]).as_deref(),
            Some("1e+04")
        );
        assert_eq!(
            percent_format("%#.0e", &[f(12345.0)]).as_deref(),
            Some("1.e+04")
        );
        assert_eq!(
            percent_format("%.2E", &[f(0.000123)]).as_deref(),
            Some("1.23E-04")
        );
        assert_eq!(
            percent_format("%.3e", &[f(0.000123456)]).as_deref(),
            Some("1.235e-04")
        );
        assert_eq!(percent_format("%s", &[f(1e16)]).as_deref(), Some("1e+16"));
        assert_eq!(percent_format("%s", &[f(1e-5)]).as_deref(), Some("1e-05"));
        assert_eq!(percent_format("%s", &[f(100.0)]).as_deref(), Some("100.0"));
        assert_eq!(
            percent_format("%s", &[PercentValue::Bool(true)]).as_deref(),
            Some("True")
        );
        assert_eq!(
            percent_format("%s", &[PercentValue::Null]).as_deref(),
            Some("None")
        );
        assert_eq!(
            percent_format("%x", &[PercentValue::Int(-255)]).as_deref(),
            Some("-ff")
        );
        assert_eq!(
            percent_format("%#x", &[PercentValue::Int(255)]).as_deref(),
            Some("0xff")
        );
        assert_eq!(
            percent_format("%#o", &[PercentValue::Int(8)]).as_deref(),
            Some("0o10")
        );
        assert_eq!(
            percent_format("%c", &[PercentValue::Int(65)]).as_deref(),
            Some("A")
        );
        assert_eq!(percent_format("%c", &[s("A")]).as_deref(), Some("A"));
        assert_eq!(
            percent_format("%5c|", &[PercentValue::Int(65)]).as_deref(),
            Some("    A|")
        );
        assert_eq!(
            percent_format("%05c|", &[PercentValue::Int(65)]).as_deref(),
            Some("    A|")
        );
        assert_eq!(percent_format("100%%", &[]).as_deref(), Some("100%"));
        assert_eq!(
            percent_format("%.*f", &[PercentValue::Int(2), f(1.23456)]).as_deref(),
            Some("1.23")
        );
        assert_eq!(
            percent_format("%*d", &[PercentValue::Int(5), PercentValue::Int(42)]).as_deref(),
            Some("   42")
        );
        assert_eq!(
            percent_format("%-*d|", &[PercentValue::Int(5), PercentValue::Int(42)]).as_deref(),
            Some("42   |")
        );
        assert_eq!(percent_format("%.f", &[f(1.5)]).as_deref(), Some("2"));
        assert_eq!(
            percent_format("%5.2f|", &[f(1.23456)]).as_deref(),
            Some(" 1.23|")
        );
        assert_eq!(
            percent_format("%+f", &[f(f64::NAN)]).as_deref(),
            Some("+nan")
        );
        assert_eq!(
            percent_format("% f", &[f(f64::INFINITY)]).as_deref(),
            Some(" inf")
        );
        // error cases (Python raises)
        assert_eq!(percent_format("abc%", &[]), None);
        assert_eq!(percent_format("%q", &[PercentValue::Int(1)]), None);
        assert_eq!(percent_format("%5%", &[]), None);
        assert_eq!(percent_format("%.1f %s", &[f(1.0)]), None); // not enough args
        assert_eq!(percent_format("%.1f", &[f(1.0), s("x")]), None); // not all converted
        assert_eq!(percent_format("%d", &[s("x")]), None);
        assert_eq!(percent_format("%d", &[f(f64::INFINITY)]), None);
        assert_eq!(percent_format("%g", &[f(1.5)]), None);
    }
}
