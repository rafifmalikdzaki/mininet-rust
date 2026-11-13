//! The `Node` core — Rust port of `class Node` in `mininet/node.py`.
//!
//! A node is a shell in a (possibly named) network namespace. See the
//! crate docs for the lifecycle and the documented deviations from
//! CPython (pure `new` + explicit `start`, pipes instead of a pty, direct
//! spawn for root-ns nodes).

use std::collections::{BTreeMap, HashMap};
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use mn_link::Intf;
use mn_netlink::Netlink;
use mn_topo::Attrs;

use crate::error::{Error, Result};
use crate::mnexec::{attach_argv, mnexec_path, send_cmd_line, shell_argv};

/// How long [`Node::start`] waits for the shell's first prompt sentinel.
const SHELL_PROMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Node::start`] waits for the holder child's readiness byte
/// (proof that its `setns` into the named netns completed) before the
/// shell is attached to the holder pid.
const HOLDER_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Output of a one-shot command run inside the node's namespace
/// (Python `Node.pexec` returns `(out, err, exitcode)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: i32,
}

impl CmdOutput {
    /// stdout and stderr merged into one string — Python runs node commands
    /// on a pty, which merges the two streams, so `Node.cmd`'s return value
    /// contains both; this reproduces that for `pexec` results.
    pub fn merged(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}{}", self.stdout, self.stderr)
        }
    }
}

/// Run `f` with a fresh [`Netlink`] handle *inside the network namespace of
/// process `pid`* — the netlink-in-namespace equivalent of running a
/// command via `mnexec -a <pid>` / `setns(2)`.
///
/// Forks a child that `setns`es into `pid`'s netns, opens a netlink socket
/// there, runs `f`, and exits with 0/1. The parent waits and maps the exit
/// code to `Ok(())` / `Err`. See [`mn_netlink::run_in_netns`] for the
/// fork-safety details (pre-allocated path, pre-fork runtime, optional
/// single-threaded assert).
///
/// Requires root / `CAP_SYS_ADMIN` (like `mnexec -a`).
pub fn run_in_netns<F>(pid: u32, f: F) -> mn_netlink::Result<()>
where
    F: FnOnce(&Netlink) -> mn_netlink::Result<()>,
{
    mn_netlink::run_in_netns(pid, f)
}

/// Parse an `ip[/prefix]` string (Python `Intf.setIP`'s `'/' in ipstr`
/// split), falling back to `default_prefix` for a bare address.
fn parse_ip_prefix(ip: &str, default_prefix: u8) -> Result<(IpAddr, u8)> {
    let invalid = |msg: &str| Error::InvalidInput(format!("bad IP {ip:?}: {msg}"));
    match ip.split_once('/') {
        Some((addr, prefix)) => {
            let addr: IpAddr = addr.parse().map_err(|_| invalid("unparseable address"))?;
            let prefix: u8 = prefix.parse().map_err(|_| invalid("unparseable prefix"))?;
            let max = match addr {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            if prefix > max {
                return Err(invalid(&format!("prefix {prefix} > {max}")));
            }
            Ok((addr, prefix))
        }
        None => {
            let addr: IpAddr = ip.parse().map_err(|_| invalid("unparseable address"))?;
            Ok((addr, default_prefix))
        }
    }
}

/// Create a pipe(2) pair as `(read_end, write_end)` (thin wrapper over
/// [`mn_netlink::pipe_pair`]).
fn pipe_pair() -> std::io::Result<(File, File)> {
    mn_netlink::pipe_pair().map_err(|e| match e {
        mn_netlink::Error::Io(e) => e,
        other => std::io::Error::other(other.to_string()),
    })
}

/// SIGKILL a direct child and reap it (best-effort, used on error paths).
fn kill_and_reap(pid: i32) {
    // SAFETY: `pid` is a direct child we forked ourselves; we only signal
    // and reap our own descendant.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(pid), None);
}

/// Put `file` into non-blocking mode (used for the shell's output pipe so
/// [`Node::wait_output`] can enforce a timeout).
fn set_nonblocking(file: &File) -> std::io::Result<()> {
    // SAFETY: `fd` is a valid open descriptor owned by `file`; F_GETFL and
    // F_SETFL only read/modify the descriptor's own status flags.
    unsafe {
        let fd = file.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A virtual network node: a shell in a network namespace. Port of
/// `class Node` in `mininet/node.py`.
///
/// Construction is pure; all kernel/process side effects happen in
/// [`Node::start`].
#[derive(Debug)]
pub struct Node {
    /// Class-name tag for `Display` (`"Node"`, `"Host"`, `"Switch"`).
    kind: &'static str,
    /// Node name (also the named-netns name for `in_namespace` nodes).
    pub name: String,
    /// First port number (`Node.portBase`: 0 for hosts, 1 for switches).
    pub port_base: u32,
    /// Whether the node lives in its own network namespace.
    pub in_namespace: bool,
    /// Stashed configuration parameters (`Node.params` / `**params`).
    pub params: BTreeMap<String, String>,
    /// Port number -> interface (`Node.intfs`).
    intfs: BTreeMap<u32, Intf>,
    /// Interface name -> port number (`Node.nameToIntf`).
    name_to_port: HashMap<String, u32>,
    /// The persistent shell process (`Node.shell`).
    shell: Option<Child>,
    /// Shell stdin (write end of the command pipe).
    shell_in: Option<ChildStdin>,
    /// Shell merged stdout+stderr (read end of a pipe, non-blocking).
    shell_out: Option<File>,
    /// Leftover output bytes not yet terminated by the prompt sentinel.
    pending: String,
    /// Holder process living in the named netns (kept alive so
    /// `mnexec -a <pid>` has a pid to attach to).
    holder_pid: Option<u32>,
    /// Shell pid (`Node.pid`) — the pid commands attach to.
    pid: Option<u32>,
    /// Named netns created by `start` (deleted by `stop`).
    netns: Option<String>,
    /// Whether we created (and therefore own) the named netns.
    netns_created: bool,
    /// Whether a command is in flight (`Node.waiting`).
    waiting: bool,
    /// Last command sent (`Node.lastCmd`).
    last_cmd: Option<String>,
    /// Last pid reported via the `^A<pid>` marker (`Node.lastPid`).
    last_pid: Option<u32>,
    /// Whether the node execed into a single process (`Node.execed`).
    execed: bool,
    /// Set by `start()` (or `mark_started` for shell-less root-ns switches
    /// like LinuxBridge, which have no pid) — `started()` is pid OR this.
    started_flag: bool,
    /// Lazily opened netlink handle in the orchestrator's namespace.
    nl: Option<Netlink>,
}

impl Node {
    /// Port of `Node.__init__` minus the side effects: build the model.
    ///
    /// `params` corresponds to Python's `**params` (opts dict).
    pub fn new(name: &str, in_namespace: bool, params: BTreeMap<String, String>) -> Self {
        Self {
            kind: "Node",
            name: name.to_string(),
            port_base: 0,
            in_namespace,
            params,
            intfs: BTreeMap::new(),
            name_to_port: HashMap::new(),
            shell: None,
            shell_in: None,
            shell_out: None,
            pending: String::new(),
            holder_pid: None,
            pid: None,
            netns: None,
            netns_created: false,
            waiting: false,
            last_cmd: None,
            last_pid: None,
            execed: false,
            started_flag: false,
            nl: None,
        }
    }

    /// Build a node from topology node attributes (`Mininet.buildFromTopo`
    /// does `cls(name, **topo.nodeInfo(name))`).
    ///
    /// `inNamespace` and `name` attrs override the arguments, like
    /// `Node.__init__`'s `params.get('inNamespace', inNamespace)`.
    pub fn with_topo_params(name: &str, default_in_namespace: bool, attrs: &Attrs) -> Self {
        let params = attrs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let in_namespace = attrs
            .get("inNamespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(default_in_namespace);
        let name = attrs
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();
        Self::new(&name, in_namespace, params)
    }

    /// Override the class tag used by `Display` (subclasses set theirs).
    pub fn set_kind(&mut self, kind: &'static str) {
        self.kind = kind;
    }

    /// The class tag used by `Display` (`"Node"`, `"Switch"`, ... — Python's
    /// `__class__.__name__` in `__repr__`).
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The node's pid (the shell's), once started (`Node.pid`).
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The named netns for this node (its name), once created.
    pub fn netns(&self) -> Option<&str> {
        self.netns.as_deref()
    }

    /// Whether `start()` has run and `stop()` has not.
    pub fn started(&self) -> bool {
        self.pid.is_some() || self.started_flag
    }

    /// Mark the node started without a shell pid — used by shell-less
    /// root-namespace switches (LinuxBridge) whose `start()` creates a
    /// kernel bridge instead of spawning a shell.
    pub fn mark_started(&mut self) {
        self.started_flag = true;
    }

    /// Whether a command is currently in flight (`Node.waiting`).
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// The last command sent to the shell (`Node.lastCmd`).
    pub fn last_cmd(&self) -> Option<&str> {
        self.last_cmd.as_deref()
    }

    /// Lazily opened netlink handle (in the orchestrator's namespace).
    pub fn nl(&mut self) -> Result<&Netlink> {
        if self.nl.is_none() {
            self.nl = Some(Netlink::new()?);
        }
        Ok(self.nl.as_ref().expect("nl just initialized"))
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /// Full lifecycle start: create the named netns + holder (if
    /// `in_namespace`), then spawn the shell via `mnexec`.
    pub fn start(&mut self) -> Result<()> {
        if self.started() {
            return Err(Error::Other(format!(
                "{}: shell is already running",
                self.name
            )));
        }
        if self.in_namespace {
            // 1. named netns (ip netns add equivalent)
            mn_netlink::netns_add(&self.name)?;
            self.netns = Some(self.name.clone());
            self.netns_created = true;
            // 2. holder child living in the named netns (fork + setns +
            //    readiness handshake + sleep). On failure, terminate() runs
            //    the full cleanup (named netns) before we return the error.
            match self.spawn_holder() {
                Ok(pid) => self.holder_pid = Some(pid),
                Err(e) => {
                    let _ = self.terminate();
                    return Err(e);
                }
            }
        }
        if let Err(e) = self.start_shell() {
            // terminate() cleans up shell + holder + named netns even on
            // the error path (no leaks).
            let _ = self.terminate();
            return Err(e);
        }
        Ok(())
    }

    /// Fork a child that `setns`es into the named netns (by NAME, via
    /// `/var/run/netns/<name>`) and sleeps, so there is a pid to attach to
    /// with `mnexec -a <pid>` (the "fork a child that holds the ns"
    /// approach). Returns the holder's pid.
    ///
    /// Race safety: the parent blocks on a readiness handshake pipe until
    /// the child confirms (one byte, written after `setns` succeeded) that
    /// it is inside the named netns; only then is the pid returned and
    /// later used by `mnexec -a`. EOF before the byte means the child died
    /// (e.g. `setns` failed) and a timeout is an error; in both cases the
    /// child is killed and reaped.
    fn spawn_holder(&mut self) -> Result<u32> {
        let name = self.name.clone();
        // Pre-allocate everything the fork child needs (the netns_add_in
        // pattern): the bind-mount path as a CString and the readiness
        // pipe. After fork the child performs raw syscalls only.
        let path = mn_netlink::netns_path(&name)?;
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| Error::InvalidInput(format!("bad netns path for {name:?}")))?;
        let (rdy_read, rdy_write) = pipe_pair()?;
        mn_netlink::check_fork_safety().map_err(Error::Netlink)?;
        // SAFETY: the child branch calls only setns_path_raw (open+setns+
        // close on the pre-allocated CString), send_ready_byte/close (raw
        // write) and a pause loop, or libc::_exit on failure — no
        // allocation, no locks, no stdio, no runtime creation after fork.
        // The parent branch only polls/reads the pipe and waitpid/kill.
        let child = unsafe { nix::unistd::fork() }.map_err(mn_netlink::Error::Errno)?;
        match child {
            nix::unistd::ForkResult::Child => {
                drop(rdy_read);
                if mn_netlink::setns_path_raw(&c_path).is_ok() {
                    mn_netlink::send_ready_byte(rdy_write.as_raw_fd());
                    // SAFETY: close(2) is async-signal-safe.
                    unsafe { libc::close(rdy_write.as_raw_fd()) };
                    loop {
                        // SAFETY: raw syscall; returns only when a signal
                        // arrives, then we pause again.
                        unsafe { libc::pause() };
                    }
                }
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(1) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                drop(rdy_write);
                let ready = mn_netlink::wait_ready_byte(rdy_read.as_raw_fd(), HOLDER_READY_TIMEOUT);
                drop(rdy_read);
                match ready {
                    Ok(true) => Ok(child.as_raw() as u32),
                    Ok(false) => {
                        kill_and_reap(child.as_raw());
                        Err(Error::Other(format!(
                            "{name}: netns holder exited before entering netns (setns failed)"
                        )))
                    }
                    Err(e) => {
                        kill_and_reap(child.as_raw());
                        Err(Error::Netlink(e))
                    }
                }
            }
        }
    }

    /// Port of `Node.startShell`: spawn the persistent interactive shell
    /// via `mnexec` (attached to the holder pid for namespace nodes) and
    /// wait for its first prompt sentinel.
    pub fn start_shell(&mut self) -> Result<()> {
        if self.shell.is_some() {
            return Err(Error::Other(format!(
                "{}: shell is already running",
                self.name
            )));
        }
        let attach = if self.in_namespace {
            match self.holder_pid {
                Some(pid) => Some(pid),
                None => {
                    return Err(Error::Other(format!(
                        "{}: in_namespace node has no holder pid; call start() first",
                        self.name
                    )))
                }
            }
        } else {
            None
        };
        let argv = shell_argv(&mnexec_path(), &self.name, attach);
        let (read_end, write_end) = pipe_pair()?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::piped())
            // Merge stderr into stdout, like Python's pty does: bash prints
            // the PS1 prompt on stderr, and the sentinel must be visible on
            // the stream we read.
            .stdout(Stdio::from(write_end.try_clone()?))
            .stderr(Stdio::from(write_end));
        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            cmd: argv.join(" "),
            source,
        })?;
        set_nonblocking(&read_end)?;
        self.shell_in = child.stdin.take();
        self.shell_out = Some(read_end);
        self.shell = Some(child);
        self.pid = Some(self.shell.as_ref().expect("shell just set").id());
        self.started_flag = true;
        // Defense in depth against the holder/setns race: with the
        // readiness handshake in spawn_holder the shell can only attach to
        // a holder that is already inside the named netns, but verify it
        // anyway — if mnexec somehow won the race, the shell would be in
        // the ROOT namespace and every command would run there.
        if self.in_namespace {
            if let Err(e) = self.verify_shell_in_netns() {
                let _ = self.terminate();
                return Err(e);
            }
        }
        self.execed = false;
        self.last_cmd = None;
        self.last_pid = None;
        self.pending.clear();
        self.waiting = false;
        // Wait for the first prompt (Python startShell's read loop).
        self.wait_for_prompt(SHELL_PROMPT_TIMEOUT)?;
        // +m: disable job control notification; no HISTFILE.
        // (stty -echo is skipped: our shell runs on pipes, not a tty.)
        self.cmd("unset HISTFILE; set +m")?;
        Ok(())
    }

    /// Verify (best effort) that the shell process actually lives in the
    /// node's named netns: compare `readlink(/proc/<shell-pid>/ns/net)`
    /// with `readlink(/var/run/netns/<name>)`. If either readlink fails
    /// (e.g. /proc not mounted) the check is skipped; a mismatch is a hard
    /// error because it means the shell landed in the wrong (root)
    /// namespace.
    fn verify_shell_in_netns(&self) -> Result<()> {
        let (Some(pid), Some(name)) = (self.pid, self.netns.as_deref()) else {
            return Ok(());
        };
        let shell_ns = std::fs::read_link(format!("/proc/{pid}/ns/net"));
        let netns = mn_netlink::netns_path(name)
            .ok()
            .and_then(|p| std::fs::read_link(p).ok());
        match (shell_ns, netns) {
            (Ok(s), Some(n)) if s == n => Ok(()),
            (Ok(s), Some(n)) => Err(Error::Other(format!(
                "{}: shell pid {pid} is in netns {s:?}, not the node netns {n:?} — \
                 refusing to run commands in the wrong namespace",
                self.name
            ))),
            // Cannot verify (no /proc?): the readiness handshake in
            // spawn_holder already guarantees the holder — and therefore
            // the attached shell — is inside the named netns.
            _ => Ok(()),
        }
    }

    /// Read until the prompt sentinel appears, discarding what came before
    /// (Python `startShell`'s `while True: data = self.read(1024); if
    /// data[-1] == chr(127): break`).
    fn wait_for_prompt(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(chunk) = self.read_shell_chunk()? {
                if chunk.contains(crate::PROMPT_SENTINEL) {
                    self.pending.clear();
                    return Ok(());
                }
                self.pending.push_str(&chunk);
            }
            if Instant::now() >= deadline {
                return Err(Error::TimedOut {
                    cmd: "<shell prompt>".to_string(),
                    timeout,
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Port of `Node.stop`: optionally delete interfaces, then terminate.
    pub fn stop(&mut self, delete_intfs: bool) -> Result<()> {
        if delete_intfs {
            self.delete_intfs(true);
        }
        self.terminate()
    }

    /// Port of `Node.terminate`: kill the shell's process group, kill the
    /// netns holder, and delete the named netns we created.
    ///
    /// All cleanup steps always run — even when one of them fails — so the
    /// error path cannot leak the holder process or the named netns. If
    /// several steps fail, the first error is returned.
    pub fn terminate(&mut self) -> Result<()> {
        let mut first_err: Option<Error> = None;
        macro_rules! record {
            ($e:expr) => {
                if first_err.is_none() {
                    first_err = Some($e);
                }
            };
        }
        if let Some(mut shell) = self.shell.take() {
            let pid = shell.id();
            // Python: os.killpg(self.shell.pid, signal.SIGHUP). mnexec -d
            // made the shell a session leader, so pgid == pid.
            // SAFETY: `pid` is our own child and its process-group id;
            // we only signal our own descendant group.
            let rc = unsafe { libc::killpg(pid as libc::pid_t, libc::SIGHUP) };
            if rc != 0 {
                let errno = nix::errno::Errno::last();
                // ESRCH just means the group is already gone.
                if errno != nix::errno::Errno::ESRCH {
                    record!(Error::Other(format!(
                        "{}: killpg(SIGHUP, pgid {pid}) failed: {errno}",
                        self.name
                    )));
                }
            }
            // Give it a moment to exit, then SIGKILL as a fallback.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match shell.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Ok(None) => {
                        // SIGKILL fallback: kill the whole process GROUP,
                        // not just the leader — the shell may have
                        // grandchildren (e.g. `cmd &`) in the same group.
                        // SAFETY: same own-descendant group as above.
                        let krc = unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
                        if krc != 0 {
                            let errno = nix::errno::Errno::last();
                            if errno == nix::errno::Errno::ESRCH {
                                // Group gone; fall back to the leader alone.
                                let _ = shell.kill();
                            } else {
                                record!(Error::Other(format!(
                                    "{}: killpg(SIGKILL, pgid {pid}) failed: {errno}",
                                    self.name
                                )));
                            }
                        }
                        if let Err(e) = shell.wait() {
                            record!(Error::Io(e));
                        }
                        break;
                    }
                    Err(e) => {
                        record!(Error::Io(e));
                        break;
                    }
                }
            }
        }
        self.shell_in = None;
        self.shell_out = None;
        if let Some(holder) = self.holder_pid.take() {
            // SAFETY: `holder` is our own forked child; SIGKILL only it.
            let rc = unsafe { libc::kill(holder as libc::pid_t, libc::SIGKILL) };
            if rc != 0 {
                let errno = nix::errno::Errno::last();
                if errno != nix::errno::Errno::ESRCH {
                    record!(Error::Other(format!(
                        "{}: kill(holder {holder}, SIGKILL) failed: {errno}",
                        self.name
                    )));
                }
            }
            // Reap the holder so it does not linger as a zombie.
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(holder as i32), None);
        }
        if self.netns_created {
            if let Some(name) = self.netns.take() {
                match mn_netlink::netns_del(&name) {
                    Ok(()) => {}
                    // Already gone: nothing to do.
                    Err(mn_netlink::Error::Errno(nix::errno::Errno::ENOENT)) => {}
                    Err(e) => record!(Error::Netlink(e)),
                }
            }
            self.netns_created = false;
        }
        self.pid = None;
        self.started_flag = false;
        self.waiting = false;
        self.last_cmd = None;
        self.last_pid = None;
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    // ------------------------------------------------------------------
    // Command execution (cmd / sendCmd / waitOutput / popen / pexec)
    // ------------------------------------------------------------------

    /// Read one non-blocking chunk from the shell's output pipe.
    /// Returns `Ok(None)` on a would-block, `Ok(Some(""))`-like EOF is
    /// signalled by an empty chunk flag: we return `Ok(None)` only for
    /// WouldBlock and `Some(String)` otherwise (possibly empty on EOF).
    fn read_shell_chunk(&mut self) -> Result<Option<String>> {
        let Some(file) = self.shell_out.as_ref() else {
            return Err(Error::NotRunning(self.name.clone()));
        };
        let mut buf = [0u8; 4096];
        match (&*file).read(&mut buf) {
            Ok(0) => Ok(Some(String::new())), // EOF: shell exited
            Ok(n) => Ok(Some(String::from_utf8_lossy(&buf[..n]).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Port of `Node.sendCmd`: queue a command line on the shell without
    /// waiting for its output.
    pub fn send_cmd(&mut self, cmd: &str, print_pid: bool) -> Result<()> {
        let Some(stdin) = self.shell_in.as_mut() else {
            return Err(Error::NotRunning(self.name.clone()));
        };
        let line = send_cmd_line(cmd, print_pid);
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(Error::Io)?;
        self.last_cmd = Some(cmd.to_string());
        self.last_pid = None;
        self.waiting = true;
        Ok(())
    }

    /// Port of `Node.waitOutput` with an optional timeout: accumulate the
    /// shell's output until the prompt sentinel `chr(127)` appears, strip
    /// the sentinel, and return the output (including trailing newline).
    pub fn wait_output(&mut self, timeout: Option<Duration>) -> Result<String> {
        if self.shell_out.is_none() {
            return Err(Error::NotRunning(self.name.clone()));
        }
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut acc = std::mem::take(&mut self.pending);
        loop {
            if let Some(chunk) = self.read_shell_chunk()? {
                acc.push_str(&chunk);
                if acc.contains(crate::PROMPT_SENTINEL) {
                    self.waiting = false;
                    // Python monitor(): sentinel at end -> strip it;
                    // sentinel inside -> remove all occurrences.
                    return Ok(acc.replace(crate::PROMPT_SENTINEL, ""));
                }
                if chunk.is_empty() {
                    // EOF: shell exited; return what we have.
                    self.waiting = false;
                    return Ok(acc);
                }
            }
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    // Keep the partial output for a possible retry.
                    self.pending = acc;
                    return Err(Error::TimedOut {
                        cmd: self.last_cmd.clone().unwrap_or_default(),
                        timeout: timeout.expect("deadline implies Some"),
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Port of `Node.cmd`: send a command and wait for its output.
    pub fn cmd(&mut self, cmd: &str) -> Result<String> {
        self.send_cmd(cmd, false)?;
        self.wait_output(None)
    }

    /// Port of `Node.popen` for a command list: spawn `cmd` inside the
    /// node's namespace and return the `Child` (stdout/stderr piped).
    ///
    /// For `in_namespace` nodes this is `mnexec -da <pid> <cmd...>`; it
    /// requires a started node — an unstarted `in_namespace` node returns
    /// [`Error::NotRunning`] instead of silently spawning in the root
    /// namespace. Root-namespace nodes spawn `cmd` directly (see crate
    /// docs).
    pub fn popen(&mut self, cmd: &[String]) -> Result<Child> {
        let argv: Vec<String> = match (self.in_namespace, self.pid) {
            (true, Some(pid)) => attach_argv(&mnexec_path(), pid, cmd),
            (true, None) => return Err(Error::NotRunning(self.name.clone())),
            (false, _) => cmd.to_vec(),
        };
        if argv.is_empty() {
            return Err(Error::InvalidInput(
                "popen() requires a command".to_string(),
            ));
        }
        Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Spawn {
                cmd: argv.join(" "),
                source,
            })
    }

    /// Port of `Node.popen` for a string: `popen("cmd arg...")` splits on
    /// whitespace; with `shell=true` it runs via `$SHELL -c`.
    pub fn popen_str(&mut self, cmd: &str, shell: bool) -> Result<Child> {
        let argv: Vec<String> = if shell {
            let shell_bin = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
            vec![shell_bin, "-c".to_string(), cmd.to_string()]
        } else {
            cmd.split_whitespace().map(|s| s.to_string()).collect()
        };
        self.popen(&argv)
    }

    /// Port of `Node.pexec` for a command list: run it in the node's
    /// namespace, capture stdout/stderr, and return them with the exit
    /// code.
    pub fn pexec(&mut self, cmd: &[String]) -> Result<CmdOutput> {
        let child = self.popen(cmd)?;
        capture_output(child)
    }

    /// Port of `Node.pexec` for a string (split on whitespace).
    pub fn pexec_str(&mut self, cmd: &str) -> Result<CmdOutput> {
        let argv: Vec<String> = cmd.split_whitespace().map(|s| s.to_string()).collect();
        self.pexec(&argv)
    }

    // ------------------------------------------------------------------
    // Interface management
    // ------------------------------------------------------------------

    /// Port of `Node.newPort`: next free port number.
    pub fn new_port(&self) -> u32 {
        self.intfs
            .keys()
            .max()
            .map(|m| m + 1)
            .unwrap_or(self.port_base)
    }

    /// Port of `Node.addIntf`: register `intf` on this node (allocating a
    /// port if none is given) and, for a started namespace node, move the
    /// interface into the node's netns (`moveIntf`: `ip link set <intf>
    /// netns <pid>`, here via netlink). Returns the port number.
    pub fn add_intf(&mut self, intf: Intf, port: Option<u32>) -> Result<u32> {
        mn_netlink::validate_ifname(&intf.name)?;
        if self.name_to_port.contains_key(&intf.name) {
            return Err(Error::InvalidInput(format!(
                "{}: interface {} already added",
                self.name, intf.name
            )));
        }
        let port = port.unwrap_or_else(|| self.new_port());
        self.name_to_port.insert(intf.name.clone(), port);
        let mut intf = intf;
        intf.port = Some(port);
        self.intfs.insert(port, intf);
        if self.in_namespace && self.started() {
            let name = self.intfs[&port].name.clone();
            let pid = self.pid.expect("started() implies pid");
            self.nl()?.move_to_netns_pid(&name, pid)?;
        }
        Ok(port)
    }

    /// Port of `Node.delIntf`: forget an interface (does not delete it in
    /// the kernel — that is [`Node::delete_intfs`]).
    pub fn del_intf(&mut self, name: &str) -> Result<()> {
        match self.name_to_port.remove(name) {
            Some(port) => {
                self.intfs.remove(&port);
                Ok(())
            }
            None => Err(Error::InvalidInput(format!(
                "{}: no such interface {name:?}",
                self.name
            ))),
        }
    }

    /// Port of `Node.intf`: the interface with the given name, the default
    /// interface if `name` is `None`, or an error if unknown.
    pub fn intf(&self, name: Option<&str>) -> Result<&Intf> {
        match name {
            None => self
                .default_intf()
                .ok_or_else(|| Error::InvalidInput(format!("{}: has no interfaces", self.name))),
            Some(n) => self
                .name_to_port
                .get(n)
                .and_then(|p| self.intfs.get(p))
                .ok_or_else(|| {
                    Error::InvalidInput(format!("{}: no such interface {n:?}", self.name))
                }),
        }
    }

    /// Port of `Node.defaultIntf`: interface with the lowest port number.
    pub fn default_intf(&self) -> Option<&Intf> {
        self.intfs.values().next()
    }

    /// Port of `Node.intfList`: interfaces sorted by port number.
    pub fn intf_list(&self) -> Vec<&Intf> {
        self.intfs.values().collect()
    }

    /// Port of `Node.intfNames`: interface names sorted by port number.
    pub fn intf_names(&self) -> Vec<String> {
        self.intfs.values().map(|i| i.name.clone()).collect()
    }

    /// Port of `Node.deleteIntfs`: best-effort kernel deletion of our
    /// interfaces (`ip link del <intf>` via netlink). Interfaces that
    /// were moved into the node's namespace are already gone from ours.
    pub fn delete_intfs(&mut self, check_name: bool) {
        let names: Vec<String> = self
            .intfs
            .values()
            .filter(|i| !check_name || i.name.contains(&self.name))
            .map(|i| i.name.clone())
            .collect();
        for name in names {
            if let Ok(nl) = self.nl() {
                if nl.link_index(&name).ok().flatten().is_some() {
                    let _ = nl.link_delete(&name);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Interface configuration (setIP / setMAC / routes / config)
    // ------------------------------------------------------------------

    /// Run a netlink op in the node's namespace when started (fork + setns
    /// + netlink, i.e. "via setns"), else in the orchestrator's namespace.
    ///
    /// An in-namespace node that is not started is an error, never a silent fallback to the root namespace.
    fn with_ns_nl(&mut self, f: impl FnOnce(&Netlink) -> mn_netlink::Result<()>) -> Result<()> {
        match (self.in_namespace, self.pid) {
            (true, Some(pid)) => Ok(run_in_netns(pid, f)?),
            (true, None) => Err(Error::NotRunning(self.name.clone())),
            (false, _) => {
                let nl = self.nl()?;
                Ok(f(nl)?)
            }
        }
    }

    /// Port of `Node.setIP` / `Intf.setIP`: set `ip[/prefix]` on an
    /// interface (default: the default interface). Updates the model and,
    /// on a started node, applies `addr_add` + link up via netlink inside
    /// the node's namespace.
    pub fn set_ip(&mut self, ip: &str, default_prefix: u8, intf: Option<&str>) -> Result<String> {
        let (addr, prefix) = parse_ip_prefix(ip, default_prefix)?;
        let target = self.intf(intf)?.name.clone();
        {
            let port = self.name_to_port[&target];
            self.intfs
                .get_mut(&port)
                .expect("name_to_port in sync")
                .set_ip(addr, prefix);
        }
        if self.started() {
            let name = target.clone();
            self.with_ns_nl(move |nl| {
                nl.addr_add(&name, addr, prefix)?;
                nl.link_set_up(&name, true)?;
                Ok(())
            })?;
        }
        Ok(format!("{addr}/{prefix}"))
    }

    /// Port of `Node.setMAC` / `Intf.setMAC`: set the MAC on an interface
    /// (default: the default interface). Updates the model and, on a
    /// started node, applies `link_set_mac` via netlink inside the ns.
    pub fn set_mac(&mut self, mac: &str, intf: Option<&str>) -> Result<String> {
        let bytes = mn_netlink::parse_mac(mac)?;
        let target = self.intf(intf)?.name.clone();
        {
            let port = self.name_to_port[&target];
            self.intfs.get_mut(&port).expect("name_to_port in sync").mac = Some(bytes);
        }
        if self.started() {
            let name = target.clone();
            self.with_ns_nl(move |nl| nl.link_set_mac(&name, bytes))?;
        }
        Ok(mac.to_string())
    }

    /// `Node.IP()`: the IP of an interface (default: default interface).
    pub fn ip(&self, intf: Option<&str>) -> Option<IpAddr> {
        self.intf(intf).ok().and_then(|i| i.ip)
    }

    /// `Node.MAC()`: the MAC of an interface (default: default interface).
    pub fn mac(&self, intf: Option<&str>) -> Option<[u8; 6]> {
        self.intf(intf).ok().and_then(|i| i.mac)
    }

    /// `Node.intfIsUp()`: whether the interface is up (cached kernel
    /// state in the model).
    pub fn intf_is_up(&self, intf: Option<&str>) -> bool {
        self.intf(intf).map(|i| i.is_up()).unwrap_or(false)
    }

    /// Port of `Node.setDefaultRoute`: run `ip route del default; ip route
    /// add default <spec>` inside the node (`spec` is an intf name, or a
    /// full `"dev <intf> via <gw>"` string, like Python).
    pub fn set_default_route(&mut self, spec: &str) -> Result<String> {
        let params = if spec.contains(' ') {
            spec.to_string()
        } else {
            // Python: params = 'dev %s' % intf — intf may be a name or Intf.
            let dev = self
                .intf(Some(spec))
                .map(|i| i.name.clone())
                .unwrap_or_else(|_| spec.to_string());
            format!("dev {dev}")
        };
        let out = self.cmd(&format!(
            "ip route del default; ip route add default {params}"
        ))?;
        Ok(out)
    }

    /// Netlink form of a gateway default route (`ip route add default via
    /// <gw>`) applied inside the node's namespace, bound to the default
    /// interface. Requires a started node.
    pub fn set_default_route_via(&mut self, gateway: IpAddr) -> Result<()> {
        let name = self.intf(None)?.name.clone();
        self.with_ns_nl(move |nl| {
            let oif = nl.link_index(&name)?;
            nl.route_add_default(gateway, oif)
        })?;
        Ok(())
    }

    /// Port of `Node.setARP`: `arp -s <ip> <mac>` inside the node.
    pub fn set_arp(&mut self, ip: &str, mac: &str) -> Result<String> {
        self.cmd(&format!("arp -s {ip} {mac}"))
    }

    /// Port of `Node.setHostRoute`: `route add -host <ip> dev <intf>`.
    pub fn set_host_route(&mut self, ip: &str, intf: &str) -> Result<String> {
        self.cmd(&format!("route add -host {ip} dev {intf}"))
    }

    /// Port of `Node.config`: apply mac/ip/defaultRoute to the default
    /// interface and bring `lo` up/down. Returns the per-parameter results
    /// (`Node.setParam`'s dict). On a not-yet-started node the interface
    /// model is still updated (kernel ops are skipped, like Python's
    /// warn-and-None on a dead shell), and the `lo` command is skipped.
    pub fn config(
        &mut self,
        mac: Option<&str>,
        ip: Option<&str>,
        default_route: Option<&str>,
        lo: &str,
    ) -> Result<BTreeMap<String, String>> {
        let mut r = BTreeMap::new();
        if let Some(mac) = mac {
            r.insert("mac".to_string(), self.set_mac(mac, None)?);
        }
        if let Some(ip) = ip {
            r.insert("ip".to_string(), self.set_ip(ip, 8, None)?);
        }
        if let Some(dr) = default_route {
            match self.set_default_route(dr) {
                Ok(out) => {
                    r.insert("defaultRoute".to_string(), out);
                }
                Err(Error::NotRunning(_)) => {} // Python: cmd() warns, returns None
                Err(e) => return Err(e),
            }
        }
        match self.cmd(&format!("ifconfig lo {lo}")) {
            Ok(out) => {
                r.insert("lo".to_string(), out);
            }
            Err(Error::NotRunning(_)) => {}
            Err(e) => return Err(e),
        }
        Ok(r)
    }

    /// Port of `Node.configDefault`: `config(**self.params)`.
    pub fn config_default(&mut self) -> Result<BTreeMap<String, String>> {
        let mac = self.params.get("mac").cloned();
        let ip = self.params.get("ip").cloned();
        let default_route = self.params.get("defaultRoute").cloned();
        let lo = self
            .params
            .get("lo")
            .cloned()
            .unwrap_or_else(|| "up".to_string());
        self.config(mac.as_deref(), ip.as_deref(), default_route.as_deref(), &lo)
    }
}

/// Wait for `child` and capture its stdout/stderr **concurrently**.
///
/// `Child::wait_with_output` polls both pipes (dropping stdin first), so
/// a child that floods one stream past the 64 KiB pipe buffer while the
/// other stream is still open cannot deadlock — the old read-stdout-to-EOF-
/// then-stderr order could.
fn capture_output(child: Child) -> Result<CmdOutput> {
    let out = child.wait_with_output()?;
    Ok(CmdOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code().unwrap_or(-1),
    })
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Python __repr__: '<%s %s: %s pid=%s> ' with '%s:%s' % (name, IP)
        let intfs = self
            .intf_list()
            .iter()
            .map(|i| {
                let ip = match i.ip {
                    Some(addr) => addr.to_string(),
                    None => "None".to_string(),
                };
                format!("{}:{}", i.name, ip)
            })
            .collect::<Vec<_>>()
            .join(",");
        let pid = match self.pid {
            Some(p) => p.to_string(),
            None => "None".to_string(),
        };
        write!(f, "<{} {}: {} pid={}> ", self.kind, self.name, intfs, pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mnexec::mnexec_available;
    use crate::root_tests_enabled;

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn node_model_and_port_allocation() {
        let mut n = Node::new("h1", true, params(&[("ip", "10.0.0.1/8")]));
        assert_eq!(n.name, "h1");
        assert_eq!(n.port_base, 0);
        assert!(n.in_namespace);
        assert_eq!(n.params.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert!(!n.started());
        assert_eq!(n.new_port(), 0);
        let p0 = n.add_intf(Intf::new("h1-eth0", "h1", None), None).unwrap();
        let p1 = n.add_intf(Intf::new("h1-eth1", "h1", None), None).unwrap();
        assert_eq!((p0, p1), (0, 1));
        assert_eq!(n.new_port(), 2);
        assert_eq!(n.intf_names(), vec!["h1-eth0", "h1-eth1"]);
        assert_eq!(n.default_intf().unwrap().name, "h1-eth0");
        assert_eq!(n.intf(Some("h1-eth1")).unwrap().port, Some(1));
        assert_eq!(n.intf(None).unwrap().name, "h1-eth0");
        n.del_intf("h1-eth0").unwrap();
        assert_eq!(n.intf_names(), vec!["h1-eth1"]);
        assert!(n.del_intf("h1-eth0").is_err());
        assert!(n.intf(Some("nope")).is_err());
    }

    #[test]
    fn add_intf_rejects_bad_and_duplicate_names() {
        let mut n = Node::new("h1", false, BTreeMap::new());
        assert!(n.add_intf(Intf::new("a/b", "h1", None), None).is_err());
        assert!(n
            .add_intf(Intf::new("way-too-long-ifname0", "h1", None), None)
            .is_err());
        n.add_intf(Intf::new("h1-eth0", "h1", None), None).unwrap();
        assert!(n.add_intf(Intf::new("h1-eth0", "h1", None), None).is_err());
    }

    #[test]
    fn node_display_matches_python_repr_shape() {
        let mut n = Node::new("h1", false, BTreeMap::new());
        n.add_intf(Intf::new("h1-eth0", "h1", Some(0)), None)
            .unwrap();
        assert_eq!(n.to_string(), "<Node h1: h1-eth0:None pid=None> ");
        n.set_ip("10.0.0.1/8", 8, None).unwrap(); // model-only: not started
        assert_eq!(n.to_string(), "<Node h1: h1-eth0:10.0.0.1 pid=None> ");
        assert_eq!(n.ip(None), Some(IpAddr::from([10, 0, 0, 1])));
    }

    #[test]
    fn with_topo_params_reads_attrs_like_python() {
        let mut attrs = Attrs::new();
        attrs.insert("ip", "10.0.0.2/8");
        attrs.insert("inNamespace", false);
        attrs.insert("custom", 42i64);
        let n = Node::with_topo_params("h9", true, &attrs);
        assert_eq!(n.name, "h9");
        assert!(!n.in_namespace);
        assert_eq!(n.params.get("ip").map(String::as_str), Some("10.0.0.2/8"));
        assert_eq!(n.params.get("custom").map(String::as_str), Some("42"));
    }

    #[test]
    fn parse_ip_prefix_cases() {
        assert_eq!(
            parse_ip_prefix("10.0.0.1/8", 24).unwrap(),
            (IpAddr::from([10, 0, 0, 1]), 8)
        );
        assert_eq!(
            parse_ip_prefix("10.0.0.1", 24).unwrap(),
            (IpAddr::from([10, 0, 0, 1]), 24)
        );
        assert_eq!(
            parse_ip_prefix("fd00::1/64", 8).unwrap(),
            ("fd00::1".parse().unwrap(), 64)
        );
        assert!(parse_ip_prefix("10.0.0.1/33", 8).is_err());
        assert!(parse_ip_prefix("10.0.0.1/x", 8).is_err());
        assert!(parse_ip_prefix("999.0.0.1", 8).is_err());
        assert!(parse_ip_prefix("fd00::1/129", 8).is_err());
    }

    #[test]
    fn set_ip_set_mac_update_model_on_unstarted_node() {
        let mut n = Node::new("h1", false, BTreeMap::new());
        // No interfaces: error, like Python's AttributeError on None.setIP.
        assert!(n.set_ip("10.0.0.1", 8, None).is_err());
        n.add_intf(Intf::new("h1-eth0", "h1", Some(0)), Some(0))
            .unwrap();
        let r = n.set_ip("10.0.0.1/8", 8, None).unwrap();
        assert_eq!(r, "10.0.0.1/8");
        assert_eq!(n.intf(None).unwrap().prefix_len, Some(8));
        let r = n.set_mac("00:11:22:33:44:55", None).unwrap();
        assert_eq!(r, "00:11:22:33:44:55");
        assert_eq!(n.mac(None), Some([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
        assert!(n.set_mac("not-a-mac", None).is_err());
    }

    #[test]
    fn config_default_populates_results_and_model() {
        let mut n = Node::new(
            "h1",
            false,
            params(&[("ip", "10.0.0.1/8"), ("mac", "00:00:00:00:00:01")]),
        );
        n.add_intf(Intf::new("h1-eth0", "h1", Some(0)), Some(0))
            .unwrap();
        let r = n.config_default().unwrap();
        assert_eq!(r.get("ip").map(String::as_str), Some("10.0.0.1/8"));
        assert_eq!(r.get("mac").map(String::as_str), Some("00:00:00:00:00:01"));
        // lo command skipped on unstarted node (Python warns and returns None)
        assert!(!r.contains_key("lo"));
        assert_eq!(n.ip(None), Some(IpAddr::from([10, 0, 0, 1])));
    }

    #[test]
    fn cmd_on_unstarted_node_is_not_running() {
        let mut n = Node::new("h1", false, BTreeMap::new());
        let err = n.cmd("echo hi").unwrap_err();
        assert!(matches!(err, Error::NotRunning(_)));
        assert!(n.wait_output(None).is_err());
        assert!(n.send_cmd("echo hi", false).is_err());
        // stop on a never-started node is a no-op, like Python.
        n.stop(false).unwrap();
        n.stop(true).unwrap();
    }

    #[test]
    fn popen_rejects_empty_command() {
        let mut n = Node::new("h1", false, BTreeMap::new());
        assert!(n.popen(&[]).is_err());
    }

    #[test]
    fn popen_pexec_on_unstarted_in_namespace_node_is_not_running() {
        // An in_namespace node that was never started has no pid to attach
        // to: popen/pexec must fail with NotRunning instead of silently
        // spawning the command in the ROOT namespace.
        let mut n = Node::new("h1", true, BTreeMap::new());
        let cmd = vec!["echo".to_string(), "pwned".to_string()];
        let err = n.popen(&cmd).unwrap_err();
        assert!(
            matches!(err, Error::NotRunning(ref name) if name == "h1"),
            "got {err:?}"
        );
        let err = n.pexec(&cmd).unwrap_err();
        assert!(matches!(err, Error::NotRunning(_)), "got {err:?}");
        let err = n.pexec_str("echo pwned").unwrap_err();
        assert!(matches!(err, Error::NotRunning(_)), "got {err:?}");
        // Root-ns nodes spawn directly even when unstarted (documented
        // deviation), so this must NOT be NotRunning — it either runs or
        // fails with Spawn/NotFound when the binary is not on PATH.
        let mut r = Node::new("r1", false, BTreeMap::new());
        match r.pexec_str("echo root-ns-ok") {
            Ok(out) => {
                assert_eq!(out.exit_code, 0);
                assert_eq!(out.stdout, "root-ns-ok\n");
            }
            Err(Error::Spawn { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("expected a direct spawn attempt, got {e:?}"),
        }
    }

    #[test]
    fn capture_output_does_not_deadlock_on_stderr_flood_over_64kib() {
        // Regression test for the pexec pipe deadlock: reading stdout to
        // EOF before touching stderr deadlocks once stderr exceeds the
        // 64 KiB pipe buffer. wait_with_output drains both concurrently.
        let script = "i=0; while [ $i -lt 20000 ]; do echo 'stderr-line-0123456789abcdef' 1>&2; i=$((i+1)); done; echo stdout-done";
        let child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let out = capture_output(child).expect("capture must not deadlock");
        assert_eq!(out.exit_code, 0);
        assert!(
            out.stdout.contains("stdout-done"),
            "stdout: {:?}",
            out.stdout
        );
        assert!(
            out.stderr.len() > 64 * 1024,
            "expected >64KiB of stderr captured, got {} bytes",
            out.stderr.len()
        );
        assert!(out.stderr.contains("stderr-line-0123456789abcdef"));
    }

    #[test]
    fn capture_output_reports_exit_code_and_both_streams() {
        let child = Command::new("sh")
            .arg("-c")
            .arg("echo out; echo err 1>&2; exit 3")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = capture_output(child).unwrap();
        assert_eq!(out.exit_code, 3);
        assert_eq!(out.stdout, "out\n");
        assert_eq!(out.stderr, "err\n");
    }

    // ------------------------------------------------------------------
    // Non-root live test: root-namespace node with a real mnexec shell.
    // Runs in this sandbox (mnexec -cd needs no privileges); skipped if
    // no mnexec binary is available.
    // ------------------------------------------------------------------

    #[test]
    fn root_ns_node_shell_cmd_and_pexec() {
        let mnexec = mnexec_path();
        if !mnexec_available(&mnexec) {
            eprintln!(
                "skipping: no mnexec binary at {} (not on PATH / not built)",
                mnexec.display()
            );
            return;
        }
        let mut n = Node::new("mntest0", false, BTreeMap::new());
        if let Err(e) = n.start() {
            // Clean machine without mnexec on PATH: skip instead of failing.
            if matches!(&e, Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
            {
                eprintln!("skipping: mnexec spawn returned NotFound: {e}");
                return;
            }
            panic!("start root-ns node: {e}");
        }
        assert!(n.started());
        assert!(n.pid().is_some());
        assert_eq!(n.netns(), None);

        let out = n.cmd("echo hello-mn-node").expect("cmd");
        assert!(out.contains("hello-mn-node"), "output was {out:?}");

        let out = n.cmd("printf 'a\\nb\\n'").expect("cmd");
        assert!(out.contains("a\nb"), "output was {out:?}");

        // send_cmd + wait_output with a timeout that elapses.
        n.send_cmd("sleep 2", false).unwrap();
        assert!(n.is_waiting());
        let err = n.wait_output(Some(Duration::from_millis(150))).unwrap_err();
        assert!(matches!(err, Error::TimedOut { .. }));
        // ... and one that succeeds after the sleep finishes.
        let out = n.wait_output(Some(Duration::from_secs(10))).unwrap();
        assert!(!n.is_waiting());
        let _ = out;

        // pexec: exit code + captured streams (direct spawn for root-ns).
        let res = n
            .pexec(&[
                "sh".to_string(),
                "-c".to_string(),
                "echo out; echo err 1>&2; exit 3".to_string(),
            ])
            .expect("pexec");
        assert_eq!(res.exit_code, 3);
        assert!(res.stdout.contains("out"));
        assert!(res.stderr.contains("err"));

        // pexec with a >64KiB stderr flood must not deadlock (pipe buffer
        // is 64KiB; the old sequential read order deadlocked here).
        let flood = "i=0; while [ $i -lt 20000 ]; do echo flood-line-0123456789 1>&2; i=$((i+1)); done; echo flood-done";
        let res = n
            .pexec(&["sh".to_string(), "-c".to_string(), flood.to_string()])
            .expect("pexec stderr flood");
        assert_eq!(res.exit_code, 0);
        assert!(res.stdout.contains("flood-done"));
        assert!(
            res.stderr.len() > 64 * 1024,
            "expected >64KiB stderr, got {}",
            res.stderr.len()
        );

        let res = n.pexec_str("echo hi-there").unwrap();
        assert_eq!(res.exit_code, 0);
        assert_eq!(res.stdout, "hi-there\n");

        n.stop(false).expect("stop");
        assert!(!n.started());
        assert!(n.pid().is_none());
    }

    // ------------------------------------------------------------------
    // Root-gated integration test: full namespace lifecycle.
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1"]
    fn namespaced_node_lifecycle_cmd_and_intf_config() {
        if !root_tests_enabled() {
            return;
        }
        use std::net::Ipv4Addr;

        let name = format!("mnnd{}", std::process::id() % 100000);
        let _ = mn_netlink::netns_del(&name);
        let mut n = Node::new(&name, true, BTreeMap::new());
        n.start().expect("start namespaced node as root");
        assert!(n.started());
        assert!(mn_netlink::netns_exists(&name));
        assert_eq!(n.netns(), Some(name.as_str()));

        // cmd through the in-namespace shell (mnexec -cda <holder>).
        let out = n.cmd("echo in-ns-$$").expect("cmd in ns");
        assert!(out.contains("in-ns-"), "output was {out:?}");

        // pexec one-shot via mnexec -da <shell pid>: exit code + output.
        let res = n
            .pexec(&[
                "sh".to_string(),
                "-c".to_string(),
                "echo one-shot; exit 7".to_string(),
            ])
            .expect("pexec in ns");
        assert_eq!(res.exit_code, 7);
        assert!(res.stdout.contains("one-shot"));

        // Attach an interface: veth pair created locally, end moved into
        // the node's ns by add_intf (moveIntf via netlink).
        let peer = format!("mnp{}", std::process::id() % 100000);
        {
            let nl = n.nl().unwrap();
            nl.veth_create(&format!("{name}-eth0"), &peer, None, None)
                .expect("veth_create as root");
        }
        n.add_intf(Intf::new(format!("{name}-eth0"), &name, Some(0)), Some(0))
            .expect("add_intf moves end into ns");
        {
            let nl = n.nl().unwrap();
            assert!(nl.link_index(&format!("{name}-eth0")).unwrap().is_none());
        }

        // set_ip applies addr_add + link up *inside* the node ns (setns).
        n.set_ip("10.123.0.1/24", 24, None).expect("set_ip in ns");
        run_in_netns(n.pid().unwrap(), |nl| {
            let addrs = nl.addr_list(Some(&format!("{name}-eth0")))?;
            assert!(
                addrs
                    .iter()
                    .any(|a| a.address == IpAddr::V4(Ipv4Addr::new(10, 123, 0, 1))
                        && a.prefix_len == 24),
                "addr missing in ns: {addrs:?}"
            );
            let link = nl.link_get(&format!("{name}-eth0"))?.expect("intf in ns");
            assert!(link.is_up);
            Ok(())
        })
        .expect("verify inside ns");

        // set_mac inside the ns, too (model updated; kernel op via setns).
        n.set_mac("00:00:00:00:12:34", None).expect("set_mac in ns");
        assert_eq!(n.mac(None), Some([0, 0, 0, 0, 0x12, 0x34]));

        // default route via gateway inside the ns (netlink form).
        n.set_default_route_via(IpAddr::V4(Ipv4Addr::new(10, 123, 0, 254)))
            .expect("default route in ns");
        run_in_netns(n.pid().unwrap(), |nl| {
            let routes = nl.route_list()?;
            assert!(
                routes.iter().any(|r| r.destination_prefix_length == 0
                    && r.gateway == Some(IpAddr::V4(Ipv4Addr::new(10, 123, 0, 254)))),
                "default route missing in ns: {routes:?}"
            );
            Ok(())
        })
        .expect("verify route inside ns");

        // stop: shell + holder killed, named netns deleted, peer cleaned.
        n.stop(false).expect("stop");
        assert!(!mn_netlink::netns_exists(&name));
        assert!(!n.started());
        let nl = Netlink::new().unwrap();
        let _ = nl.link_delete(&peer);
    }
}
