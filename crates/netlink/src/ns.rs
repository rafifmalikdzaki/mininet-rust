//! Named network namespaces — the Rust equivalent of `ip netns add/del/list`
//! and `ip netns exec` (via `setns(2)`).
//!
//! `ip netns add <name>` works by creating an empty file under
//! `/var/run/netns/<name>`, then `unshare(CLONE_NEWNET)` in a short-lived
//! child and bind-mounting the child's `/proc/<pid>/ns/net` (i.e.
//! `/proc/self/ns/net` inside the child) onto that file. The bind mount
//! keeps the namespace alive after the child exits. We mirror that
//! exactly, using `nix`/`libc` syscalls instead of shelling out.

use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::mount::{umount2, MntFlags};

use crate::{Error, Netlink, Result};

/// Default directory for named netns bind mounts (same as iproute2).
pub const NETNS_PATH: &str = "/var/run/netns";

/// Validate a named-netns name the way iproute2 does: non-empty, at most 255
/// bytes, and only `[A-Za-z0-9._-]` (in particular no `/`).
pub fn validate_netns_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 255 {
        return Err(Error::InvalidInput(format!(
            "netns name must be 1..=255 bytes: {name:?}"
        )));
    }
    // "." and ".." would escape the netns directory.
    if name == "." || name == ".." {
        return Err(Error::InvalidInput(format!(
            "netns name may not be {name:?}"
        )));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidInput(format!(
            "netns name may only contain [A-Za-z0-9._-]: {name:?}"
        )));
    }
    Ok(())
}

/// Build and validate the bind-mount path for `name` under `dir`.
pub fn netns_path_in(dir: &Path, name: &str) -> Result<PathBuf> {
    validate_netns_name(name)?;
    Ok(dir.join(name))
}

/// Path of named netns `name` under the default [`NETNS_PATH`].
pub fn netns_path(name: &str) -> Result<PathBuf> {
    netns_path_in(Path::new(NETNS_PATH), name)
}

/// Does a named netns `name` exist under [`NETNS_PATH`]?
pub fn netns_exists(name: &str) -> bool {
    netns_path(name).is_ok_and(|p| p.exists())
}

/// Create a new named network namespace (equivalent of `ip netns add`).
///
/// Requires root / `CAP_SYS_ADMIN`. The namespace persists via the bind
/// mount on `/var/run/netns/<name>` after the helper child exits.
pub fn netns_add(name: &str) -> Result<()> {
    netns_add_in(Path::new(NETNS_PATH), name)
}

/// [`netns_add`] rooted at a custom directory (used by tests).
pub fn netns_add_in(dir: &Path, name: &str) -> Result<()> {
    let path = netns_path_in(dir, name)?;
    fs::create_dir_all(dir)?;
    // Create the (empty) mount point file. Fails if it already exists,
    // mirroring `ip netns add` on an existing name.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Other(format!("netns {name:?} already exists"))
            } else {
                Error::Io(e)
            }
        })?;

    // Pre-allocate everything the child needs so the child only performs
    // raw syscalls after fork(2).
    let src = CString::new("/proc/self/ns/net")
        .map_err(|_| Error::InvalidInput("bad ns source path".to_string()))?;
    let tgt = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("bad netns path".to_string()))?;
    let null = CString::new("").unwrap();

    // SAFETY: the child branch below calls only raw libc syscalls
    // (unshare/mount/_exit) on CStrings allocated before the fork, plus
    // libc errno reads. No allocation, no locks, no stdio — async-signal-
    // safe. The parent branch only calls waitpid. The process is
    // effectively single-threaded at this point (this crate never spawns
    // OS threads; the tokio runtime inside Netlink is current-thread and
    // idle while we fork).
    let child = unsafe { nix::unistd::fork()? };
    match child {
        nix::unistd::ForkResult::Child => {
            // SAFETY: raw syscalls only, on pre-allocated CStrings; see above.
            unsafe {
                if libc::unshare(libc::CLONE_NEWNET) != 0 {
                    libc::_exit(1);
                }
                let rc = libc::mount(
                    src.as_ptr(),
                    tgt.as_ptr(),
                    null.as_ptr(),
                    libc::MS_BIND,
                    std::ptr::null(),
                );
                if rc != 0 {
                    libc::_exit(2);
                }
                libc::_exit(0);
            }
        }
        nix::unistd::ForkResult::Parent { child: pid } => {
            use nix::sys::wait::{waitpid, WaitStatus};
            match waitpid(pid, None)? {
                WaitStatus::Exited(_, 0) => Ok(()),
                WaitStatus::Exited(_, code) => Err(Error::Other(format!(
                    "netns_add({name:?}): helper child failed with exit code {code}"
                ))),
                other => Err(Error::Other(format!(
                    "netns_add({name:?}): helper child ended unexpectedly: {other:?}"
                ))),
            }
        }
    }
}

/// Delete a named network namespace (equivalent of `ip netns del`):
/// lazy-unmount the bind mount, then remove the file.
///
/// Requires root / `CAP_SYS_ADMIN`.
pub fn netns_del(name: &str) -> Result<()> {
    netns_del_in(Path::new(NETNS_PATH), name)
}

/// [`netns_del`] rooted at a custom directory (used by tests).
pub fn netns_del_in(dir: &Path, name: &str) -> Result<()> {
    let path = netns_path_in(dir, name)?;
    if !path.exists() {
        return Err(Error::Errno(nix::errno::Errno::ENOENT));
    }
    // Detach the bind mount. EINVAL means "not a mount point" (plain file);
    // iproute2 tolerates that case too, so we only propagate real errors.
    match umount2(&path, MntFlags::MNT_DETACH) {
        Ok(()) | Err(nix::errno::Errno::EINVAL) => {}
        Err(e) => return Err(Error::Errno(e)),
    }
    fs::remove_file(&path)?;
    Ok(())
}

/// List named netns under a custom directory. A missing directory yields an
/// empty list (like `ip netns list` on a fresh host).
pub fn netns_list_in(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// List named netns under [`NETNS_PATH`] (equivalent of `ip netns list`).
pub fn netns_list() -> Result<Vec<String>> {
    netns_list_in(Path::new(NETNS_PATH))
}

/// Enter the network namespace at `path` via `setns(2)` **without any
/// allocation** — safe to call in a fork(2) child on a `CStr` that was
/// pre-allocated before the fork (the `netns_add_in` pattern).
///
/// This changes the **calling process's** namespace. Requires root /
/// `CAP_SYS_ADMIN` in the target namespace's user namespace.
pub fn setns_path_raw(path: &CStr) -> Result<()> {
    // SAFETY: `path` is a valid NUL-terminated C string owned by the
    // caller; open(2)/setns(2)/close(2) are raw syscalls and perform no
    // heap allocation.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(Error::Errno(nix::errno::Errno::last()));
    }
    // nstype 0 = any namespace type, like the C mnexec which calls
    // setns(fd, 0).
    let rc = unsafe { libc::setns(fd, 0) };
    let errno = if rc != 0 {
        Some(nix::errno::Errno::last())
    } else {
        None
    };
    // SAFETY: `fd` is a valid open descriptor.
    unsafe { libc::close(fd) };
    match rc {
        0 => Ok(()),
        _ => Err(Error::Errno(errno.expect("rc != 0 implies errno set"))),
    }
}

/// Enter the network namespace of process `pid` via `setns(2)`
/// (the syscall behind `ip netns exec` / `mnexec -a <pid>`).
///
/// This changes the **calling process's** namespace. Requires root /
/// `CAP_SYS_ADMIN` in the target namespace's user namespace.
pub fn setns_pid(pid: u32) -> Result<()> {
    let path = format!("/proc/{pid}/ns/net");
    let c_path = CString::new(path)
        .map_err(|_| Error::InvalidInput(format!("bad ns path for pid {pid}")))?;
    setns_path_raw(&c_path)
}

/// Enter the named netns `name` under [`NETNS_PATH`] via `setns(2)`.
///
/// This changes the **calling process's** namespace. Requires root /
/// `CAP_SYS_ADMIN`.
pub fn setns_name(name: &str) -> Result<()> {
    let path = netns_path(name)?;
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput(format!("bad netns path for {name:?}")))?;
    setns_path_raw(&c_path)
}

// ----------------------------------------------------------------------
// Fork-safety helpers
// ----------------------------------------------------------------------

/// Create a pipe(2) pair as `(read_end, write_end)`, both `CLOEXEC`-free
/// owned [`File`]s. Used for parent/child readiness handshakes around
/// fork(2) (and for the node shell's output pipe in `mn-node`).
pub fn pipe_pair() -> Result<(File, File)> {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` is a valid writable 2-element array; on success pipe(2)
    // stores two fresh, owned descriptors in it.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(Error::Errno(nix::errno::Errno::last()));
    }
    // SAFETY: fds[0] and fds[1] are owned descriptors returned by pipe(2);
    // from_raw_fd takes ownership, so nothing is double-closed.
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

/// Child side of a readiness handshake: write one byte to the pipe's
/// write end. Raw `write(2)` only — async-signal-safe, no allocation,
/// safe to call in a fork(2) child.
pub fn send_ready_byte(fd: RawFd) {
    let byte = [1u8];
    // SAFETY: `write(2)` is async-signal-safe; `fd` is the valid write end
    // of a pipe created before the fork. The return value is intentionally
    // ignored: if the parent died, the child is about to be killed anyway.
    unsafe {
        libc::write(fd, byte.as_ptr().cast::<libc::c_void>(), 1);
    }
}

/// Parent side of a readiness handshake: block until the forked child
/// writes its readiness byte to the pipe's read end.
///
/// Returns `Ok(true)` when the byte arrived, `Ok(false)` on EOF (the
/// child exited before signalling, e.g. its `setns` failed), and `Err`
/// on timeout or poll/read failure. Uses `poll(2)` so the wait is
/// bounded by `timeout`.
pub fn wait_ready_byte(fd: RawFd, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Other(format!(
                "timed out after {timeout:?} waiting for child readiness byte"
            )));
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a valid pollfd; poll(2) with a millisecond
        // timeout performs no allocation.
        let rc = unsafe {
            libc::poll(
                &mut pfd,
                1,
                remaining.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if rc < 0 {
            let e = nix::errno::Errno::last();
            if e == nix::errno::Errno::EINTR {
                continue;
            }
            return Err(Error::Errno(e));
        }
        if rc == 0 {
            return Err(Error::Other(format!(
                "timed out after {timeout:?} waiting for child readiness byte"
            )));
        }
        if pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            let mut byte = [0u8; 1];
            // SAFETY: `byte` is a valid 1-byte buffer; read(2) performs no
            // allocation.
            let n = unsafe { libc::read(fd, byte.as_mut_ptr().cast::<libc::c_void>(), 1) };
            if n == 1 {
                return Ok(true);
            }
            if n == 0 {
                // EOF: the child closed its end without signalling (it
                // exited, e.g. setns failed).
                return Ok(false);
            }
            let e = nix::errno::Errno::last();
            if e == nix::errno::Errno::EINTR {
                continue;
            }
            return Err(Error::Errno(e));
        }
    }
}

/// Number of OS threads in this process, read from `/proc/self/status`
/// (`Threads:`). `None` when `/proc` is unavailable (non-Linux).
pub fn thread_count() -> Option<usize> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
}

/// Assert that this process is single-threaded — the precondition for
/// calling fork(2) and only using async-signal-safe operations in the
/// child. Returns `Err` when `/proc/self/status` reports more than one
/// thread.
pub fn assert_single_threaded() -> Result<()> {
    match thread_count() {
        Some(1) => Ok(()),
        Some(n) => Err(Error::Other(format!(
            "refusing to fork: process has {n} threads (fork children must be \
             async-signal-safe, which requires a single-threaded parent)"
        ))),
        // /proc unavailable: nothing to check.
        None => Ok(()),
    }
}

/// Fork-safety gate for the fork helpers below.
///
/// With `MN_RUST_STRICT_FORK=1` a multi-threaded process is a hard error
/// (production orchestrators should set it). Without it the check is a
/// no-op, because the `cargo test` harness itself runs tests on worker
/// threads; the fork helpers still pre-allocate everything the child
/// needs so the child only performs raw syscalls.
pub fn check_fork_safety() -> Result<()> {
    if std::env::var_os("MN_RUST_STRICT_FORK").is_some_and(|v| v == "1") {
        assert_single_threaded()?;
    }
    Ok(())
}

/// Run `f` with a [`Netlink`] handle inside the network namespace at
/// `path` (e.g. `/var/run/netns/<name>` or `/proc/<pid>/ns/net`).
///
/// Forks a child that `setns`es into that namespace (via the
/// allocation-free [`setns_path_raw`] on a path `CStr` pre-allocated
/// before the fork) and runs `f` there on a tokio current-thread
/// runtime that was **created before the fork** — fork children never
/// allocate paths or create runtimes after fork(2). The parent blocks in
/// `waitpid` and maps the child's exit status to `Ok(())` / `Err`.
///
/// Requires root / `CAP_SYS_ADMIN` (like `mnexec -a`).
pub fn run_in_netns_path<F>(path: &CStr, f: F) -> Result<()>
where
    F: FnOnce(&Netlink) -> Result<()>,
{
    check_fork_safety()?;
    // The tokio runtime is built pre-fork and moved into the child, so no
    // runtime is created after fork(2).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // SAFETY: the child branch calls only setns_path_raw (open+setns+close
    // on the pre-allocated `path`), Netlink::from_runtime (socket setup on
    // the pre-fork runtime) and libc::_exit. No path/CString allocation
    // and no runtime creation happen after the fork; see check_fork_safety
    // for the single-threaded gate (MN_RUST_STRICT_FORK=1). The parent
    // branch only calls waitpid.
    let child = unsafe { nix::unistd::fork()? };
    match child {
        nix::unistd::ForkResult::Child => {
            let rc = (|| -> Result<i32> {
                setns_path_raw(path)?;
                let nl = Netlink::from_runtime(rt)?;
                f(&nl)?;
                Ok(0)
            })();
            let code = rc.unwrap_or(1);
            // SAFETY: raw syscall only.
            unsafe { libc::_exit(code) }
        }
        nix::unistd::ForkResult::Parent { child } => {
            use nix::sys::wait::{waitpid, WaitStatus};
            match waitpid(child, None)? {
                WaitStatus::Exited(_, 0) => Ok(()),
                WaitStatus::Exited(_, code) => Err(Error::Other(format!(
                    "netlink op inside netns {path:?} failed (exit code {code})"
                ))),
                other => Err(Error::Other(format!(
                    "netlink-op child for netns {path:?} ended unexpectedly: {other:?}"
                ))),
            }
        }
    }
}

/// Run `f` with a [`Netlink`] handle inside the network namespace of
/// process `pid` — the netlink-in-namespace equivalent of running a
/// command via `mnexec -a <pid>` / `setns(2)`.
///
/// See [`run_in_netns_path`] for the fork-safety details.
pub fn run_in_netns<F>(pid: u32, f: F) -> Result<()>
where
    F: FnOnce(&Netlink) -> Result<()>,
{
    let c_path = CString::new(format!("/proc/{pid}/ns/net"))
        .map_err(|_| Error::InvalidInput(format!("bad ns path for pid {pid}")))?;
    run_in_netns_path(&c_path, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;
    use std::os::fd::AsRawFd;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mn-netlink-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_netns_name_accepts_typical_names() {
        for name in ["h1", "s1", "mn-test_1", "a.b-c_d", "0"] {
            assert!(
                validate_netns_name(name).is_ok(),
                "{name:?} should be valid"
            );
        }
    }

    #[test]
    fn validate_netns_name_rejects_bad_names() {
        for name in ["", "a/b", "a b", "a:b", "..", ".", "naïve"] {
            assert!(
                validate_netns_name(name).is_err(),
                "{name:?} should be invalid"
            );
        }
        let long = "x".repeat(256);
        assert!(validate_netns_name(&long).is_err());
    }

    #[test]
    fn netns_path_in_joins_and_validates() {
        let dir = Path::new("/var/run/netns");
        assert_eq!(
            netns_path_in(dir, "h1").unwrap(),
            PathBuf::from("/var/run/netns/h1")
        );
        assert!(netns_path_in(dir, "bad/name").is_err());
    }

    #[test]
    fn netns_list_in_empty_and_missing_dirs() {
        let dir = temp_dir("list-empty");
        assert_eq!(netns_list_in(&dir).unwrap(), Vec::<String>::new());
        let missing = dir.join("does-not-exist");
        assert_eq!(netns_list_in(&missing).unwrap(), Vec::<String>::new());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn netns_list_in_lists_and_sorts_entries() {
        let dir = temp_dir("list-entries");
        for name in ["s2", "h1", "h10"] {
            fs::write(dir.join(name), b"").unwrap();
        }
        assert_eq!(netns_list_in(&dir).unwrap(), vec!["h1", "h10", "s2"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    // ------------------------------------------------------------------
    // Fork-handshake helpers (no root needed: plain fork + pipe syscalls)
    // ------------------------------------------------------------------

    #[test]
    fn pipe_pair_roundtrips_bytes() {
        let (mut r, mut w) = pipe_pair().unwrap();
        use std::io::{Read, Write};
        w.write_all(b"x").unwrap();
        let mut b = [0u8; 1];
        r.read_exact(&mut b).unwrap();
        assert_eq!(b, [b'x']);
    }

    #[test]
    fn readiness_handshake_byte_arrives_after_child_work() {
        // The race being fixed: the parent must not proceed until the child
        // has finished its setup (here: a simulated setns). The child writes
        // the readiness byte only after "work"; the parent blocks on it.
        let (r, w) = pipe_pair().unwrap();
        // SAFETY: child only does raw syscalls (write/close/_exit) on the
        // pre-fork pipe fds — async-signal-safe, no allocation.
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                drop(r);
                send_ready_byte(w.as_raw_fd());
                // SAFETY: close(2) is async-signal-safe.
                unsafe { libc::close(w.as_raw_fd()) };
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(0) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                drop(w);
                let ready =
                    wait_ready_byte(r.as_raw_fd(), Duration::from_secs(5)).expect("no timeout");
                drop(r);
                assert!(ready, "readiness byte must arrive");
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("handshake child failed: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn readiness_handshake_eof_means_child_failed() {
        // Child exits without writing the byte (e.g. its setns failed):
        // the parent must see EOF => Ok(false), not block forever.
        let (r, w) = pipe_pair().unwrap();
        // SAFETY: child only calls _exit.
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                drop(r);
                drop(w); // close write end => parent sees EOF
                         // SAFETY: raw syscall only.
                unsafe { libc::_exit(1) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                drop(w);
                let ready = wait_ready_byte(r.as_raw_fd(), Duration::from_secs(5))
                    .expect("EOF is not an error");
                drop(r);
                assert!(!ready, "EOF before the byte must report not-ready");
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 1) => {}
                    other => panic!("expected exit code 1, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn readiness_handshake_times_out_when_child_never_signals() {
        let (r, w) = pipe_pair().unwrap();
        // SAFETY: child holds the write end open but never writes, then
        // exits after the parent's wait has already timed out.
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                drop(r);
                // SAFETY: raw syscall only; returns on SIGKILL.
                unsafe { libc::pause() };
                unsafe { libc::_exit(0) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                drop(w);
                let res = wait_ready_byte(r.as_raw_fd(), Duration::from_millis(200));
                drop(r);
                assert!(res.is_err(), "must time out, got {res:?}");
                // SAFETY: `child` is our own forked child.
                unsafe { libc::kill(child.as_raw(), libc::SIGKILL) };
                let _ = nix::sys::wait::waitpid(child, None);
            }
        }
    }

    #[test]
    fn thread_count_reads_proc_status() {
        let n = thread_count().expect("/proc/self/status must be readable on Linux");
        assert!(n >= 1);
        // The cargo-test harness runs tests on worker threads, so under
        // `cargo test` the process is multi-threaded; assert_single_threaded
        // must agree with thread_count either way.
        assert_eq!(assert_single_threaded().is_ok(), n == 1);
    }

    #[test]
    fn check_fork_safety_noop_unless_strict_env() {
        // Default (env unset): never errors even when multi-threaded.
        if std::env::var_os("MN_RUST_STRICT_FORK").is_none() {
            assert!(check_fork_safety().is_ok());
        }
    }

    // ------------------------------------------------------------------
    // Root-gated tests. These create/destroy kernel namespaces and call
    // setns(2); they must run on a privileged Linux host/CI runner:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1
    // (--test-threads=1 because some tests fork children that change
    // namespaces; run them serialized.)
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn netns_add_list_del_roundtrip() {
        if !root_tests_enabled() {
            return;
        }
        let name = format!("mnrt{}", std::process::id());
        let _ = netns_del(&name); // best-effort cleanup of stale ns
        netns_add(&name).expect("netns_add as root");
        assert!(netns_exists(&name));
        assert!(netns_list().unwrap().contains(&name));
        netns_del(&name).expect("netns_del as root");
        assert!(!netns_exists(&name));
        // Deleting a missing ns must fail with ENOENT.
        assert!(matches!(
            netns_del(&name),
            Err(Error::Errno(nix::errno::Errno::ENOENT))
        ));
        // Adding the same name twice must fail.
        netns_add(&name).unwrap();
        assert!(netns_add(&name).is_err());
        netns_del(&name).unwrap();
    }

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn setns_into_named_netns_changes_netns() {
        if !root_tests_enabled() {
            return;
        }
        let name = format!("mnrt{}", std::process::id());
        let _ = netns_del(&name);
        netns_add(&name).unwrap();

        let before = fs::read_link("/proc/self/ns/net").unwrap();
        // Pre-allocate the CStrings the fork child needs (netns_add_in
        // pattern): after fork the child performs raw syscalls only.
        let ns_path = CString::new(netns_path(&name).unwrap().as_os_str().as_bytes()).unwrap();
        let init_path = CString::new("/proc/1/ns/net").unwrap();
        // SAFETY: single-threaded-at-fork child performs setns_path_raw
        // (open+setns+close on pre-allocated CStrings) + readlink + _exit
        // only; no allocation after fork.
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                let rc = (|| -> Result<()> {
                    setns_path_raw(&ns_path)?;
                    let after = fs::read_link("/proc/self/ns/net")?;
                    if after == before {
                        return Err(Error::Other("setns did not change netns".to_string()));
                    }
                    // Also exercise setns by pid (mnexec -a path) back to init ns.
                    setns_path_raw(&init_path).ok();
                    Ok(())
                })();
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(if rc.is_ok() { 0 } else { 1 }) };
            }
            nix::unistd::ForkResult::Parent { child: pid } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(pid, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("setns child failed: {other:?}"),
                }
            }
        }
        netns_del(&name).unwrap();
    }

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn run_in_netns_executes_closure_inside_netns() {
        if !root_tests_enabled() {
            return;
        }
        let name = format!("mnrt{}", std::process::id());
        let _ = netns_del(&name);
        netns_add(&name).unwrap();

        let before = fs::read_link("/proc/self/ns/net").unwrap();
        // The closure runs in the forked child, inside the named netns; the
        // parent's namespace must be untouched afterwards.
        run_in_netns_path(
            &CString::new(netns_path(&name).unwrap().as_os_str().as_bytes()).unwrap(),
            |nl| {
                let after = fs::read_link("/proc/self/ns/net")?;
                if after == before {
                    return Err(Error::Other("child did not enter the netns".to_string()));
                }
                // A netlink dump works inside the fresh (loopback-only) ns.
                let links = nl.link_list()?;
                if !links.iter().any(|l| l.name == "lo") {
                    return Err(Error::Other(format!("no lo in fresh netns: {links:?}")));
                }
                Ok(())
            },
        )
        .expect("run_in_netns_path as root");
        // Parent is still in its own ns.
        assert_eq!(fs::read_link("/proc/self/ns/net").unwrap(), before);
        netns_del(&name).unwrap();
    }
}
