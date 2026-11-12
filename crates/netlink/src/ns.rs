//! Named network namespaces — the Rust equivalent of `ip netns add/del/list`
//! and `ip netns exec` (via `setns(2)`).
//!
//! `ip netns add <name>` works by creating an empty file under
//! `/var/run/netns/<name>`, then `unshare(CLONE_NEWNET)` in a short-lived
//! child and bind-mounting the child's `/proc/<pid>/ns/net` (i.e.
//! `/proc/self/ns/net` inside the child) onto that file. The bind mount
//! keeps the namespace alive after the child exits. We mirror that
//! exactly, using `nix`/`libc` syscalls instead of shelling out.

use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};

use nix::fcntl::{open, OFlag};
use nix::mount::{umount2, MntFlags};
use nix::sched::{setns, CloneFlags};
use nix::sys::stat::Mode;

use crate::{Error, Result};

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

/// Enter the network namespace of process `pid` via `setns(2)`
/// (the syscall behind `ip netns exec` / `mnexec -a <pid>`).
///
/// This changes the **calling process's** namespace. Requires root /
/// `CAP_SYS_ADMIN` in the target namespace's user namespace.
pub fn setns_pid(pid: u32) -> Result<()> {
    let path = format!("/proc/{pid}/ns/net");
    let fd = open(
        path.as_str(),
        OFlag::O_RDONLY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    // nstype 0 = any namespace type, like the C mnexec which calls
    // setns(fd, 0).
    setns(fd.as_fd(), CloneFlags::empty())?;
    Ok(())
}

/// Enter the named netns `name` under [`NETNS_PATH`] via `setns(2)`.
///
/// This changes the **calling process's** namespace. Requires root /
/// `CAP_SYS_ADMIN`.
pub fn setns_name(name: &str) -> Result<()> {
    let path = netns_path(name)?;
    let fd = open(
        path.as_os_str(),
        OFlag::O_RDONLY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    setns(fd.as_fd(), CloneFlags::empty())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

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
        // SAFETY: single-threaded child performs setns + readlink + _exit
        // only (raw syscalls / std calls on pre-existing data).
        let child = unsafe { nix::unistd::fork().unwrap() };
        match child {
            nix::unistd::ForkResult::Child => {
                let rc = (|| -> Result<()> {
                    setns_name(&name)?;
                    let after = fs::read_link("/proc/self/ns/net")?;
                    if after == before {
                        return Err(Error::Other("setns did not change netns".to_string()));
                    }
                    // Also exercise setns by pid (mnexec -a path) back to init ns.
                    setns_pid(1).ok();
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
}
