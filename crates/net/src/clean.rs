//! `mininet.clean.cleanup()` — `mn --clean` — plus the root check.
//!
//! Port of the essentials of `mininet/clean.py`'s `Cleanup.cleanup()`:
//! kill zombie processes, remove `/tmp` junk, delete stale OVS bridges,
//! delete `*-eth<N>` links, delete leftover netns, kill stale mininet
//! shells. Like Python's `sh()`, every step is **best-effort** (errors are
//! ignored) — cleanup must never fail loudly. It mutates the system, so
//! callers gate it on [`is_root`] (`mn --clean` as non-root is an error).
//!
//! Deviations from CPython (documented): X11 tunnel/screen cleanup
//! (`cleanUpScreens`), `dpctl deldp`, and `~/.ssh/mn` removal are skipped;
//! netns deletion (which Python relies on via killing `mnexec` holders) is
//! done directly via [`mn_netlink::netns_list`]/[`mn_netlink::netns_del`].

use crate::error::Result;

/// Whether the process runs as root (euid == 0) — the gate for every
/// kernel/system-mutating operation (`start`, `--clean`, ...), Python's
/// `ensureRoot()`.
///
/// Read from `/proc/self/status` (`Uid: <real> <effective> ...`) so this
/// crate needs no direct libc dependency (which would churn the workspace
/// `Cargo.lock`); unreadable `/proc` fails closed (non-root).
pub fn is_root() -> bool {
    match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => {
            status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|rest| rest.split_whitespace().nth(1)) // effective uid
            == Some("0")
        }
        Err(_) => false,
    }
}

/// Run a shell command best-effort (Python's `sh()` ignores failures here).
fn sh(cmd: &str) {
    let _ = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Python `killprocs(pattern)`: `pkill -9 -f <pattern>` until gone.
fn killprocs(pattern: &str) {
    sh(&format!("pkill -9 -f {pattern} 2>/dev/null"));
    sh(&format!("pkill -9 -f {pattern} 2>/dev/null"));
}

/// Port of `mininet.clean.cleanup()` (`mn --clean`). Best-effort, root-only
/// (callers check [`is_root`]); returns Ok even if individual steps fail.
pub fn cleanup() -> Result<()> {
    eprintln!("*** Removing excess controllers/ofprotocols/ofdatapaths/pings/noxes");
    let zombies = "controller ofprotocol ofdatapath ping nox_core lt-nox_core \
                   ovs-openflowd ovs-controller ovs-testcontroller udpbwtest mnexec \
                   ivs ryu-manager";
    for pat in zombies.split_whitespace() {
        killprocs(pat);
    }
    sh("pkill -9 -f \"sudo mnexec\" 2>/dev/null");

    eprintln!("*** Removing junk from /tmp");
    sh("rm -f /tmp/vconn* /tmp/vlogs* /tmp/*.out /tmp/*.log 2>/dev/null");

    eprintln!("*** Removing OVS datapaths");
    // `ovs-vsctl --timeout=1 list-br`, then del-br each (twice, like Python).
    let bridges = std::process::Command::new("ovs-vsctl")
        .args(["--timeout=1", "list-br"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let brs: Vec<&str> = bridges
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if !brs.is_empty() {
        let del = brs
            .iter()
            .map(|b| format!("--if-exists del-br {b}"))
            .collect::<Vec<_>>()
            .join(" -- ");
        sh(&format!("ovs-vsctl {del} 2>/dev/null"));
        for b in &brs {
            sh(&format!("ovs-vsctl del-br {b} 2>/dev/null"));
        }
    }

    eprintln!("*** Removing all links of the pattern foo-ethX");
    // `ip link show | egrep -o '([-_.[:alnum:]]+-eth[[:digit:]]+)'`
    let links_out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("ip link show 2>/dev/null | egrep -o '([-_.[:alnum:]]+-eth[[:digit:]]+)'")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    for link in links_out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        sh(&format!("ip link del {link} 2>/dev/null"));
    }

    eprintln!("*** Deleting leftover network namespaces");
    if let Ok(names) = mn_netlink::netns_list() {
        for name in names {
            let _ = mn_netlink::netns_del(&name);
        }
    }

    eprintln!("*** Killing stale mininet node processes");
    killprocs("mininet:");
    killprocs("Tunnel=Ethernet");

    eprintln!("*** Cleanup complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_root_matches_proc_status_euid() {
        // In the non-root sandbox this is false; on a root host true. Must
        // agree with the effective uid in /proc/self/status.
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let euid = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|rest| rest.split_whitespace().nth(1))
            .map(str::to_string);
        assert_eq!(is_root(), euid.as_deref() == Some("0"));
    }

    // NOTE: no unit test calls cleanup() itself — it pkills/deletes by
    // design (that is its job, root-gated at the call site).
}
