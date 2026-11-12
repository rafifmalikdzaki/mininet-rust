//! Minimal interactive loop for `mn` — the 10b stand-in for
//! `mininet/cli.py`'s `CLI` class. Task 10c ports the full command set
//! (`nodes links dump net ports hosts switches controllers ping pingall
//! iperf dpctl route xterm exit help source ...`) with golden tests; this
//! loop only proves the wiring: `mn` enters it when stdin is a tty, reads
//! `mininet> ` commands, and a small core set works.
//!
//! Python parity notes: the prompt is `mininet> ` on stdout; `info()`
//! chatter (`*** Starting CLI:`) goes to stderr; `output()` text (dump,
//! links, ping results, ...) goes to `out`.

use std::io::{BufRead, Write};

use mn_net::Mininet;

const HELP: &str = "\
Available commands (10b minimal set; task 10c ports the full cli.py set):
  nodes | hosts | switches | controllers   list nodes
  dump | links                             node reprs / link status
  pingall | pingpair | ping <a> <b>        ping tests
  iperf                                    TCP bandwidth h1<->hlast
  exit | quit                              leave the CLI
";

/// Run the interactive loop until `exit`/`quit`/EOF. Returns the exit code.
pub fn run(net: &mut Mininet, out: &mut dyn Write) -> i32 {
    eprintln!("*** Starting CLI:");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        let _ = out.write_all(b"mininet> ");
        let _ = out.flush();
        let line = match lines.next() {
            None => break, // EOF
            Some(Ok(l)) => l,
            Some(Err(_)) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue; // Python emptyline(): don't repeat last command
        }
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        match cmd {
            "exit" | "quit" => break,
            "help" | "?" => {
                let _ = out.write_all(HELP.as_bytes());
            }
            "nodes" => {
                let mut keys = net.keys();
                keys.sort();
                let _ = writeln!(out, "{}", keys.join(" "));
            }
            "hosts" => {
                let mut v: Vec<&str> = net.hosts.keys().collect();
                v.sort();
                let _ = writeln!(out, "{}", v.join(" "));
            }
            "switches" => {
                let mut v: Vec<&str> = net.switches.keys().collect();
                v.sort();
                let _ = writeln!(out, "{}", v.join(" "));
            }
            "controllers" => {
                let mut v: Vec<&str> = net.controllers.keys().collect();
                v.sort();
                let _ = writeln!(out, "{}", v.join(" "));
            }
            "dump" => {
                let _ = write!(out, "{}", net.dump());
            }
            "links" => {
                let _ = write!(out, "{}", net.links());
            }
            "pingall" => match net.ping_all(None) {
                Ok(r) => {
                    let _ = write!(out, "{}", r.output);
                }
                Err(e) => eprintln!("*** pingall failed: {e}"),
            },
            "pingpair" => match net.ping_pair() {
                Ok(r) => {
                    let _ = write!(out, "{}", r.output);
                }
                Err(e) => eprintln!("*** pingpair failed: {e}"),
            },
            "ping" => {
                if rest.len() == 2 {
                    match net.ping(Some(rest.clone()), None) {
                        Ok(r) => {
                            let _ = write!(out, "{}", r.output);
                        }
                        Err(e) => eprintln!("*** ping failed: {e}"),
                    }
                } else {
                    eprintln!("*** usage: ping <host> <host>");
                }
            }
            "iperf" => match net.iperf(None, "TCP", "10M", 5, 5001) {
                Ok(r) => {
                    let _ = write!(out, "{}", r.output);
                }
                Err(e) => eprintln!("*** iperf failed: {e}"),
            },
            other => {
                // Python's default() runs the line as a node command; the
                // full passthrough is task 10c/10d.
                eprintln!("*** Unknown command: {other} (full CLI command set is task 10c)");
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_lists_core_commands() {
        for c in ["nodes", "dump", "links", "pingall", "iperf", "exit"] {
            assert!(HELP.contains(c), "help missing {c}");
        }
    }
}
