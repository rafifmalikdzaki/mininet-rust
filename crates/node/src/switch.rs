//! `Switch` family — port of `class Switch( Node )` and its subclasses
//! (`UserSwitch`, `OVSSwitch`/`OVSKernelSwitch`, `OVSBridge`, `IVSSwitch`)
//! in `mininet/node.py`.
//!
//! Per the migration plan (`tasks/plan.md`, risk table), the OVS control
//! plane stays behind **`ovs-vsctl` subprocess calls** (not ported to
//! netlink). All of the `ovs-vsctl` argument vectors are built by **pure,
//! unit-tested functions** ([`vsctl_add_br`], [`vsctl_del_br`],
//! [`vsctl_set_controller`], [`vsctl_set_fail_mode`],
//! [`vsctl_set_datapath_id`], [`vsctl_add_port`], [`vsctl_del_port`], ...);
//! the switch types only run them via `Node::pexec`.
//!
//! # Porting notes / deviations from CPython (documented)
//!
//! - Python's `OVSSwitch.start` issues ONE `ovs-vsctl` command
//!   (`--id=@c create Controller ... -- --if-exists del-br ... -- add-br
//!   ... -- set bridge ... controller=[@...] <bridgeOpts> -- add-port ...`)
//!   through the node shell. Here the same operations run as separate
//!   `ovs-vsctl` subprocess argv (del-br → add-br → set bridge opts →
//!   set-controller → add-port per interface), because `pexec` passes argv
//!   directly and needs no shell quoting. `reconnectms` (Python's
//!   `max_backoff` on the Controller row) is kept as a model field but is
//!   not expressible via the dedicated `set-controller` command.
//! - `vsctl()` runs through `Node::pexec` (direct subprocess) rather than
//!   Python's `self.cmd('ovs-vsctl', *args)` shell string, so it works
//!   whether or not the node shell is up; like Python's `cmd`, a non-zero
//!   `ovs-vsctl` exit code does not raise — the merged output is returned.
//! - `TCReapply` (OVS vs. tc fighting over qdiscs) is a no-op TODO: qdiscs
//!   are owned by `mn-link` (`TCIntf` equivalent), which re-applies them.
//! - `OVSIntf` patch-port options (`type=patch options:peer=...`) need the
//!   link object, which the node model does not hold; `intf_opts_args`
//!   accepts an optional `patch_peer` for callers that know it.
//! - `UserSwitch`/`IVSSwitch` `start()` launch the `ofdatapath`/`ofprotocol`
//!   resp. `ivs` subprocesses through the node shell and are **root-gated**
//!   ([`require_root`]); their `pathCheck` moves from `__init__` to `start`
//!   so the model stays constructible (and unit-testable) on machines
//!   without those binaries.
//! - Python's `UserSwitch.dpidLen` is 12 (vs. `Switch.dpidLen` 16) — see
//!   [`USER_DPID_LEN`] / [`default_dpid_len`].

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

use mn_link::Intf;

use crate::controller::Controller;
use crate::error::{Error, Result};
use crate::mnexec::which;
use crate::node::Node;

/// Datapath ID length in hex digits (`Switch.dpidLen`).
pub const DPID_LEN: usize = 16;

/// `UserSwitch.dpidLen` is 12 (Python class attribute override).
pub const USER_DPID_LEN: usize = 12;

/// The OVS control-plane binary (retained subprocess per the plan).
pub const OVS_VSCTL: &str = "ovs-vsctl";

/// The OpenFlow control binary used by `dpctl`.
pub const OVS_OFCTL: &str = "ovs-ofctl";

// ---------------------------------------------------------------------
// dpid derivation (Switch.defaultDpid)
// ---------------------------------------------------------------------

/// Port of `Switch.defaultDpid` with an explicit digit count: a given dpid
/// is validated (hex, ≤ `len` digits, colons stripped); otherwise the hex
/// of the **first digit run** in the switch name is used
/// (`s1 -> 1`, `s1a2 -> 1`), zero-padded to `len`.
/// Python: `nums = re.findall(r'\d+', name); dpid = hex(int(nums[0]))[2:]`.
pub fn default_dpid_len(name: &str, dpid: Option<&str>, len: usize) -> Result<String> {
    if let Some(dpid) = dpid {
        let cleaned: String = dpid.chars().filter(|c| *c != ':').collect();
        if cleaned.is_empty() || cleaned.len() > len || u128::from_str_radix(&cleaned, 16).is_err()
        {
            return Err(Error::InvalidInput(format!(
                "invalid dpid {dpid:?}: must be 1..={len} hex digits"
            )));
        }
        return Ok(format!("{:0>width$}", cleaned, width = len));
    }
    // First run of ASCII digits (re.findall(r'\d+', name)[0]), not the
    // concatenation of all digits: "s1a2" must yield 1, not 12.
    let mut first_run: Option<String> = None;
    let mut cur = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            first_run = Some(std::mem::take(&mut cur));
            break;
        }
    }
    let digits = first_run.or(if cur.is_empty() { None } else { Some(cur) });
    let digits = digits.ok_or_else(|| {
        Error::InvalidInput(format!(
            "Unable to derive default datapath ID for {name:?} - please either \
             specify a dpid or use a canonical switch name such as s23."
        ))
    })?;
    let n = digits
        .parse::<u128>()
        .map_err(|_| Error::InvalidInput(format!("switch name number too large: {name:?}")))?;
    Ok(format!("{:0>width$}", format!("{n:x}"), width = len))
}

/// Port of `Switch.defaultDpid` (16 hex digits).
pub fn default_dpid(name: &str, dpid: Option<&str>) -> Result<String> {
    default_dpid_len(name, dpid, DPID_LEN)
}

// ---------------------------------------------------------------------
// Pure ovs-vsctl argv builders (unit-tested; no subprocess)
// ---------------------------------------------------------------------

/// `ovs-vsctl add-br <br>` — Python `OVSSwitch.start`'s `-- add-br %s`.
pub fn vsctl_add_br(br: &str) -> Vec<String> {
    vec![OVS_VSCTL.into(), "add-br".into(), br.into()]
}

/// `ovs-vsctl [--if-exists] del-br <br>` — Python start's
/// `-- --if-exists del-br %s` (non-old OVS) and stop's `ovs-vsctl del-br`.
pub fn vsctl_del_br(br: &str, if_exists: bool) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into()];
    if if_exists {
        v.push("--if-exists".into());
    }
    v.extend(["del-br".into(), br.into()]);
    v
}

/// `ovs-vsctl set-controller <br> <target...>` — the dedicated-command
/// equivalent of Python's `--id=@c create Controller target=...` +
/// `-- set bridge <br> controller=[@c...]` pair. Targets are
/// [`controller_target`] (`tcp:ip:port`) or [`ptcp_target`] (`ptcp:port`).
pub fn vsctl_set_controller(br: &str, targets: &[String]) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into(), "set-controller".into(), br.into()];
    v.extend(targets.iter().cloned());
    v
}

/// `ovs-vsctl del-controller <br>`.
pub fn vsctl_del_controller(br: &str) -> Vec<String> {
    vec![OVS_VSCTL.into(), "del-controller".into(), br.into()]
}

/// `ovs-vsctl set-fail-mode <br> <secure|standalone>`.
pub fn vsctl_set_fail_mode(br: &str, mode: &str) -> Vec<String> {
    vec![
        OVS_VSCTL.into(),
        "set-fail-mode".into(),
        br.into(),
        mode.into(),
    ]
}

/// `ovs-vsctl set bridge <br> other_config:datapath-id=<dpid>` — the
/// datapath ID from [`default_dpid`].
pub fn vsctl_set_datapath_id(br: &str, dpid: &str) -> Vec<String> {
    vec![
        OVS_VSCTL.into(),
        "set".into(),
        "bridge".into(),
        br.into(),
        format!("other_config:datapath-id={dpid}"),
    ]
}

/// `ovs-vsctl set bridge <br> <column[:key]=value>...` — generic form used
/// with [`bridge_opts`].
pub fn vsctl_set_bridge(br: &str, assignments: &[String]) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into(), "set".into(), "bridge".into(), br.into()];
    v.extend(assignments.iter().cloned());
    v
}

/// `ovs-vsctl add-port <br> <intf> [extra...]` — Python's
/// `-- add-port %s %s` + `intfOpts`; `extra` comes from [`intf_opts_args`].
pub fn vsctl_add_port(br: &str, intf: &str, extra: &[String]) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into(), "add-port".into(), br.into(), intf.into()];
    v.extend(extra.iter().cloned());
    v
}

/// `ovs-vsctl [--if-exists] del-port <br> <intf>`.
pub fn vsctl_del_port(br: &str, intf: &str, if_exists: bool) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into()];
    if if_exists {
        v.push("--if-exists".into());
    }
    v.extend(["del-port".into(), br.into(), intf.into()]);
    v
}

/// `ovs-vsctl -- get Bridge <br> Controller` — Python
/// `controllerUUIDs`' query.
pub fn vsctl_get_bridge_controller(br: &str) -> Vec<String> {
    vec![
        OVS_VSCTL.into(),
        "--".into(),
        "get".into(),
        "Bridge".into(),
        br.into(),
        "Controller".into(),
    ]
}

/// `ovs-vsctl -- get Controller <uuid> is_connected` — Python
/// `connected()`'s per-UUID query.
pub fn vsctl_get_controller_is_connected(uuid: &str) -> Vec<String> {
    vec![
        OVS_VSCTL.into(),
        "--".into(),
        "get".into(),
        "Controller".into(),
        uuid.into(),
        "is_connected".into(),
    ]
}

/// One `ovs-vsctl` invocation deleting several bridges — Python
/// `batchShutdown`: `'ovs-vsctl ' + ' -- '.join(delcmd % s)` with
/// `delcmd = '--if-exists del-br %s'` (non-old OVS) or `'del-br %s'`.
pub fn vsctl_batch_del_br(brs: &[&str], if_exists: bool) -> Vec<String> {
    let mut v = vec![OVS_VSCTL.into()];
    for (i, br) in brs.iter().enumerate() {
        if i > 0 {
            v.push("--".into());
        }
        if if_exists {
            v.push("--if-exists".into());
        }
        v.extend(["del-br".into(), (*br).to_string()]);
    }
    v
}

/// `ovs-ofctl <args[0]> <br> <args[1..]>` — Python `OVSSwitch.dpctl`:
/// `self.cmd('ovs-ofctl', args[0], self, *args[1:])`.
pub fn ofctl_dpctl_argv(br: &str, args: &[String]) -> Result<Vec<String>> {
    if args.is_empty() {
        return Err(Error::InvalidInput("dpctl requires arguments".to_string()));
    }
    let mut v = vec![OVS_OFCTL.into(), args[0].clone(), br.into()];
    v.extend(args[1..].iter().cloned());
    Ok(v)
}

/// Controller connection target — Python `'%s:%s:%d' % (c.protocol,
/// c.IP(), c.port)` → `tcp:127.0.0.1:6653`.
pub fn controller_target(protocol: &str, ip: IpAddr, port: u16) -> String {
    format!("{protocol}:{ip}:{port}")
}

/// Passive listen-port controller target — Python
/// `'ptcp:%s' % self.listenPort` → `ptcp:6640`.
pub fn ptcp_target(port: u16) -> String {
    format!("ptcp:{port}")
}

/// Port of `OVSSwitch.bridgeOpts()`: the `set bridge <br> ...` column
/// assignments. Python builds one string; split by the shell into these
/// separate argv words (`other_config:...`/`other-config:...` are the same
/// column; OVS accepts both spellings, Python uses both).
#[allow(clippy::too_many_arguments)]
pub fn bridge_opts(
    dpid: &str,
    fail_mode: &str,
    inband: bool,
    datapath: &str,
    protocols: Option<&str>,
    stp: bool,
    name: &str,
    old_ovs: bool,
) -> Vec<String> {
    let mut opts = vec![
        format!("other_config:datapath-id={dpid}"),
        format!("fail_mode={fail_mode}"),
    ];
    if !inband {
        opts.push("other-config:disable-in-band=true".to_string());
    }
    if datapath == "user" {
        opts.push("datapath_type=netdev".to_string());
    }
    if let Some(p) = protocols {
        if !old_ovs {
            opts.push(format!("protocols={p}"));
        }
    }
    if stp && fail_mode == "standalone" {
        opts.push("stp_enable=true".to_string());
    }
    opts.push(format!("other-config:dp-desc={name}"));
    opts
}

/// Port of `OVSSwitch.intfOpts()` as argv words to append after
/// `add-port <br> <intf>`: `-- set Interface <intf> ofport_request=<port>
/// [type=patch options:peer=<peer>]`. Empty when old OVS or no port.
pub fn intf_opts_args(
    intf: &str,
    port: Option<u32>,
    patch_peer: Option<&str>,
    old_ovs: bool,
) -> Vec<String> {
    let mut column_args: Vec<String> = Vec::new();
    if !old_ovs {
        // ofport_request is not supported on old OVS
        if let Some(p) = port {
            column_args.push(format!("ofport_request={p}"));
        }
        // Patch ports don't work well with old OVS
        if let Some(peer) = patch_peer {
            column_args.push("type=patch".to_string());
            column_args.push(format!("options:peer={peer}"));
        }
    }
    if column_args.is_empty() {
        return Vec::new();
    }
    let mut v = vec![
        "--".to_string(),
        "set".to_string(),
        "Interface".to_string(),
        intf.to_string(),
    ];
    v.extend(column_args);
    v
}

/// Parse the output of `ovs-vsctl -- get Bridge <br> Controller` into UUIDs
/// (Python `controllerUUIDs`: strip `[...]`, split on `,`, trim; a
/// non-bracketed non-empty output leaves the cache empty).
pub fn parse_controller_uuids(out: &str) -> Vec<String> {
    let t = out.trim();
    let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    if inner.trim().is_empty() {
        Vec::new()
    } else {
        inner.split(',').map(|s| s.trim().to_string()).collect()
    }
}

// ---------------------------------------------------------------------
// OVS version helpers (OVSSwitch.setup / isOldOVS)
// ---------------------------------------------------------------------

/// Extract the first `\d+\.\d+` version from `ovs-vsctl --version` output —
/// Python `findall(r'\d+\.\d+', version)[0]`.
pub fn ovs_version_from(output: &str) -> Option<String> {
    let b = output.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                return Some(output[start..j].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Numeric dot-version `<` (StrictVersion-style for the numeric versions
/// OVS prints, e.g. `1.9 < 1.10`).
fn version_lt(a: &str, b: &str) -> bool {
    let pa: Vec<&str> = a.split('.').collect();
    let pb: Vec<&str> = b.split('.').collect();
    for (x, y) in pa.iter().zip(pb.iter()) {
        match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(xn), Ok(yn)) => {
                if xn != yn {
                    return xn < yn;
                }
            }
            _ => {
                if x != y {
                    return x < y;
                }
            }
        }
    }
    pa.len() < pb.len()
}

/// Python `OVSSwitch.isOldOVS`: version < 1.10.
pub fn is_old_ovs_version(version: &str) -> bool {
    version_lt(version, "1.10")
}

// ---------------------------------------------------------------------
// UserSwitch / IVSSwitch command builders (pure)
// ---------------------------------------------------------------------

/// The `--listen=` option `UserSwitch.__init__` appends to `self.opts`.
pub fn user_switch_listen_opt(listen_port: Option<u16>, name: &str) -> String {
    match listen_port {
        Some(p) => format!(" --listen=ptcp:{p} "),
        None => format!(" --listen=punix:/tmp/{name}.listen "),
    }
}

/// `UserSwitch.dpctl`'s listen address: `tcp:127.0.0.1:<port>` or
/// `unix:/tmp/<name>.listen`.
pub fn dpctl_listen_addr(listen_port: Option<u16>, name: &str) -> String {
    match listen_port {
        Some(p) => format!("tcp:127.0.0.1:{p}"),
        None => format!("unix:/tmp/{name}.listen"),
    }
}

/// The `ofdatapath ...` shell line of `UserSwitch.start`
/// (Python: `'ofdatapath -i ' + intfs + ' punix:/tmp/' + name + ' -d %s '
/// + dpopts + ' 1> ofdlog 2> ofdlog &'`, intfs comma-joined).
pub fn ofdatapath_cmd(name: &str, dpid: &str, dpopts: &str, intfs: &[String], log: &str) -> String {
    format!(
        "ofdatapath -i {} punix:/tmp/{name} -d {dpid} {dpopts} 1> {log} 2> {log} &",
        intfs.join(",")
    )
}

/// The `ofprotocol ...` shell line of `UserSwitch.start`.
pub fn ofprotocol_cmd(name: &str, clist: &str, opts: &str, log: &str) -> String {
    format!("ofprotocol unix:/tmp/{name} {clist} --fail=closed {opts} 1> {log} 2> {log} &")
}

/// `IVSSwitch.start`'s argv: `ivs --name <name> --dpid <dpid> [--verbose]
/// [-i <intf>]* [-c <ip:port>]* [--listen 127.0.0.1:<port>] [<opts>]`.
/// (Python appends `self.opts` even when empty; in argv form an empty word
/// is dropped.)
pub fn ivs_start_argv(
    name: &str,
    dpid: &str,
    verbose: bool,
    intfs: &[String],
    controllers: &[(IpAddr, u16)],
    listen_port: Option<u16>,
    opts: &str,
) -> Vec<String> {
    let mut args = vec![
        "ivs".to_string(),
        "--name".to_string(),
        name.to_string(),
        "--dpid".to_string(),
        dpid.to_string(),
    ];
    if verbose {
        args.push("--verbose".to_string());
    }
    for i in intfs {
        args.extend(["-i".to_string(), i.clone()]);
    }
    for (ip, port) in controllers {
        args.extend(["-c".to_string(), format!("{ip}:{port}")]);
    }
    if let Some(p) = listen_port {
        args.extend(["--listen".to_string(), format!("127.0.0.1:{p}")]);
    }
    if !opts.is_empty() {
        args.push(opts.to_string());
    }
    args
}

/// `IVSSwitch.start`'s logfile — Python `'/tmp/ivs.%s.log' % self.name`.
pub fn ivs_logfile(name: &str) -> String {
    format!("/tmp/ivs.{name}.log")
}

/// Root gate for the user-space switch launches (`ofdatapath`/`ofprotocol`,
/// `ivs` run in the root namespace and need `CAP_SYS_ADMIN`/TUN).
pub(crate) fn require_root(what: &str) -> Result<()> {
    // SAFETY: geteuid(2) is a pure query syscall.
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{what} requires root (euid != 0) in this sandbox"
        )))
    }
}

// ---------------------------------------------------------------------
// Switch (base)
// ---------------------------------------------------------------------

/// A switch node: a [`Node`] with a datapath ID. Port of
/// `class Switch( Node )`.
#[derive(Debug)]
pub struct Switch {
    node: Node,
    /// Datapath ID, 16 hex digits (`Switch.dpid`).
    pub dpid: String,
    /// Extra switch options (`Switch.opts`).
    pub opts: String,
    /// Port to listen on for dpctl connections (`Switch.listenPort`).
    pub listen_port: Option<u16>,
    /// Control interface (`lo`) for root-ns switches.
    pub control_intf: Option<Intf>,
}

impl Switch {
    /// Port of `Switch.__init__` (model only).
    pub fn new(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_with_dpid_len(
            name,
            dpid,
            opts,
            listen_port,
            in_namespace,
            DPID_LEN,
            params,
        )
    }

    /// [`Switch::new`] with an explicit dpid length (`UserSwitch` uses 12).
    pub fn new_with_dpid_len(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        dpid_len: usize,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut node = Node::new(name, in_namespace, params);
        node.port_base = 1; // Switch.portBase: switches start with port 1
        node.set_kind("Switch");
        let dpid = default_dpid_len(name, dpid, dpid_len)?;
        let control_intf = if !in_namespace {
            Some(Intf::new("lo", name, Some(0)))
        } else {
            None
        };
        Ok(Self {
            node,
            dpid,
            opts: opts.to_string(),
            listen_port,
            control_intf,
        })
    }

    /// Base `Switch` has no datapath startup — Python's `Switch` defines no
    /// `start` (only the subclasses do); Mininet only ever instantiates
    /// subclasses. Kept as an explicit error so orchestrators calling
    /// `switch.start(..)` uniformly get a clear message.
    pub fn start(&mut self, _controllers: &[&Controller]) -> Result<()> {
        Err(Error::Other(format!(
            "{}: base Switch has no start() — Python's Switch defines none; \
             use OVSSwitch/OVSBridge/UserSwitch/IVSSwitch",
            self.node.name
        )))
    }

    /// Port of `Switch.stop`: only deletes interfaces. Python's default is
    /// `deleteIntfs=True`.
    pub fn stop(&mut self) {
        self.stop_with(true);
    }

    /// Port of `Switch.stop(deleteIntfs)`: delete interfaces when asked.
    pub fn stop_with(&mut self, delete_intfs: bool) {
        if delete_intfs {
            self.node.delete_intfs(true);
        }
    }

    /// Port of `Switch.defaultIntf`: the control interface (`lo`) when the
    /// switch runs in the root namespace, else the node's default
    /// interface — Python: `return self.controlIntf or
    /// Node.defaultIntf(self)`.
    pub fn default_intf(&self) -> Option<&Intf> {
        self.control_intf
            .as_ref()
            .or_else(|| self.node.default_intf())
    }

    /// Python `Switch.connected()` assumes connected by default.
    pub fn connected(&self) -> bool {
        true
    }

    /// Make sure the node shell is running (Python starts it in
    /// `Node.__init__`; the Rust lifecycle defers it to `Node::start`).
    pub(crate) fn ensure_shell(&mut self) -> Result<()> {
        if !self.node.started() {
            self.node.start()?;
        }
        Ok(())
    }

    /// The inner node.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Mutable access to the inner node.
    pub fn node_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl Deref for Switch {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Switch {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Switch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

// ---------------------------------------------------------------------
// OVSSwitch
// ---------------------------------------------------------------------

/// `OVSSwitch` — port of `class OVSSwitch( Switch )` ("Depends on
/// ovs-vsctl"). `OVSKernelSwitch = OVSSwitch` in Python.
#[derive(Debug)]
pub struct OVSSwitch {
    switch: Switch,
    /// Controller loss behavior (`failMode='secure'`): `secure` or
    /// `standalone`.
    pub fail_mode: String,
    /// `datapath='kernel'`: `kernel` or `user`.
    pub datapath: String,
    /// Use in-band control (`inband=False`).
    pub inband: bool,
    /// OpenFlow version(s), e.g. `OpenFlow13` (`protocols=None`).
    pub protocols: Option<String>,
    /// Max reconnect timeout in ms (`reconnectms=1000`; 0/None = OVS
    /// default). Model only — see module docs (not expressible via the
    /// dedicated `set-controller` command).
    pub reconnect_ms: Option<u32>,
    /// Enable STP (`stp=False`, requires `failMode=standalone`).
    pub stp: bool,
    /// Batch startup: queue vsctl argvs instead of running them
    /// (`batch=False`; drained by [`OVSSwitch::batch_startup`]).
    pub batch: bool,
    /// Queued vsctl argvs when `batch` is set (Python `self.commands`).
    pub commands: Vec<Vec<String>>,
    /// Cached controller UUIDs (Python `self._uuids`).
    uuids: Vec<String>,
    /// Detected OVS version (`ovs-vsctl --version`), if `setup` ran.
    ovs_version: Option<String>,
    /// The `ovs-vsctl` binary to exec (overridable in tests).
    vsctl_bin: String,
}

impl OVSSwitch {
    /// Port of `OVSSwitch.__init__`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        fail_mode: &str,
        datapath: &str,
        inband: bool,
        protocols: Option<&str>,
        reconnect_ms: Option<u32>,
        stp: bool,
        batch: bool,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        if !matches!(fail_mode, "secure" | "standalone") {
            return Err(Error::InvalidInput(format!(
                "invalid failMode {fail_mode:?}: expected 'secure' or 'standalone' \
                 (Python fails later, inside ovs-vsctl)"
            )));
        }
        if !matches!(datapath, "kernel" | "user") {
            return Err(Error::InvalidInput(format!(
                "invalid datapath {datapath:?}: expected 'kernel' or 'user'"
            )));
        }
        let mut switch = Switch::new(name, dpid, opts, listen_port, in_namespace, params)?;
        switch.node_mut().set_kind("OVSSwitch");
        Ok(Self {
            switch,
            fail_mode: fail_mode.to_string(),
            datapath: datapath.to_string(),
            inband,
            protocols: protocols.map(str::to_string),
            reconnect_ms,
            stp,
            batch,
            commands: Vec::new(),
            uuids: Vec::new(),
            ovs_version: None,
            vsctl_bin: OVS_VSCTL.to_string(),
        })
    }

    /// Convenience with Python's defaults (`failMode='secure'`,
    /// `datapath='kernel'`, inband/stp/batch=false, reconnectms=1000).
    pub fn with_defaults(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new(
            name,
            dpid,
            opts,
            listen_port,
            "secure",
            "kernel",
            false,
            None,
            Some(1000),
            false,
            false,
            in_namespace,
            params,
        )
    }

    /// Record the detected OVS version (Python `cls.OVSVersion`).
    pub fn set_ovs_version(&mut self, version: &str) {
        self.ovs_version = Some(version.to_string());
    }

    /// Python `OVSSwitch.isOldOVS`: version < 1.10 (false when unknown).
    pub fn is_old_ovs(&self) -> bool {
        self.ovs_version.as_deref().is_some_and(is_old_ovs_version)
    }

    /// Override the `ovs-vsctl` binary (tests use a fake script).
    pub fn set_vsctl_bin(&mut self, bin: &str) {
        self.vsctl_bin = bin.to_string();
    }

    /// Port of `OVSSwitch.setup`: check `ovs-vsctl` is on PATH and can talk
    /// to ovsdb (`ovs-vsctl -t 1 show`), and return the detected version
    /// (`ovs-vsctl --version`). Python `exit(1)`s on failure — here an
    /// error is returned.
    pub fn setup() -> Result<String> {
        if which(OVS_VSCTL).is_none() {
            return Err(Error::InvalidInput(format!(
                "Cannot find required executable {OVS_VSCTL:?} (Open vSwitch); \
                 please make sure it is installed and available in your $PATH"
            )));
        }
        let show = std::process::Command::new(OVS_VSCTL)
            .args(["-t", "1", "show"])
            .output()
            .map_err(|source| Error::Spawn {
                cmd: format!("{OVS_VSCTL} -t 1 show"),
                source,
            })?;
        if !show.status.success() {
            return Err(Error::Other(format!(
                "*** Error connecting to ovs-db with ovs-vsctl (exit {:?}):\n{}{}\n\
                 Make sure that Open vSwitch is installed, that ovsdb-server is \
                 running, and that \"ovs-vsctl show\" works correctly.",
                show.status.code(),
                String::from_utf8_lossy(&show.stdout),
                String::from_utf8_lossy(&show.stderr)
            )));
        }
        let version_out = std::process::Command::new(OVS_VSCTL)
            .arg("--version")
            .output()
            .map_err(|source| Error::Spawn {
                cmd: format!("{OVS_VSCTL} --version"),
                source,
            })?;
        let version = ovs_version_from(&String::from_utf8_lossy(&version_out.stdout))
            .ok_or_else(|| Error::Other("could not parse ovs-vsctl --version".to_string()))?;
        Ok(version)
    }

    /// Run one `ovs-vsctl` argv now (substituting the configured binary).
    fn run_vsctl(&mut self, argv: &[String]) -> Result<String> {
        debug_assert_eq!(argv.first().map(String::as_str), Some(OVS_VSCTL));
        let mut argv = argv.to_vec();
        argv[0] = self.vsctl_bin.clone();
        let out = self.switch.node_mut().pexec(&argv)?;
        Ok(out.merged())
    }

    /// Port of `OVSSwitch.vsctl`: run an `ovs-vsctl` command (or queue it
    /// for later execution when `batch` is set). Returns the merged
    /// stdout+stderr, like Python's `cmd` on the pty.
    pub fn vsctl(&mut self, argv: &[String]) -> Result<String> {
        if self.batch {
            self.commands.push(argv.to_vec());
            return Ok(String::new());
        }
        self.run_vsctl(argv)
    }

    /// Port of `OVSSwitch.dpctl`: `ovs-ofctl <args[0]> <br> <args[1..]>`.
    pub fn dpctl(&mut self, args: &[String]) -> Result<String> {
        let argv = ofctl_dpctl_argv(&self.switch.name, args)?;
        let out = self.switch.node_mut().pexec(&argv)?;
        Ok(out.merged())
    }

    /// Port of `OVSSwitch.attach`: `add-port` + `ifconfig <intf> up` +
    /// TCReapply (TODO no-op — see module docs).
    pub fn attach(&mut self, intf: &str) -> Result<()> {
        self.vsctl(&vsctl_add_port(&self.switch.name, intf, &[]))?;
        let _ = self.switch.node_mut().cmd(&format!("ifconfig {intf} up"));
        // TODO(8b): TCReapply — mn-link owns qdiscs.
        Ok(())
    }

    /// Port of `OVSSwitch.detach`: `del-port`.
    pub fn detach(&mut self, intf: &str) -> Result<()> {
        self.vsctl(&vsctl_del_port(&self.switch.name, intf, false))?;
        Ok(())
    }

    /// Port of `OVSSwitch.controllerUUIDs(update=False)`.
    pub fn controller_uuids(&mut self, update: bool) -> Vec<String> {
        if self.uuids.is_empty() || update {
            if let Ok(out) = self.vsctl(&vsctl_get_bridge_controller(&self.switch.name)) {
                self.uuids = parse_controller_uuids(&out);
            }
        }
        self.uuids.clone()
    }

    /// Port of `OVSSwitch.connected`: any controller UUID `is_connected`,
    /// else `failMode == 'standalone'`.
    pub fn connected(&mut self) -> bool {
        for uuid in self.controller_uuids(false) {
            if let Ok(out) = self.vsctl(&vsctl_get_controller_is_connected(&uuid)) {
                if out.contains("true") {
                    return true;
                }
            }
        }
        self.fail_mode == "standalone"
    }

    /// Port of `OVSSwitch.start(controllers)` — start up a new OVS bridge
    /// using `ovs-vsctl` subprocess calls.
    pub fn start(&mut self, controllers: &[&Controller]) -> Result<()> {
        if self.switch.in_namespace {
            return Err(Error::Other(format!(
                "{}: OVS kernel switch does not work in a namespace",
                self.switch.name
            )));
        }
        // Python: int(self.dpid, 16) — dpid must be a hex string (guaranteed
        // by default_dpid_len, which validated it).
        let br = self.switch.name.clone();
        let old = self.is_old_ovs();
        // Try to delete any existing bridge with the same name.
        self.vsctl(&vsctl_del_br(&br, !old))?;
        // Create the bridge.
        self.vsctl(&vsctl_add_br(&br))?;
        // Bridge options: datapath-id (from default_dpid), fail_mode,
        // disable-in-band, datapath_type, protocols, stp, dp-desc.
        let opts = bridge_opts(
            &self.switch.dpid,
            &self.fail_mode,
            self.inband,
            &self.datapath,
            self.protocols.as_deref(),
            self.stp,
            &br,
            old,
        );
        self.vsctl(&vsctl_set_bridge(&br, &opts))?;
        // Controller entries: tcp:ip:port per controller, plus the passive
        // ptcp:<listenPort> when set.
        let mut targets: Vec<String> = controllers
            .iter()
            .map(|c| controller_target(&c.protocol, c.ip(), c.port))
            .collect();
        if let Some(lp) = self.switch.listen_port {
            targets.push(ptcp_target(lp));
        }
        if !targets.is_empty() {
            self.vsctl(&vsctl_set_controller(&br, &targets))?;
        }
        // Add the data ports: interfaces with a port and no IP, with
        // ofport_request/patch opts (Python folds these into the one big
        // vsctl command via intfOpts).
        let ported: Vec<(String, Option<u32>)> = self
            .switch
            .intf_list()
            .iter()
            .filter(|i| i.port.is_some() && i.ip.is_none())
            .map(|i| (i.name.clone(), i.port))
            .collect();
        for (intf, port) in ported {
            let extra = intf_opts_args(&intf, port, None, old);
            self.vsctl(&vsctl_add_port(&br, &intf, &extra))?;
        }
        // TODO(8b): TCReapply per interface — mn-link owns qdiscs.
        Ok(())
    }

    /// Port of `OVSSwitch.batchStartup`: run each switch's queued vsctl
    /// argvs (Python concatenates them into shell commands bounded by
    /// ARG_MAX; here each queued argv is exec'd directly).
    pub fn batch_startup(switches: &mut [&mut OVSSwitch]) -> Result<()> {
        for sw in switches.iter_mut() {
            let cmds = std::mem::take(&mut sw.commands);
            for argv in cmds {
                sw.run_vsctl(&argv)?;
            }
            sw.batch = false;
        }
        Ok(())
    }

    /// Port of `OVSSwitch.stop`: `ovs-vsctl del-br <br>`, plus
    /// `ip link del <br>` for user datapaths, then delete interfaces.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        let _ = self.vsctl(&vsctl_del_br(&self.switch.name, false));
        if self.datapath == "user" {
            let name = self.switch.name.clone();
            let _ = self.switch.node_mut().cmd(&format!("ip link del {name}"));
        }
        self.switch.stop_with(delete_intfs);
        Ok(())
    }

    /// Port of `OVSSwitch.batchShutdown`: one `ovs-vsctl` command deleting
    /// all bridges, then SIGHUP each switch's process group (via
    /// `Node::terminate`, which sends SIGHUP to the group) and terminate.
    pub fn batch_shutdown(switches: &mut [&mut OVSSwitch]) -> Result<()> {
        if switches.is_empty() {
            return Ok(());
        }
        let old = switches[0].is_old_ovs();
        let names: Vec<&str> = switches.iter().map(|s| s.switch.name.as_str()).collect();
        let mut argv = vsctl_batch_del_br(&names, !old);
        argv[0] = switches[0].vsctl_bin.clone();
        let _ = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .map_err(|source| Error::Spawn {
                cmd: argv.join(" "),
                source,
            })?;
        for sw in switches.iter_mut() {
            // Python: run('kill -HUP ' + pids) then switch.terminate();
            // terminate() already killpg(SIGHUP)s the shell's group.
            sw.switch.node_mut().terminate()?;
        }
        Ok(())
    }

    /// The inner base switch.
    pub fn switch(&self) -> &Switch {
        &self.switch
    }

    /// Mutable access to the inner base switch.
    pub fn switch_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl Deref for OVSSwitch {
    type Target = Switch;
    fn deref(&self) -> &Switch {
        &self.switch
    }
}

impl DerefMut for OVSSwitch {
    fn deref_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl fmt::Display for OVSSwitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.switch.fmt(f)
    }
}

// ---------------------------------------------------------------------
// OVSBridge
// ---------------------------------------------------------------------

/// `OVSBridge` — port of `class OVSBridge( OVSSwitch )`: an OVSSwitch in
/// standalone/bridge mode (`failMode='standalone'` forced).
#[derive(Debug)]
pub struct OVSBridge {
    ovs: OVSSwitch,
}

impl OVSBridge {
    /// Port of `OVSBridge.__init__`: `kwargs.update(failMode='standalone')`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        datapath: &str,
        inband: bool,
        protocols: Option<&str>,
        reconnect_ms: Option<u32>,
        stp: bool,
        batch: bool,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut ovs = OVSSwitch::new(
            name,
            dpid,
            opts,
            listen_port,
            "standalone",
            datapath,
            inband,
            protocols,
            reconnect_ms,
            stp,
            batch,
            in_namespace,
            params,
        )?;
        ovs.switch.node_mut().set_kind("OVSBridge");
        Ok(Self { ovs })
    }

    /// Port of `OVSBridge.start`: start the bridge, ignoring controllers.
    pub fn start(&mut self, _controllers: &[&Controller]) -> Result<()> {
        self.ovs.start(&[])
    }

    /// Port of `OVSBridge.connected`: with STP, wait for
    /// `STP_FORWARD` (and not `STP_LEARN`) in `ovs-ofctl show`; else true.
    pub fn connected(&mut self) -> bool {
        if self.ovs.stp {
            if let Ok(status) = self.ovs.dpctl(&["show".to_string()]) {
                return status.contains("STP_FORWARD") && !status.contains("STP_LEARN");
            }
            return false;
        }
        true
    }

    /// The inner OVS switch.
    pub fn ovs(&self) -> &OVSSwitch {
        &self.ovs
    }

    /// Mutable access to the inner OVS switch.
    pub fn ovs_mut(&mut self) -> &mut OVSSwitch {
        &mut self.ovs
    }
}

impl Deref for OVSBridge {
    type Target = OVSSwitch;
    fn deref(&self) -> &OVSSwitch {
        &self.ovs
    }
}

impl DerefMut for OVSBridge {
    fn deref_mut(&mut self) -> &mut OVSSwitch {
        &mut self.ovs
    }
}

impl fmt::Display for OVSBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ovs.fmt(f)
    }
}

// ---------------------------------------------------------------------
// UserSwitch
// ---------------------------------------------------------------------

/// `UserSwitch` — port of `class UserSwitch( Switch )`, the OpenFlow
/// reference user switch (`ofdatapath`/`ofprotocol`). The subprocess launch
/// is root-gated.
#[derive(Debug)]
pub struct UserSwitch {
    switch: Switch,
    /// Additional arguments to ofdatapath (`dpopts='--no-slicing'`).
    pub dpopts: String,
}

impl UserSwitch {
    /// Port of `UserSwitch.__init__` (dpidLen 12; appends the `--listen=`
    /// opt). Python's `pathCheck('ofdatapath', 'ofprotocol')` moved to
    /// [`UserSwitch::start`] — see module docs.
    pub fn new(
        name: &str,
        dpopts: Option<&str>,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let dpopts = dpopts.unwrap_or("--no-slicing");
        let mut switch = Switch::new_with_dpid_len(
            name,
            dpid,
            opts,
            listen_port,
            in_namespace,
            USER_DPID_LEN,
            params,
        )?;
        switch
            .opts
            .push_str(&user_switch_listen_opt(listen_port, name));
        switch.node_mut().set_kind("UserSwitch");
        Ok(Self {
            switch,
            dpopts: dpopts.to_string(),
        })
    }

    /// Port of `UserSwitch.dpctl`: `dpctl <args...> <listenAddr>`.
    pub fn dpctl(&mut self, args: &[String]) -> Result<String> {
        let listen = dpctl_listen_addr(self.switch.listen_port, &self.switch.name);
        self.switch
            .node_mut()
            .cmd(&format!("dpctl {} {listen}", args.join(" ")))
    }

    /// Port of `UserSwitch.connected`: dpctl status contains both
    /// `remote.is-connected=true` and `local.is-connected=true`.
    pub fn connected(&mut self) -> bool {
        match self.dpctl(&["status".to_string()]) {
            Ok(status) => {
                status.contains("remote.is-connected=true")
                    && status.contains("local.is-connected=true")
            }
            Err(_) => false,
        }
    }

    /// Port of `UserSwitch.start(controllers)` — launch `ofdatapath` +
    /// `ofprotocol` in the node shell (root-gated).
    pub fn start(&mut self, controllers: &[&Controller]) -> Result<()> {
        require_root("UserSwitch::start (ofdatapath/ofprotocol)")?;
        for prog in ["ofdatapath", "ofprotocol"] {
            // Python: pathCheck('ofdatapath', 'ofprotocol', ...)
            if which(prog).is_none() {
                return Err(Error::InvalidInput(format!(
                    "Cannot find required executable {prog:?} (the OpenFlow reference user \
                     switch); please make sure it is installed and available in your $PATH"
                )));
            }
        }
        self.switch.ensure_shell()?;
        let name = self.switch.name.clone();
        let clist = controllers
            .iter()
            .map(|c| format!("tcp:{}:{}", c.ip(), c.port))
            .collect::<Vec<_>>()
            .join(",");
        let intfs: Vec<String> = self
            .switch
            .intf_list()
            .iter()
            .filter(|i| i.ip.is_none())
            .map(|i| i.name.clone())
            .collect();
        let ofdlog = format!("/tmp/{name}-ofd.log");
        let ofplog = format!("/tmp/{name}-ofp.log");
        let dpid = self.switch.dpid.clone();
        let opts = self.switch.opts.clone();
        self.switch
            .node_mut()
            .cmd(&ofdatapath_cmd(&name, &dpid, &self.dpopts, &intfs, &ofdlog))?;
        self.switch
            .node_mut()
            .cmd(&ofprotocol_cmd(&name, &clist, &opts, &ofplog))?;
        if !self.dpopts.contains("no-slicing") {
            // Only TCReapply if slicing is enabled; Python sleeps 1s first.
            std::thread::sleep(Duration::from_secs(1));
            // TODO(8b): TCReapply per interface — mn-link owns qdiscs.
        }
        Ok(())
    }

    /// Port of `UserSwitch.stop`: kill ofdatapath/ofprotocol, then the base
    /// stop.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        if self.switch.node().started() {
            let _ = self.switch.node_mut().cmd("kill %ofdatapath");
            let _ = self.switch.node_mut().cmd("kill %ofprotocol");
        }
        self.switch.stop_with(delete_intfs);
        Ok(())
    }

    /// The inner base switch.
    pub fn switch(&self) -> &Switch {
        &self.switch
    }

    /// Mutable access to the inner base switch.
    pub fn switch_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl Deref for UserSwitch {
    type Target = Switch;
    fn deref(&self) -> &Switch {
        &self.switch
    }
}

impl DerefMut for UserSwitch {
    fn deref_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl fmt::Display for UserSwitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.switch.fmt(f)
    }
}

// ---------------------------------------------------------------------
// IVSSwitch
// ---------------------------------------------------------------------

/// `IVSSwitch` — port of `class IVSSwitch( Switch )` (Indigo Virtual
/// Switch). The `ivs` subprocess launch is root-gated.
#[derive(Debug)]
pub struct IVSSwitch {
    switch: Switch,
    /// `--verbose` flag (Python `verbose=False`).
    pub verbose: bool,
}

impl IVSSwitch {
    /// Port of `IVSSwitch.__init__`. Python's `setup()` pathCheck of
    /// `ivs-ctl`/`ivs` moved to [`IVSSwitch::start`].
    pub fn new(
        name: &str,
        verbose: bool,
        dpid: Option<&str>,
        opts: &str,
        listen_port: Option<u16>,
        in_namespace: bool,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut switch = Switch::new(name, dpid, opts, listen_port, in_namespace, params)?;
        switch.node_mut().set_kind("IVSSwitch");
        Ok(Self { switch, verbose })
    }

    /// Port of `IVSSwitch.start(controllers)` — launch `ivs` in the node
    /// shell, logging to `/tmp/ivs.<name>.log` (root-gated).
    pub fn start(&mut self, controllers: &[&Controller]) -> Result<()> {
        require_root("IVSSwitch::start (ivs)")?;
        for prog in ["ivs-ctl", "ivs"] {
            // Python: pathCheck('ivs-ctl', 'ivs', ...)
            if which(prog).is_none() {
                return Err(Error::InvalidInput(format!(
                    "Cannot find required executable {prog:?} (Indigo Virtual Switch); \
                     please make sure it is installed and available in your $PATH"
                )));
            }
        }
        self.switch.ensure_shell()?;
        let name = self.switch.name.clone();
        let intfs: Vec<String> = self
            .switch
            .intf_list()
            .iter()
            .filter(|i| i.ip.is_none())
            .map(|i| i.name.clone())
            .collect();
        let ctrls: Vec<(IpAddr, u16)> = controllers.iter().map(|c| (c.ip(), c.port)).collect();
        let argv = ivs_start_argv(
            &name,
            &self.switch.dpid,
            self.verbose,
            &intfs,
            &ctrls,
            self.switch.listen_port,
            &self.switch.opts,
        );
        let log = ivs_logfile(&name);
        // Python: self.cmd(' '.join(args) + ' >' + logfile + ' 2>&1 </dev/null &')
        self.switch
            .node_mut()
            .cmd(&format!("{} >{log} 2>&1 </dev/null &", argv.join(" ")))?;
        Ok(())
    }

    /// Port of `IVSSwitch.stop`: `kill %ivs` + `wait`, then base stop.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        if self.switch.node().started() {
            let _ = self.switch.node_mut().cmd("kill %ivs");
            let _ = self.switch.node_mut().cmd("wait");
        }
        self.switch.stop_with(delete_intfs);
        Ok(())
    }

    /// Port of `IVSSwitch.attach`: `ivs-ctl add-port --datapath <name> <intf>`.
    pub fn attach(&mut self, intf: &str) -> Result<()> {
        let name = self.switch.name.clone();
        self.switch.node_mut().pexec(&[
            "ivs-ctl".to_string(),
            "add-port".to_string(),
            "--datapath".to_string(),
            name,
            intf.to_string(),
        ])?;
        Ok(())
    }

    /// Port of `IVSSwitch.detach`: `ivs-ctl del-port --datapath <name> <intf>`.
    pub fn detach(&mut self, intf: &str) -> Result<()> {
        let name = self.switch.name.clone();
        self.switch.node_mut().pexec(&[
            "ivs-ctl".to_string(),
            "del-port".to_string(),
            "--datapath".to_string(),
            name,
            intf.to_string(),
        ])?;
        Ok(())
    }

    /// Port of `IVSSwitch.dpctl`: needs the passive listening port;
    /// `ovs-ofctl <args...> tcp:127.0.0.1:<port>`.
    pub fn dpctl(&mut self, args: &[String]) -> Result<String> {
        match self.switch.listen_port {
            None => Ok("can't run dpctl without passive listening port".to_string()),
            Some(p) => self
                .switch
                .node_mut()
                .cmd(&format!("ovs-ofctl {} tcp:127.0.0.1:{p}", args.join(" "))),
        }
    }

    /// Port of `IVSSwitch.batchShutdown`: `kill %ivs` on each switch.
    pub fn batch_shutdown(switches: &mut [&mut IVSSwitch]) -> Result<()> {
        for sw in switches.iter_mut() {
            if sw.switch.node().started() {
                let _ = sw.switch.node_mut().cmd("kill %ivs");
            }
        }
        Ok(())
    }

    /// The inner base switch.
    pub fn switch(&self) -> &Switch {
        &self.switch
    }

    /// Mutable access to the inner base switch.
    pub fn switch_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl Deref for IVSSwitch {
    type Target = Switch;
    fn deref(&self) -> &Switch {
        &self.switch
    }
}

impl DerefMut for IVSSwitch {
    fn deref_mut(&mut self) -> &mut Switch {
        &mut self.switch
    }
}

impl fmt::Display for IVSSwitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.switch.fmt(f)
    }
}
// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // ---- dpid ----

    #[test]
    fn default_dpid_from_name_like_python() {
        // Python: nums = re.findall(r'\d+', name); dpid = hex(int(nums[0]))[2:]
        assert_eq!(default_dpid("s1", None).unwrap(), "0000000000000001");
        assert_eq!(default_dpid("s23", None).unwrap(), "0000000000000017");
        assert_eq!(default_dpid("s10", None).unwrap(), "000000000000000a");
        // The FIRST digit group wins, not the concatenation of all digits:
        // "s1a2" -> 1 (Python re.findall(r'\d+')[0] == '1'), not 12.
        assert_eq!(default_dpid("s1a2", None).unwrap(), "0000000000000001");
        assert_eq!(default_dpid("s10x20", None).unwrap(), "000000000000000a");
        assert_eq!(default_dpid("a2b3c", None).unwrap(), "0000000000000002");
        assert_eq!(default_dpid("s007", None).unwrap(), "0000000000000007");
        assert!(default_dpid("switch", None).is_err());
        // A '-' is not a digit, so the first digit run of "s-1" is "1".
        assert_eq!(default_dpid("s-1", None).unwrap(), "0000000000000001");
    }

    #[test]
    fn default_dpid_validates_explicit_dpid() {
        assert_eq!(
            default_dpid("s1", Some("00:00:00:00:00:00:00:2a")).unwrap(),
            "000000000000002a"
        );
        assert_eq!(default_dpid("s1", Some("2a")).unwrap(), "000000000000002a");
        assert!(default_dpid("s1", Some("not-hex")).is_err());
        assert!(default_dpid("s1", Some(&"f".repeat(17))).is_err());
        assert!(default_dpid("s1", Some("")).is_err());
        // UserSwitch.dpidLen = 12:
        assert_eq!(
            default_dpid_len("s1", None, USER_DPID_LEN).unwrap(),
            "000000000001"
        );
        assert!(default_dpid_len("s1", Some(&"f".repeat(13)), USER_DPID_LEN).is_err());
    }

    // ---- base Switch model ----

    #[test]
    fn switch_model_port_base_and_control_intf() {
        let s = Switch::new("s1", None, "", None, false, BTreeMap::new()).unwrap();
        assert_eq!(s.port_base, 1);
        assert_eq!(s.dpid, "0000000000000001");
        assert!(s.control_intf.is_some());
        assert_eq!(s.control_intf.as_ref().unwrap().name, "lo");
        assert_eq!(s.new_port(), 1);
        // Switch.defaultIntf: control intf (lo) for root-ns switches even
        // when the node has real interfaces on other ports.
        let mut s_root = Switch::new("s1", None, "", None, false, BTreeMap::new()).unwrap();
        s_root
            .node_mut()
            .add_intf(Intf::new("s1-eth1", "s1", Some(1)), Some(1))
            .unwrap();
        assert_eq!(s_root.default_intf().unwrap().name, "lo");
        assert_eq!(s_root.node().default_intf().unwrap().name, "s1-eth1");
        let mut s2 = Switch::new("s2", None, "", None, true, BTreeMap::new()).unwrap();
        assert!(s2.control_intf.is_none());
        // ... and for namespaced switches it falls back to the node default.
        s2.node_mut()
            .add_intf(Intf::new("s2-eth1", "s2", Some(1)), Some(1))
            .unwrap();
        assert_eq!(s2.default_intf().unwrap().name, "s2-eth1");
        // Base Switch has no start (Python defines none on Switch).
        let err = s2.start(&[]).unwrap_err().to_string();
        assert!(err.contains("base Switch has no start"), "got {err}");
        assert!(s2.connected());
        s2.stop_with(true);
        s2.stop(); // Python default: deleteIntfs=True
        assert_eq!(s2.to_string(), "<Switch s2: s2-eth1:None pid=None> ");
    }

    // ---- pure ovs-vsctl argv builders ----

    #[test]
    fn vsctl_builder_argvs_match_python_commands() {
        assert_eq!(vsctl_add_br("s1"), s(&["ovs-vsctl", "add-br", "s1"]));
        assert_eq!(vsctl_del_br("s1", false), s(&["ovs-vsctl", "del-br", "s1"]));
        assert_eq!(
            vsctl_del_br("s1", true),
            s(&["ovs-vsctl", "--if-exists", "del-br", "s1"])
        );
        assert_eq!(
            vsctl_set_controller(
                "s1",
                &["tcp:127.0.0.1:6653".to_string(), "ptcp:6640".to_string()]
            ),
            s(&[
                "ovs-vsctl",
                "set-controller",
                "s1",
                "tcp:127.0.0.1:6653",
                "ptcp:6640"
            ])
        );
        assert_eq!(
            vsctl_del_controller("s1"),
            s(&["ovs-vsctl", "del-controller", "s1"])
        );
        assert_eq!(
            vsctl_set_fail_mode("s1", "secure"),
            s(&["ovs-vsctl", "set-fail-mode", "s1", "secure"])
        );
        assert_eq!(
            vsctl_set_datapath_id("s1", "0000000000000001"),
            s(&[
                "ovs-vsctl",
                "set",
                "bridge",
                "s1",
                "other_config:datapath-id=0000000000000001"
            ])
        );
        assert_eq!(
            vsctl_set_bridge("s1", &["fail_mode=secure".to_string()]),
            s(&["ovs-vsctl", "set", "bridge", "s1", "fail_mode=secure"])
        );
        assert_eq!(
            vsctl_add_port("s1", "s1-eth1", &[]),
            s(&["ovs-vsctl", "add-port", "s1", "s1-eth1"])
        );
        assert_eq!(
            vsctl_add_port(
                "s1",
                "s1-eth1",
                &s(&["--", "set", "Interface", "s1-eth1", "ofport_request=1"])
            ),
            s(&[
                "ovs-vsctl",
                "add-port",
                "s1",
                "s1-eth1",
                "--",
                "set",
                "Interface",
                "s1-eth1",
                "ofport_request=1"
            ])
        );
        assert_eq!(
            vsctl_del_port("s1", "s1-eth1", false),
            s(&["ovs-vsctl", "del-port", "s1", "s1-eth1"])
        );
        assert_eq!(
            vsctl_del_port("s1", "s1-eth1", true),
            s(&["ovs-vsctl", "--if-exists", "del-port", "s1", "s1-eth1"])
        );
        assert_eq!(
            vsctl_get_bridge_controller("s1"),
            s(&["ovs-vsctl", "--", "get", "Bridge", "s1", "Controller"])
        );
        assert_eq!(
            vsctl_get_controller_is_connected("abc-uuid"),
            s(&[
                "ovs-vsctl",
                "--",
                "get",
                "Controller",
                "abc-uuid",
                "is_connected"
            ])
        );
        // batchShutdown: 'ovs-vsctl ' + ' -- '.join('--if-exists del-br %s')
        assert_eq!(
            vsctl_batch_del_br(&["s1", "s2"], true),
            s(&[
                "ovs-vsctl",
                "--if-exists",
                "del-br",
                "s1",
                "--",
                "--if-exists",
                "del-br",
                "s2"
            ])
        );
        assert_eq!(
            vsctl_batch_del_br(&["s1", "s2"], false),
            s(&["ovs-vsctl", "del-br", "s1", "--", "del-br", "s2"])
        );
        // dpctl: cmd('ovs-ofctl', args[0], self, *args[1:])
        assert_eq!(
            ofctl_dpctl_argv("s1", &s(&["show"])).unwrap(),
            s(&["ovs-ofctl", "show", "s1"])
        );
        assert_eq!(
            ofctl_dpctl_argv("s1", &s(&["dump-flows", "s1"])).unwrap(),
            s(&["ovs-ofctl", "dump-flows", "s1", "s1"])
        );
        assert!(ofctl_dpctl_argv("s1", &[]).is_err());
    }

    #[test]
    fn controller_and_listen_targets_match_python() {
        // Python: '%s:%s:%d' % (c.protocol, c.IP(), c.port)
        assert_eq!(
            controller_target("tcp", IpAddr::from([127, 0, 0, 1]), 6653),
            "tcp:127.0.0.1:6653"
        );
        // Python: 'ptcp:%s' % self.listenPort
        assert_eq!(ptcp_target(6640), "ptcp:6640");
    }

    #[test]
    fn bridge_opts_match_python_bridge_opts() {
        // Default: secure/kernel, not inband, no protocols, no stp.
        assert_eq!(
            bridge_opts(
                "0000000000000001",
                "secure",
                false,
                "kernel",
                None,
                false,
                "s1",
                false
            ),
            s(&[
                "other_config:datapath-id=0000000000000001",
                "fail_mode=secure",
                "other-config:disable-in-band=true",
                "other-config:dp-desc=s1"
            ])
        );
        // inband -> no disable-in-band; user datapath -> netdev; protocols
        // only on non-old OVS; stp only with standalone.
        assert_eq!(
            bridge_opts(
                "1",
                "standalone",
                true,
                "user",
                Some("OpenFlow13"),
                true,
                "s2",
                false
            ),
            s(&[
                "other_config:datapath-id=1",
                "fail_mode=standalone",
                "datapath_type=netdev",
                "protocols=OpenFlow13",
                "stp_enable=true",
                "other-config:dp-desc=s2"
            ])
        );
        // stp with secure fail mode is dropped (Python condition); old OVS
        // drops protocols.
        assert_eq!(
            bridge_opts(
                "1",
                "secure",
                false,
                "kernel",
                Some("OpenFlow13"),
                true,
                "s2",
                false
            ),
            s(&[
                "other_config:datapath-id=1",
                "fail_mode=secure",
                "other-config:disable-in-band=true",
                "protocols=OpenFlow13",
                "other-config:dp-desc=s2"
            ])
        );
        assert_eq!(
            bridge_opts(
                "1",
                "secure",
                false,
                "kernel",
                Some("OpenFlow13"),
                false,
                "s2",
                true
            ),
            s(&[
                "other_config:datapath-id=1",
                "fail_mode=secure",
                "other-config:disable-in-band=true",
                "other-config:dp-desc=s2"
            ])
        );
    }

    #[test]
    fn intf_opts_args_match_python_intf_opts() {
        assert_eq!(
            intf_opts_args("s1-eth1", Some(1), None, false),
            s(&["--", "set", "Interface", "s1-eth1", "ofport_request=1"])
        );
        // Patch peer (OVSIntf case):
        assert_eq!(
            intf_opts_args("s1-eth1", Some(1), Some("s2-eth1"), false),
            s(&[
                "--",
                "set",
                "Interface",
                "s1-eth1",
                "ofport_request=1",
                "type=patch",
                "options:peer=s2-eth1"
            ])
        );
        // Old OVS: no ofport_request, no patch.
        assert!(intf_opts_args("s1-eth1", Some(1), Some("s2-eth1"), true).is_empty());
        // No port: empty.
        assert!(intf_opts_args("s1-eth1", None, None, false).is_empty());
    }

    #[test]
    fn parse_controller_uuids_matches_python() {
        assert_eq!(
            parse_controller_uuids("[uuid-1, uuid-2]\n"),
            s(&["uuid-1", "uuid-2"])
        );
        assert!(parse_controller_uuids("[]").is_empty());
        assert!(parse_controller_uuids("").is_empty());
        // Non-bracketed output leaves the cache empty (Python's condition).
        assert!(parse_controller_uuids("uuid-1").is_empty());
    }

    // ---- version helpers ----

    #[test]
    fn ovs_version_extraction_and_old_ovs_check() {
        // Python: findall(r'\d+\.\d+', version)[0]
        assert_eq!(
            ovs_version_from("ovs-vsctl (Open vSwitch) 2.17.8\nDB Schema 8.3.0\n"),
            Some("2.17".to_string())
        );
        assert_eq!(ovs_version_from("ovs-vsctl 1.9"), Some("1.9".to_string()));
        assert_eq!(ovs_version_from("no version here"), None);
        // Python: StrictVersion(v) < StrictVersion('1.10')
        assert!(is_old_ovs_version("0.9"));
        assert!(is_old_ovs_version("1.9"));
        assert!(!is_old_ovs_version("1.10"));
        assert!(!is_old_ovs_version("2.17"));
        assert!(!is_old_ovs_version("10.0"));
    }

    // ---- UserSwitch / IVSSwitch pure builders ----

    #[test]
    fn user_switch_command_builders_match_python() {
        assert_eq!(
            user_switch_listen_opt(Some(6640), "s1"),
            " --listen=ptcp:6640 "
        );
        assert_eq!(
            user_switch_listen_opt(None, "s1"),
            " --listen=punix:/tmp/s1.listen "
        );
        assert_eq!(dpctl_listen_addr(Some(6640), "s1"), "tcp:127.0.0.1:6640");
        assert_eq!(dpctl_listen_addr(None, "s1"), "unix:/tmp/s1.listen");
        assert_eq!(
            ofdatapath_cmd(
                "s1",
                "000000000001",
                "--no-slicing",
                &s(&["s1-eth1", "s1-eth2"]),
                "/tmp/s1-ofd.log"
            ),
            "ofdatapath -i s1-eth1,s1-eth2 punix:/tmp/s1 -d 000000000001 --no-slicing \
             1> /tmp/s1-ofd.log 2> /tmp/s1-ofd.log &"
        );
        assert_eq!(
            ofprotocol_cmd(
                "s1",
                "tcp:127.0.0.1:6653",
                " --listen=ptcp:6640 ",
                "/tmp/s1-ofp.log"
            ),
            "ofprotocol unix:/tmp/s1 tcp:127.0.0.1:6653 --fail=closed  --listen=ptcp:6640  \
             1> /tmp/s1-ofp.log 2> /tmp/s1-ofp.log &"
        );
    }

    #[test]
    fn ivs_argv_matches_python_start() {
        assert_eq!(
            ivs_start_argv(
                "s1",
                "0000000000000001",
                true,
                &s(&["s1-eth1"]),
                &[(IpAddr::from([127, 0, 0, 1]), 6653)],
                Some(6640),
                "--foo"
            ),
            s(&[
                "ivs",
                "--name",
                "s1",
                "--dpid",
                "0000000000000001",
                "--verbose",
                "-i",
                "s1-eth1",
                "-c",
                "127.0.0.1:6653",
                "--listen",
                "127.0.0.1:6640",
                "--foo"
            ])
        );
        // Empty opts word is dropped in argv form (Python's shell drops it).
        assert_eq!(
            ivs_start_argv("s1", "1", false, &[], &[], None, ""),
            s(&["ivs", "--name", "s1", "--dpid", "1"])
        );
        assert_eq!(ivs_logfile("s1"), "/tmp/ivs.s1.log");
    }

    // ---- OVSSwitch model + batch-queued start (pure, no subprocess) ----

    fn test_controller(name: &str, port: u16) -> Controller {
        Controller::new(name, "controller", port, false, BTreeMap::new())
    }

    #[test]
    fn ovs_switch_model_and_validation() {
        let s = OVSSwitch::with_defaults("s1", None, "", None, false, BTreeMap::new()).unwrap();
        assert_eq!(s.dpid, "0000000000000001");
        assert_eq!(s.fail_mode, "secure");
        assert_eq!(s.datapath, "kernel");
        assert!(!s.inband);
        assert_eq!(s.reconnect_ms, Some(1000));
        assert!(!s.stp);
        assert!(!s.batch);
        assert!(!s.is_old_ovs());
        assert!(s.to_string().starts_with("<OVSSwitch s1: "));
        // start in a namespace is Python's exception.
        let mut ns = OVSSwitch::with_defaults("s2", None, "", None, true, BTreeMap::new()).unwrap();
        let err = ns.start(&[]).unwrap_err().to_string();
        assert!(err.contains("does not work in a namespace"), "got {err}");
        // bad failMode/datapath are rejected early (Python fails in ovs-vsctl).
        let bad = OVSSwitch::new(
            "s1",
            None,
            "",
            None,
            "bogus",
            "kernel",
            false,
            None,
            None,
            false,
            false,
            false,
            BTreeMap::new(),
        );
        assert!(bad.is_err());
        let bad = OVSSwitch::new(
            "s1",
            None,
            "",
            None,
            "secure",
            "bogus",
            false,
            None,
            None,
            false,
            false,
            false,
            BTreeMap::new(),
        );
        assert!(bad.is_err());
    }

    #[test]
    fn ovs_switch_batch_start_queues_exact_vsctl_argvs() {
        let c = test_controller("c0", 6653);
        let mut s = OVSSwitch::new(
            "s1",
            None,
            "",
            Some(6640),
            "secure",
            "kernel",
            false,
            Some("OpenFlow13"),
            Some(1000),
            false,
            true, // batch: queue instead of running
            false,
            BTreeMap::new(),
        )
        .unwrap();
        s.node_mut()
            .add_intf(Intf::new("s1-eth1", "s1", Some(1)), Some(1))
            .unwrap();
        // An intf WITH an IP is not added as a data port (Python filter).
        let mut with_ip = Intf::new("s1-eth2", "s1", Some(2));
        with_ip.set_ip(IpAddr::from([10, 0, 0, 1]), 8);
        s.node_mut().add_intf(with_ip, Some(2)).unwrap();
        s.start(&[&c]).unwrap();
        let cmds = &s.commands;
        assert_eq!(cmds.len(), 5, "queued: {cmds:?}");
        assert_eq!(cmds[0], vsctl_del_br("s1", true)); // non-old OVS: --if-exists
        assert_eq!(cmds[1], vsctl_add_br("s1"));
        assert_eq!(
            cmds[2],
            vsctl_set_bridge(
                "s1",
                &bridge_opts(
                    "0000000000000001",
                    "secure",
                    false,
                    "kernel",
                    Some("OpenFlow13"),
                    false,
                    "s1",
                    false
                )
            )
        );
        // datapath-id comes from default_dpid:
        assert!(cmds[2].contains(&"other_config:datapath-id=0000000000000001".to_string()));
        assert_eq!(
            cmds[3],
            vsctl_set_controller(
                "s1",
                &["tcp:127.0.0.1:6653".to_string(), "ptcp:6640".to_string()]
            )
        );
        assert_eq!(
            cmds[4],
            vsctl_add_port(
                "s1",
                "s1-eth1",
                &intf_opts_args("s1-eth1", Some(1), None, false)
            )
        );
        // batch flag still set until batch_startup drains it.
        assert!(s.batch);
    }

    #[test]
    fn ovs_bridge_forces_standalone_and_ignores_controllers() {
        let c = test_controller("c0", 6653);
        let mut b = OVSBridge::new(
            "s1",
            None,
            "",
            None,
            "kernel",
            false,
            None,
            None,
            false,
            true, // batch
            false,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(b.fail_mode, "standalone");
        assert!(b.to_string().starts_with("<OVSBridge s1: "));
        b.start(&[&c]).unwrap(); // controllers ignored
                                 // queued: del-br, add-br, set bridge (fail_mode=standalone), NO
                                 // set-controller (no controllers, no listenPort), no ports.
        assert_eq!(b.ovs.commands.len(), 3);
        assert!(b.ovs.commands[2].contains(&"fail_mode=standalone".to_string()));
        // stp=false -> connected() == true
        assert!(b.connected());
    }

    // ---- vsctl execution through a fake ovs-vsctl (non-root) ----

    /// Write a fake `ovs-vsctl` script that appends its args to `log`.
    fn fake_vsctl(dir: &std::path::Path, log: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("ovs-vsctl");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_string_lossy().into_owned()
    }

    #[test]
    fn vsctl_runs_subprocess_and_batch_startup_drains_queue() {
        let dir = std::env::temp_dir().join(format!("mn-vsctl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("vsctl.log");
        let bin = fake_vsctl(&dir, &log);

        let mut s = OVSSwitch::with_defaults("s1", None, "", None, false, BTreeMap::new()).unwrap();
        s.set_vsctl_bin(&bin);
        // Non-batch vsctl executes immediately:
        let out = s.vsctl(&vsctl_add_br("s1")).unwrap();
        assert_eq!(out, "");
        // attach/detach go through add-port/del-port (ifconfig failure on an
        // unstarted node is ignored, like Python's warn-and-continue):
        s.attach("s1-eth1").unwrap();
        s.detach("s1-eth1").unwrap();
        // stop issues del-br:
        s.stop(true).unwrap();
        let logged = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(
            lines,
            vec![
                "add-br s1",
                "add-port s1 s1-eth1",
                "del-port s1 s1-eth1",
                "del-br s1"
            ]
        );

        // Batch: queue via start(), then batch_startup runs them in order.
        let mut s2 =
            OVSSwitch::with_defaults("s2", None, "", None, false, BTreeMap::new()).unwrap();
        s2.set_vsctl_bin(&bin);
        s2.batch = true;
        s2.node_mut()
            .add_intf(Intf::new("s2-eth1", "s2", Some(1)), Some(1))
            .unwrap();
        s2.start(&[]).unwrap();
        assert!(!s2.commands.is_empty());
        let queued = s2.commands.len();
        OVSSwitch::batch_startup(&mut [&mut s2]).unwrap();
        assert!(s2.commands.is_empty());
        assert!(!s2.batch);
        let logged = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = logged.lines().collect();
        assert_eq!(
            &lines[4..],
            &[
                "--if-exists del-br s2",
                "add-br s2",
                "set bridge s2 other_config:datapath-id=0000000000000002 fail_mode=secure \
                 other-config:disable-in-band=true other-config:dp-desc=s2",
                "add-port s2 s2-eth1 -- set Interface s2-eth1 ofport_request=1",
            ][..]
        );
        assert_eq!(queued, 4);

        // batch_shutdown issues one multi-del-br command and terminates.
        let mut s3 =
            OVSSwitch::with_defaults("s3", None, "", None, false, BTreeMap::new()).unwrap();
        s3.set_vsctl_bin(&bin);
        OVSSwitch::batch_shutdown(&mut [&mut s2, &mut s3]).unwrap();
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(logged
            .lines()
            .last()
            .unwrap()
            .contains("del-br s2 -- --if-exists del-br s3"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- UserSwitch / IVSSwitch model + root gate ----

    #[test]
    fn user_switch_model_and_root_gate() {
        let mut us =
            UserSwitch::new("s1", None, None, "", Some(6640), false, BTreeMap::new()).unwrap();
        assert_eq!(us.dpid, "000000000001"); // dpidLen 12
        assert_eq!(us.dpopts, "--no-slicing");
        assert_eq!(us.opts, " --listen=ptcp:6640 ");
        assert!(us.to_string().starts_with("<UserSwitch s1: "));
        // Root gate: in this non-root sandbox start() must fail with the
        // require-root error (before any pathCheck).
        let err = us.start(&[]).unwrap_err().to_string();
        assert!(err.contains("requires root"), "got {err}");
        us.stop(true).unwrap(); // safe on unstarted node
    }

    #[test]
    fn ivs_switch_model_and_root_gate() {
        let mut ivs =
            IVSSwitch::new("s1", true, None, "", Some(6640), false, BTreeMap::new()).unwrap();
        assert!(ivs.verbose);
        assert_eq!(ivs.dpid, "0000000000000001");
        assert!(ivs.to_string().starts_with("<IVSSwitch s1: "));
        let err = ivs.start(&[]).unwrap_err().to_string();
        assert!(err.contains("requires root"), "got {err}");
        // dpctl without a listen port returns Python's message string.
        let mut ivs2 = IVSSwitch::new("s2", false, None, "", None, false, BTreeMap::new()).unwrap();
        assert_eq!(
            ivs2.dpctl(&["show".to_string()]).unwrap(),
            "can't run dpctl without passive listening port"
        );
        ivs.stop(true).unwrap();
    }

    // ---- root-gated integration tests ----
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1

    /// Real-OVS OVSSwitch start/stop. Needs root + a running ovsdb-server;
    /// returns early when either is missing (documented in crate docs).
    #[test]
    #[ignore = "requires root + running ovsdb-server on a privileged host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn ovs_switch_root_start_stop_with_real_ovs() {
        if !root_tests_enabled() {
            return;
        }
        if OVSSwitch::setup().is_err() {
            // No ovsdb-server reachable: nothing to test against here.
            return;
        }
        let name = format!("mnsw{}", std::process::id() % 100000);
        let mut sw =
            OVSSwitch::with_defaults(&name, None, "", None, false, BTreeMap::new()).expect("model");
        sw.node_mut().start().expect("root-ns shell as root");
        let c = test_controller("c0", 6653);
        sw.start(&[&c]).expect("vsctl start");
        // Bridge exists now:
        let out = std::process::Command::new(OVS_VSCTL)
            .args(["list-br"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(&name));
        // Controller row was set:
        let out = std::process::Command::new(OVS_VSCTL)
            .args(["get", "bridge", &name, "controller"])
            .output()
            .unwrap();
        assert!(!out.stdout.is_empty());
        sw.stop(true).expect("vsctl stop");
        let out = std::process::Command::new(OVS_VSCTL)
            .args(["list-br"])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&out.stdout).contains(&name));
        sw.node_mut().stop(false).unwrap();
    }

    /// UserSwitch launch as root (ofdatapath/ofprotocol must be installed).
    #[test]
    #[ignore = "requires root + ofdatapath/ofprotocol; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn user_switch_root_launch() {
        if !root_tests_enabled() {
            return;
        }
        if which("ofdatapath").is_none() || which("ofprotocol").is_none() {
            return; // reference switch not installed on this host
        }
        let name = format!("mnus{}", std::process::id() % 100000);
        let mut us =
            UserSwitch::new(&name, None, None, "", None, false, BTreeMap::new()).expect("model");
        us.start(&[]).expect("ofdatapath/ofprotocol launch as root");
        let out = us
            .node_mut()
            .cmd("pgrep -f ofdatapath | head -1")
            .expect("pgrep");
        assert!(!out.trim().is_empty(), "ofdatapath not running: {out:?}");
        us.stop(true).expect("stop");
        let out = us
            .node_mut()
            .cmd("pgrep -f ofdatapath || true")
            .unwrap_or_default();
        assert!(out.trim().is_empty(), "ofdatapath still running: {out:?}");
        us.node_mut().stop(false).unwrap();
    }

    /// IVSSwitch launch as root (ivs/ivs-ctl must be installed).
    #[test]
    #[ignore = "requires root + ivs/ivs-ctl; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn ivs_switch_root_launch() {
        if !root_tests_enabled() {
            return;
        }
        if which("ivs").is_none() || which("ivs-ctl").is_none() {
            return; // IVS not installed on this host
        }
        let name = format!("mniv{}", std::process::id() % 100000);
        let mut ivs =
            IVSSwitch::new(&name, false, None, "", None, false, BTreeMap::new()).expect("model");
        ivs.start(&[]).expect("ivs launch as root");
        let out = ivs.node_mut().cmd("pgrep -x ivs | head -1").expect("pgrep");
        assert!(!out.trim().is_empty(), "ivs not running: {out:?}");
        ivs.stop(true).expect("stop");
        ivs.node_mut().stop(false).unwrap();
    }
}
