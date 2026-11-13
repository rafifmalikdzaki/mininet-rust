//! Side-effecting operations of `mnexec`, ported 1:1 from `mnexec.c`.
//!
//! Every function mirrors the corresponding case of the C `switch` in
//! `main()` and returns a human-readable error string on failure (the C
//! code `perror()`s and exits 1; `main.rs` does the exiting).

use std::convert::Infallible;
use std::ffi::CString;
use std::io::Write;
use std::os::unix::io::AsFd;
use std::path::Path;

use nix::fcntl::{open, OFlag};
use nix::mount::{mount, MsFlags};
use nix::sched::{setns, unshare, CloneFlags};
use nix::sys::stat::Mode;

/// Outcome of `-d` (detach from tty).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detach {
    /// This process should keep going (we were the child, or not a session
    /// leader to begin with).
    Continued,
    /// We were the parent of the fork: exit 0 immediately, like the C code.
    ParentExit,
}

/// `-c`: close all file descriptors except stdin/out/error.
///
/// Mirrors the C code: iterate `/proc/self/fd` and close everything > 2;
/// if that fails, fall back to closing 3.._SC_OPEN_MAX like the old
/// `getdtablesize()` loop.
pub fn close_fds_except_stdio() {
    match std::fs::read_dir("/proc/self/fd") {
        Ok(entries) => {
            let mut fds: Vec<i32> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
                .filter(|&fd| fd > 2)
                .collect();
            fds.sort_unstable();
            fds.dedup();
            for fd in fds {
                // EBADF etc. are fine: the fd may already be closed.
                let _ = nix::unistd::close(fd);
            }
        }
        Err(_) => {
            // SAFETY: sysconf(_SC_OPEN_MAX) is a plain query syscall.
            let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }.clamp(0, 1 << 20) as i32;
            for fd in (3..=max).rev() {
                let _ = nix::unistd::close(fd);
            }
        }
    }
}

/// `-d`: detach from the controlling tty by calling `setsid()`.
///
/// Mirrors the C code: if we are a process group leader, fork and let the
/// parent exit (the caller sees success immediately); the child calls
/// `setsid()`.
pub fn detach_from_tty() -> Result<Detach, String> {
    if nix::unistd::getpgrp() == nix::unistd::getpid() {
        // SAFETY: `mnexec` is single-threaded at this point (no threads are
        // ever spawned before this call), so fork(2) is safe. The child
        // immediately calls setsid(2) — a raw syscall — exactly like the C
        // original.
        match unsafe { nix::unistd::fork() } {
            Ok(nix::unistd::ForkResult::Parent { .. }) => return Ok(Detach::ParentExit),
            Ok(nix::unistd::ForkResult::Child) => {}
            Err(e) => return Err(format!("fork: {e}")),
        }
    }
    nix::unistd::setsid().map_err(|e| format!("setsid: {e}"))?;
    Ok(Detach::Continued)
}

/// `-n`: run in new network and mount namespaces.
///
/// Mirrors the C code: `unshare(CLONE_NEWNET|CLONE_NEWNS)`, remount `/`
/// recursively private so our mounts don't propagate, then mount a fresh
/// sysfs so `/sys` reflects the new network namespace.
pub fn unshare_net_mount_ns() -> Result<(), String> {
    unshare(CloneFlags::CLONE_NEWNET | CloneFlags::CLONE_NEWNS)
        .map_err(|e| format!("unshare: {e}"))?;
    mount(
        Some("none"),
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(|e| format!("remount: {e}"))?;
    mount(
        Some("sysfs"),
        "/sys",
        Some("sysfs"),
        MsFlags::MS_MGC_VAL,
        None::<&str>,
    )
    .map_err(|e| format!("mount: {e}"))?;
    Ok(())
}

/// `-p`: print `^A` + pid to `out` (the C code prints `"\001%d\n"` and
/// flushes stdout).
pub fn print_pid<W: Write>(out: &mut W) {
    let _ = writeln!(out, "\x01{}", nix::unistd::getpid().as_raw());
    let _ = out.flush();
}

/// `-a <pid>`: attach to pid's network and mount namespaces.
///
/// Mirrors the C code exactly: `setns()` on `/proc/<pid>/ns/net`, then
/// plan A `setns()` on `/proc/<pid>/ns/mnt`, plan B `chroot()` into
/// `/proc/<pid>/root`, then `chdir()` back to the working directory we
/// started in.
pub fn attach_to_pid(pid: i32, cwd: &Path) -> Result<(), String> {
    let net = format!("/proc/{pid}/ns/net");
    let fd =
        open(net.as_str(), OFlag::O_RDONLY, Mode::empty()).map_err(|e| format!("{net}: {e}"))?;
    // nstype 0 = any namespace type, like the C mnexec which calls
    // setns(fd, 0).
    setns(fd.as_fd(), CloneFlags::empty()).map_err(|e| format!("setns: {e}"))?;

    let mnt = format!("/proc/{pid}/ns/mnt");
    let mnt_ok = match open(mnt.as_str(), OFlag::O_RDONLY, Mode::empty()) {
        Ok(fd) => setns(fd.as_fd(), CloneFlags::empty()).is_ok(),
        Err(_) => false,
    };
    if !mnt_ok {
        // Plan B: chroot into pid's root file system.
        let root = format!("/proc/{pid}/root");
        nix::unistd::chroot(root.as_str()).map_err(|e| format!("{root}: {e}"))?;
    }
    nix::unistd::chdir(cwd).map_err(|e| format!("{}: {e}", cwd.display()))?;
    Ok(())
}

/// Validate a cgroup path component the way the C `validate()` does:
/// alphanumeric characters and `/` only.
pub fn validate_cgroup_path(g: &str) -> Result<(), String> {
    if g.bytes().any(|b| !b.is_ascii_alphanumeric() && b != b'/') {
        return Err(format!("invalid path: {g}"));
    }
    Ok(())
}

/// `-g <group>`: add our pid to the cgroup, trying the legacy v1
/// controllers `cpu`, `cpuacct`, `cpuset` exactly like the C code.
pub fn add_to_cgroup(group: &str) -> Result<(), String> {
    validate_cgroup_path(group)?;
    let pid = nix::unistd::getpid().as_raw();
    let mut count = 0usize;
    for controller in ["cpu", "cpuacct", "cpuset"] {
        let path = format!("/sys/fs/cgroup/{controller}/{group}/tasks");
        if std::fs::write(&path, format!("{pid}\n")).is_ok() {
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!("cgroup: could not add to cgroup {group}"));
    }
    Ok(())
}

/// `-r <rtprio>`: run with SCHED_RR at the given priority.
pub fn set_rt_priority(prio: i32) -> Result<(), String> {
    let param = libc::sched_param {
        sched_priority: prio,
    };
    // SAFETY: `sched_setscheduler` with pid 0 (calling thread), policy
    // SCHED_RR and a fully initialized `sched_param` only changes the
    // calling thread's scheduling attributes; `param` outlives the call.
    let rc = unsafe { libc::sched_setscheduler(0, libc::SCHED_RR, &param) };
    if rc != 0 {
        return Err(format!(
            "sched_setscheduler: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// execvp the command. Only returns on failure (like the C `execvp` +
/// `perror` path); the returned `Ok` is uninhabited.
pub fn exec_cmd(cmd: &[String]) -> Result<Infallible, String> {
    if cmd.is_empty() {
        return Err("no command given".to_string());
    }
    let cstrings: Vec<CString> = cmd
        .iter()
        .map(|s| CString::new(s.as_str()))
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{}: invalid argument (NUL byte)", cmd[0]))?;
    // execvp searches PATH and only returns on error.
    nix::unistd::execvp(&cstrings[0], &cstrings).map_err(|e| format!("{}: {e}", cmd[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    #[test]
    fn validate_cgroup_path_accepts_alnum_and_slashes() {
        assert!(validate_cgroup_path("h1").is_ok());
        assert!(validate_cgroup_path("foo1/bar2/baz").is_ok());
    }

    #[test]
    fn validate_cgroup_path_rejects_dangerous_chars() {
        for bad in ["../etc", "a;b", "a b", "a$(x)", "a`id`", ""] {
            // Note: "" is rejected by the C code too only if it contains a
            // bad char — empty string passes validate() in C; we mirror
            // that: only non-alnum non-slash bytes are rejected.
            if bad.is_empty() {
                assert!(validate_cgroup_path(bad).is_ok());
            } else {
                assert!(validate_cgroup_path(bad).is_err(), "{bad:?}");
            }
        }
    }

    #[test]
    fn exec_cmd_rejects_empty_command() {
        assert!(exec_cmd(&[]).is_err());
    }

    #[test]
    fn print_pid_writes_soh_prefix() {
        let mut buf = Vec::new();
        print_pid(&mut buf);
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, format!("\x01{}\n", std::process::id()));
    }

    // ------------------------------------------------------------------
    // Root-gated tests. setns/unshare/exec paths change namespaces or
    // replace the process, so they run in forked children and only on a
    // privileged Linux host/CI runner:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mnexec -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + namespace support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mnexec -- --ignored --test-threads=1"]
    fn unshare_net_mount_ns_in_child() {
        if !root_tests_enabled() {
            return;
        }
        let before = std::fs::read_link("/proc/self/ns/net").unwrap();
        // SAFETY: single-threaded child (run with --test-threads=1); child
        // calls unshare_net_mount_ns + readlink + raw _exit only.
        match unsafe { nix::unistd::fork() }.unwrap() {
            nix::unistd::ForkResult::Child => {
                let ok = unshare_net_mount_ns().is_ok()
                    && std::fs::read_link("/proc/self/ns/net").unwrap() != before;
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(if ok { 0 } else { 1 }) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("unshare child failed: {other:?}"),
                }
            }
        }
    }

    #[test]
    #[ignore = "requires root + namespace support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mnexec -- --ignored --test-threads=1"]
    fn attach_to_own_pid_in_child() {
        if !root_tests_enabled() {
            return;
        }
        let pid = std::process::id() as i32;
        let cwd = std::env::current_dir().unwrap();
        // SAFETY: single-threaded child (run with --test-threads=1); child
        // calls attach_to_pid (setns to its own ns) + raw _exit only.
        match unsafe { nix::unistd::fork() }.unwrap() {
            nix::unistd::ForkResult::Child => {
                let ok = attach_to_pid(pid, &cwd).is_ok();
                // SAFETY: raw syscall only.
                unsafe { libc::_exit(if ok { 0 } else { 1 }) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("attach child failed: {other:?}"),
                }
            }
        }
    }

    #[test]
    #[ignore = "exec path test: run on a privileged Linux host/CI with MN_RUST_ROOT_TESTS=1 cargo test -p mnexec -- --ignored --test-threads=1"]
    fn exec_cmd_replaces_child_with_true() {
        if !root_tests_enabled() {
            return;
        }
        // SAFETY: single-threaded child (run with --test-threads=1); child
        // calls exec_cmd which execvp's /bin/true and never returns.
        match unsafe { nix::unistd::fork() }.unwrap() {
            nix::unistd::ForkResult::Child => {
                let _ = exec_cmd(&["/bin/true".to_string()]);
                // SAFETY: raw syscall only (exec failed).
                unsafe { libc::_exit(1) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                use nix::sys::wait::{waitpid, WaitStatus};
                match waitpid(child, None).unwrap() {
                    WaitStatus::Exited(_, 0) => {}
                    other => panic!("exec child did not exit 0: {other:?}"),
                }
            }
        }
    }
}
