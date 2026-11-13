//! `mininet.clean.cleanup()` — `mn --clean` — plus the root check.
//!
//! Port of the essentials of `mininet/clean.py`'s `Cleanup.cleanup()`:
//! kill zombie processes, remove `/tmp` junk, delete stale OVS bridges,
//! delete `*-eth<N>` links, delete leftover netns, kill stale mininet
//! shells. Like Python's `sh()`, every step is **best-effort** (errors are
//! ignored) — cleanup must never fail loudly. It mutates the system, so
//! callers gate it on [`is_root`] (`mn --clean` as non-root is an error).
//!
//! Every command runs as a direct **argv exec**
//! (`Command::new(cmd).args(..)`, no `/bin/sh -c` string interpolation).
//! The two places where Python's shell does extra work — the `/tmp` glob
//! and the `ip link show | egrep -o ...` pipeline — are expanded/filtered
//! in userspace instead (see [`expand_tmp_junk`] / [`egrep_eth_intfs`]).
//!
//! Deviations from CPython (documented): X11 tunnel/screen cleanup
//! (`cleanUpScreens`), `dpctl deldp`, and `~/.ssh/mn` removal are skipped;
//! netns deletion (which Python relies on via killing `mnexec` holders) is
//! done directly via [`mn_netlink::netns_list`]/[`mn_netlink::netns_del`];
//! [`killprocs`] runs `pkill` twice instead of Python's pgrep/pkill loop.

use crate::error::Result;

/// Whether the process runs as root (euid == 0) — the gate for every
/// kernel/system-mutating operation (`start`, `--clean`, ...), Python's
/// `ensureRoot()`.
///
/// Re-export of the **canonical** implementation in `mn-util`
/// ([`mn_util::is_root`], reading `/proc/self/status`), so there is exactly
/// one root check in the workspace (`mn-node` calls `mn_util::is_root`
/// too).
pub use mn_util::is_root;

/// Run a command best-effort as an argv exec (no shell), ignoring failures
/// (Python's `sh()` ignores failures here).
fn run(cmd: &str, args: &[&str]) {
    let _ = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Python `killprocs(pattern)`: `pkill -9 -f <pattern>`. Python loops
/// (`pgrep -f` + `pkill -9 -f` + `sleep .5`) **until no process matches**;
/// this port runs `pkill -9 -f` **twice** (documented deviation — cleanup
/// is best-effort either way).
fn killprocs(pattern: &str) {
    run("pkill", &["-9", "-f", pattern]);
    run("pkill", &["-9", "-f", pattern]);
}

/// Expand Python's `rm -f /tmp/vconn* /tmp/vlogs* /tmp/*.out /tmp/*.log`
/// glob in userspace (shell `*` does not match leading dots), returning
/// the matching `/tmp` entries.
fn expand_tmp_junk() -> Vec<String> {
    let mut junk = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/tmp") {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue; // shell `*` does not match leading dots
            }
            if name.starts_with("vconn")
                || name.starts_with("vlogs")
                || name.ends_with(".out")
                || name.ends_with(".log")
            {
                junk.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    junk
}

/// A char of the `[-_.[:alnum:]]` class of clean.py's egrep pattern.
fn pat_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// Leftmost-longest match of `[-_.[:alnum:]]+-eth[[:digit:]]+` starting at
/// or after `from`, as `(start, end)` (POSIX ERE leftmost-longest).
fn match_from(chars: &[char], from: usize) -> Option<(usize, usize)> {
    for start in from..chars.len() {
        if !pat_char(chars[start]) {
            continue;
        }
        // maximal run of pattern chars from `start`
        let mut run_end = start;
        while run_end < chars.len() && pat_char(chars[run_end]) {
            run_end += 1;
        }
        // the match ends with `-eth<digits>` inside that run; the largest
        // split point gives the longest overall match
        let mut q = run_end;
        while q > start {
            if q + 4 <= chars.len()
                && chars[q..q + 4] == ['-', 'e', 't', 'h']
                && chars.get(q + 4).is_some_and(|c| c.is_ascii_digit())
            {
                let mut end = q + 4;
                while end < chars.len() && chars[end].is_ascii_digit() {
                    end += 1;
                }
                return Some((start, end));
            }
            q -= 1;
        }
    }
    None
}

/// The `egrep -o '([-_.[:alnum:]]+-eth[[:digit:]]+)'` of clean.py, applied
/// in userspace: all non-overlapping leftmost-longest matches per line.
fn egrep_eth_intfs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match match_from(&chars, i) {
                Some((start, end)) => {
                    out.push(chars[start..end].iter().collect());
                    i = end;
                }
                None => i += 1,
            }
        }
    }
    out
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
    run("pkill", &["-9", "-f", "sudo mnexec"]);

    eprintln!("*** Removing junk from /tmp");
    // `rm -f /tmp/vconn* /tmp/vlogs* /tmp/*.out /tmp/*.log` — glob expanded
    // in userspace, then one argv `rm -f`.
    let junk = expand_tmp_junk();
    if !junk.is_empty() {
        let mut args: Vec<&str> = vec!["-f"];
        args.extend(junk.iter().map(String::as_str));
        run("rm", &args);
    }

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
        // `ovs-vsctl --if-exists del-br b1 -- --if-exists del-br b2 ...`
        let mut args: Vec<String> = Vec::new();
        for (i, b) in brs.iter().enumerate() {
            if i > 0 {
                args.push("--".to_string());
            }
            args.push("--if-exists".to_string());
            args.push("del-br".to_string());
            args.push((*b).to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run("ovs-vsctl", &arg_refs);
        for b in &brs {
            run("ovs-vsctl", &["del-br", b]);
        }
    }

    eprintln!("*** Removing all links of the pattern foo-ethX");
    // `ip link show | egrep -o '([-_.[:alnum:]]+-eth[[:digit:]]+)'` — `ip
    // link show` runs as argv, the egrep pattern is applied in userspace,
    // then each match is deleted with an argv `ip link del <link>`.
    let links_out = std::process::Command::new("ip")
        .args(["link", "show"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    for link in egrep_eth_intfs(&links_out) {
        run("ip", &["link", "del", &link]);
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

    #[test]
    fn egrep_eth_intfs_matches_the_clean_py_pattern() {
        // `ip link show` style lines: one intf name per line, plus noise.
        let out = "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN\n\
                   2: h1-eth0@if5: <BROADCAST,MULTICAST> mtu 1500 qdisc noop state DOWN\n\
                   3: s1-eth12: <BROADCAST> mtu 1500 state DOWN\n\
                   link/ether 00:00:00:00:00:01 brd ff:ff:ff:ff:ff:ff\n\
                   9: eth0: no dash-eth here\n\
                   10: a.b_c-eth3: x\n";
        assert_eq!(
            egrep_eth_intfs(out),
            vec!["h1-eth0", "s1-eth12", "a.b_c-eth3"]
        );
        assert!(egrep_eth_intfs("no matches here").is_empty());
        // `-eth` needs digits after it and a pattern char before it:
        // `x-eth` (no digit), `eth0` (no `-eth`), and ` -eth1` (space before
        // the dash, not a pattern char) all fail to match.
        assert_eq!(egrep_eth_intfs("x-eth eth0 -eth1"), Vec::<String>::new());
        assert_eq!(egrep_eth_intfs("x-eth1"), vec!["x-eth1"]);
    }

    #[test]
    fn expand_tmp_junk_matches_the_shell_glob() {
        // Only checks the name filter (read-only): entries matching
        // vconn*/vlogs*/*.out/*.log, excluding dotfiles. Create nothing;
        // just ensure the function runs and returns paths under /tmp.
        for p in expand_tmp_junk() {
            let name = p.rsplit('/').next().unwrap_or(&p);
            assert!(
                name.starts_with("vconn")
                    || name.starts_with("vlogs")
                    || name.ends_with(".out")
                    || name.ends_with(".log"),
                "unexpected junk entry {p}"
            );
            assert!(!name.starts_with('.'), "dotfile matched: {p}");
        }
    }

    // NOTE: no unit test calls cleanup() itself — it pkills/deletes by
    // design (that is its job, root-gated at the call site).
}
