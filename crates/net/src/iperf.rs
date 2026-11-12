//! `iperf` — port of `Mininet.iperf` and `_iperfVals` in `mininet/net.py`
//! (`--test iperf` / `iperfudp` in `bin/mn`). Real runs need root (host
//! shells in netns); the CSV parsing and result formatting are pure and
//! unit-tested.
//!
//! Deviations from CPython (documented):
//! - `waitListening(client, serverip, port)` (a telnet probe *from the
//!   client host*) is replaced by a direct TCP connect probe
//!   ([`mn_node::is_listening_timeout`]) from the orchestrator — same
//!   reachability question on a started net.
//! - Python polls the server shell with `monitor()` and then `sendInt()`;
//!   here the server shell is polled with `wait_output(Some(..))` (which
//!   keeps partial output for the next poll) and the server is stopped with
//!   `killall -9 iperf` afterwards.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::Mininet;

/// Port of `Mininet._iperfVals`: parse `iperf -y C` CSV output into the
/// field dict. The **last** comma-bearing line wins (Python's dict update
/// per line), and when `cip == serverip` the client/server fields are
/// swapped (Python's correction for server-side output).
#[derive(Debug, Clone, PartialEq)]
pub struct IperfVals {
    /// Client IP (`cip` after the swap).
    pub cip: String,
    /// Client port (`cport` after the swap).
    pub cport: String,
    /// Server IP (`sip` after the swap).
    pub sip: String,
    /// Server port (`sport` after the swap) — matched against the client's
    /// `sport` to pair a server report with its client session.
    pub sport: String,
    /// Rate in bits/sec (`rate` field, as f64).
    pub rate: f64,
}

/// `iperf -y C` CSV field order (Python: `'date cip cport sip sport ipver
/// interval sent rate'`).
const FIELDS: [&str; 9] = [
    "date", "cip", "cport", "sip", "sport", "ipver", "interval", "sent", "rate",
];

/// Port of `Mininet._iperfVals(iperfcsv, serverip)`.
pub fn iperf_vals(iperfcsv: &str, serverip: &str) -> Option<IperfVals> {
    let mut svals: Option<Vec<(&str, String)>> = None;
    for line in iperfcsv.trim().lines() {
        if !line.contains(',') {
            continue;
        }
        let mut map: Vec<(&str, String)> = FIELDS
            .into_iter()
            .zip(line.split(',').map(str::to_string))
            .collect();
        // Return client in cip:cport, server in sip:sport.
        let cip = map
            .iter()
            .find(|(k, _)| *k == "cip")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        if cip == serverip {
            // Python: svals['cip'], svals['sip'] = svals['sip'], svals['cip']
            // (and cport/sport) — swap the VALUES, keeping the keys.
            for (a, b) in [("cip", "sip"), ("cport", "sport")] {
                let ia = map.iter().position(|(k, _)| *k == a).expect("field");
                let ib = map.iter().position(|(k, _)| *k == b).expect("field");
                let va = map[ia].1.clone();
                map[ia].1 = map[ib].1.clone();
                map[ib].1 = va;
            }
        }
        svals = Some(map);
    }
    let map = svals?;
    let get = |key: &str| {
        map.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let rate = get("rate").parse::<f64>().unwrap_or(0.0);
    Some(IperfVals {
        cip: get("cip"),
        cport: get("cport"),
        sip: get("sip"),
        sport: get("sport"),
        rate,
    })
}

/// Result of [`Mininet::iperf`]: the exact `output()` text plus the Python
/// result list (`[server rate, client rate]`, with the UDP bandwidth
/// prepended for UDP).
#[derive(Debug, Clone, PartialEq)]
pub struct IperfResult {
    /// The `output()` text (`*** Iperf: testing ...` + `*** Results: ...`).
    pub output: String,
    /// Python's `result` list (rate strings; `udpBw` first for UDP).
    pub rates: Vec<String>,
}

impl Mininet {
    /// Port of `Mininet.iperf(hosts=None, l4Type='TCP', udpBw='10M',
    /// fmt=None, seconds=5, port=5001)`. `hosts` are node names; `None`
    /// (or empty) uses the first and last hosts, like Python's
    /// `hosts or [self.hosts[0], self.hosts[-1]]`.
    pub fn iperf(
        &mut self,
        hosts: Option<Vec<&str>>,
        l4_type: &str,
        udp_bw: &str,
        seconds: u32,
        port: u16,
    ) -> Result<IperfResult> {
        if !matches!(l4_type, "TCP" | "UDP") {
            return Err(Error::InvalidInput(format!(
                "iperf l4Type must be TCP or UDP, got {l4_type:?}"
            )));
        }
        let all: Vec<String> = self.hosts.keys().map(str::to_string).collect();
        let names: Vec<String> = match hosts {
            // Python: `hosts = hosts or [self.hosts[0], self.hosts[-1]]`
            None => vec![
                all.first().cloned().unwrap_or_default(),
                all.last().cloned().unwrap_or_default(),
            ],
            Some(v) if v.is_empty() => vec![
                all.first().cloned().unwrap_or_default(),
                all.last().cloned().unwrap_or_default(),
            ],
            Some(v) => v.iter().map(|s| s.to_string()).collect(),
        };
        if names.len() != 2 {
            return Err(Error::InvalidInput(format!(
                "iperf requires exactly 2 hosts, got {}",
                names.len()
            )));
        }
        let (client, server) = (names[0].clone(), names[1].clone());
        // Python: output('*** Iperf: testing', l4Type, 'bandwidth between',
        // client, 'and', server, '\n') — space-joined, trailing space.
        let mut out =
            format!("*** Iperf: testing {l4_type} bandwidth between {client} and {server} \n");
        let serverip = self
            .host(&server)
            .and_then(|h| h.ip(None))
            .map(|a| a.to_string())
            .ok_or_else(|| Error::InvalidInput(format!("server {server:?} has no IP")))?;
        {
            let s = self
                .host_mut(&server)
                .ok_or_else(|| Error::InvalidInput(format!("node {server:?} not in network")))?;
            let _ = s.node_mut().cmd("killall -9 iperf");
        }
        let mut iperf_args = format!("iperf -y C -p {port} ");
        let bw_args = if l4_type == "UDP" {
            iperf_args.push_str("-u ");
            format!("-b {udp_bw} ")
        } else {
            String::new()
        };
        {
            let s = self
                .host_mut(&server)
                .ok_or_else(|| Error::InvalidInput(format!("node {server:?} not in network")))?;
            s.node_mut().send_cmd(&format!("{iperf_args}-s"), false)?;
        }
        if l4_type == "TCP" {
            // Python: waitListening(client, serverip, port) else raise.
            let addr: IpAddr = serverip
                .parse()
                .map_err(|_| Error::InvalidInput(format!("bad server IP {serverip:?}")))?;
            if !mn_node::is_listening_timeout(SocketAddr::new(addr, port), Duration::from_secs(5)) {
                return Err(Error::Other(format!(
                    "Could not connect to iperf on port {port}"
                )));
            }
        }
        let cliout = {
            let c = self
                .host_mut(&client)
                .ok_or_else(|| Error::InvalidInput(format!("node {client:?} not in network")))?;
            c.node_mut()
                .cmd(&format!("{iperf_args}-t {seconds} -c {serverip} {bw_args}"))?
        };
        let cvals = iperf_vals(&cliout, &serverip).ok_or_else(|| {
            Error::Other(format!("could not parse iperf client output: {cliout}"))
        })?;
        // Wait for the server's CSV report (Python's monitor() loop: break
        // when the server line's sport matches the client's and rate > 0).
        let mut serverout = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let svals = loop {
            let s = self
                .host_mut(&server)
                .ok_or_else(|| Error::InvalidInput(format!("node {server:?} not in network")))?;
            match s.node_mut().wait_output(Some(Duration::from_millis(500))) {
                Ok(chunk) => serverout.push_str(&chunk),
                Err(mn_node::Error::TimedOut { .. }) => {} // keep polling
                Err(e) => return Err(e.into()),
            }
            if let Some(v) = iperf_vals(&serverout, &serverip) {
                if v.sport == cvals.sport && v.rate > 0.0 {
                    break v;
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::Other(format!(
                    "timed out waiting for iperf server output: {serverout}"
                )));
            }
        };
        // Python: server.sendInt() + waitOutput() — stop the server.
        if let Some(s) = self.host_mut(&server) {
            let _ = s.node_mut().cmd("killall -9 iperf");
        }
        let fmt = |rate: f64| mn_util::fmt_bps(rate).unwrap_or_else(|| format!("{rate} bits/sec"));
        let mut rates = vec![fmt(svals.rate), fmt(cvals.rate)];
        if l4_type == "UDP" {
            rates.insert(0, udp_bw.to_string());
        }
        // Python: output('*** Results: %s\n' % result) — a list repr.
        out.push_str(&format!(
            "*** Results: [{}]\n",
            rates
                .iter()
                .map(|r| format!("'{r}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        Ok(IperfResult { output: out, rates })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iperf_vals_parses_client_csv() {
        // iperf -y C client output (cip is the client, != serverip).
        let out =
            "2026-11-12T10:00:00+00:00,10.0.0.1,43210,10.0.0.2,5001,4,0.0-5.0,6553600,10485760\n";
        let v = iperf_vals(out, "10.0.0.2").unwrap();
        assert_eq!(v.cip, "10.0.0.1");
        assert_eq!(v.cport, "43210");
        assert_eq!(v.sip, "10.0.0.2");
        assert_eq!(v.sport, "5001");
        assert_eq!(v.rate, 10485760.0);
    }

    #[test]
    fn iperf_vals_swaps_when_cip_is_server_like_python() {
        // Server-side CSV: cip == serverip -> swap so cip is the client.
        let out =
            "2026-11-12T10:00:00+00:00,10.0.0.2,5001,10.0.0.1,43210,4,0.0-5.0,6553600,10485760\n";
        let v = iperf_vals(out, "10.0.0.2").unwrap();
        assert_eq!(v.cip, "10.0.0.1");
        assert_eq!(v.cport, "43210");
        assert_eq!(v.sip, "10.0.0.2");
        assert_eq!(v.sport, "5001");
        assert_eq!(v.rate, 10485760.0);
    }

    #[test]
    fn iperf_vals_last_comma_line_wins_and_ignores_noise() {
        let out = "iperf: some warning\n\
                   2026-11-12T10:00:00,10.0.0.1,1,10.0.0.2,5001,4,0.0-1.0,1000,8000\n\
                   2026-11-12T10:00:01,10.0.0.1,1,10.0.0.2,5001,4,1.0-5.0,5000,9000\n";
        let v = iperf_vals(out, "10.0.0.2").unwrap();
        assert_eq!(v.rate, 9000.0);
        assert!(iperf_vals("no csv here", "10.0.0.2").is_none());
    }

    #[test]
    fn iperf_result_list_repr_matches_python() {
        // Python: output('*** Results: %s\n' % ['10M', '1.0 Gbits/sec', ...])
        let rates = ["10M".to_string(), "1.0 Gbits/sec".to_string()];
        let rendered = format!(
            "*** Results: [{}]\n",
            rates
                .iter()
                .map(|r| format!("'{r}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(rendered, "*** Results: ['10M', '1.0 Gbits/sec']\n");
    }

    #[test]
    fn fmt_bps_matches_python_fmt_bps() {
        // Python fmtBps: '%.1f %sbits/sec' with unitScale (1000-based).
        assert_eq!(
            mn_util::fmt_bps(10485760.0).as_deref(),
            Some("10.5 Mbits/sec")
        );
        assert_eq!(mn_util::fmt_bps(8000.0).as_deref(), Some("8.0 kbits/sec"));
        assert_eq!(mn_util::fmt_bps(500.0).as_deref(), Some("500.0 bits/sec"));
        assert_eq!(
            mn_util::fmt_bps(3_000_000_000.0).as_deref(),
            Some("3.0 Gbits/sec")
        );
    }
}
