//! Terminal creation and cleanup — the port of `mininet/term.py`
//! (task 10d).
//!
//! Pure, unit-tested builders:
//! - [`term_title`] — `makeTerm`'s `"%s: %s"` title (plus ` (root)` for
//!   root-namespace nodes),
//! - [`term_cmd_prefix`] — the `cmds[term]` argv prefix for `xterm`/`gterm`
//!   (Err with `invalid terminal type: <term>` otherwise, like `term.py`),
//! - [`term_tail`] — the `[display, '-e', 'env TERM=ansi <cmd>']` suffix,
//! - [`tunnel_spec`] — `tunnelX11`'s DISPLAY parsing: local (`""`/`unix`
//!   host → run `xhost +si:localuser:root`, no tunnel) vs TCP
//!   (`host:screen` → `socat TCP-LISTEN:<6000+screen>` ↔ `mnexec -a 1 socat
//!   STDIO TCP\:<host>\:<port>` tunnel, term display `localhost:<screen>`).
//!
//! Runtime spawners ([`make_term`]/[`make_terms`]/[`run_x11`]/
//! [`start_terms`]) spawn real `xterm`/`socat` subprocesses via
//! `Node::popen` — root-gated at runtime by the callers (they only run
//! against a *started* net; `mn --xterms` spawns them after
//! [`Mininet::start`](mn_net::Mininet::start), which is root-only, and the
//! `xterm`/`x` CLI commands run inside the interactive loop of a started
//! net).
//!
//! Documented deviations from `term.py`:
//! - A malformed `DISPLAY` (not exactly `host:screen`) raises `ValueError`
//!   in Python (crashing the CLI); here [`tunnel_spec`] returns Err and the
//!   caller prints the message on `err`.
//! - On a non-started node, Python's `node.popen` still spawns `mnexec`
//!   (which then fails in the background, silently); Rust's
//!   `Node::popen` returns `Err(NotRunning)`, which is reported on `err`
//!   instead of failing silently.

use std::io::Write;
use std::process::{Child, Command, Stdio};

use mn_net::{Mininet, NodeRefMut};

// ---------------------------------------------------------------------
// Pure argv builders (unit-tested below)
// ---------------------------------------------------------------------

/// `makeTerm`'s title: `'"%s: %s"' % ( title, node.name )`, plus
/// `' (root)'` when the node is not in a namespace.
pub fn term_title(title: &str, node_name: &str, in_namespace: bool) -> String {
    let mut t = format!("\"{title}: {node_name}\"");
    if !in_namespace {
        t.push_str(" (root)");
    }
    t
}

/// The `cmds[ term ]` prefix of `makeTerm`:
/// `xterm → ['xterm', '-title', title, '-display']`,
/// `gterm → ['gnome-terminal', '--title', title, '--display']`.
/// Err is `invalid terminal type: <term>` (as `term.py` prints).
pub fn term_cmd_prefix(term: &str, title: &str) -> Result<Vec<String>, String> {
    match term {
        "xterm" => Ok(vec![
            "xterm".to_string(),
            "-title".to_string(),
            title.to_string(),
            "-display".to_string(),
        ]),
        "gterm" => Ok(vec![
            "gnome-terminal".to_string(),
            "--title".to_string(),
            title.to_string(),
            "--display".to_string(),
        ]),
        other => Err(format!("invalid terminal type: {other}")),
    }
}

/// The `[ display, '-e', 'env TERM=ansi %s' % cmd ]` tail of `makeTerm`.
pub fn term_tail(display: &str, cmd: &str) -> Vec<String> {
    vec![
        display.to_string(),
        "-e".to_string(),
        format!("env TERM=ansi {cmd}"),
    ]
}

/// Parsed `tunnelX11` display target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelSpec {
    /// `""`/`unix` host: no tunnel; the term uses `display` as-is (and
    /// `xhost +si:localuser:root` is run first at runtime).
    Local(String),
    /// TCP display: spawn the socat listener `listen_argv`; the term uses
    /// `display` (`localhost:<screen>`).
    Tcp {
        /// Display string passed to the terminal (`localhost:<screen>`).
        display: String,
        /// The `socat TCP-LISTEN:...` listener argv (`node.popen` target).
        listen_argv: Vec<String>,
    },
}

/// `tunnelX11`'s DISPLAY parsing: `host, screen = display.split( ':')`;
/// empty/`unix` host → local; otherwise `port = 6000 + int(float(screen))`
/// and the tunnel cmd is
/// `["socat", "TCP-LISTEN:<port>,fork,reuseaddr",
///   "EXEC:'mnexec -a 1 socat STDIO TCP\\:<host>\\:<port>'"]`.
pub fn tunnel_spec(display: &str) -> Result<TunnelSpec, String> {
    let parts: Vec<&str> = display.split(':').collect();
    if parts.len() != 2 {
        // Python: `host, screen = display.split(':')` raises ValueError
        // (uncaught, crashing the CLI) — we report instead (documented).
        return Err(format!("invalid DISPLAY {display:?}: expected host:screen"));
    }
    let (host, screen) = (parts[0], parts[1]);
    if host.is_empty() || host == "unix" {
        return Ok(TunnelSpec::Local(display.to_string()));
    }
    let screen_num: i64 = screen
        .parse::<f64>()
        .map(|f| f as i64) // int(float(screen))
        .map_err(|_| format!("invalid DISPLAY screen {screen:?}"))?;
    let port = 6000 + screen_num;
    let connection = format!(r"TCP\:{host}\:{port}");
    Ok(TunnelSpec::Tcp {
        display: format!("localhost:{screen}"),
        listen_argv: vec![
            "socat".to_string(),
            format!("TCP-LISTEN:{port},fork,reuseaddr"),
            format!("EXEC:'mnexec -a 1 socat STDIO {connection}'"),
        ],
    })
}

// ---------------------------------------------------------------------
// Runtime spawners (root-gated by callers: started nets only)
// ---------------------------------------------------------------------

/// `util.quietRun`: run `cmd` via the shell, discarding all output.
pub fn quiet_run(cmd: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `term.py cleanUpScreens`: `errRun( "pkill -9 -f mnexec.*socat" )`.
pub fn clean_up_screens() {
    let _ = Command::new("sh")
        .arg("-c")
        .arg("pkill -9 -f mnexec.*socat")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `term.py tunnelX11`: resolve the tunnel for `node`.
/// Returns the display string the terminal should use, or `None` (after
/// printing `Error: Cannot connect to display` on `err`) when there is no
/// display. A spawned socat listener is pushed onto `terms`.
pub fn tunnel_x11(
    node: &mut NodeRefMut<'_>,
    display: Option<&str>,
    err: &mut dyn Write,
    terms: &mut Vec<Child>,
) -> Option<String> {
    let Some(display) = display else {
        let _ = writeln!(err, "Error: Cannot connect to display");
        return None;
    };
    match tunnel_spec(display) {
        Ok(TunnelSpec::Local(disp)) => {
            // GDM3 doesn't put credentials in .Xauthority, so allow root
            // to just connect.
            quiet_run("xhost +si:localuser:root");
            Some(disp)
        }
        Ok(TunnelSpec::Tcp {
            display: disp,
            listen_argv,
        }) => match node.node_mut().popen(&listen_argv) {
            Ok(child) => {
                terms.push(child);
                Some(disp)
            }
            Err(e) => {
                // Python's Popen succeeds here and the mnexec child fails
                // later, silently; Rust reports the failure (documented).
                let _ = writeln!(err, "*** x11 tunnel spawn failed: {e}");
                None
            }
        },
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            None
        }
    }
}

/// `term.py makeTerm`: create the X11 tunnel (if any) and spawn the
/// terminal for `node`. Spawned processes are pushed onto `terms`.
pub fn make_term(
    node: &mut NodeRefMut<'_>,
    title: &str,
    term: &str,
    display: Option<&str>,
    cmd: &str,
    err: &mut dyn Write,
    terms: &mut Vec<Child>,
) {
    let name = node.node_mut().name.clone();
    let in_namespace = node.node_mut().in_namespace;
    let prefix = match term_cmd_prefix(term, &term_title(title, &name, in_namespace)) {
        Ok(p) => p,
        Err(msg) => {
            // term.py: error( 'invalid terminal type: %s' ); return None
            // (makeTerms would then crash on `terms += None`; we just skip).
            let _ = writeln!(err, "{msg}");
            return;
        }
    };
    let Some(disp) = tunnel_x11(node, display, err, terms) else {
        return; // makeTerm: `if display is None: return []`
    };
    let argv: Vec<String> = prefix.into_iter().chain(term_tail(&disp, cmd)).collect();
    match node.node_mut().popen(&argv) {
        Ok(child) => terms.push(child),
        Err(e) => {
            let _ = writeln!(err, "*** {term} spawn failed for {name}: {e}");
        }
    }
}

/// `term.py makeTerms`: `makeTerm` for each named node.
pub fn make_terms(
    net: &mut Mininet,
    names: &[String],
    title: &str,
    term: &str,
    display: Option<&str>,
    err: &mut dyn Write,
    terms: &mut Vec<Child>,
) {
    for name in names {
        if let Some(mut node) = net.get_mut(name) {
            make_term(&mut node, title, term, display, "bash", err, terms);
        }
    }
}

/// `term.py runX11`: tunnel + `node.popen( cmd )` for the `x` CLI command.
pub fn run_x11(
    node: &mut NodeRefMut<'_>,
    cmd: &[String],
    display: Option<&str>,
    err: &mut dyn Write,
    terms: &mut Vec<Child>,
) {
    let name = node.node_mut().name.clone();
    if tunnel_x11(node, display, err, terms).is_none() {
        return; // runX11: `if _display is None: return []`
    }
    if cmd.is_empty() {
        // Root-namespace nodes: Python's Popen([]) raises IndexError
        // (crashing the CLI); we report instead (documented).
        let _ = writeln!(err, "*** x: no command given for {name}");
        return;
    }
    match node.node_mut().popen(cmd) {
        Ok(child) => terms.push(child),
        Err(e) => {
            let _ = writeln!(err, "*** x spawn failed for {name}: {e}");
        }
    }
}

/// Port of `Mininet.startTerms` (`net.py`): the `--xterms` path —
/// controllers, switches, then hosts, each with its own title.
/// `display` is the already-resolved `$DISPLAY` (None = unset).
pub fn start_terms(
    net: &mut Mininet,
    display: Option<&str>,
    err: &mut dyn Write,
    terms: &mut Vec<Child>,
) {
    let Some(display) = display else {
        let _ = writeln!(err, "Error starting terms: Cannot connect to display");
        return;
    };
    // info( "*** Running terms on %s\n" % os.environ['DISPLAY'] ) — info
    // chatter goes to real stderr (same convention as repl's
    // '*** Starting CLI').
    eprintln!("*** Running terms on {display}");
    clean_up_screens();
    let controllers: Vec<String> = net.controllers.keys().map(str::to_string).collect();
    let switches: Vec<String> = net.switches.keys().map(str::to_string).collect();
    let hosts: Vec<String> = net.hosts.keys().map(str::to_string).collect();
    make_terms(
        net,
        &controllers,
        "controller",
        "xterm",
        Some(display),
        err,
        terms,
    );
    make_terms(net, &switches, "switch", "xterm", Some(display), err, terms);
    make_terms(net, &hosts, "host", "xterm", Some(display), err, terms);
}

/// Port of `Mininet.stopXterms` (`net.py stop()` calls it when xterms were
/// started): SIGKILL each spawned term, then remove the socat tunnels.
pub fn stop_xterms(terms: &mut Vec<Child>) {
    for term in terms.iter_mut() {
        let _ = term.kill();
    }
    terms.clear();
    clean_up_screens();
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_title_matches_term_py() {
        assert_eq!(term_title("Node", "h1", true), "\"Node: h1\"");
        assert_eq!(term_title("host", "h2", false), "\"host: h2\" (root)");
        assert_eq!(term_title("switch", "s1", false), "\"switch: s1\" (root)");
    }

    #[test]
    fn term_cmd_prefix_matches_term_py_cmds() {
        assert_eq!(
            term_cmd_prefix("xterm", "\"Node: h1\"").unwrap(),
            vec!["xterm", "-title", "\"Node: h1\"", "-display"]
        );
        assert_eq!(
            term_cmd_prefix("gterm", "\"Node: h1\"").unwrap(),
            vec!["gnome-terminal", "--title", "\"Node: h1\"", "--display"]
        );
        assert_eq!(
            term_cmd_prefix("bogus", "x").unwrap_err(),
            "invalid terminal type: bogus"
        );
    }

    #[test]
    fn term_tail_matches_term_py() {
        assert_eq!(
            term_tail(":0", "bash"),
            vec![":0", "-e", "env TERM=ansi bash"]
        );
        assert_eq!(
            term_tail("localhost:10.0", "bash"),
            vec!["localhost:10.0", "-e", "env TERM=ansi bash"]
        );
    }

    #[test]
    fn tunnel_spec_local_displays() {
        // host empty or 'unix' -> no tunnel, display unchanged
        assert_eq!(
            tunnel_spec(":0").unwrap(),
            TunnelSpec::Local(":0".to_string())
        );
        assert_eq!(
            tunnel_spec("unix:0").unwrap(),
            TunnelSpec::Local("unix:0".to_string())
        );
        assert_eq!(
            tunnel_spec(":10.0").unwrap(),
            TunnelSpec::Local(":10.0".to_string())
        );
    }

    #[test]
    fn tunnel_spec_tcp_builds_socat_argv_like_term_py() {
        // port = 6000 + int(float(screen)); connection = TCP\:<host>\:<port>
        let t = tunnel_spec("localhost:10.0").unwrap();
        assert_eq!(
            t,
            TunnelSpec::Tcp {
                display: "localhost:10.0".to_string(),
                listen_argv: vec![
                    "socat".to_string(),
                    "TCP-LISTEN:6010,fork,reuseaddr".to_string(),
                    "EXEC:'mnexec -a 1 socat STDIO TCP\\:localhost\\:6010'".to_string(),
                ],
            }
        );
        let t = tunnel_spec("10.0.0.1:2").unwrap();
        assert_eq!(
            t,
            TunnelSpec::Tcp {
                display: "localhost:2".to_string(),
                listen_argv: vec![
                    "socat".to_string(),
                    "TCP-LISTEN:6002,fork,reuseaddr".to_string(),
                    "EXEC:'mnexec -a 1 socat STDIO TCP\\:10.0.0.1\\:6002'".to_string(),
                ],
            }
        );
    }

    #[test]
    fn tunnel_spec_rejects_malformed_display() {
        // Python raises ValueError (crash) here; we return Err.
        assert!(tunnel_spec("noColon").is_err());
        assert!(tunnel_spec("a:b:c").is_err());
        assert!(tunnel_spec("host:abc").is_err());
    }

    #[test]
    fn start_terms_without_display_prints_python_error_line() {
        let mut net = mn_net::Mininet::dry_run(
            "single,3",
            mn_net::MininetOptions {
                controller: None,
                ..Default::default()
            },
        )
        .unwrap();
        let mut err: Vec<u8> = Vec::new();
        let mut terms: Vec<Child> = Vec::new();
        start_terms(&mut net, None, &mut err, &mut terms);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Error starting terms: Cannot connect to display\n"
        );
        assert!(terms.is_empty());
    }
}
