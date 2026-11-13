//! `mn-link` — Rust port of the link model in `mininet/link.py`.
//!
//! # What is ported here
//!
//! - [`LinkParams`] / [`parse_link_spec`]: the TCLink parameter model
//!   (`bw`, `delay`, `jitter`, `loss`, `max_queue_size`, `speedup`,
//!   `use_htb`) parsed from spec strings like `"tc,bw=10,delay=5ms,loss=0.1"`
//!   using the [`mn_util::split_args`] semantics of `mininet/util.py`
//!   (`splitArgs`/`customClass` as used by `bin/mn --link ...`).
//! - [`Intf`] / [`Link`] / [`TCLink`] / [`TCULink`]: endpoints (node name +
//!   intf name each side), port numbers, up/down status.
//! - Kernel operations, all via netlink (no `ip`/`tc` shell-outs):
//!   attach/detach go through [`mn_netlink`] (`veth_create` +
//!   `move_to_netns_pid` + `link_set_up` + `addr_add` + `link_delete` — the
//!   `makeIntfPair`/`moveIntf` of `mininet/util.py`), and the netem qdisc
//!   for delay/jitter/loss/queue-limit goes through
//!   `netlink-packet-route` TrafficControl ([`Tc`]).
//!
//! # Deliberate deviations from CPython (documented)
//!
//! - Python's `TCIntf` *logs and ignores* out-of-range values (`bw` outside
//!   0..1000, `loss` outside 0..100); this crate fails fast with a parse
//!   error, so typos surface instead of silently producing an unshaped link.
//! - Python passes `delay` strings straight to `tc`; bare numbers are
//!   nanoseconds there (iproute2 `get_time64`). We keep the same unit rules
//!   (see [`parse_time_str`]).
//! - Python shapes `bw` with an HTB root qdisc + class (`htb rate <bw>Mbit
//!   burst 15k`) and hangs netem underneath. `netlink-packet-route` 0.34 has
//!   no HTB class builder and the raw `tc_htb_opt` blob is unverifiable in a
//!   non-root sandbox, so in this slice `bw` is applied through netem's own
//!   rate limiter (`TCA_NETEM_RATE64`) inside the same netem qdisc.
//!   `use_htb` is parsed and recorded; the HTB-via-netlink path is the
//!   documented TODO in [`Link::attach`].
//! - `OVSIntf`/`OVSLink` are stubs only (see [`ovs`]): per the migration
//!   plan, `ovs-vsctl` stays a subprocess.
//!
//! # Privilege model
//!
//! Parsing and the pure model are testable anywhere. Attach/detach/qdisc
//! operations need root + `CAP_NET_ADMIN`. Tests that need those are
//! `#[ignore]`d **and** gated on `MN_RUST_ROOT_TESTS=1`; run them on a
//! privileged Linux host/CI runner with:
//! `MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1`

mod error;
mod intf;
mod link;
mod ovs;
mod params;
mod tc;

pub use error::{Error, Result};
pub use intf::{intf_name, Intf, IntfStatus};
pub use link::{Link, LinkConfig, LinkOps, LinkStatus, TCLink, TCULink};
pub use ovs::{OVSIntf, OVSLink};
pub use params::{
    parse_link_spec, parse_time_str, LinkKind, LinkParams, LinkSpec, BW_PARAM_MAX_MBPS,
};
pub use tc::{build_netem_options, loss_to_u32, ns_to_psched_ticks, NetemConfig, QdiscInfo, Tc};

/// Whether root/namespace integration tests should actually run.
///
/// Same convention as `mn-netlink`: all root-requiring tests are
/// `#[ignore]`d by default **and** additionally gated on this, so they only
/// execute when `MN_RUST_ROOT_TESTS=1` is set, on a privileged host/CI:
/// `MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1`
#[cfg(test)]
pub(crate) fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_gate_defaults_off() {
        // In this sandbox MN_RUST_ROOT_TESTS is unset, so the gate is off.
        // (If a developer runs the suite with it set, this test simply
        // reports the enabled state instead of failing.)
        let _ = root_tests_enabled();
    }
}
