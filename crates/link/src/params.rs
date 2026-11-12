//! Link parameter model and spec-string parsing — the Rust port of the
//! `TCLink`/`TCIntf` parameter handling in `mininet/link.py` and of
//! `splitArgs`/`customClass` in `mininet/util.py` as used by `bin/mn --link`.
//!
//! Spec strings look like `"tc,bw=10,delay=5ms,loss=0.1"`: the first
//! comma-separated token names the link class (the `LINKS` dict in
//! `bin/mn`: `default`/`tc`/`tcu`/`ovs`), the rest are positional args or
//! `key=value` kwargs whose values pass through [`mn_util::make_numeric`].
//!
//! Deviation from CPython (documented in the crate docs): Python *logs and
//! ignores* out-of-range values; here they are parse errors.

use std::time::Duration;

use mn_util::{python_float, split_args, Numeric};

use crate::{Error, Result};

/// `TCIntf.bwParamMax` in `mininet/link.py`: bandwidth above 1000 Mbps is
/// rejected by Python (logged + ignored there, a parse error here).
pub const BW_PARAM_MAX_MBPS: f64 = 1000.0;

/// Link class selected by the first token of a link spec string
/// (`customClass(LINKS, spec)` in `bin/mn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// Plain veth-pair link (`Link` / spec `default`).
    Default,
    /// Link with TC (netem) interfaces (`TCLink` / spec `tc`).
    Tc,
    /// TCLink tuned for UserSwitch, txo/rxo off (`TCULink` / spec `tcu`).
    TcU,
    /// OVS link (`OVSLink` / spec `ovs`) — stub only, see [`crate::ovs`].
    Ovs,
}

impl LinkKind {
    /// Map a spec-string class name to a [`LinkKind`], mirroring the
    /// `LINKS` dict of `bin/mn` (`customClass` raises on unknown names).
    pub fn from_spec_name(name: &str) -> Result<Self> {
        match name {
            "" | "default" => Ok(Self::Default),
            "tc" => Ok(Self::Tc),
            "tcu" => Ok(Self::TcU),
            "ovs" => Ok(Self::Ovs),
            other => Err(Error::Parse(format!(
                "unknown link type {other:?}; expected one of: default, tc, tcu, ovs"
            ))),
        }
    }

    /// Whether this kind applies TC params to its interfaces.
    pub fn uses_tc(&self) -> bool {
        matches!(self, Self::Tc | Self::TcU)
    }
}

/// TCLink parameters, ported from the `TCIntf.config` signature in
/// `mininet/link.py`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkParams {
    /// Bandwidth limit in Mbps (Python `bw`, shaped via `rate %fMbit`).
    pub bw: Option<f64>,
    /// Transmit delay (Python `delay`, e.g. `"5ms"`).
    pub delay: Option<Duration>,
    /// Jitter (Python `jitter`, e.g. `"1ms"`).
    pub jitter: Option<Duration>,
    /// Random packet loss in percent, 0..=100 (Python `loss`).
    pub loss: Option<f64>,
    /// Netem queue limit in packets (Python `max_queue_size` -> `limit %d`).
    pub max_queue_size: Option<u32>,
    /// Experimental switch-side bandwidth override (Python `speedup`).
    pub speedup: u32,
    /// Select HTB for bandwidth shaping (Python default path; recorded here,
    /// see the crate docs for the netem-rate interim).
    pub use_htb: bool,
}

impl Default for LinkParams {
    fn default() -> Self {
        Self {
            bw: None,
            delay: None,
            jitter: None,
            loss: None,
            max_queue_size: None,
            speedup: 0,
            use_htb: true,
        }
    }
}

impl LinkParams {
    /// Whether any traffic-control parameter is set. Mirrors the early
    /// return in `TCIntf.config`: `bw is None and not delay and not loss
    /// and max_queue_size is None`.
    pub fn has_tc(&self) -> bool {
        self.bw.is_some()
            || self.delay.is_some()
            || self.jitter.is_some()
            || self.loss.is_some()
            || self.max_queue_size.is_some()
    }

    /// Effective bandwidth for an interface on `node_name`, mirroring
    /// `TCIntf.bwCmds`: `if speedup > 0 and node.name[0:1] == 's': bw = speedup`.
    pub fn effective_bw_mbps(&self, node_name: &str) -> Option<f64> {
        if self.speedup > 0 && node_name.starts_with('s') {
            Some(f64::from(self.speedup))
        } else {
            self.bw
        }
    }
}

/// Result of [`parse_link_spec`]: link class plus params, with positional
/// args and unknown kwargs preserved (Python passes those through
/// `**params` to the constructor/`config`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkSpec {
    /// Link class named by the first token.
    pub kind: LinkKind,
    /// Parsed TC params.
    pub params: LinkParams,
    /// Positional args (rare in link specs; Python would append them to the
    /// constructor call).
    pub args: Vec<Numeric>,
    /// Unknown kwargs, passed through untouched.
    pub extra: Vec<(String, Numeric)>,
}

/// Parse a link spec string like `"tc,bw=10,delay=5ms,loss=0.1"` using
/// [`mn_util::split_args`] (the port of `splitArgs`).
pub fn parse_link_spec(spec: &str) -> Result<LinkSpec> {
    let split = split_args(spec);
    let kind = LinkKind::from_spec_name(&split.func)?;
    let mut params = LinkParams::default();
    let mut extra = Vec::new();
    for (key, val) in &split.kwargs {
        match key.as_str() {
            "bw" => params.bw = Some(parse_bw(val)?),
            "delay" => params.delay = Some(parse_time_value(val, "delay")?),
            "jitter" => params.jitter = Some(parse_time_value(val, "jitter")?),
            "loss" => params.loss = Some(parse_loss(val)?),
            "max_queue_size" => params.max_queue_size = Some(parse_max_queue_size(val)?),
            "speedup" => params.speedup = parse_speedup(val)?,
            "use_htb" => params.use_htb = parse_bool(val, "use_htb")?,
            _ => extra.push((key.clone(), val.clone())),
        }
    }
    Ok(LinkSpec {
        kind,
        params,
        args: split.args,
        extra,
    })
}

/// Numeric value as f64; text values are rejected (`bw=ten` is an error).
fn numeric_f64(v: &Numeric, key: &str) -> Result<f64> {
    match v {
        Numeric::Int(i) => Ok(*i as f64),
        Numeric::Float(f) => Ok(*f),
        Numeric::Text(t) => Err(Error::Parse(format!("{key}: expected a number, got {t:?}"))),
    }
}

fn parse_bw(v: &Numeric) -> Result<f64> {
    let bw = numeric_f64(v, "bw")?;
    // Python: `if bw and (bw < 0 or bw > bwParamMax): error(...)` and ignores.
    // Here: fail fast. bw = 0 would emit `rate 0Mbit`, which tc rejects, so
    // it is an error too.
    if !bw.is_finite() || bw <= 0.0 || bw > BW_PARAM_MAX_MBPS {
        return Err(Error::Parse(format!(
            "bw={bw} is outside the supported range (0, {BW_PARAM_MAX_MBPS}] Mbps (TCIntf.bwParamMax)"
        )));
    }
    Ok(bw)
}

fn parse_loss(v: &Numeric) -> Result<f64> {
    let loss = numeric_f64(v, "loss")?;
    // Python: `if loss and (loss < 0 or loss > 100): error(...)`.
    // (NaN fails the range check too.)
    if !loss.is_finite() || !(0.0..=100.0).contains(&loss) {
        return Err(Error::Parse(format!(
            "loss={loss} is outside the supported range 0..=100 percent"
        )));
    }
    Ok(loss)
}

fn parse_max_queue_size(v: &Numeric) -> Result<u32> {
    match v {
        Numeric::Int(i) if *i >= 0 => u32::try_from(*i)
            .map_err(|_| Error::Parse(format!("max_queue_size={i} does not fit in u32"))),
        Numeric::Float(f) if *f >= 0.0 && f.fract() == 0.0 => u32::try_from(*f as i64)
            .map_err(|_| Error::Parse(format!("max_queue_size={f} does not fit in u32"))),
        _ => Err(Error::Parse(format!(
            "max_queue_size: expected a non-negative integer, got {v:?}"
        ))),
    }
}

fn parse_speedup(v: &Numeric) -> Result<u32> {
    match v {
        Numeric::Int(i) if *i >= 0 => {
            u32::try_from(*i).map_err(|_| Error::Parse(format!("speedup={i} does not fit in u32")))
        }
        Numeric::Float(f) if *f >= 0.0 && f.fract() == 0.0 => u32::try_from(*f as i64)
            .map_err(|_| Error::Parse(format!("speedup={f} does not fit in u32"))),
        _ => Err(Error::Parse(format!(
            "speedup: expected a non-negative integer, got {v:?}"
        ))),
    }
}

fn parse_bool(v: &Numeric, key: &str) -> Result<bool> {
    match v {
        Numeric::Int(i) => match i {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Parse(format!("{key}: expected 0/1, got {i}"))),
        },
        Numeric::Float(f) if *f == 0.0 => Ok(false),
        Numeric::Float(f) if *f == 1.0 => Ok(true),
        Numeric::Text(t) => match t.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            _ => Err(Error::Parse(format!(
                "{key}: expected a boolean (true/false/yes/no/on/off/1/0), got {t:?}"
            ))),
        },
        _ => Err(Error::Parse(format!(
            "{key}: expected a boolean, got {v:?}"
        ))),
    }
}

/// Time-unit suffixes accepted by iproute2 `get_time64`, ordered so longer
/// suffixes are tried first (`"5us"` must not parse as `"5u"` + `"s"`).
/// Values are in nanoseconds.
const TIME_SUFFIXES_NS: &[(&str, u64)] = &[
    ("msecs", 1_000_000),
    ("usecs", 1_000),
    ("nsecs", 1),
    ("secs", 1_000_000_000),
    ("msec", 1_000_000),
    ("usec", 1_000),
    ("nsec", 1),
    ("sec", 1_000_000_000),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("ns", 1),
    ("s", 1_000_000_000),
];

/// Parse a time value from a spec token. Follows iproute2 `get_time64`:
/// a bare number is **nanoseconds**, and `s`/`sec`/`secs`, `ms`/`msec`/
/// `msecs`, `us`/`usec`/`usecs`, `ns`/`nsec`/`nsecs` suffixes
/// (case-insensitive) scale it. Negative or non-finite values are errors.
pub fn parse_time_str(s: &str, key: &str) -> Result<Duration> {
    let lower = s.to_ascii_lowercase();
    for (suffix, mult) in TIME_SUFFIXES_NS {
        if let Some(num) = lower.strip_suffix(suffix) {
            let val = python_float(num)
                .ok_or_else(|| Error::Parse(format!("{key}: invalid time value {s:?}")))?;
            return nanos_to_duration(val, *mult, s, key);
        }
    }
    // Bare number: nanoseconds, exactly like `tc ... delay 5` (get_time64).
    let val = python_float(&lower)
        .ok_or_else(|| Error::Parse(format!("{key}: invalid time value {s:?}")))?;
    nanos_to_duration(val, 1, s, key)
}

fn nanos_to_duration(val: f64, mult: u64, orig: &str, key: &str) -> Result<Duration> {
    if !val.is_finite() || val < 0.0 {
        return Err(Error::Parse(format!(
            "{key}: time value {orig:?} must be finite and non-negative"
        )));
    }
    let nanos = val * mult as f64;
    if nanos >= u64::MAX as f64 {
        return Err(Error::Parse(format!(
            "{key}: time value {orig:?} overflows Duration"
        )));
    }
    Ok(Duration::from_nanos(nanos as u64))
}

/// Parse a delay/jitter token: a bare [`Numeric::Int`]/[`Numeric::Float`]
/// is nanoseconds (what Python passes through to `tc`), text goes through
/// [`parse_time_str`].
fn parse_time_value(v: &Numeric, key: &str) -> Result<Duration> {
    match v {
        Numeric::Int(i) => {
            if *i < 0 {
                return Err(Error::Parse(format!("{key}: negative time {i}")));
            }
            Ok(Duration::from_nanos(*i as u64))
        }
        Numeric::Float(f) => nanos_to_duration(*f, 1, &f.to_string(), key),
        Numeric::Text(t) => parse_time_str(t, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mn_util::Numeric::{Float, Int, Text};

    #[test]
    fn parses_full_tc_spec() {
        let spec = parse_link_spec("tc,bw=10,delay=5ms,loss=0.1").unwrap();
        assert_eq!(spec.kind, LinkKind::Tc);
        assert_eq!(spec.params.bw, Some(10.0));
        assert_eq!(spec.params.delay, Some(Duration::from_millis(5)));
        assert_eq!(spec.params.loss, Some(0.1));
        assert!(spec.params.use_htb);
        assert_eq!(spec.params.speedup, 0);
        assert!(spec.params.max_queue_size.is_none());
        assert!(spec.args.is_empty());
        assert!(spec.extra.is_empty());
        assert!(spec.params.has_tc());
    }

    #[test]
    fn spec_class_names() {
        assert_eq!(parse_link_spec("").unwrap().kind, LinkKind::Default);
        assert_eq!(parse_link_spec("default").unwrap().kind, LinkKind::Default);
        assert_eq!(parse_link_spec("tc").unwrap().kind, LinkKind::Tc);
        assert_eq!(parse_link_spec("tcu,bw=5").unwrap().kind, LinkKind::TcU);
        assert_eq!(parse_link_spec("ovs").unwrap().kind, LinkKind::Ovs);
        let err = parse_link_spec("foo,bw=10").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("foo") && msg.contains("tc"), "{msg}");
        // default link with no params needs no tc
        assert!(!parse_link_spec("default").unwrap().params.has_tc());
    }

    #[test]
    fn parses_each_param() {
        let p =
            parse_link_spec("tc,delay=1s,jitter=100us,max_queue_size=1000,speedup=5,use_htb=false")
                .unwrap()
                .params;
        assert_eq!(p.delay, Some(Duration::from_secs(1)));
        assert_eq!(p.jitter, Some(Duration::from_micros(100)));
        assert_eq!(p.max_queue_size, Some(1000));
        assert_eq!(p.speedup, 5);
        assert!(!p.use_htb);

        let p = parse_link_spec("tc,bw=2.5,loss=100").unwrap().params;
        assert_eq!(p.bw, Some(2.5));
        assert_eq!(p.loss, Some(100.0));

        // loss = 0 is valid (means "no loss", like Python's `loss > 0` check)
        assert_eq!(parse_link_spec("tc,loss=0").unwrap().params.loss, Some(0.0));
    }

    #[test]
    fn time_units_match_tc_get_time64() {
        let d = |s: &str| parse_time_str(s, "delay").unwrap();
        assert_eq!(d("5ms"), Duration::from_millis(5));
        assert_eq!(d("5MS"), Duration::from_millis(5));
        assert_eq!(d("5msec"), Duration::from_millis(5));
        assert_eq!(d("5msecs"), Duration::from_millis(5));
        assert_eq!(d("100us"), Duration::from_micros(100));
        assert_eq!(d("100usec"), Duration::from_micros(100));
        assert_eq!(d("500ns"), Duration::from_nanos(500));
        assert_eq!(d("500nsec"), Duration::from_nanos(500));
        assert_eq!(d("1s"), Duration::from_secs(1));
        assert_eq!(d("2sec"), Duration::from_secs(2));
        assert_eq!(d("2secs"), Duration::from_secs(2));
        assert_eq!(d("1.5ms"), Duration::from_nanos(1_500_000));
        assert_eq!(d("1e3ms"), Duration::from_secs(1)); // strtod-style exponent
        assert_eq!(d("5"), Duration::from_nanos(5)); // bare number = ns
        assert_eq!(d("0"), Duration::ZERO);
        // "5us" must not parse as "5u"+"s"
        assert_eq!(d("5us"), Duration::from_micros(5));
    }

    #[test]
    fn time_value_rejects_garbage() {
        for bad in [
            "abc",
            "-5ms",
            "5 fortnights",
            "ms",
            "",
            "1.2.3ms",
            "5x",
            "nan",
        ] {
            assert!(
                parse_time_str(bad, "delay").is_err(),
                "{bad:?} should be rejected"
            );
        }
        // Numeric tokens: bare numbers are ns; negatives rejected
        assert_eq!(
            parse_time_value(&Int(5), "delay").unwrap(),
            Duration::from_nanos(5)
        );
        assert_eq!(
            parse_time_value(&Float(2.5), "jitter").unwrap(),
            Duration::from_nanos(2)
        );
        assert!(parse_time_value(&Int(-1), "delay").is_err());
        assert!(parse_time_value(&Text("10ms".to_string()), "delay").is_ok());
    }

    #[test]
    fn invalid_values_are_errors() {
        for bad in [
            "tc,bw=-5",
            "tc,bw=0",
            "tc,bw=1000.5",
            "tc,bw=1001",
            "tc,bw=ten",
            "tc,bw=10k",
            "tc,loss=-0.5",
            "tc,loss=100.5",
            "tc,loss=abc",
            "tc,delay=abc",
            "tc,delay=-5ms",
            "tc,jitter=oops",
            "tc,max_queue_size=-3",
            "tc,max_queue_size=2.5",
            "tc,max_queue_size=abc",
            "tc,speedup=-1",
            "tc,use_htb=maybe",
        ] {
            let err = parse_link_spec(bad).unwrap_err();
            assert!(matches!(err, Error::Parse(_)), "{bad} -> {err:?}");
        }
    }

    #[test]
    fn boundary_values_are_ok() {
        let p = parse_link_spec("tc,bw=1000,loss=0").unwrap().params;
        assert_eq!(p.bw, Some(1000.0));
        assert_eq!(p.loss, Some(0.0));
        assert_eq!(
            parse_link_spec("tc,bw=0.001").unwrap().params.bw,
            Some(0.001)
        );
        assert_eq!(
            parse_link_spec("tc,max_queue_size=4294967295")
                .unwrap()
                .params
                .max_queue_size,
            Some(u32::MAX)
        );
    }

    #[test]
    fn bool_parsing_variants() {
        for (s, want) in [
            ("use_htb=true", true),
            ("use_htb=TRUE", true),
            ("use_htb=yes", true),
            ("use_htb=on", true),
            ("use_htb=1", true),
            ("use_htb=false", false),
            ("use_htb=No", false),
            ("use_htb=off", false),
            ("use_htb=0", false),
        ] {
            assert_eq!(
                parse_link_spec(&format!("tc,{s}")).unwrap().params.use_htb,
                want,
                "{s}"
            );
        }
    }

    #[test]
    fn kwargs_repeat_last_wins_and_extras_pass_through() {
        // mn-util split_args keeps first position, last value (Python dict)
        let spec = parse_link_spec("tc,bw=10,bw=20,foo=bar,port1=3").unwrap();
        assert_eq!(spec.params.bw, Some(20.0));
        assert_eq!(
            spec.extra,
            vec![
                ("foo".to_string(), Text("bar".to_string())),
                ("port1".to_string(), Int(3)),
            ]
        );
        // positional args are captured, not treated as kwargs
        let spec = parse_link_spec("tc,7").unwrap();
        assert_eq!(spec.args, vec![Int(7)]);
    }

    #[test]
    fn effective_bw_speedup_rule() {
        // Python bwCmds: speedup>0 and node name starts with 's' -> bw = speedup
        let p = parse_link_spec("tc,bw=10,speedup=100").unwrap().params;
        assert_eq!(p.effective_bw_mbps("s1"), Some(100.0));
        assert_eq!(p.effective_bw_mbps("h1"), Some(10.0));
        let p = parse_link_spec("tc,bw=10").unwrap().params;
        assert_eq!(p.effective_bw_mbps("s1"), Some(10.0));
        let p = LinkParams::default();
        assert_eq!(p.effective_bw_mbps("s1"), None);
        assert!(!p.has_tc());
    }
}
