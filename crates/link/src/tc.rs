//! Traffic-control (netem qdisc) operations over rtnetlink — the Rust
//! equivalent of the `tc qdisc add dev <intf> ... netem ...` shell-outs in
//! `TCIntf.delayCmds` of `mininet/link.py`.
//!
//! # Approach: netlink, no `tc` shell-out
//!
//! `netlink-packet-route` 0.34 has no typed netem qdisc, but its
//! [`TcAttribute::Other`] accepts a raw [`DefaultNla`], which lets us emit
//! the exact wire format the kernel's `netem_change()` expects (verified
//! against `net/sched/sch_netem.c` and iproute2 `tc/q_netem.c`):
//!
//! - the qdisc message carries `TCA_KIND = "netem"` and a single
//!   `TCA_OPTIONS` attribute;
//! - the `TCA_OPTIONS` payload is a **flat `struct tc_netem_qopt`** (24
//!   bytes, native-endian u32s: `latency`, `limit`, `loss`, `gap`,
//!   `duplicate`, `jitter`) **followed by nested `TCA_NETEM_*` NLAs**
//!   (`qopt = nla_data(opt)`; `parse_attr()` parses nested attrs after
//!   `NLA_ALIGN(sizeof(qopt))`). iproute2 builds the same layout via its
//!   `tail->rta_len = ...` trick.
//!
//! Delay/jitter go into the legacy tick fields (`PSCHED_NS2TICKS`,
//! 1 tick = 64 ns) *and* into the modern `TCA_NETEM_LATENCY64` /
//! `TCA_NETEM_JITTER64` s64-nanosecond attrs, exactly like current iproute2
//! (which only uses the 64-bit attrs); older kernels fall back to ticks.
//! Loss is the fixed-point percent `rint(pct/100 * u32::MAX)`
//! (iproute2 `set_percent`). `max_queue_size` maps to `qopt.limit`
//! (tc's default is 1000 packets). Bandwidth (`bw`) is applied through
//! netem's own rate limiter `TCA_NETEM_RATE64` (bytes/sec) — see the crate
//! docs for why this slice does not build the HTB qdisc+class via netlink.

use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use mn_netlink::Netlink;
use netlink_packet_core::{
    DefaultNla, NetlinkMessage, NetlinkPayload, NLA_F_NESTED, NLM_F_ACK, NLM_F_CREATE, NLM_F_EXCL,
    NLM_F_REQUEST,
};
use netlink_packet_route::tc::{TcAttribute, TcHandle, TcMessage};
use netlink_packet_route::RouteNetlinkMessage;
use rtnetlink::Handle;

use crate::{Error, Result};

/// `TCA_OPTIONS` qdisc attribute kind (`include/uapi/linux/rtnetlink.h`).
pub(crate) const TCA_OPTIONS: u16 = 2;
/// `TCA_NETEM_RATE64`: u64 rate in bytes/sec.
pub(crate) const TCA_NETEM_RATE64: u16 = 8;
/// `TCA_NETEM_LATENCY64`: s64 delay in nanoseconds.
pub(crate) const TCA_NETEM_LATENCY64: u16 = 10;
/// `TCA_NETEM_JITTER64`: s64 jitter in nanoseconds.
pub(crate) const TCA_NETEM_JITTER64: u16 = 11;

/// `PSCHED_SHIFT` from `include/net/pkt_sched.h`: the legacy
/// `tc_netem_qopt` time fields are in ticks of 2^6 = 64 ns.
pub(crate) const PSCHED_SHIFT: u32 = 6;

/// netem queue limit used when the user does not set `max_queue_size`
/// (iproute2's `tc_netem_qopt` default: `.limit = 1000`).
pub(crate) const NETEM_DEFAULT_LIMIT: u32 = 1000;

/// Qdisc handle `10:` used by Python's `TCIntf.delayCmds`
/// (`handle 10: netem ...`), i.e. major 10 minor 0.
pub(crate) const NETEM_HANDLE: u32 = 0x00A0_0000;

/// `TC_H_ROOT` (0xFFFFFFFF) as u32 — parent value identifying the root
/// qdisc in [`QdiscInfo::parent`].
pub(crate) const TC_H_ROOT: u32 = 0xFFFF_FFFF;

/// Parameters of the netem qdisc we install (the `netemargs` of
/// `TCIntf.delayCmds`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetemConfig {
    /// `delay <d>` — added latency.
    pub delay: Option<Duration>,
    /// jitter — random variation of latency (second arg of `delay`).
    pub jitter: Option<Duration>,
    /// `loss <pct>%` — random packet loss in percent.
    pub loss_percent: Option<f64>,
    /// `limit <n>` — queue limit in packets (`max_queue_size`).
    pub limit: Option<u32>,
    /// Rate limit in bytes/sec (`TCA_NETEM_RATE64`); used for `bw` in this
    /// slice (see crate docs).
    pub rate_bytes_per_sec: Option<u64>,
}

/// iproute2 `set_percent`: `*percent = rint(per * UINT32_MAX)` with
/// `per = pct / 100` — the fixed-point loss probability the kernel compares
/// against a u32 PRNG draw (2^32-1 == 100%).
pub fn loss_to_u32(loss_percent: f64) -> u32 {
    ((loss_percent / 100.0) * f64::from(u32::MAX)).round() as u32
}

/// `PSCHED_NS2TICKS`: nanoseconds to legacy psched ticks, capped at u32
/// (the `tc_netem_qopt` fields are u32).
pub fn ns_to_psched_ticks(ns: u128) -> u32 {
    (ns >> PSCHED_SHIFT).min(u128::from(u32::MAX)) as u32
}

/// Append one netlink NLA (`u16 len, u16 kind, payload`, 4-byte aligned).
fn push_nla(buf: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    let len = (4 + payload.len()) as u16;
    buf.extend_from_slice(&len.to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(payload);
    let pad = (4 - payload.len() % 4) % 4;
    buf.extend(std::iter::repeat_n(0u8, pad));
}

/// Serialize the `TCA_OPTIONS` payload of a netem qdisc: flat
/// `tc_netem_qopt` followed by nested `TCA_NETEM_*` NLAs (see module docs).
pub fn build_netem_options(cfg: &NetemConfig) -> Vec<u8> {
    let delay_ns = cfg.delay.map_or(0, |d| d.as_nanos());
    let jitter_ns = cfg.jitter.map_or(0, |d| d.as_nanos());

    // struct tc_netem_qopt { u32 latency; u32 limit; u32 loss; u32 gap;
    //                        u32 duplicate; u32 jitter; }
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(&ns_to_psched_ticks(delay_ns).to_ne_bytes());
    payload.extend_from_slice(&cfg.limit.unwrap_or(NETEM_DEFAULT_LIMIT).to_ne_bytes());
    payload.extend_from_slice(&cfg.loss_percent.map_or(0, loss_to_u32).to_ne_bytes());
    payload.extend_from_slice(&0u32.to_ne_bytes()); // gap (reorder gap; unused)
    payload.extend_from_slice(&0u32.to_ne_bytes()); // duplicate
    payload.extend_from_slice(&ns_to_psched_ticks(jitter_ns).to_ne_bytes());

    if let Some(delay) = cfg.delay {
        push_nla(
            &mut payload,
            TCA_NETEM_LATENCY64,
            &(delay.as_nanos() as i64).to_ne_bytes(),
        );
    }
    if let Some(jitter) = cfg.jitter {
        push_nla(
            &mut payload,
            TCA_NETEM_JITTER64,
            &(jitter.as_nanos() as i64).to_ne_bytes(),
        );
    }
    if let Some(rate) = cfg.rate_bytes_per_sec {
        push_nla(&mut payload, TCA_NETEM_RATE64, &rate.to_ne_bytes());
    }
    payload
}

/// One qdisc as reported by `RTM_GETQDISC` (`tc qdisc show dev <intf>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QdiscInfo {
    /// Qdisc handle (e.g. `0x00A00000` for `10:`).
    pub handle: u32,
    /// Parent handle (`0xFFFFFFFF` for root).
    pub parent: u32,
    /// Qdisc kind (`"netem"`, `"noqueue"`, `"htb"`, ...).
    pub kind: String,
}

/// Synchronous facade over an rtnetlink connection for TrafficControl
/// messages. Same pattern as [`mn_netlink::Netlink`]: owns a current-thread
/// tokio runtime; opening the socket needs no privileges, qdisc
/// add/change/del need root + `CAP_NET_ADMIN`.
///
/// (mn-link keeps its own connection because `mn-netlink` deliberately does
/// not expose its handle or runtime; interface-index resolution is shared
/// by taking a `&Netlink` — see `require_index`. The veth/address ops
/// still go through `mn_netlink::Netlink` — see [`crate::LinkOps`].)
#[derive(Debug)]
pub struct Tc {
    rt: tokio::runtime::Runtime,
    handle: Handle,
}

impl Tc {
    /// Open a `NETLINK_ROUTE` socket and spawn the connection driver on an
    /// internal current-thread tokio runtime.
    pub fn new() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Self::from_runtime(rt)
    }

    /// Build a `Tc` on an existing current-thread runtime. Fork children must
    /// use this with a runtime created **before** fork(2) — creating a
    /// runtime after fork can deadlock on the malloc lock (same rule as
    /// [`mn_netlink::Netlink::from_runtime`]).
    pub fn from_runtime(rt: tokio::runtime::Runtime) -> Result<Self> {
        let handle = rt.block_on(async {
            let (connection, handle, _messages) =
                rtnetlink::new_connection().map_err(|e| Error::Tc(e.to_string()))?;
            tokio::spawn(connection);
            Ok::<Handle, Error>(handle)
        })?;
        Ok(Self { rt, handle })
    }

    fn run<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }

    /// Send one qdisc message (`RTM_NEWQDISC`/`RTM_DELQDISC`) and turn an
    /// NLMSG_ERROR payload into an [`Error::Tc`].
    fn send_qdisc(&self, message: RouteNetlinkMessage, extra_flags: u16) -> Result<()> {
        self.run(async {
            let mut handle = self.handle.clone();
            let mut req = NetlinkMessage::from(message);
            req.header.flags = NLM_F_REQUEST | NLM_F_ACK | extra_flags;
            let mut resp = handle
                .request(req)
                .map_err(|e| Error::Tc(format!("request failed: {e}")))?;
            while let Some(m) = resp.next().await {
                if let NetlinkPayload::Error(e) = m.payload {
                    if e.raw_code() != 0 {
                        return Err(Error::Tc(format!(
                            "kernel rejected qdisc op: {}",
                            e.to_io()
                        )));
                    }
                }
            }
            Ok(())
        })
    }

    /// Install a netem root qdisc on `ifname` (equivalent of
    /// `tc qdisc add dev <ifname> root handle 10: netem <args>`).
    /// `nl` resolves the interface name to its index.
    pub fn add_netem(&self, nl: &Netlink, ifname: &str, cfg: &NetemConfig) -> Result<()> {
        let index = require_index(nl, ifname)?;
        self.add_netem_index(index, cfg)
    }

    /// [`Tc::add_netem`] by interface index.
    pub fn add_netem_index(&self, ifindex: u32, cfg: &NetemConfig) -> Result<()> {
        let mut msg = TcMessage::with_index(ifindex as i32);
        msg.header.handle = TcHandle::from(NETEM_HANDLE);
        msg.header.parent = TcHandle::ROOT;
        msg.attributes.push(TcAttribute::Kind("netem".to_string()));
        msg.attributes.push(TcAttribute::Other(DefaultNla::new(
            TCA_OPTIONS | NLA_F_NESTED,
            build_netem_options(cfg),
        )));
        self.send_qdisc(
            RouteNetlinkMessage::NewQueueDiscipline(msg),
            NLM_F_CREATE | NLM_F_EXCL,
        )
    }

    /// Delete the root qdisc of `ifname` (`tc qdisc del dev <ifname> root`).
    /// `nl` resolves the interface name to its index.
    pub fn del_root_qdisc(&self, nl: &Netlink, ifname: &str) -> Result<()> {
        let index = require_index(nl, ifname)?;
        // Delete by the root qdisc's actual handle: with handle=UNSPEC the
        // kernel can match a handle-0 `noqueue` qdisc and ACK without
        // removing the netem qdisc (observed on ubuntu-24.04 CI).
        // If nothing but a handle-0 default qdisc (noqueue) sits at root,
        // there is nothing to delete — and deleting handle 0 would just
        // ENOENT. (Python likewise skips noqueue/prio.)
        let Some(handle) = self
            .qdisc_list(nl, ifname)?
            .into_iter()
            .find(|q| q.parent == TC_H_ROOT && q.handle != 0)
            .map(|q| q.handle)
        else {
            return Ok(());
        };
        let mut msg = TcMessage::with_index(index as i32);
        msg.header.parent = TcHandle::ROOT;
        msg.header.handle = TcHandle::from(handle);
        self.send_qdisc(RouteNetlinkMessage::DelQueueDiscipline(msg), 0)
    }

    /// List the qdiscs of `ifname` (`tc qdisc show dev <ifname>`).
    /// Works unprivileged. `nl` resolves the interface name to its index.
    pub fn qdisc_list(&self, nl: &Netlink, ifname: &str) -> Result<Vec<QdiscInfo>> {
        let index = require_index(nl, ifname)?;
        self.run(async {
            let mut stream = self.handle.qdisc().get().index(index as i32).execute();
            let mut out = Vec::new();
            while let Some(msg) = stream.next().await {
                let msg = msg.map_err(|e| Error::Tc(e.to_string()))?;
                // Filter in userspace too: the kernel-side index filter is
                // not reliably applied by all rtnetlink versions, and an
                // unfiltered dump would leak other interfaces' qdiscs.
                if msg.header.index != index as i32 {
                    continue;
                }
                let kind = msg
                    .attributes
                    .iter()
                    .find_map(|a| match a {
                        TcAttribute::Kind(k) => Some(k.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                out.push(QdiscInfo {
                    handle: u32::from(msg.header.handle),
                    parent: u32::from(msg.header.parent),
                    kind,
                });
            }
            Ok(out)
        })
    }
}

/// Resolve `ifname` to its interface index via the shared `mn-netlink`
/// facade (one `RTM_GETLINK` dump); error where the interface does not
/// exist.
fn require_index(nl: &Netlink, ifname: &str) -> Result<u32> {
    nl.link_index(ifname)?
        .ok_or_else(|| Error::InvalidInput(format!("no such interface: {ifname:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    fn read_u32(buf: &[u8], off: usize) -> u32 {
        u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap())
    }

    #[test]
    fn loss_fixed_point_matches_iproute2_set_percent() {
        // set_percent: rint(per * UINT32_MAX), per = pct/100
        assert_eq!(loss_to_u32(0.0), 0);
        assert_eq!(loss_to_u32(100.0), u32::MAX);
        assert_eq!(loss_to_u32(0.1), 4_294_967); // rint(0.001 * 4294967295)
        assert_eq!(loss_to_u32(50.0), 2_147_483_648); // rint(0.5 * u32::MAX)
    }

    #[test]
    fn ns_to_ticks_matches_psched_shift() {
        assert_eq!(ns_to_psched_ticks(0), 0);
        assert_eq!(ns_to_psched_ticks(64), 1);
        assert_eq!(ns_to_psched_ticks(5_000_000), 78_125); // 5 ms
        assert_eq!(ns_to_psched_ticks(u128::from(u32::MAX) << 6), u32::MAX);
        assert_eq!(ns_to_psched_ticks(u128::MAX), u32::MAX); // capped
    }

    #[test]
    fn netem_options_payload_layout() {
        let cfg = NetemConfig {
            delay: Some(Duration::from_millis(5)),
            jitter: Some(Duration::from_micros(100)),
            loss_percent: Some(0.1),
            limit: Some(200),
            rate_bytes_per_sec: Some(1_250_000),
        };
        let p = build_netem_options(&cfg);
        // flat tc_netem_qopt first: latency, limit, loss, gap, duplicate, jitter
        assert_eq!(read_u32(&p, 0), 78_125); // 5ms in ticks
        assert_eq!(read_u32(&p, 4), 200); // limit
        assert_eq!(read_u32(&p, 8), 4_294_967); // loss 0.1%
        assert_eq!(read_u32(&p, 12), 0); // gap
        assert_eq!(read_u32(&p, 16), 0); // duplicate
        assert_eq!(read_u32(&p, 20), 1_562); // 100us in ticks
                                             // then nested NLAs at offset 24 (already 4-aligned)
        let mut off = 24;
        let len = u16::from_ne_bytes(p[off..off + 2].try_into().unwrap());
        let kind = u16::from_ne_bytes(p[off + 2..off + 4].try_into().unwrap());
        assert_eq!((len, kind), (4 + 8, TCA_NETEM_LATENCY64));
        assert_eq!(
            i64::from_ne_bytes(p[off + 4..off + 12].try_into().unwrap()),
            5_000_000
        );
        off += len as usize;
        let len = u16::from_ne_bytes(p[off..off + 2].try_into().unwrap());
        let kind = u16::from_ne_bytes(p[off + 2..off + 4].try_into().unwrap());
        assert_eq!((len, kind), (4 + 8, TCA_NETEM_JITTER64));
        assert_eq!(
            i64::from_ne_bytes(p[off + 4..off + 12].try_into().unwrap()),
            100_000
        );
        off += len as usize;
        let len = u16::from_ne_bytes(p[off..off + 2].try_into().unwrap());
        let kind = u16::from_ne_bytes(p[off + 2..off + 4].try_into().unwrap());
        assert_eq!((len, kind), (4 + 8, TCA_NETEM_RATE64));
        assert_eq!(
            u64::from_ne_bytes(p[off + 4..off + 12].try_into().unwrap()),
            1_250_000
        );
        off += len as usize;
        assert_eq!(off, p.len()); // no trailing garbage
    }

    #[test]
    fn netem_options_minimal_uses_tc_default_limit() {
        let p = build_netem_options(&NetemConfig::default());
        assert_eq!(p.len(), 24, "no nested attrs when nothing is set");
        assert_eq!(read_u32(&p, 4), NETEM_DEFAULT_LIMIT);
        assert_eq!(read_u32(&p, 8), 0);
    }

    #[test]
    fn netem_options_nla_alignment() {
        // 1-byte payload would not occur for our attrs, but padding rule:
        // NLA payload padded to 4 bytes.
        let mut buf = Vec::new();
        push_nla(&mut buf, 99, &[7u8]);
        assert_eq!(buf.len(), 8); // 4 header + 1 payload + 3 pad
        assert_eq!(&buf[4..], &[7, 0, 0, 0]);
    }

    // Non-root tests: opening the socket and dumping qdiscs need no
    // privileges.

    #[test]
    fn can_open_tc_connection_without_root() {
        Tc::new().expect("NETLINK_ROUTE socket must open unprivileged");
    }

    #[test]
    fn qdisc_list_runs_without_root() {
        let nl = Netlink::new().unwrap();
        let tc = Tc::new().unwrap();
        let qdiscs = tc
            .qdisc_list(&nl, "lo")
            .expect("RTM_GETQDISC dump works unprivileged");
        assert!(
            !qdiscs.is_empty() && qdiscs.iter().any(|q| q.kind == "noqueue"),
            "expected noqueue on lo, got {qdiscs:?}"
        );
    }

    #[test]
    fn qdisc_list_unknown_interface_is_invalid_input() {
        let nl = Netlink::new().unwrap();
        let tc = Tc::new().unwrap();
        assert!(matches!(
            tc.qdisc_list(&nl, "no-such-intf0"),
            Err(Error::InvalidInput(_))
        ));
    }

    // ------------------------------------------------------------------
    // Root-gated test: install/remove a netem qdisc on a real veth pair.
    // The kernel rejecting our hand-built TCA_OPTIONS with EINVAL is what
    // this test guards against, so it must run on a privileged host:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + CAP_NET_ADMIN on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-link -- --ignored --test-threads=1"]
    fn netem_qdisc_add_and_del_on_veth() {
        if !root_tests_enabled() {
            return;
        }
        use mn_netlink::Netlink;
        let nl = Netlink::new().unwrap();
        let tc = Tc::new().unwrap();
        let suffix = std::process::id() % 100000;
        let a = format!("mntc{suffix}a");
        let b = format!("mntc{suffix}b");
        nl.veth_create_replacing(&a, &b, None, None).unwrap();
        nl.link_set_up(&a, true).unwrap();

        tc.add_netem(
            &nl,
            &a,
            &NetemConfig {
                delay: Some(Duration::from_millis(5)),
                jitter: Some(Duration::from_micros(100)),
                loss_percent: Some(0.1),
                limit: Some(200),
                rate_bytes_per_sec: Some(1_250_000), // 10 Mbps
            },
        )
        .expect("kernel must accept our netem TCA_OPTIONS");
        let qdiscs = tc.qdisc_list(&nl, &a).unwrap();
        assert!(
            qdiscs
                .iter()
                .any(|q| q.kind == "netem" && q.handle == NETEM_HANDLE),
            "netem qdisc missing in {qdiscs:?}"
        );

        tc.del_root_qdisc(&nl, &a).unwrap();
        let qdiscs = tc.qdisc_list(&nl, &a).unwrap();
        assert!(
            !qdiscs.iter().any(|q| q.kind == "netem"),
            "netem qdisc still present in {qdiscs:?}"
        );

        nl.link_delete(&a).unwrap();
        // Deleting one veth end removes the whole pair, so b is already
        // gone — the second delete must be best-effort (Python's
        // Link.delete is likewise a no-op for a missing interface).
        let _ = nl.link_delete(&b);
    }
}
