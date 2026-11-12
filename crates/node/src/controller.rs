//! `Controller` family — port of `class Controller( Node )` and its
//! subclasses (`OVSController`, `NOX`, `Ryu`, `RemoteController`) in
//! `mininet/node.py`.
//!
//! Controllers run in the root namespace by default (`inNamespace=False`).
//! `Controller.start` spawns `command cargs % port` as a background job in
//! the node's shell, logging to `/tmp/<name>.log` (Python does the same).
//!
//! # Porting notes / deviations from CPython (documented)
//!
//! - Python's `Controller.checkListening` shells out to
//!   `echo A | telnet -e A <ip> <port>` and `netstat`; here the probe is a
//!   plain [`TcpStream::connect_timeout`] ([`is_listening`]), which works
//!   unprivileged on localhost and cannot hang. The `netstat` listing in
//!   the error message is not reproduced.
//! - Python's `RemoteController.isListening` warns via the `logging`
//!   module; no logging backend is ported yet, so the probe results are
//!   recorded in [`RemoteController::probes`] instead of printed.
//! - `pathCheck(self.command)` in `Controller.start` is ported as a
//!   [`which`] lookup of the command's first word (Python passes the whole
//!   `command` string — e.g. `"ryu run"` — to `which`, which only makes
//!   sense for single-word commands).
//! - The `ip:port` shorthand split in `Controller.__init__` keeps Python's
//!   exact behavior, including its limitation: an IPv6 literal contains
//!   `':'` and is rejected, exactly like CPython's `ip.split(':')` unpack.
//! - The 5-argument [`Controller::new`] kept for orchestrator compatibility
//!   builds the model only (like the 8a stub); the full Python `__init__`
//!   — including the `checkListening` port probe — is [`Controller::new_full`].
//! - `NOX`/`Ryu` are stubs with TODO: their constructors port Python's
//!   command/`cargs` computation (unit-tested), but `start()` is not wired
//!   to the NOX/Ryu runtimes yet, and their constructors skip the port
//!   probe that `Controller.__init__` would perform.

use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::ops::{Deref, DerefMut};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::mnexec::which;
use crate::node::Node;

/// Default OpenFlow controller port (Python `Controller.__init__`'s
/// `port=6653`).
pub const DEFAULT_PORT: u16 = 6653;

/// Default controller command (Python `command='controller'`).
pub const DEFAULT_COMMAND: &str = "controller";

/// Default controller args template (Python `cargs='ptcp:%d'`), formatted
/// with the port by [`cargs_with_port`].
pub const DEFAULT_CARGS: &str = "ptcp:%d";

/// Default controller protocol (Python `protocol='tcp'`).
pub const DEFAULT_PROTOCOL: &str = "tcp";

/// Default controller IP (Python `ip="127.0.0.1"`).
pub const DEFAULT_IP: &str = "127.0.0.1";

/// Timeout for the [`is_listening`] TCP probe. Python's telnet probe has no
/// short timeout (it can hang on a firewalled host); we bound the connect
/// so controller checks cannot stall.
pub const LISTEN_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Python `RemoteController.isListening(ip, port)` / the probe in
/// `Controller.checkListening`: is something accepting TCP connections at
/// `ip:port`? Ported from `echo A | telnet -e A <ip> <port>` + `'Connected'
/// in output` to a plain `connect(2)` with a timeout — works unprivileged
/// on localhost (connection refused returns immediately).
pub fn is_listening(ip: IpAddr, port: u16) -> bool {
    is_listening_timeout(SocketAddr::new(ip, port), LISTEN_PROBE_TIMEOUT)
}

/// [`is_listening`] with an explicit connect timeout (split out for tests).
pub fn is_listening_timeout(addr: SocketAddr, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Python `self.cargs % self.port`: format the cargs template with the
/// port. Handles the `%d`/`%s` conversions used by the built-in templates
/// (`ptcp:%d`, `... -i ptcp:%s <args>`); a template without a conversion
/// is returned unchanged.
pub fn cargs_with_port(cargs: &str, port: u16) -> String {
    let pos = match (cargs.find("%d"), cargs.find("%s")) {
        (Some(d), Some(s)) => Some(d.min(s)),
        (d, s) => d.or(s),
    };
    match pos {
        Some(i) => format!("{}{}{}", &cargs[..i], port, &cargs[i + 2..]),
        None => cargs.to_string(),
    }
}

/// Python `Controller.__init__`'s `ip:port` shorthand:
/// `if ':' in ip: ip, port = ip.split(':'); port = int(port)`.
/// Keeps CPython's exact semantics (a second `':'` is an error, so bare
/// IPv6 literals are rejected just like in Python).
fn split_ip_port(ip: &str, port: u16) -> Result<(String, u16)> {
    if !ip.contains(':') {
        return Ok((ip.to_string(), port));
    }
    let parts: Vec<&str> = ip.split(':').collect();
    if parts.len() != 2 {
        return Err(Error::InvalidInput(format!(
            "invalid controller ip {ip:?}: Python's ip.split(':') unpack fails the same way"
        )));
    }
    let p: u16 = parts[1]
        .parse()
        .map_err(|_| Error::InvalidInput(format!("invalid controller port {:?}", parts[1])))?;
    Ok((parts[0].to_string(), p))
}

/// An OpenFlow controller node. Port of `class Controller( Node )` in
/// `mininet/node.py`.
#[derive(Debug)]
pub struct Controller {
    node: Node,
    /// The controller command (Python `command='controller'`).
    pub command: String,
    /// Controller args template, printf-style with the port (Python
    /// `cargs='ptcp:%d'`).
    pub cargs: String,
    /// Working directory for the controller process (Python `cdir=None`).
    pub cdir: Option<String>,
    /// The control IP (Python `self.ip`).
    pub ip: IpAddr,
    /// TCP port the controller listens on (Python `self.port`).
    pub port: u16,
    /// Controller protocol (`tcp`, Python `protocol='tcp'`), used when
    /// building `protocol:ip:port` targets for switches.
    pub protocol: String,
}

impl Controller {
    /// Back-compatible model constructor (same shape as the 8a stub):
    /// `Controller::new(name, command, port, in_namespace, params)` with the
    /// Python defaults `cargs='ptcp:%d'`, `cdir=None`, `protocol='tcp'`, and
    /// `ip` from `params["ip"]` or loopback. No port probe is run here —
    /// use [`Controller::check_listening`] or [`Controller::new_full`].
    pub fn new(
        name: &str,
        command: &str,
        port: u16,
        in_namespace: bool,
        mut params: BTreeMap<String, String>,
    ) -> Self {
        let ip_str = params
            .get("ip")
            .cloned()
            .unwrap_or_else(|| DEFAULT_IP.to_string());
        params.insert("ip".to_string(), ip_str.clone());
        // The `ip` param may carry a prefix ("10.0.0.99/8"); the control IP
        // is the address part (like the 8a stub's `ip()`).
        let ip = ip_str
            .split('/')
            .next()
            .and_then(|a| a.parse().ok())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let mut node = Node::new(name, in_namespace, params);
        node.set_kind("Controller");
        Self {
            node,
            command: command.to_string(),
            cargs: DEFAULT_CARGS.to_string(),
            cdir: None,
            ip,
            port,
            protocol: DEFAULT_PROTOCOL.to_string(),
        }
    }

    /// Python default constructor shape: `Controller(name, **params)`.
    pub fn with_params(name: &str, params: BTreeMap<String, String>) -> Self {
        let port = params
            .get("port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Self::new(name, DEFAULT_COMMAND, port, false, params)
    }

    /// Full port of `Controller.__init__` (all keyword args) followed by
    /// [`Controller::check_listening`] (Python calls `self.checkListening()`
    /// at the end of `__init__`).
    #[allow(clippy::too_many_arguments)]
    pub fn new_full(
        name: &str,
        in_namespace: bool,
        command: &str,
        cargs: &str,
        cdir: Option<&str>,
        ip: &str,
        port: u16,
        protocol: &str,
        verbose: bool,
        mut params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut cargs = cargs.to_string();
        if verbose {
            // Python: if verbose: cargs = '-v ' + cargs
            cargs = format!("-v {cargs}");
        }
        let (ip_str, port) = split_ip_port(ip, port)?;
        let ip: IpAddr = ip_str
            .parse()
            .map_err(|_| Error::InvalidInput(format!("invalid controller ip {ip_str:?}")))?;
        // Python passes ip=ip into Node.__init__'s **params.
        params.insert("ip".to_string(), ip_str);
        let mut node = Node::new(name, in_namespace, params);
        node.set_kind("Controller");
        let c = Self {
            node,
            command: command.to_string(),
            cargs,
            cdir: cdir.map(str::to_string),
            ip,
            port,
            protocol: protocol.to_string(),
        };
        c.check_listening()?;
        Ok(c)
    }

    /// Port of `Controller.checkListening`: make sure no controller is
    /// already running on our port. Python probes with telnet and raises
    /// (with a netstat listing) when the port answers; here the probe is
    /// [`is_listening`] and the error names the port and address.
    pub fn check_listening(&self) -> Result<()> {
        if is_listening(self.ip, self.port) {
            return Err(Error::Other(format!(
                "Please shut down the controller which is running on port {}:\n\
                 ({}:{} answered a TCP probe)",
                self.port, self.ip, self.port
            )));
        }
        Ok(())
    }

    /// Port of `Controller.start`: spawn `<command> <cargs % port>` as a
    /// background job in the node's shell, logging to `/tmp/<name>.log`.
    /// Starts the node shell first if needed (Python starts it in
    /// `Node.__init__`).
    pub fn start(&mut self) -> Result<()> {
        // Python: pathCheck(self.command) — check the executable is on PATH.
        let prog = self.command.split_whitespace().next().unwrap_or("");
        if prog.is_empty() || which(prog).is_none() {
            return Err(Error::InvalidInput(format!(
                "Cannot find required executable {prog:?} for controller {}; \
                 please make sure it is installed and available in your $PATH",
                self.node.name
            )));
        }
        if !self.node.started() {
            self.node.start()?;
        }
        let cout = format!("/tmp/{}.log", self.node.name);
        if let Some(cdir) = &self.cdir {
            self.node.cmd(&format!("cd {cdir}"))?;
        }
        // Python: self.cmd( self.command + ' ' + self.cargs % self.port +
        //                   ' 1>' + cout + ' 2>' + cout + ' &' )
        let line = format!(
            "{} {} 1>{cout} 2>{cout} &",
            self.command,
            cargs_with_port(&self.cargs, self.port)
        );
        self.node.cmd(&line)?;
        // Python sets self.execed = False; Node::start_shell already did.
        Ok(())
    }

    /// Port of `Controller.stop`: `kill %<command>` + `wait %<command>` in
    /// the shell, then `Node.stop`. On an unstarted node the shell commands
    /// are skipped (Python's `cmd` just warns on a dead shell).
    pub fn stop(&mut self) -> Result<()> {
        if self.node.started() {
            let _ = self.node.cmd(&format!("kill %{}", self.command));
            let _ = self.node.cmd(&format!("wait %{}", self.command));
        }
        self.node.terminate()
    }

    /// Port of `Controller.IP()` with no argument: the default interface's
    /// IP if the controller has interfaces, else the configured control IP.
    pub fn ip(&self) -> IpAddr {
        self.ip_intf(None)
    }

    /// Port of `Controller.IP(intf=None)`: the IP of the given interface if
    /// the controller has interfaces, else the configured control IP.
    pub fn ip_intf(&self, intf: Option<&str>) -> IpAddr {
        if !self.node.intf_list().is_empty() {
            self.node.ip(intf).unwrap_or(self.ip)
        } else {
            self.ip
        }
    }

    /// Port of `Controller.isAvailable`: is the `controller` binary on PATH?
    pub fn is_available() -> bool {
        which(DEFAULT_COMMAND).is_some()
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

impl Deref for Controller {
    type Target = Node;
    fn deref(&self) -> &Node {
        &self.node
    }
}

impl DerefMut for Controller {
    fn deref_mut(&mut self) -> &mut Node {
        &mut self.node
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Python __repr__: '<%s %s: %s:%s pid=%s> ' % (cls, name, IP(), port, pid)
        let pid = self
            .node
            .pid()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "None".to_string());
        write!(
            f,
            "<{} {}: {}:{} pid={}> ",
            self.node.kind(),
            self.node.name,
            self.ip(),
            self.port,
            pid
        )
    }
}

/// `OVSController` — port of `class OVSController( Controller )`: the
/// command is the first available of `ovs-controller`/`test-controller`/
/// `ovs-testcontroller`, else the literal `ovs-controller`.
#[derive(Debug)]
pub struct OVSController {
    controller: Controller,
}

impl OVSController {
    /// Port of `OVSController.isAvailable`: `which('ovs-controller') or
    /// which('test-controller') or which('ovs-testcontroller')` — returns
    /// the resolved path (like Python's `which` result).
    pub fn is_available() -> Option<String> {
        which("ovs-controller")
            .or_else(|| which("test-controller"))
            .or_else(|| which("ovs-testcontroller"))
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Python `OVSController.__init__`: `kwargs.setdefault('command',
    /// self.isAvailable() or 'ovs-controller')`, then `Controller.__init__`
    /// (including the port probe).
    pub fn new(name: &str, params: BTreeMap<String, String>) -> Result<Self> {
        Self::new_full(name, DEFAULT_PORT, params)
    }

    /// [`OVSController::new`] with an explicit port (used by tests so the
    /// `checkListening` probe does not depend on 6653 being free).
    pub fn new_full(name: &str, port: u16, params: BTreeMap<String, String>) -> Result<Self> {
        let command = Self::is_available().unwrap_or_else(|| "ovs-controller".to_string());
        let mut controller = Controller::new_full(
            name,
            false,
            &command,
            DEFAULT_CARGS,
            None,
            DEFAULT_IP,
            port,
            DEFAULT_PROTOCOL,
            false,
            params,
        )?;
        controller.node_mut().set_kind("OVSController");
        Ok(Self { controller })
    }

    /// Unwrap into the base [`Controller`].
    pub fn into_controller(self) -> Controller {
        self.controller
    }
}

impl Deref for OVSController {
    type Target = Controller;
    fn deref(&self) -> &Controller {
        &self.controller
    }
}

impl DerefMut for OVSController {
    fn deref_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }
}

impl fmt::Display for OVSController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.controller.fmt(f)
    }
}

/// `NOX` — stub port of `class NOX( Controller )`. The constructor ports
/// Python's command/cargs computation; `start()` is a TODO (the NOX runtime
/// is not wired up yet).
#[derive(Debug)]
pub struct NOX {
    controller: Controller,
    /// The NOX modules/apps to run (Python `noxArgs`, default `packetdump`).
    pub nox_args: Vec<String>,
}

impl NOX {
    /// Port of `NOX.__init__`: defaults to `packetdump` when no modules are
    /// given, requires `NOX_CORE_DIR` (Python calls `exit(1)` when it is
    /// missing — here an error), command `<NOX_CORE_DIR>/nox_core`,
    /// cargs `--libdir=/usr/local/lib -v -i ptcp:%s <args>`, cdir =
    /// `NOX_CORE_DIR`.
    ///
    /// Deviation: the constructor skips the port probe that
    /// `Controller.__init__` would run (this is a stub; `start()` is TODO).
    pub fn new(name: &str, nox_args: &[String], params: BTreeMap<String, String>) -> Result<Self> {
        // Python: if not noxArgs: warn(...); noxArgs = ['packetdump']
        let args: Vec<String> = if nox_args.is_empty() {
            vec!["packetdump".to_string()]
        } else {
            nox_args.to_vec()
        };
        let nox_core_dir = std::env::var("NOX_CORE_DIR").map_err(|_| {
            Error::Other("exiting; please set missing NOX_CORE_DIR env var".to_string())
        })?;
        let command = format!("{nox_core_dir}/nox_core");
        let cargs = format!("--libdir=/usr/local/lib -v -i ptcp:%s {}", args.join(" "));
        let node = {
            let mut params = params;
            params.insert("ip".to_string(), DEFAULT_IP.to_string());
            let mut n = Node::new(name, false, params);
            n.set_kind("NOX");
            n
        };
        let controller = Controller {
            node,
            command,
            cargs,
            cdir: Some(nox_core_dir),
            ip: IpAddr::from([127, 0, 0, 1]),
            port: DEFAULT_PORT,
            protocol: DEFAULT_PROTOCOL.to_string(),
        };
        Ok(Self {
            controller,
            nox_args: args,
        })
    }

    /// TODO(8b): NOX runtime startup (`nox_core` with app modules) is not
    /// ported yet — the generic [`Controller::start`] would work, but NOX
    /// app loading/verification is not wired up.
    pub fn start(&mut self) -> Result<()> {
        Err(Error::Other(format!(
            "TODO(8b): NOX controller {} startup (nox_core + {:?}) not ported yet",
            self.controller.node.name, self.nox_args
        )))
    }

    /// Port of `Controller.stop` passthrough.
    pub fn stop(&mut self) -> Result<()> {
        self.controller.stop()
    }
}

impl Deref for NOX {
    type Target = Controller;
    fn deref(&self) -> &Controller {
        &self.controller
    }
}

impl DerefMut for NOX {
    fn deref_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }
}

impl fmt::Display for NOX {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.controller.fmt(f)
    }
}

/// `Ryu` — stub port of `class Ryu( Controller )`. The constructor ports
/// Python's command/cargs computation; `start()` is a TODO.
#[derive(Debug)]
pub struct Ryu {
    controller: Controller,
    /// The Ryu apps/modules (Python `ryuArgs='ryu.app.simple_switch'`).
    pub ryu_args: String,
}

impl Ryu {
    /// Port of `Ryu.__init__`: `ryuArgs` (list → joined), `command='ryu
    /// run'`, `cargs = kwargs.pop('cargs', ryuArgs + ' --ofp-tcp-listen-port %s')`.
    ///
    /// Deviation: like `NOX`, the constructor skips the port probe.
    pub fn new(
        name: &str,
        ryu_args: &[String],
        command: Option<&str>,
        cargs_override: Option<&str>,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        // Python: if isinstance(ryuArgs, (list, tuple)): ryuArgs = ' '.join(ryuArgs)
        let ryu_args = ryu_args.join(" ");
        let command = command.unwrap_or("ryu run");
        let cargs = cargs_override
            .map(str::to_string)
            .unwrap_or_else(|| format!("{ryu_args} --ofp-tcp-listen-port %s"));
        let mut params = params;
        params.insert("ip".to_string(), DEFAULT_IP.to_string());
        let mut node = Node::new(name, false, params);
        node.set_kind("Ryu");
        let controller = Controller {
            node,
            command: command.to_string(),
            cargs,
            cdir: None,
            ip: IpAddr::from([127, 0, 0, 1]),
            port: DEFAULT_PORT,
            protocol: DEFAULT_PROTOCOL.to_string(),
        };
        Ok(Self {
            controller,
            ryu_args,
        })
    }

    /// TODO(8b): Ryu runtime startup (`ryu run <apps>`) is not ported yet.
    pub fn start(&mut self) -> Result<()> {
        Err(Error::Other(format!(
            "TODO(8b): Ryu controller {} startup (ryu run {:?}) not ported yet",
            self.controller.node.name, self.ryu_args
        )))
    }

    /// Port of `Controller.stop` passthrough.
    pub fn stop(&mut self) -> Result<()> {
        self.controller.stop()
    }
}

impl Deref for Ryu {
    type Target = Controller;
    fn deref(&self) -> &Controller {
        &self.controller
    }
}

impl DerefMut for Ryu {
    fn deref_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }
}

impl fmt::Display for Ryu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.controller.fmt(f)
    }
}

/// `RemoteController` — port of `class RemoteController( Controller )`: a
/// controller running outside of Mininet's control. `start`/`stop` are
/// no-ops; `check_listening` only probes (and records) whether the remote
/// controller answers.
#[derive(Debug)]
pub struct RemoteController {
    controller: Controller,
    /// The port the user asked for (`None` → probe 6653/6633 like Python).
    requested_port: Option<u16>,
    /// Recorded `(port, listening)` probe results from the last
    /// [`RemoteController::check_listening`] (Python prints warnings via
    /// the logging module, which is not ported yet).
    pub probes: Vec<(u16, bool)>,
}

impl RemoteController {
    /// Port of `RemoteController.__init__` + its overridden
    /// `checkListening`.
    pub fn new(
        name: &str,
        ip: &str,
        port: Option<u16>,
        params: BTreeMap<String, String>,
    ) -> Result<Self> {
        let requested = port;
        let (ip_str, port) = split_ip_port(ip, port.unwrap_or(DEFAULT_PORT))?;
        let ip_addr: IpAddr = ip_str
            .parse()
            .map_err(|_| Error::InvalidInput(format!("invalid controller ip {ip_str:?}")))?;
        let mut params = params;
        params.insert("ip".to_string(), ip_str);
        let mut node = Node::new(name, false, params);
        node.set_kind("RemoteController");
        let controller = Controller {
            node,
            command: DEFAULT_COMMAND.to_string(),
            cargs: DEFAULT_CARGS.to_string(),
            cdir: None,
            ip: ip_addr,
            port,
            protocol: DEFAULT_PROTOCOL.to_string(),
        };
        let mut rc = Self {
            controller,
            requested_port: requested,
            probes: Vec::new(),
        };
        rc.check_listening();
        Ok(rc)
    }

    /// Port of `RemoteController.checkListening`: with an explicit port,
    /// probe it (Python warns when unreachable); without one, try 6653 then
    /// 6633 and adopt the first listening port, else fall back to 6653.
    pub fn check_listening(&mut self) {
        self.probes.clear();
        match self.requested_port {
            Some(p) => {
                let listening = is_listening(self.controller.ip, p);
                self.probes.push((p, listening));
                self.controller.port = p;
            }
            None => {
                let mut found = None;
                for p in [DEFAULT_PORT, 6633] {
                    let listening = is_listening(self.controller.ip, p);
                    self.probes.push((p, listening));
                    if listening {
                        found = Some(p);
                        break;
                    }
                }
                // Python: if self.port is None: self.port = 6653; warn(...)
                self.controller.port = found.unwrap_or(DEFAULT_PORT);
            }
        }
    }

    /// Port of `RemoteController.isListening`: is a remote controller
    /// listening at `ip:port`? (std `TcpStream` probe with timeout — works
    /// unprivileged on localhost.)
    pub fn is_listening(&self, ip: IpAddr, port: u16) -> bool {
        is_listening(ip, port)
    }

    /// Python `RemoteController.start`: "Overridden to do nothing."
    pub fn start(&mut self) -> Result<()> {
        Ok(())
    }

    /// Python `RemoteController.stop`: "Overridden to do nothing."
    pub fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    /// The inner controller.
    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    /// Mutable access to the inner controller.
    pub fn controller_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }
}

impl Deref for RemoteController {
    type Target = Controller;
    fn deref(&self) -> &Controller {
        &self.controller
    }
}

impl DerefMut for RemoteController {
    fn deref_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }
}

impl fmt::Display for RemoteController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.controller.fmt(f)
    }
}

/// Port of `findController`/`DefaultController` over Python's
/// `DefaultControllers = (Controller, OVSController)`: return a controller
/// of the first available class (including the `checkListening` probe), or
/// an error if none is installed.
pub fn default_controller(name: &str, params: BTreeMap<String, String>) -> Result<Controller> {
    if Controller::is_available() {
        return Controller::new_full(
            name,
            false,
            DEFAULT_COMMAND,
            DEFAULT_CARGS,
            None,
            DEFAULT_IP,
            DEFAULT_PORT,
            DEFAULT_PROTOCOL,
            false,
            params,
        );
    }
    if OVSController::is_available().is_some() {
        return OVSController::new(name, params).map(OVSController::into_controller);
    }
    Err(Error::Other(
        "Could not find a default OpenFlow controller".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    /// Bind a loopback listener on an ephemeral port and return the port
    /// (the listener stays alive for the duration of the test).
    fn listen_on_free_port() -> (TcpListener, u16) {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        (l, p)
    }

    /// A port that is (almost certainly) free: bind, read the port, drop.
    fn free_port() -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn cargs_with_port_formats_python_templates() {
        // Python: self.cargs % self.port
        assert_eq!(cargs_with_port("ptcp:%d", 6653), "ptcp:6653");
        assert_eq!(
            cargs_with_port("--libdir=/usr/local/lib -v -i ptcp:%s packetdump", 6653),
            "--libdir=/usr/local/lib -v -i ptcp:6653 packetdump"
        );
        assert_eq!(
            cargs_with_port("ryu.app.simple_switch --ofp-tcp-listen-port %s", 6633),
            "ryu.app.simple_switch --ofp-tcp-listen-port 6633"
        );
        // No conversion: returned unchanged.
        assert_eq!(cargs_with_port("--foo", 1), "--foo");
        // The earliest conversion wins.
        assert_eq!(cargs_with_port("%d-%s", 7), "7-%s");
    }

    #[test]
    fn is_listening_probes_localhost_without_privileges() {
        let (listener, port) = listen_on_free_port();
        assert!(is_listening(IpAddr::from([127, 0, 0, 1]), port));
        assert!(is_listening_timeout(
            SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port),
            Duration::from_millis(500)
        ));
        drop(listener);
        // Closed port: connection refused -> not listening (fast on lo).
        assert!(!is_listening(IpAddr::from([127, 0, 0, 1]), port));
        assert!(!is_listening_timeout(
            SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port),
            Duration::from_millis(500)
        ));
    }

    #[test]
    fn controller_model_defaults_match_python() {
        let c = Controller::with_params("c0", BTreeMap::new());
        assert_eq!(c.name, "c0");
        assert!(!c.in_namespace); // Python Controller: inNamespace=False
        assert_eq!(c.command, "controller");
        assert_eq!(c.cargs, "ptcp:%d");
        assert_eq!(c.cdir, None);
        assert_eq!(c.port, 6653);
        assert_eq!(c.protocol, "tcp");
        assert_eq!(c.ip(), IpAddr::from([127, 0, 0, 1]));
        assert_eq!(c.node().name, "c0");
        assert_eq!(c.to_string(), "<Controller c0: 127.0.0.1:6653 pid=None> ");
        // ip from params (Python passes ip=ip into Node params):
        let mut params = BTreeMap::new();
        params.insert("ip".to_string(), "10.0.0.99/8".to_string());
        let c = Controller::with_params("c1", params);
        assert_eq!(c.ip(), IpAddr::from([10, 0, 0, 99]));
        assert!(c.to_string().starts_with("<Controller c1: 10.0.0.99:"));
        let mut c = c;
        c.stop().unwrap(); // no-op terminate on unstarted node
        assert!(!c.started());
    }

    #[test]
    fn new_full_handles_ip_port_shorthand_verbose_and_probe() {
        let port = free_port();
        let c = Controller::new_full(
            "c0",
            false,
            "controller",
            "ptcp:%d",
            None,
            &format!("127.0.0.1:{port}"),
            6653, // overridden by the shorthand, like Python
            "tcp",
            false,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(c.ip, IpAddr::from([127, 0, 0, 1]));
        assert_eq!(c.port, port);
        // ip=ip lands in the node params like Python's Node.__init__(ip=ip).
        assert_eq!(c.params.get("ip").map(String::as_str), Some("127.0.0.1"));

        // verbose prefixes cargs with '-v ' (Python).
        let port2 = free_port();
        let c = Controller::new_full(
            "c0",
            false,
            "controller",
            "ptcp:%d",
            Some("/tmp"),
            "127.0.0.1",
            port2,
            "tcp",
            true,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(c.cargs, "-v ptcp:%d");
        assert_eq!(c.cdir.as_deref(), Some("/tmp"));

        // Bad ip / too many colons / bad port are errors (like Python's
        // int()/unpack failures).
        for bad in ["not-an-ip", "::1", "127.0.0.1:notaport", "1.2.3.4:99999"] {
            assert!(
                Controller::new_full(
                    "c2",
                    false,
                    "controller",
                    "ptcp:%d",
                    None,
                    bad,
                    6653,
                    "tcp",
                    false,
                    BTreeMap::new()
                )
                .is_err(),
                "expected error for {bad:?}"
            );
        }
    }

    #[test]
    fn check_listening_errors_when_port_is_taken() {
        let (_listener, port) = listen_on_free_port();
        let err = Controller::new_full(
            "c0",
            false,
            "controller",
            "ptcp:%d",
            None,
            "127.0.0.1",
            port,
            "tcp",
            false,
            BTreeMap::new(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("shut down the controller"), "got {msg}");
        assert!(msg.contains(&port.to_string()), "got {msg}");
        // ... and passes on a free port.
        assert!(Controller::new_full(
            "c0",
            false,
            "controller",
            "ptcp:%d",
            None,
            "127.0.0.1",
            free_port(),
            "tcp",
            false,
            BTreeMap::new()
        )
        .is_ok());
    }

    #[test]
    fn controller_ip_prefers_intf_ip_when_intfs_exist() {
        use mn_link::Intf;
        let mut c = Controller::new("c0", "controller", free_port(), false, BTreeMap::new());
        c.params.insert("ip".to_string(), "10.0.0.9/8".to_string());
        assert_eq!(c.ip(), IpAddr::from([127, 0, 0, 1])); // ip field default
        c.add_intf(Intf::new("c0-eth0", "c0", Some(0)), Some(0))
            .unwrap();
        c.node_mut()
            .set_ip("10.0.0.9/8", 8, Some("c0-eth0"))
            .unwrap();
        // Python IP(): with intfs, Node.IP(intf) wins.
        assert_eq!(c.ip(), IpAddr::from([10, 0, 0, 9]));
        assert_eq!(c.ip_intf(Some("c0-eth0")), IpAddr::from([10, 0, 0, 9]));
    }

    #[test]
    fn controller_start_requires_command_on_path() {
        let mut c = Controller::new(
            "c0",
            "definitely-not-a-real-controller-xyz",
            free_port(),
            false,
            BTreeMap::new(),
        );
        // pathCheck: missing executable -> error before touching the shell.
        let err = c.start().unwrap_err();
        assert!(err
            .to_string()
            .contains("definitely-not-a-real-controller-xyz"));
    }

    #[test]
    fn ovs_controller_picks_available_binary() {
        let available = which("ovs-controller")
            .or_else(|| which("test-controller"))
            .or_else(|| which("ovs-testcontroller"));
        assert_eq!(
            OVSController::is_available(),
            available.map(|p| p.to_string_lossy().into_owned())
        );
        if let Some(cmd) = OVSController::is_available() {
            // Retry a few ephemeral ports: parallel tests may grab a port we
            // just released, which would trip the checkListening probe.
            let mut o = None;
            for _ in 0..8 {
                match OVSController::new_full("oc0", free_port(), BTreeMap::new()) {
                    Ok(v) => {
                        o = Some(v);
                        break;
                    }
                    Err(_) => continue,
                }
            }
            let o = o.expect("a free port should be found within 8 tries");
            assert_eq!(o.command, cmd);
            assert!(o.to_string().starts_with("<OVSController oc0: "));
        }
    }

    #[test]
    fn nox_stub_builds_python_command_and_cargs() {
        // Single test mutating NOX_CORE_DIR to avoid env races.
        std::env::remove_var("NOX_CORE_DIR");
        assert!(NOX::new("nox0", &[], BTreeMap::new()).is_err());
        let dir = std::env::temp_dir().join(format!("mn-nox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("NOX_CORE_DIR", &dir);
        // Default: packetdump only.
        let nox = NOX::new("nox0", &[], BTreeMap::new()).unwrap();
        assert_eq!(nox.nox_args, vec!["packetdump".to_string()]);
        assert_eq!(nox.command, format!("{}/nox_core", dir.display()));
        assert_eq!(
            nox.cargs,
            "--libdir=/usr/local/lib -v -i ptcp:%s packetdump"
        );
        assert_eq!(nox.cdir.as_deref(), Some(dir.to_str().unwrap()));
        assert!(nox.to_string().starts_with("<NOX nox0: "));
        // Explicit modules:
        let mut nox =
            NOX::new("nox1", &["a".to_string(), "b".to_string()], BTreeMap::new()).unwrap();
        assert_eq!(nox.cargs, "--libdir=/usr/local/lib -v -i ptcp:%s a b");
        // start() is the documented TODO stub.
        let err = nox.start().unwrap_err();
        assert!(err.to_string().contains("TODO"), "got {err}");
        nox.stop().unwrap();
        std::env::remove_var("NOX_CORE_DIR");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ryu_stub_builds_python_command_and_cargs() {
        let ryu = Ryu::new(
            "ryu0",
            &["ryu.app.simple_switch".to_string()],
            None,
            None,
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(ryu.command, "ryu run");
        assert_eq!(ryu.cargs, "ryu.app.simple_switch --ofp-tcp-listen-port %s");
        assert_eq!(ryu.ryu_args, "ryu.app.simple_switch");
        assert!(ryu.to_string().starts_with("<Ryu ryu0: "));
        // Multiple apps are joined (Python ' '.join(ryuArgs)); command and
        // cargs can be overridden.
        let mut ryu = Ryu::new(
            "ryu1",
            &["a".to_string(), "b".to_string()],
            Some("ryu-manager"),
            Some("custom %s"),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(ryu.command, "ryu-manager");
        assert_eq!(ryu.cargs, "custom %s");
        assert_eq!(ryu.ryu_args, "a b");
        let err = ryu.start().unwrap_err();
        assert!(err.to_string().contains("TODO"), "got {err}");
        ryu.stop().unwrap();
    }

    #[test]
    fn remote_controller_probes_and_noops() {
        // Explicit port that IS listening: adopted, probe recorded true.
        let (_listener, port) = listen_on_free_port();
        let mut rc =
            RemoteController::new("rc0", "127.0.0.1", Some(port), BTreeMap::new()).unwrap();
        assert_eq!(rc.port, port);
        assert_eq!(rc.probes, vec![(port, true)]);
        assert!(rc.is_listening(IpAddr::from([127, 0, 0, 1]), port));
        assert!(rc.to_string().starts_with("<RemoteController rc0: "));
        // start/stop are overridden no-ops:
        rc.start().unwrap();
        rc.stop().unwrap();

        // Explicit free port: kept, probe recorded false (Python warns).
        let free = free_port();
        let rc = RemoteController::new("rc1", "127.0.0.1", Some(free), BTreeMap::new()).unwrap();
        assert_eq!(rc.port, free);
        assert_eq!(rc.probes, vec![(free, false)]);

        // No port: probes 6653 then 6633, falls back to 6653 when neither
        // listens (both are free in this sandbox).
        let rc = RemoteController::new("rc2", "127.0.0.1", None, BTreeMap::new()).unwrap();
        assert_eq!(rc.probes.len(), 2);
        assert_eq!(rc.probes[0].0, 6653);
        assert_eq!(rc.probes[1].0, 6633);
        if !rc.probes[0].1 && !rc.probes[1].1 {
            assert_eq!(rc.port, 6653); // Python's fallback + warn
        }
    }

    #[test]
    fn default_controller_uses_first_available_class() {
        if which("controller").is_some() {
            // `controller` exists in this sandbox; 6653 is free, so the
            // probe in new_full passes.
            let c = default_controller("dc0", BTreeMap::new()).unwrap();
            assert_eq!(c.command, "controller");
            assert_eq!(c.port, DEFAULT_PORT);
        } else if OVSController::is_available().is_some() {
            let c = default_controller("dc0", BTreeMap::new()).unwrap();
            assert_ne!(c.command, "controller");
        } else {
            assert!(default_controller("dc0", BTreeMap::new()).is_err());
        }
    }
}
