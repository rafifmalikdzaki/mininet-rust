//! `ping` / `pingAll` — port of the test methods of `mininet/net.py`
//! (`Mininet.ping`, `_parsePing`, `pingAll`, `pingPair`), driven by host
//! `cmd()` calls exactly like Python.

use crate::error::{Error, Result};
use crate::Mininet;

/// Result of [`Mininet::ping`]: the exact `output()` text plus the packet
/// loss percentage Python returns.
#[derive(Debug, Clone, PartialEq)]
pub struct PingResult {
    /// The `output()` text (`*** Ping: ...`, `h1 -> h2 X`, `*** Results: ...`).
    pub output: String,
    /// `ploss` — packet loss percentage (0.0 when no packets were sent).
    pub ploss: f64,
}

/// Python's `if not hosts: hosts = self.hosts` — **`None` or an empty list
/// both mean "all hosts"** (an empty list is falsy in Python). Shared by
/// [`Mininet::ping`] and [`Mininet::iperf`]; returns the selected host
/// names and whether the all-hosts default kicked in (ping prints its
/// `*** Ping:` header only in that case).
pub(crate) fn hosts_or_all(hosts: Option<Vec<&str>>, all: &[String]) -> (Vec<String>, bool) {
    match hosts {
        None => (all.to_vec(), true),
        Some(v) if v.is_empty() => (all.to_vec(), true),
        Some(v) => (v.iter().map(|s| s.to_string()).collect(), false),
    }
}

/// Port of `Mininet._parsePing`: parse `ping -c1` output into
/// `(packets sent, packets received)`. A downed link
/// (`connect: Network is unreachable`) counts as `(1, 0)`; unparseable
/// output also counts as `(1, 0)` (Python logs an error and does the same).
pub fn parse_ping(ping_output: &str) -> (u64, u64) {
    if ping_output.contains("connect: Network is unreachable") {
        return (1, 0);
    }
    // r'(\d+) packets transmitted, (\d+)( packets)? received'
    let Some(pos) = ping_output.find(" packets transmitted, ") else {
        return (1, 0);
    };
    // digits immediately before " packets transmitted,"
    let before = &ping_output[..pos];
    let sent_start = before
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    let Ok(sent) = before[sent_start..].parse::<u64>() else {
        return (1, 0);
    };
    let after = &ping_output[pos + " packets transmitted, ".len()..];
    let digits_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    // the regex tail `( packets)? received`
    let tail = &after[digits_end..];
    if !tail.starts_with(" received") && !tail.starts_with(" packets received") {
        return (1, 0);
    }
    let Ok(received) = after[..digits_end].parse::<u64>() else {
        return (1, 0);
    };
    (sent, received)
}

impl Mininet {
    /// Port of `Mininet.ping(hosts=None, timeout=None)`: ping between all
    /// given hosts (default: all hosts) via `node.cmd('LANG=C ping -c1 ...')`.
    /// `hosts` are node names. Returns the output text and ploss.
    ///
    /// Python's `if not hosts: hosts = self.hosts` makes an **empty** list
    /// mean "all hosts" too, so `Some(vec![])` behaves like `None`.
    pub fn ping(&mut self, hosts: Option<Vec<&str>>, timeout: Option<&str>) -> Result<PingResult> {
        let mut out = String::new();
        // Python: `if not hosts: hosts = self.hosts` — empty list is falsy,
        // so None and Some([]) both mean all hosts (header printed then).
        let all: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        let (names, defaulted_to_all) = hosts_or_all(hosts, &all);
        if defaulted_to_all {
            out.push_str("*** Ping: testing ping reachability\n");
        }
        // Pre-compute per-destination (has intfs, IP string) — Python reads
        // dest.intfs / dest.IP() per pair.
        let mut dests: Vec<(String, bool, String)> = Vec::new();
        for n in &names {
            let r = self
                .get(n)
                .ok_or_else(|| Error::InvalidInput(format!("node {n:?} not in network")))?;
            let node = r.node();
            let has_intfs = !node.intf_list().is_empty();
            let ip = node
                .ip(None)
                .map(|a| a.to_string())
                .unwrap_or_else(|| "None".to_string());
            dests.push((n.to_string(), has_intfs, ip));
        }
        let mut packets: u64 = 0;
        let mut lost: u64 = 0;
        for src in &names {
            out.push_str(&format!("{src} -> "));
            for (dst, has_intfs, ip) in &dests {
                if src == dst {
                    continue;
                }
                let (sent, received) = if *has_intfs {
                    let opts = timeout.map(|t| format!("-W {t}")).unwrap_or_default();
                    let mut r = self.get_mut(src).ok_or_else(|| {
                        Error::InvalidInput(format!("node {src:?} not in network"))
                    })?;
                    // Python: 'LANG=C ping -c1 %s %s' % (opts, dest.IP())
                    let result = r.node_mut().cmd(&format!("LANG=C ping -c1 {opts} {ip}"))?;
                    parse_ping(&result)
                } else {
                    (0, 0)
                };
                packets += sent;
                if received > sent {
                    return Err(Error::Other(format!(
                        "received too many packets from {src} -> {dst}"
                    )));
                }
                lost += sent - received;
                if received > 0 {
                    out.push_str(&format!("{dst} "));
                } else {
                    out.push_str("X ");
                }
            }
            out.push('\n');
        }
        let ploss = if packets > 0 {
            let ploss = 100.0 * lost as f64 / packets as f64;
            // Python: "*** Results: %i%% dropped (%d/%d received)\n" — %i truncates
            out.push_str(&format!(
                "*** Results: {}% dropped ({}/{} received)\n",
                ploss as i64,
                packets - lost,
                packets
            ));
            ploss
        } else {
            out.push_str("*** Warning: No packets sent\n");
            0.0
        };
        Ok(PingResult { output: out, ploss })
    }

    /// Port of `Mininet.pingAll(timeout)`: ping between all hosts.
    pub fn ping_all(&mut self, timeout: Option<&str>) -> Result<PingResult> {
        self.ping(None, timeout)
    }

    /// Port of `Mininet.pingPair()`: ping between the first two hosts.
    pub fn ping_pair(&mut self) -> Result<PingResult> {
        let names: Vec<String> = self.hosts.keys().take(2).map(str::to_string).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.ping(Some(refs), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ping_typical_output() {
        let out = "PING 10.0.0.2 (10.0.0.2) 56(84) bytes of data.\n\
                   64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time=0.042 ms\n\n\
                   --- 10.0.0.2 ping statistics ---\n\
                   1 packets transmitted, 1 received, 0% packet loss, time 0ms\n\
                   rtt min/avg/max/mdev = 0.042/0.042/0.042/0.000 ms\n";
        assert_eq!(parse_ping(out), (1, 1));
    }

    #[test]
    fn empty_hosts_list_means_all_hosts_like_python() {
        // Python ping(): `if not hosts: hosts = self.hosts` — Some([]) must
        // behave like None (all hosts + the "*** Ping:" header). Hosts here
        // have no intfs, so every pair is (0, 0) and no cmd() is attempted.
        use crate::options::MininetOptions;
        use mn_topo::Attrs;
        let mut net = crate::Mininet::new(MininetOptions {
            build: false,
            controller: None,
            ..Default::default()
        })
        .unwrap();
        net.add_host("h1", &Attrs::new()).unwrap();
        net.add_host("h2", &Attrs::new()).unwrap();
        let r = net.ping(Some(vec![]), None).unwrap();
        assert!(
            r.output
                .starts_with("*** Ping: testing ping reachability\n"),
            "{}",
            r.output
        );
        assert!(r.output.contains("h1 -> "), "{}", r.output);
        assert!(r.output.contains("h2 -> "), "{}", r.output);
        assert!(
            r.output.contains("*** Warning: No packets sent\n"),
            "{}",
            r.output
        );
        // ... and an explicit host list still restricts to it (no header)
        let r = net.ping(Some(vec!["h1"]), None).unwrap();
        assert!(!r.output.contains("*** Ping:"), "{}", r.output);
        assert!(r.output.contains("h1 -> "), "{}", r.output);
        assert!(!r.output.contains("h2 -> "), "{}", r.output);
    }

    #[test]
    fn parse_ping_loss_and_unreachable() {
        let loss = "1 packets transmitted, 0 packets received, 100% packet loss, time 0ms\n";
        assert_eq!(parse_ping(loss), (1, 0));
        let unreach = "connect: Network is unreachable\n";
        assert_eq!(parse_ping(unreach), (1, 0));
        let garbage = "ping: unknown host\n";
        assert_eq!(parse_ping(garbage), (1, 0));
        let multi = "5 packets transmitted, 3 packets received, +2 errors, 40% packet loss\n";
        assert_eq!(parse_ping(multi), (5, 3));
    }
}
