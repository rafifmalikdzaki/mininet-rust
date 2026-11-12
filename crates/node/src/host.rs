//! `Host` — port of `class Host( Node ): "A host is simply a Node"`, plus
//! the automatic IP/MAC assignment hooks that `Mininet.addHost` applies in
//! `mininet/net.py`, and `CPULimitedHost` (cgroup v2 `cpu.max` quota via
//! file writes).

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::Duration;

use mn_topo::Attrs;

use crate::error::{Error, Result};
use crate::node::Node;

/// A virtual host: simply a [`Node`] (Python `class Host(Node): pass`).
///
/// Adds the `net.py addHost` defaulting hooks on top: `ip`/`mac` params
/// derived from a host index via `mn_util::ip_add_with` /
/// `mn_util::mac_colon_hex`.
#[derive(Debug)]
pub struct Host {
    node: Node,
}

impl Host {
    /// New host (Python `Host(name, inNamespace=True, **params)`).
    pub fn new(name: &str, in_namespace: bool, params: BTreeMap<String, String>) -> Self {
        let mut node = Node::new(name, in_namespace, params);
        node.set_kind("Host");
        Self { node }
    }

    /// New host with `inNamespace=True` (Python's default).
    pub fn with_params(name: &str, params: BTreeMap<String, String>) -> Self {
        Self::new(name, true, params)
    }

    /// New host from topology node attrs (`buildFromTopo` passes
    /// `topo.nodeInfo(name)` as `**params`).
    pub fn from_topo_params(name: &str, attrs: &Attrs) -> Self {
        let mut host = Self::new(name, true, BTreeMap::new());
        host.node = Node::with_topo_params(name, true, attrs);
        host.node.set_kind("Host");
        host
    }

    /// Port of `Mininet.addHost`'s defaulting (net.py):
    /// `ip = ipAdd(nextIP, ipBaseNum, prefixLen) + '/prefixLen'` and, when
    /// `autoSetMacs`, `mac = macColonHex(nextIP)`.
    ///
    /// Stores the resulting `ip`/`mac` entries in `params` **without
    /// clobbering caller-supplied values** — Python does
    /// `defaults.update(params)`, so `params` wins; here that is
    /// `entry().or_insert_with()` semantics. Returns the computed
    /// `"ip/prefix"` string, if any.
    pub fn add_host_defaults(
        &mut self,
        next_ip: i64,
        ip_base_num: i64,
        prefix_len: i64,
        auto_set_macs: bool,
    ) -> Option<String> {
        let ip = mn_util::ip_add_with(next_ip, prefix_len, ip_base_num);
        if let Some(ip) = &ip {
            self.node
                .params
                .entry("ip".to_string())
                .or_insert_with(|| format!("{ip}/{prefix_len}"));
        }
        if auto_set_macs {
            self.node
                .params
                .entry("mac".to_string())
                .or_insert_with(|| mn_util::mac_colon_hex(next_ip));
        }
        ip.map(|ip| format!("{ip}/{prefix_len}"))
    }

    /// Assign an IP from a host index (the `ipAdd` hook): sets
    /// `params["ip"] = "<ip>/<prefix>"` and returns it.
    pub fn assign_ip(&mut self, index: i64, prefix_len: i64, ip_base_num: i64) -> Option<String> {
        let ip = mn_util::ip_add_with(index, prefix_len, ip_base_num)?;
        let entry = format!("{ip}/{prefix_len}");
        self.node.params.insert("ip".to_string(), entry.clone());
        Some(entry)
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

impl Deref for Host {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Host {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

// ---------------------------------------------------------------------
// CPULimitedHost — cgroup v2 cpu.max quota via file writes (no netlink)
// ---------------------------------------------------------------------

/// The cgroup v2 unified root, if this host uses cgroup v2 with the `cpu`
/// controller available (`/sys/fs/cgroup/cgroup.controllers` contains
/// `cpu`). Read-only check, works unprivileged.
pub fn cgroup_v2_root() -> Option<PathBuf> {
    let controllers = std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers").ok()?;
    controllers
        .split_whitespace()
        .any(|c| c == "cpu")
        .then(|| PathBuf::from("/sys/fs/cgroup"))
}

/// The cgroup directory of node `name` under the cgroup v2 `root`
/// (Python: `cgcreate -g cpu,cpuacct,cpuset:/<name>` -> `/sys/fs/cgroup/<name>`).
pub fn cgroup_dir(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

/// Port of `CPULimitedHost.cfsInfo` for cgroup v2 (`cgversion ==
/// 'cgroup2'`, where `pstr='max'`, `qstr=''`): returns `(quota, period)`
/// with quota `"max"` (unlimited) or a microsecond number.
///
/// `quota = int(period_us * f * num_cores)` (wall clock period, CPU-time
/// quota); if `f > 0` and the quota would be under the 1 ms CFS minimum,
/// raise it to 1000 us and grow the period accordingly; a negative quota
/// resets to unlimited.
pub fn cfs_info(cpu_frac: f64, period_us: u64, num_cores: u64) -> (String, u64) {
    // Python int() truncates toward zero, like `as i64` on a float.
    let mut quota = (period_us as f64 * cpu_frac * num_cores as f64) as i64;
    let mut period = period_us;
    if cpu_frac > 0.0 && quota < 1000 {
        quota = 1000;
        // Python would ZeroDivisionError on numCores()==0; keep total.
        period = if num_cores > 0 {
            (quota as f64 / cpu_frac / num_cores as f64) as u64
        } else {
            period_us
        };
    }
    if quota < 0 {
        ("max".to_string(), period)
    } else {
        (quota.to_string(), period)
    }
}

/// Content of the cgroup v2 `cpu.max` file: `"<quota> <period>"`
/// (Python cgroup2: `cgset -r cpu.max=<quota> <period> /<name>`).
pub fn cpu_max_content(quota: &str, period: u64) -> String {
    format!("{quota} {period}")
}

/// Parse the `cpu` param: a float fraction of the whole system's CPU
/// (e.g. `"0.5"`, `"-1"` for unlimited) — Python's `makeNumeric` float.
pub fn parse_cpu_param(s: &str) -> Result<f64> {
    s.trim().parse::<f64>().map_err(|_| {
        Error::InvalidInput(format!(
            "bad cpu param {s:?}: expected a float fraction (e.g. 0.5 or -1)"
        ))
    })
}

/// `CPULimitedHost` — port of `class CPULimitedHost( Host )` in node.py:
/// a host whose CPU bandwidth is limited through a **cgroup v2** `cpu.max`
/// quota, applied by plain file writes under `/sys/fs/cgroup/<name>`
/// (no netlink involved).
///
/// Deviations from CPython (documented):
/// - cgroup **v2 only** (`cgversion='cgroup2'`); the v1 `cgset`/`cgget`
///   split (`cfs_period_us`/`cfs_quota_us`) is not ported.
/// - `sched="rt"` (`chrt` + `rt_runtime_us` + `CONFIG_RT_GROUP_SCHED`
///   check) is not ported — `set_cpu_frac(.., "rt")` returns an explicit
///   error. CFS via `cpu.max` is the ported path.
/// - Python creates the cgroup with `cgcreate` and classifies the shell
///   with `cgclassify`; here both are file ops (`create_dir_all` +
///   `cgroup.procs` write), still root-gated in practice.
#[derive(Debug)]
pub struct CPULimitedHost {
    /// The wrapped host.
    pub host: Host,
    /// Scheduler: `"cfs"` (default) or `"rt"` (rt unsupported — see docs).
    pub sched: String,
    /// CFS period in microseconds (Python `period_us`, default 100000).
    pub period_us: u64,
    /// The cgroup name (= host name) once [`CPULimitedHost::init_cgroups`]
    /// ran (`self.cgroup`).
    pub cgroup: Option<String>,
    /// Python `self.cgroupsInited`.
    cgroups_inited: bool,
}

impl CPULimitedHost {
    /// New CPU-limited host (Python `CPULimitedHost(name, sched='cfs',
    /// **params)`).
    pub fn new(name: &str, sched: &str, params: BTreeMap<String, String>) -> Self {
        let period_us = params
            .get("period_us")
            .and_then(|v| v.parse().ok())
            .unwrap_or(100000);
        Self {
            host: Host::with_params(name, params),
            sched: sched.to_string(),
            period_us,
            cgroup: None,
            cgroups_inited: false,
        }
    }

    fn cgroup_dir_path(&self) -> Result<PathBuf> {
        let root = cgroup_v2_root().ok_or_else(|| {
            Error::Other(
                "cgroup v2 not mounted at /sys/fs/cgroup (cpu controller missing)".to_string(),
            )
        })?;
        Ok(cgroup_dir(&root, &self.host.name))
    }

    /// Port of `CPULimitedHost.initCgroups` (cgroup v2): create
    /// `/sys/fs/cgroup/<name>` (`cgcreate`) and move the shell pid into it
    /// (`cgclassify` -> `cgroup.procs` write). Requires root.
    pub fn init_cgroups(&mut self) -> Result<()> {
        if self.cgroups_inited {
            return Ok(());
        }
        let dir = self.cgroup_dir_path()?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Other(format!("cgcreate {}: {e}", dir.display())))?;
        // cgclassify -g cpu,cpuacct:/<name> <pid>
        if let Some(pid) = self.host.pid() {
            std::fs::write(dir.join("cgroup.procs"), pid.to_string()).map_err(|e| {
                Error::Other(format!("cgclassify {} pid {pid}: {e}", dir.display()))
            })?;
        }
        self.cgroup = Some(self.host.name.clone());
        self.cgroups_inited = true;
        Ok(())
    }

    /// Port of `CPULimitedHost.setCPUFrac` (cgroup v2, cfs): compute
    /// quota/period via [`cfs_info`] and write them to `<cgroup>/cpu.max`.
    /// Returns the written `cpu.max` content. `f = -1` resets to unlimited
    /// (`max <period>`).
    pub fn set_cpu_frac(&mut self, f: f64, sched: Option<&str>) -> Result<String> {
        let sched = sched.unwrap_or(&self.sched);
        if sched == "rt" {
            return Err(Error::Other(format!(
                "{}: sched=rt (chrt + rt_runtime_us) is not ported — \
                 cgroup v2 cpu.max covers CFS only",
                self.host.name
            )));
        }
        if sched != "cfs" {
            return Ok(String::new()); // Python: else: return
        }
        self.init_cgroups()?;
        let (quota, period) = cfs_info(f, self.period_us, mn_util::num_cores() as u64);
        let content = cpu_max_content(&quota, period);
        let dir = self.cgroup_dir_path()?;
        std::fs::write(dir.join("cpu.max"), &content).map_err(|e| {
            Error::Other(format!("cgset cpu.max={content} on {}: {e}", dir.display()))
        })?;
        Ok(content)
    }

    /// Port of `CPULimitedHost.setCPUs` (cgroup v2 cpuset): write
    /// `cpuset.cpus` (and `cpuset.mems=0`, which must be set anyway), then
    /// move the shell pid in (`cgroup.procs` — the cgclassify cpuset step).
    /// `cores` is a CPU list string (`"0-1"`, `"0,2"`); Python list input
    /// is `','`-joined by the caller.
    pub fn set_cpus(&mut self, cores: &str) -> Result<()> {
        if cores.is_empty() {
            return Ok(()); // Python: if not cores: return
        }
        self.init_cgroups()?;
        let dir = self.cgroup_dir_path()?;
        std::fs::write(dir.join("cpuset.cpus"), cores).map_err(|e| {
            Error::Other(format!(
                "cgset cpuset.cpus={cores} on {}: {e}",
                dir.display()
            ))
        })?;
        std::fs::write(dir.join("cpuset.mems"), "0")
            .map_err(|e| Error::Other(format!("cgset cpuset.mems=0 on {}: {e}", dir.display())))?;
        if let Some(pid) = self.host.pid() {
            std::fs::write(dir.join("cgroup.procs"), pid.to_string()).map_err(|e| {
                Error::Other(format!(
                    "cgclassify cpuset {} pid {pid}: {e}",
                    dir.display()
                ))
            })?;
        }
        Ok(())
    }

    /// Port of `CPULimitedHost.config(cpu=-1, cores=None, **params)`:
    /// `Node.config(**params)`, then `initCgroups`, then
    /// `setParam(r, 'setCPUFrac', cpu=cpu)` and `setParam(r, 'setCPUs',
    /// cores=cores)` (the latter skipped when `cores` is None).
    pub fn config(&mut self, cpu: f64, cores: Option<&str>) -> Result<BTreeMap<String, String>> {
        let mut r = self.host.config_default()?;
        self.init_cgroups()?;
        // cpu defaults to -1 (not None), so setCPUFrac always runs; -1
        // writes "max <period>" (unlimited).
        let written = self.set_cpu_frac(cpu, None)?;
        if !written.is_empty() {
            r.insert("setCPUFrac".to_string(), written);
        }
        if let Some(cores) = cores {
            self.set_cpus(cores)?;
            r.insert("setCPUs".to_string(), cores.to_string());
        }
        Ok(r)
    }

    /// `configDefault` equivalent: pulls `cpu`/`cores` from `params`
    /// (parsing the float `cpu` param) and calls [`CPULimitedHost::config`].
    pub fn config_default(&mut self) -> Result<BTreeMap<String, String>> {
        let cpu = match self.host.params.get("cpu") {
            Some(v) => parse_cpu_param(v)?,
            None => -1.0,
        };
        let cores = self.host.params.get("cores").cloned();
        self.config(cpu, cores.as_deref())
    }

    /// Port of `CPULimitedHost.unlimit`: reset to unlimited when a `cpu`
    /// param other than -1 is set.
    pub fn unlimit(&mut self) -> Result<()> {
        if self.sched == "cfs" {
            if let Some(cpu) = self.host.params.get("cpu") {
                if parse_cpu_param(cpu)? != -1.0 {
                    self.set_cpu_frac(-1.0, None)?;
                }
            }
        }
        Ok(())
    }

    /// Port of `CPULimitedHost.cleanup`: terminate the node, then delete
    /// the cgroup (`cgdelete`), retrying 3x like Python; "no such file"
    /// counts as deleted.
    pub fn cleanup(&mut self) -> Result<()> {
        self.host.terminate()?;
        if self.cgroups_inited {
            if let Some(root) = cgroup_v2_root() {
                let dir = cgroup_dir(&root, &self.host.name);
                mn_util::retry(
                    3,
                    Duration::from_millis(100),
                    || match std::fs::remove_dir(&dir) {
                        Ok(()) => true,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                        Err(_) => false,
                    },
                );
            }
        }
        Ok(())
    }
}

impl Deref for CPULimitedHost {
    type Target = Host;
    fn deref(&self) -> &Host {
        &self.host
    }
}

impl DerefMut for CPULimitedHost {
    fn deref_mut(&mut self) -> &mut Host {
        &mut self.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mn_link::Intf;
    use std::net::IpAddr;

    #[test]
    fn host_is_a_node_with_host_kind() {
        let h = Host::with_params("h1", BTreeMap::new());
        assert!(h.in_namespace); // Python default inNamespace=True
        assert_eq!(h.name, "h1");
        assert_eq!(h.to_string(), "<Host h1:  pid=None> ");
    }

    #[test]
    fn add_host_defaults_matches_net_py() {
        // net.py: ip = ipAdd(nextIP, ipBaseNum=0x0a000000, prefixLen=8) + '/8'
        //         mac = macColonHex(nextIP)  (autoSetMacs)
        let mut h = Host::with_params("h1", BTreeMap::new());
        let ip = h.add_host_defaults(
            1,
            mn_util::DEFAULT_IP_BASE_NUM,
            mn_util::DEFAULT_PREFIX_LEN,
            true,
        );
        assert_eq!(ip.as_deref(), Some("10.0.0.1/8"));
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert_eq!(
            h.params.get("mac").map(String::as_str),
            Some("00:00:00:00:00:01")
        );
        let mut h2 = Host::with_params("h2", BTreeMap::new());
        let ip = h2.add_host_defaults(300, mn_util::DEFAULT_IP_BASE_NUM, 8, false);
        assert_eq!(ip.as_deref(), Some("10.0.1.44/8"));
        assert!(!h2.params.contains_key("mac"));
        // Out-of-range index: Python asserts; mn-util returns None.
        let mut h3 = Host::with_params("h3", BTreeMap::new());
        assert!(h3
            .add_host_defaults(0x1000000, mn_util::DEFAULT_IP_BASE_NUM, 8, false)
            .is_none());
    }

    #[test]
    fn add_host_defaults_does_not_clobber_caller_params() {
        // Python net.py: defaults.update(params) — caller-supplied ip/mac win.
        let mut h = Host::with_params(
            "h1",
            [
                ("ip".to_string(), "192.168.7.7/24".to_string()),
                ("mac".to_string(), "02:00:00:00:00:99".to_string()),
            ]
            .into_iter()
            .collect(),
        );
        let ip = h.add_host_defaults(1, mn_util::DEFAULT_IP_BASE_NUM, 8, true);
        // The computed default is still returned ...
        assert_eq!(ip.as_deref(), Some("10.0.0.1/8"));
        // ... but params keep the caller-supplied values.
        assert_eq!(
            h.params.get("ip").map(String::as_str),
            Some("192.168.7.7/24")
        );
        assert_eq!(
            h.params.get("mac").map(String::as_str),
            Some("02:00:00:00:00:99")
        );
        // Partial override: only ip supplied, mac still gets the default.
        let mut h2 = Host::with_params(
            "h2",
            [("ip".to_string(), "172.16.0.5/12".to_string())]
                .into_iter()
                .collect(),
        );
        h2.add_host_defaults(2, mn_util::DEFAULT_IP_BASE_NUM, 8, true);
        assert_eq!(
            h2.params.get("ip").map(String::as_str),
            Some("172.16.0.5/12")
        );
        assert_eq!(
            h2.params.get("mac").map(String::as_str),
            Some("00:00:00:00:00:02")
        );
    }

    #[test]
    fn assign_ip_uses_ip_add_hook() {
        let mut h = Host::with_params("h5", BTreeMap::new());
        assert_eq!(
            h.assign_ip(5, 8, mn_util::DEFAULT_IP_BASE_NUM).as_deref(),
            Some("10.0.0.5/8")
        );
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.5/8"));
    }

    #[test]
    fn host_config_default_applies_params_to_default_intf() {
        let mut h = Host::with_params(
            "h1",
            [("ip".to_string(), "10.0.0.1/8".to_string())]
                .into_iter()
                .collect(),
        );
        h.add_intf(Intf::new("h1-eth0", "h1", Some(0)), Some(0))
            .unwrap();
        let r = h.config_default().unwrap();
        assert_eq!(r.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert_eq!(h.ip(None), Some(IpAddr::from([10, 0, 0, 1])));
        assert_eq!(h.default_intf().unwrap().name, "h1-eth0");
    }

    #[test]
    fn host_from_topo_params() {
        let mut attrs = Attrs::new();
        attrs.insert("ip", "10.0.0.9/8");
        let h = Host::from_topo_params("h9", &attrs);
        assert_eq!(h.name, "h9");
        assert!(h.in_namespace);
        assert_eq!(h.params.get("ip").map(String::as_str), Some("10.0.0.9/8"));
        assert!(h.to_string().starts_with("<Host h9: "));
    }

    #[test]
    fn cfs_info_matches_python_cfs_info_cgroup2() {
        // Python cfsInfo (cgversion='cgroup2'): quota = int(period_us * f * numCores())
        let (q, p) = cfs_info(0.5, 100000, 4);
        assert_eq!((q.as_str(), p), ("200000", 100000)); // int(100000*0.5*4)
        let (q, p) = cfs_info(1.0, 100000, 4);
        assert_eq!((q.as_str(), p), ("400000", 100000));
        // negative quota resets to unlimited ('max' in cgroup2)
        let (q, p) = cfs_info(-1.0, 100000, 4);
        assert_eq!((q.as_str(), p), ("max", 100000));
        // f > 0 with quota < 1000us: raise quota to the 1ms CFS minimum and
        // grow the period: period = int(1000 / f / cores). CPython computes
        // int(1000/0.00001/4) == 24999999 (f64 truncation), and so do we.
        let (q, p) = cfs_info(0.00001, 100000, 4);
        assert_eq!((q.as_str(), p), ("1000", 24999999));
        // f = 0 -> quota 0 (not negative, not raised)
        let (q, p) = cfs_info(0.0, 100000, 4);
        assert_eq!((q.as_str(), p), ("0", 100000));
        // custom period_us
        let (q, p) = cfs_info(0.25, 200000, 2);
        assert_eq!((q.as_str(), p), ("100000", 200000));
    }

    #[test]
    fn cpu_max_content_and_cpu_param_parsing() {
        assert_eq!(cpu_max_content("max", 100000), "max 100000");
        assert_eq!(cpu_max_content("200000", 100000), "200000 100000");
        assert_eq!(parse_cpu_param("0.5").unwrap(), 0.5);
        assert_eq!(parse_cpu_param("-1").unwrap(), -1.0);
        assert_eq!(parse_cpu_param("  0.25 ").unwrap(), 0.25);
        assert!(parse_cpu_param("abc").is_err());
        assert!(parse_cpu_param("").is_err());
        let err = parse_cpu_param("half").unwrap_err().to_string();
        assert!(err.contains("cpu param"), "got {err}");
    }

    #[test]
    fn cgroup_v2_root_detection_is_read_only() {
        match cgroup_v2_root() {
            Some(root) => {
                assert_eq!(root, PathBuf::from("/sys/fs/cgroup"));
                let controllers = std::fs::read_to_string(root.join("cgroup.controllers")).unwrap();
                assert!(controllers.split_whitespace().any(|c| c == "cpu"));
            }
            None => {
                // cgroup v1 or no cgroup fs: detection reports None.
                assert!(std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers").is_err());
            }
        }
        assert_eq!(
            cgroup_dir(Path::new("/sys/fs/cgroup"), "h1"),
            PathBuf::from("/sys/fs/cgroup/h1")
        );
    }

    #[test]
    fn cpu_limited_host_model_and_sched_rules() {
        let h = CPULimitedHost::new(
            "h1",
            "cfs",
            [("period_us".to_string(), "50000".to_string())]
                .into_iter()
                .collect(),
        );
        assert_eq!(h.period_us, 50000);
        assert_eq!(h.sched, "cfs");
        assert_eq!(h.cgroup, None);
        assert_eq!(h.host.name, "h1");
        let h2 = CPULimitedHost::new("h2", "cfs", BTreeMap::new());
        assert_eq!(h2.period_us, 100000); // Python default

        // sched=rt is not ported (chrt + rt_runtime_us): explicit error.
        let mut rt = CPULimitedHost::new("h3", "rt", BTreeMap::new());
        let err = rt.set_cpu_frac(0.5, None).unwrap_err().to_string();
        assert!(err.contains("rt"), "got {err}");
        // Unknown sched: Python's `else: return` — no-op, empty result.
        let mut h4 = CPULimitedHost::new("h4", "cfs", BTreeMap::new());
        assert_eq!(h4.set_cpu_frac(0.5, Some("bogus")).unwrap(), "");
        // config_default parses the cpu param and rejects garbage.
        let mut bad = CPULimitedHost::new(
            "h5",
            "cfs",
            [("cpu".to_string(), "half".to_string())]
                .into_iter()
                .collect(),
        );
        let err = bad.config_default().unwrap_err();
        assert!(matches!(err, crate::Error::InvalidInput(_)), "got {err:?}");
    }

    #[test]
    fn cpu_limited_host_apply_needs_root() {
        // In this sandbox we are non-root and /sys/fs/cgroup is not
        // writable, so init_cgroups must fail; as root it succeeds and we
        // clean up the cgroup again.
        let name = format!("mnclhselftest{}", std::process::id() % 100000);
        let mut h = CPULimitedHost::new(&name, "cfs", BTreeMap::new());
        let r = h.set_cpu_frac(0.5, None);
        // canonical root check (mn_util::is_root, also re-exported by mn-net)
        if mn_util::is_root() {
            let content = r.expect("root can create the cgroup and write cpu.max");
            let (q, p) = cfs_info(0.5, 100000, mn_util::num_cores() as u64);
            assert_eq!(content, cpu_max_content(&q, p));
            h.cleanup().unwrap();
            let root = cgroup_v2_root().unwrap();
            assert!(!cgroup_dir(&root, &name).exists(), "cgroup cleaned up");
        } else {
            assert!(r.is_err(), "non-root must not be able to create cgroups");
        }
    }

    #[test]
    #[ignore = "requires root + cgroup v2 mounted rw at /sys/fs/cgroup; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn cpu_limited_host_cgroup_v2_root() {
        if !crate::root_tests_enabled() {
            return;
        }
        let name = format!("mnclh{}", std::process::id() % 100000);
        let mut h = CPULimitedHost::new(&name, "cfs", BTreeMap::new());
        let root = cgroup_v2_root().expect("cgroup v2 on the test host");
        let dir = cgroup_dir(&root, &name);
        let _ = std::fs::remove_dir(&dir); // in case a previous run leaked

        // config(): Node.config + initCgroups + setCPUFrac + setCPUs
        let r = h.config(0.5, Some("0")).expect("cgroup ops as root");
        assert!(r.contains_key("setCPUFrac"));
        assert_eq!(r.get("setCPUs").map(String::as_str), Some("0"));
        assert!(dir.is_dir(), "cgcreate made {dir:?}");
        let (q, p) = cfs_info(0.5, h.period_us, mn_util::num_cores() as u64);
        let content = cpu_max_content(&q, p);
        assert_eq!(
            std::fs::read_to_string(dir.join("cpu.max")).unwrap().trim(),
            content
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("cpuset.cpus"))
                .unwrap()
                .trim(),
            "0"
        );
        // unlimited reset
        let content = h.set_cpu_frac(-1.0, None).unwrap();
        assert_eq!(content, format!("max {}", h.period_us));
        assert_eq!(
            std::fs::read_to_string(dir.join("cpu.max")).unwrap().trim(),
            content
        );
        // cleanup() deletes the cgroup (cgdelete with retry)
        h.cleanup().unwrap();
        assert!(!dir.exists(), "cgdelete removed {dir:?}");
    }
}
