//! Built-in topologies — port of the topology classes of
//! `mininet/topolib.py` (`TreeTopo`, `TorusTopo`) and `mininet/topo.py`
//! (`SingleSwitchTopo`, `SingleSwitchReversedTopo`, `MinimalTopo`,
//! `LinearTopo`), plus `build_topo`, the port of `buildTopo(TOPOS, spec)`
//! used by `bin/mn --topo ...`.
//!
//! Each Python class becomes a struct wrapping a [`Topo`] (exposed via
//! [`Deref`](std::ops::Deref), [`topo()`](SingleSwitchTopo::topo) and
//! [`into_topo()`](SingleSwitchTopo::into_topo)); the Python `build()` method
//! is kept as a method so a wrapper can be rebuilt.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;

use mn_util::{irange, split_args, Numeric};

use crate::topo::{Topo, TopoOptions};
use crate::value::Attrs;

/// Port of `SingleSwitchTopo`: single switch `s1` connected to `k` hosts
/// `h1..hk`.
#[derive(Debug, Clone)]
pub struct SingleSwitchTopo {
    topo: Topo,
    /// Number of hosts (`self.k`).
    pub k: i64,
}

impl SingleSwitchTopo {
    /// `SingleSwitchTopo(k)`.
    pub fn new(k: i64) -> Self {
        Self::with_options(k, TopoOptions::default())
    }

    /// `SingleSwitchTopo(k, hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(k: i64, options: TopoOptions) -> Self {
        let mut t = Self {
            topo: Topo::with_options(options),
            k,
        };
        t.build();
        t
    }

    /// Port of `build(k=2, **_opts)` (uses the stored [`Self::k`]).
    pub fn build(&mut self) {
        let switch = self.topo.add_switch("s1", Attrs::new());
        for h in irange(1, self.k) {
            let host = self.topo.add_host(&format!("h{h}"), Attrs::new());
            self.topo
                .add_link(&host, &switch, None, None, None, Attrs::new());
        }
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "SingleSwitchTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.topo
    }
}

impl Default for SingleSwitchTopo {
    fn default() -> Self {
        Self::new(2)
    }
}

impl Deref for SingleSwitchTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.topo
    }
}

/// Port of `SingleSwitchReversedTopo`: single switch connected to `k` hosts
/// with reversed ports — the lowest-numbered host gets the highest-numbered
/// switch port (`port1=0, port2=k-h+1`).
#[derive(Debug, Clone)]
pub struct SingleSwitchReversedTopo {
    topo: Topo,
    /// Number of hosts (`self.k`).
    pub k: i64,
}

impl SingleSwitchReversedTopo {
    /// `SingleSwitchReversedTopo(k)`.
    pub fn new(k: i64) -> Self {
        Self::with_options(k, TopoOptions::default())
    }

    /// `SingleSwitchReversedTopo(k, hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(k: i64, options: TopoOptions) -> Self {
        let mut t = Self {
            topo: Topo::with_options(options),
            k,
        };
        t.build();
        t
    }

    /// Port of `build(k=2)` (uses the stored [`Self::k`]).
    pub fn build(&mut self) {
        let switch = self.topo.add_switch("s1", Attrs::new());
        for h in irange(1, self.k) {
            let host = self.topo.add_host(&format!("h{h}"), Attrs::new());
            self.topo.add_link(
                &host,
                &switch,
                Some(0),
                Some(self.k - h + 1),
                None,
                Attrs::new(),
            );
        }
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "SingleSwitchReversedTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.topo
    }
}

impl Default for SingleSwitchReversedTopo {
    fn default() -> Self {
        Self::new(2)
    }
}

impl Deref for SingleSwitchReversedTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.topo
    }
}

/// Port of `MinimalTopo`: two hosts and one switch (`SingleSwitchTopo(k=2)`).
#[derive(Debug, Clone)]
pub struct MinimalTopo {
    /// The inner single-switch topo (`self.k == 2`).
    pub inner: SingleSwitchTopo,
}

impl MinimalTopo {
    /// `MinimalTopo()`.
    pub fn new() -> Self {
        Self::with_options(TopoOptions::default())
    }

    /// `MinimalTopo(hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(options: TopoOptions) -> Self {
        Self {
            inner: SingleSwitchTopo::with_options(2, options),
        }
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "MinimalTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.inner.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.inner.into_topo()
    }
}

impl Default for MinimalTopo {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for MinimalTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.inner.topo
    }
}

/// Port of `LinearTopo`: linear topology of `k` switches with `n` hosts per
/// switch.
#[derive(Debug, Clone)]
pub struct LinearTopo {
    topo: Topo,
    /// Number of switches (`self.k`).
    pub k: i64,
    /// Number of hosts per switch (`self.n`).
    pub n: i64,
}

impl LinearTopo {
    /// `LinearTopo(k, n)`.
    pub fn new(k: i64, n: i64) -> Self {
        Self::with_options(k, n, TopoOptions::default())
    }

    /// `LinearTopo(k, n, hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(k: i64, n: i64, options: TopoOptions) -> Self {
        let mut t = Self {
            topo: Topo::with_options(options),
            k,
            n,
        };
        t.build();
        t
    }

    /// Port of `build(k=2, n=1, **_opts)` (uses the stored [`Self::k`]/[`Self::n`]).
    pub fn build(&mut self) {
        let (k, n) = (self.k, self.n);
        // n == 1 -> 'h<i>'; otherwise 'h<j>s<i>'
        let mut last_switch: Option<String> = None;
        for i in irange(1, k) {
            // Add switch
            let switch = self.topo.add_switch(&format!("s{i}"), Attrs::new());
            // Add hosts to switch
            for j in irange(1, n) {
                let host_name = if n == 1 {
                    format!("h{i}")
                } else {
                    format!("h{j}s{i}")
                };
                let host = self.topo.add_host(&host_name, Attrs::new());
                self.topo
                    .add_link(&host, &switch, None, None, None, Attrs::new());
            }
            // Connect switch to previous
            if let Some(last) = &last_switch {
                self.topo
                    .add_link(&switch, last, None, None, None, Attrs::new());
            }
            last_switch = Some(switch);
        }
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "LinearTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.topo
    }
}

impl Default for LinearTopo {
    fn default() -> Self {
        Self::new(2, 1)
    }
}

impl Deref for LinearTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.topo
    }
}

/// Port of `TreeTopo` from `mininet/topolib.py`: tree network with a given
/// depth and fanout. Numbering: `h1..N`, `s1..M`.
#[derive(Debug, Clone)]
pub struct TreeTopo {
    topo: Topo,
    /// Next host number (`self.hostNum`, starts at 1).
    pub host_num: i64,
    /// Next switch number (`self.switchNum`, starts at 1).
    pub switch_num: i64,
}

impl TreeTopo {
    /// `TreeTopo(depth, fanout)`.
    pub fn new(depth: i64, fanout: i64) -> Self {
        Self::with_options(depth, fanout, TopoOptions::default())
    }

    /// `TreeTopo(depth, fanout, hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(depth: i64, fanout: i64, options: TopoOptions) -> Self {
        let mut t = Self {
            topo: Topo::with_options(options),
            host_num: 1,
            switch_num: 1,
        };
        t.build(depth, fanout);
        t
    }

    /// Port of `build(depth=1, fanout=2)`: resets the numbering counters and
    /// calls [`TreeTopo::add_tree`].
    pub fn build(&mut self, depth: i64, fanout: i64) {
        // Numbering: h1..N, s1..M
        self.host_num = 1;
        self.switch_num = 1;
        // Build topology
        self.add_tree(depth, fanout);
    }

    /// Port of `addTree(depth, fanout)`: add a subtree; returns the last node
    /// added.
    pub fn add_tree(&mut self, depth: i64, fanout: i64) -> String {
        let is_switch = depth > 0;
        if is_switch {
            let node = self
                .topo
                .add_switch(&format!("s{}", self.switch_num), Attrs::new());
            self.switch_num += 1;
            for _ in 0..fanout {
                let child = self.add_tree(depth - 1, fanout);
                self.topo
                    .add_link(&node, &child, None, None, None, Attrs::new());
            }
            node
        } else {
            let node = self
                .topo
                .add_host(&format!("h{}", self.host_num), Attrs::new());
            self.host_num += 1;
            node
        }
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "TreeTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.topo
    }
}

impl Default for TreeTopo {
    fn default() -> Self {
        Self::new(1, 2)
    }
}

impl Deref for TreeTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.topo
    }
}

/// Port of `TreeNet(depth=1, fanout=2, **kwargs)` from `mininet/topolib.py`.
///
/// Deviation: the Python original returns `Mininet(TreeTopo(depth, fanout),
/// **kwargs)`; the `Mininet` orchestrator is out of scope for this crate (it
/// is ported later, on top of `mn-topo`), so this returns the built
/// [`TreeTopo`] itself.
pub fn tree_net(depth: i64, fanout: i64) -> TreeTopo {
    TreeTopo::new(depth, fanout)
}

/// Port of `TorusTopo` from `mininet/topolib.py`: 2-D torus of `x` by `y`
/// switches with `n` hosts per switch.
///
/// WARNING (from the Python docstring): this topology has LOOPS and will not
/// work with the default controller or any Ethernet bridge without STP.
#[derive(Debug, Clone)]
pub struct TorusTopo {
    topo: Topo,
}

impl TorusTopo {
    /// `TorusTopo(x, y, n)`; errors when `x < 3 || y < 3` like the Python
    /// original.
    pub fn new(x: i64, y: i64, n: i64) -> Result<Self, TopoError> {
        Self::with_options(x, y, n, TopoOptions::default())
    }

    /// `TorusTopo(x, y, n, hopts=..., sopts=..., lopts=...)`.
    pub fn with_options(x: i64, y: i64, n: i64, options: TopoOptions) -> Result<Self, TopoError> {
        let mut t = Self {
            topo: Topo::with_options(options),
        };
        t.build(x, y, n)?;
        Ok(t)
    }

    /// Port of `build(x, y, n=1)`.
    pub fn build(&mut self, x: i64, y: i64, n: i64) -> Result<(), TopoError> {
        if x < 3 || y < 3 {
            return Err(TopoError::TorusTooSmall);
        }
        // n == 1 -> 'h<loc>'; otherwise 'h<loc>x<k>'
        let mut hosts: HashMap<(i64, i64, i64), String> = HashMap::new();
        let mut switches: HashMap<(i64, i64), String> = HashMap::new();
        // Create and wire interior
        for i in 0..x {
            for j in 0..y {
                let loc = format!("{}x{}", i + 1, j + 1);
                // dpid cannot be zero for OVS
                let dpid = (i + 1) * 256 + (j + 1);
                let switch = self.topo.add_switch(
                    &format!("s{loc}"),
                    crate::attrs!("dpid" => format!("{dpid:x}")),
                );
                switches.insert((i, j), switch.clone());
                for k in 0..n {
                    let host_name = if n == 1 {
                        format!("h{loc}")
                    } else {
                        format!("h{loc}x{}", k + 1)
                    };
                    let host = self.topo.add_host(&host_name, Attrs::new());
                    hosts.insert((i, j, k), host.clone());
                    self.topo
                        .add_link(&host, &switch, None, None, None, Attrs::new());
                }
            }
        }
        // Connect switches
        for i in 0..x {
            for j in 0..y {
                let sw1 = &switches[&(i, j)];
                let sw2 = &switches[&(i, (j + 1) % y)];
                let sw3 = &switches[&((i + 1) % x, j)];
                self.topo.add_link(sw1, sw2, None, None, None, Attrs::new());
                self.topo.add_link(sw1, sw3, None, None, None, Attrs::new());
            }
        }
        Ok(())
    }

    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        "TorusTopo"
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        &self.topo
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        self.topo
    }
}

impl Deref for TorusTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        &self.topo
    }
}

/// A built-in topology produced by [`build_topo`] (the `TOPOS` dict of
/// `bin/mn`).
#[derive(Debug, Clone)]
pub enum BuiltTopo {
    /// `minimal` — [`MinimalTopo`].
    Minimal(MinimalTopo),
    /// `single` — [`SingleSwitchTopo`].
    SingleSwitch(SingleSwitchTopo),
    /// `reversed` — [`SingleSwitchReversedTopo`].
    SingleSwitchReversed(SingleSwitchReversedTopo),
    /// `linear` — [`LinearTopo`].
    Linear(LinearTopo),
    /// `tree` — [`TreeTopo`].
    Tree(TreeTopo),
    /// `torus` — [`TorusTopo`].
    Torus(TorusTopo),
}

impl BuiltTopo {
    /// Class name as `type(topo).__name__` would report.
    pub fn class_name(&self) -> &'static str {
        match self {
            BuiltTopo::Minimal(t) => t.class_name(),
            BuiltTopo::SingleSwitch(t) => t.class_name(),
            BuiltTopo::SingleSwitchReversed(t) => t.class_name(),
            BuiltTopo::Linear(t) => t.class_name(),
            BuiltTopo::Tree(t) => t.class_name(),
            BuiltTopo::Torus(t) => t.class_name(),
        }
    }

    /// The backing [`Topo`].
    pub fn topo(&self) -> &Topo {
        match self {
            BuiltTopo::Minimal(t) => t.topo(),
            BuiltTopo::SingleSwitch(t) => t.topo(),
            BuiltTopo::SingleSwitchReversed(t) => t.topo(),
            BuiltTopo::Linear(t) => t.topo(),
            BuiltTopo::Tree(t) => t.topo(),
            BuiltTopo::Torus(t) => t.topo(),
        }
    }

    /// Consume the wrapper, returning the backing [`Topo`].
    pub fn into_topo(self) -> Topo {
        match self {
            BuiltTopo::Minimal(t) => t.into_topo(),
            BuiltTopo::SingleSwitch(t) => t.into_topo(),
            BuiltTopo::SingleSwitchReversed(t) => t.into_topo(),
            BuiltTopo::Linear(t) => t.into_topo(),
            BuiltTopo::Tree(t) => t.into_topo(),
            BuiltTopo::Torus(t) => t.into_topo(),
        }
    }
}

impl Deref for BuiltTopo {
    type Target = Topo;
    fn deref(&self) -> &Topo {
        self.topo()
    }
}

/// Errors from [`build_topo`] / [`TorusTopo`], mapped to the Python exception
/// classes the original raises (see [`TopoError::python_exception`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopoError {
    /// Python `Exception('Invalid topo name %s')` from `buildTopo`.
    InvalidName(String),
    /// Python `Exception('Please use 3x3 or greater for compatibility with
    /// 2.1')` from `TorusTopo.build`.
    TorusTooSmall,
    /// Python `TypeError` from wrong/missing/non-int build arguments or an
    /// unsupported keyword argument.
    BadArgs(String),
}

impl TopoError {
    /// The exception class the Python original raises for this error.
    pub fn python_exception(&self) -> &'static str {
        match self {
            TopoError::InvalidName(_) | TopoError::TorusTooSmall => "Exception",
            TopoError::BadArgs(_) => "TypeError",
        }
    }
}

impl fmt::Display for TopoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopoError::InvalidName(name) => write!(f, "Invalid topo name {name}"),
            TopoError::TorusTooSmall => {
                write!(f, "Please use 3x3 or greater for compatibility with 2.1")
            }
            TopoError::BadArgs(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for TopoError {}

/// Port of `buildTopo(TOPOS, topoStr)` from `mininet/util.py` with the
/// `TOPOS` dict of `bin/mn`: parse `name,arg1,kw=val...` with
/// [`mn_util::split_args`] and construct the matching built-in topology.
///
/// Defaults and arities mirror the Python `build` signatures
/// (`single`/`reversed`: `k=2`; `linear`: `k=2, n=1`; `tree`: `depth=1,
/// fanout=2`; `torus`: `x, y, n=1` required-ish; `minimal`: no args).
/// Keyword arguments are only accepted where the Python `build(**_opts)`
/// swallows them (`single`, `linear`); `hopts`/`sopts`/`lopts` scalars from a
/// spec string are rejected with a `TypeError`-mapped error, matching the
/// Python failure when the popped scalar is later unpacked as node opts.
pub fn build_topo(spec: &str) -> Result<BuiltTopo, TopoError> {
    let parsed = split_args(spec);
    let name = parsed.func.as_str();
    let accepts_kwargs = matches!(name, "single" | "linear");
    for (k, _) in &parsed.kwargs {
        match k.as_str() {
            "hopts" | "sopts" | "lopts" => {
                return Err(TopoError::BadArgs(format!(
                    "keyword argument '{k}' must be a dict, got a scalar from the topo string"
                )));
            }
            _ if !accepts_kwargs => {
                return Err(TopoError::BadArgs(format!(
                    "build() got an unexpected keyword argument '{k}'"
                )));
            }
            _ => {}
        }
    }
    let opt = |i: usize, default: i64| -> Result<i64, TopoError> {
        match parsed.args.get(i) {
            None => Ok(default),
            Some(Numeric::Int(v)) => Ok(*v),
            Some(other) => Err(TopoError::BadArgs(format!(
                "argument {} must be an int, got {other:?}",
                i + 1
            ))),
        }
    };
    let req = |i: usize, what: &str| -> Result<i64, TopoError> {
        match parsed.args.get(i) {
            Some(Numeric::Int(v)) => Ok(*v),
            Some(other) => Err(TopoError::BadArgs(format!(
                "argument {what} must be an int, got {other:?}"
            ))),
            None => Err(TopoError::BadArgs(format!(
                "build() missing required positional argument '{what}'"
            ))),
        }
    };
    let check_arity = |max: usize| -> Result<(), TopoError> {
        if parsed.args.len() > max {
            return Err(TopoError::BadArgs(format!(
                "build() takes at most {} positional argument(s), got {}",
                max + 1,
                parsed.args.len() + 1
            )));
        }
        Ok(())
    };
    match name {
        "minimal" => {
            check_arity(0)?;
            Ok(BuiltTopo::Minimal(MinimalTopo::new()))
        }
        "single" => {
            check_arity(1)?;
            Ok(BuiltTopo::SingleSwitch(SingleSwitchTopo::new(opt(0, 2)?)))
        }
        "reversed" => {
            check_arity(1)?;
            Ok(BuiltTopo::SingleSwitchReversed(
                SingleSwitchReversedTopo::new(opt(0, 2)?),
            ))
        }
        "linear" => {
            check_arity(2)?;
            Ok(BuiltTopo::Linear(LinearTopo::new(opt(0, 2)?, opt(1, 1)?)))
        }
        "tree" => {
            check_arity(2)?;
            Ok(BuiltTopo::Tree(TreeTopo::new(opt(0, 1)?, opt(1, 2)?)))
        }
        "torus" => {
            check_arity(3)?;
            let x = req(0, "x")?;
            let y = req(1, "y")?;
            let n = opt(2, 1)?;
            Ok(BuiltTopo::Torus(TorusTopo::new(x, y, n)?))
        }
        other => Err(TopoError::InvalidName(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::EdgeKey;
    use crate::topo::PortLookup;
    use crate::value::Value;

    #[test]
    fn single_switch_topo() {
        let t = SingleSwitchTopo::new(3);
        assert_eq!(t.k, 3);
        assert_eq!(t.class_name(), "SingleSwitchTopo");
        assert_eq!(t.nodes(true), vec!["h1", "h2", "h3", "s1"]);
        assert_eq!(t.hosts(true), vec!["h1", "h2", "h3"]);
        assert_eq!(t.switches(true), vec!["s1"]);
        assert_eq!(
            t.node_info("s1").unwrap().get("isSwitch"),
            Some(&Value::Bool(true))
        );
        let links = t.links(true, true, true);
        assert_eq!(links.len(), 3);
        for (i, l) in links.iter().enumerate() {
            let h = i + 1;
            assert_eq!((l.node1, l.node2), (format!("h{h}").as_str(), "s1"));
            assert_eq!(l.key, Some(&EdgeKey::Int(1)));
            assert_eq!(l.info.unwrap().get("port1"), Some(&Value::Int(0)));
            assert_eq!(l.info.unwrap().get("port2"), Some(&Value::Int(h as i64)));
        }
        assert_eq!(t.port("h2", "s1"), Some(PortLookup::Single(0, 2)));
    }

    #[test]
    fn single_switch_reversed_topo() {
        let t = SingleSwitchReversedTopo::new(3);
        // h1 -> port 3, h2 -> port 2, h3 -> port 1
        assert_eq!(t.port("h1", "s1"), Some(PortLookup::Single(0, 3)));
        assert_eq!(t.port("h2", "s1"), Some(PortLookup::Single(0, 2)));
        assert_eq!(t.port("h3", "s1"), Some(PortLookup::Single(0, 1)));
        assert_eq!(t.port("s1", "h1"), Some(PortLookup::Single(3, 0)));
    }

    #[test]
    fn minimal_topo() {
        let t = MinimalTopo::new();
        assert_eq!(t.class_name(), "MinimalTopo");
        assert_eq!(t.inner.k, 2);
        assert_eq!(t.hosts(true), vec!["h1", "h2"]);
        assert_eq!(t.switches(true), vec!["s1"]);
        assert_eq!(t.links(false, false, false).len(), 2);
    }

    #[test]
    fn linear_topo_naming() {
        let t = LinearTopo::new(3, 1);
        assert_eq!(t.hosts(true), vec!["h1", "h2", "h3"]);
        assert_eq!(t.switches(true), vec!["s1", "s2", "s3"]);
        // switch links: s2-s1, s3-s2 (added as (s<i>, s<i-1>))
        let links: Vec<(String, String)> = t
            .links(true, false, false)
            .iter()
            .map(|l| (l.node1.to_string(), l.node2.to_string()))
            .collect();
        assert_eq!(
            links,
            vec![
                ("h1".to_string(), "s1".to_string()),
                ("h2".to_string(), "s2".to_string()),
                ("h3".to_string(), "s3".to_string()),
                ("s2".to_string(), "s1".to_string()),
                ("s3".to_string(), "s2".to_string()),
            ]
        );
        let t2 = LinearTopo::new(2, 2);
        assert_eq!(t2.hosts(true), vec!["h1s1", "h1s2", "h2s1", "h2s2"]);
    }

    #[test]
    fn tree_topo_counts_and_numbering() {
        let t = TreeTopo::new(2, 2);
        assert_eq!(t.host_num, 5, "4 hosts numbered");
        assert_eq!(t.switch_num, 4, "3 switches numbered");
        assert_eq!(t.hosts(true), vec!["h1", "h2", "h3", "h4"]);
        assert_eq!(t.switches(true), vec!["s1", "s2", "s3"]);
        // depth 1 fanout 3: s1 with h1..h3
        let t1 = TreeTopo::new(1, 3);
        assert_eq!(t1.hosts(true), vec!["h1", "h2", "h3"]);
        assert_eq!(t1.switches(true), vec!["s1"]);
        assert_eq!(t1.links(false, false, false).len(), 3);
        // edge case tree,1,1: one switch, one host
        let t11 = TreeTopo::new(1, 1);
        assert_eq!(t11.nodes(true), vec!["h1", "s1"]);
        assert_eq!(t11.port("s1", "h1"), Some(PortLookup::Single(1, 0)));
        // fanout 0 with depth: bare switch
        let t0 = TreeTopo::new(1, 0);
        assert_eq!(t0.nodes(true), vec!["s1"]);
    }

    #[test]
    fn tree_net_returns_tree_topo() {
        let t = tree_net(2, 2);
        assert_eq!(t.class_name(), "TreeTopo");
        assert_eq!(t.hosts(true).len(), 4);
    }

    #[test]
    fn torus_topo_rejects_small() {
        assert_eq!(
            TorusTopo::new(2, 3, 1).unwrap_err(),
            TopoError::TorusTooSmall
        );
        assert_eq!(
            TorusTopo::new(3, 2, 1).unwrap_err().python_exception(),
            "Exception"
        );
    }

    #[test]
    fn torus_topo_3x3() {
        let t = TorusTopo::new(3, 3, 1).expect("3x3 ok");
        assert_eq!(t.switches(true).len(), 9);
        assert_eq!(t.hosts(true).len(), 9);
        // dpid of s1x1 = (0+1)*256 + (0+1) = 257 = 0x101
        assert_eq!(
            t.node_info("s1x1").unwrap().get("dpid"),
            Some(&Value::Str("101".into()))
        );
        // dpid of s3x3 = 3*256+3 = 771 = 0x303
        assert_eq!(
            t.node_info("s3x3").unwrap().get("dpid"),
            Some(&Value::Str("303".into()))
        );
        // 9 host links + 9*2 switch links = 27
        assert_eq!(t.links(false, false, false).len(), 27);
        // n=2 host naming: h1x1x1, h1x1x2
        let t2 = TorusTopo::new(3, 3, 2).expect("ok");
        assert!(t2.hosts(true).contains(&"h1x1x2".to_string()));
        assert_eq!(t2.hosts(true).len(), 18);
    }

    #[test]
    fn build_topo_specs() {
        let t = build_topo("single,3").expect("ok");
        assert_eq!(t.class_name(), "SingleSwitchTopo");
        assert_eq!(t.hosts(true).len(), 3);

        let t = build_topo("linear,4").expect("ok");
        assert_eq!(t.class_name(), "LinearTopo");
        assert_eq!(t.switches(true).len(), 4);

        let t = build_topo("tree,2,2").expect("ok");
        assert_eq!(t.class_name(), "TreeTopo");

        let t = build_topo("minimal").expect("ok");
        assert_eq!(t.class_name(), "MinimalTopo");

        let t = build_topo("single").expect("default k=2");
        match &t {
            BuiltTopo::SingleSwitch(s) => assert_eq!(s.k, 2),
            _ => panic!(),
        }

        let t = build_topo("tree").expect("defaults");
        match &t {
            BuiltTopo::Tree(tr) => {
                assert_eq!(tr.hosts(true).len(), 2);
                assert_eq!(tr.switches(true).len(), 1);
            }
            _ => panic!(),
        }

        let t = build_topo("torus,3,3").expect("ok");
        assert_eq!(t.class_name(), "TorusTopo");

        let t = build_topo("reversed,4").expect("ok");
        assert_eq!(t.class_name(), "SingleSwitchReversedTopo");
        assert_eq!(t.port("h1", "s1"), Some(PortLookup::Single(0, 4)));
    }

    #[test]
    fn build_topo_errors() {
        assert_eq!(
            build_topo("bogus,1").unwrap_err(),
            TopoError::InvalidName("bogus".to_string())
        );
        assert_eq!(
            build_topo("bogus").unwrap_err().python_exception(),
            "Exception"
        );
        assert_eq!(
            build_topo("torus,2,2").unwrap_err(),
            TopoError::TorusTooSmall
        );
        let e = build_topo("torus,3").unwrap_err();
        assert_eq!(e.python_exception(), "TypeError", "missing y");
        let e = build_topo("tree,2,2,foo=1").unwrap_err();
        assert_eq!(
            e.python_exception(),
            "TypeError",
            "tree build takes no kwargs"
        );
        let e = build_topo("single,3,hopts=1").unwrap_err();
        assert_eq!(e.python_exception(), "TypeError");
        // single/linear swallow extra kwargs like build(**_opts)
        assert!(build_topo("single,3,foo=1").is_ok());
        assert!(build_topo("linear,2,foo=1").is_ok());
        let e = build_topo("minimal,1").unwrap_err();
        assert_eq!(e.python_exception(), "TypeError", "too many args");
        let e = build_topo("single,2.5").unwrap_err();
        assert_eq!(e.python_exception(), "TypeError", "non-int k");
    }

    #[test]
    fn build_topo_edge_cases() {
        // single,1 / linear,1 / tree,1,1
        let t = build_topo("single,1").expect("ok");
        assert_eq!(t.nodes(true), vec!["h1", "s1"]);
        let t = build_topo("linear,1").expect("ok");
        assert_eq!(t.nodes(true), vec!["h1", "s1"]);
        assert_eq!(t.links(false, false, false).len(), 1);
        let t = build_topo("tree,1,1").expect("ok");
        assert_eq!(t.nodes(true), vec!["h1", "s1"]);
        let t = build_topo("tree,3,4").expect("ok");
        // hosts = 4^3 = 64, switches = (4^3 - 1)/3 = 21
        assert_eq!(t.hosts(true).len(), 64);
        assert_eq!(t.switches(true).len(), 21);
    }
}
