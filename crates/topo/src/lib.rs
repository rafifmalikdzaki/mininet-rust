//! Topology graph model ported from `mininet/topo.py` (`MultiGraph`, `Topo`,
//! `SingleSwitchTopo`, `SingleSwitchReversedTopo`, `MinimalTopo`,
//! `LinearTopo`) and `mininet/topolib.py` (`TreeTopo`, `TreeNet`,
//! `TorusTopo`).
//!
//! # Layout
//!
//! - [`value`]: [`Value`] / [`Attrs`] — the node/edge attribute dicts
//!   (insertion-ordered, like CPython dicts).
//! - [`graph`]: [`MultiGraph`], [`EdgeKey`], [`EdgeEntry`] —
//!   the networkx.MultiGraph replacement with Python's edge-key
//!   auto-assignment (`max([0] + int keys) + 1`) and aliased
//!   `edge[src][dst] == edge[dst][src]` entries.
//! - [`topo`]: [`Topo`], [`TopoOptions`], [`Ports`], [`PortLookup`], [`Link`] —
//!   the topology database: `add_node`/`add_host`/`add_switch`/`add_link`,
//!   `nodes`/`hosts`/`switches`/`is_switch`, `links(sort, with_keys,
//!   with_info)`, `node_info`/`link_info`, and the legacy port numbering
//!   (`add_port`/`port`, hosts port 0, switches from 1).
//! - [`topolib`]: the built-in topologies ([`SingleSwitchTopo`],
//!   [`SingleSwitchReversedTopo`], [`MinimalTopo`], [`LinearTopo`],
//!   [`TreeTopo`], [`TorusTopo`]), [`tree_net`], and [`build_topo`] — the
//!   port of `buildTopo(TOPOS, spec)` used by `bin/mn --topo ...`.
//!
//! # Fidelity notes / deliberate deviations from CPython
//!
//! - Python raises `KeyError`/`TypeError` on unknown nodes/edges and bad
//!   argument counts; this crate returns `None`/`Err` (the `mn-util`
//!   convention). [`TopoError::python_exception`] maps errors back to the
//!   Python exception class for differential testing.
//! - Python attr values are arbitrary objects; only the JSON-ish subset that
//!   occurs in Mininet (`bool`/`int`/`float`/`str`) is modeled by [`Value`].
//! - `MultiGraph.convert_to` aliases attr dicts in Python (shared references);
//!   here attrs are cloned (Rust ownership).
//! - `Topo::is_switch` on an unknown node returns `false` (Python:
//!   `KeyError`); `Topo::port` returns `None` (Python: `KeyError`).
//! - `TreeNet` returns the [`TreeTopo`] itself; the Python original wraps it
//!   in a `Mininet`, which is out of scope for this crate.
//! - `mininet/topo.py`'s `Topo.convertTo(cls)` accepts any networkx-like
//!   class; here it always produces a [`MultiGraph`].
//! - Build parameters are `i64`; Python accepts arbitrary ints (and, by
//!   accident, floats in some `build` signatures).

pub mod graph;
pub mod topo;
pub mod topolib;
pub mod value;

pub use graph::{EdgeEntry, EdgeKey, MultiGraph};
pub use topo::{Link, PortLookup, PortMap, Ports, Topo, TopoOptions};
pub use topolib::{
    build_topo, tree_net, BuiltTopo, LinearTopo, MinimalTopo, SingleSwitchReversedTopo,
    SingleSwitchTopo, TopoError, TorusTopo, TreeTopo,
};
pub use value::{Attrs, Value};

/// Convenient result alias used throughout the crate (errors are always
/// [`TopoError`]).
pub type Result<T> = std::result::Result<T, TopoError>;
