//! Golden-output tests for the task 10c interactive loop (`mn::repl`, the
//! port of `mininet/cli.py`'s `CLI` class): each test drives
//! `repl::run_with_io` with a scripted stdin against a dry-run-built net
//! (non-root) and compares `out`/`err` byte-for-byte with the expected text.
//!
//! The expected strings were captured from real `python3` sessions driving
//! `mininet.cli.CLI` against a non-started `Mininet` (shells stubbed, logger
//! redirected per `scripts/diff_topology.sh`'s convention; `mininet> `
//! prompts stripped because the loop only prints the prompt on a tty — see
//! repl.rs docs). The prompt-on-tty behavior is covered separately.
//!
//! Sessions:
//! - single3: `Mininet::dry_run("single,3")` with no controller — the Python
//!   capture used `SingleSwitchTopo(3)`, `controller=None`, `listenPort=6654`.
//! - isolated: two hosts (`h1`, `h2`) with no links — the Python capture used
//!   a custom `Topo` with just `addHost('h1'); addHost('h2')`. Hosts without
//!   intfs make the ping family deterministic (Python's `ping()` skips the
//!   `cmd()` when `dest.intfs` is empty, yielding `X` + "No packets sent").

use mn::repl;
use mn_net::{Mininet, MininetOptions};
use mn_topo::Attrs;

/// `single,3` dry-run net, no controller (matches the Python capture).
fn single3() -> Mininet {
    Mininet::dry_run(
        "single,3",
        MininetOptions {
            controller: None,
            listen_port: Some(6654),
            ..Default::default()
        },
    )
    .unwrap()
}

/// Two isolated hosts h1/h2, no links (matches the Python "isolated" topo).
fn isolated() -> Mininet {
    let mut net = Mininet::new(MininetOptions {
        build: false,
        controller: None,
        ..Default::default()
    })
    .unwrap();
    net.add_host("h1", &Attrs::new()).unwrap();
    net.add_host("h2", &Attrs::new()).unwrap();
    net.build().unwrap();
    net
}

/// Drive the repl with `script` as stdin; returns `(out, err, exit_code)`.
fn session(net: &mut Mininet, script: &str) -> (String, String, i32) {
    session_tty(net, script, false)
}

fn session_tty(net: &mut Mininet, script: &str, tty: bool) -> (String, String, i32) {
    session_tty_display(net, script, tty, None)
}

fn session_tty_display(
    net: &mut Mininet,
    script: &str,
    tty: bool,
    display: Option<&str>,
) -> (String, String, i32) {
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = repl::run_with_io(net, &mut input, &mut out, &mut err, tty, display);
    (
        String::from_utf8(out).expect("out is UTF-8"),
        String::from_utf8(err).expect("err is UTF-8"),
        code,
    )
}

/// The bare-`help` output captured from the real Python CLI (prompts
/// stripped) — listing + `helpStr`.
const HELP_GOLDEN: &str = "\n\
Documented commands (type help <topic>):\n\
========================================\n\
EOF    gterm  iperfudp  nodes        pingpair      py      switch  xterm\n\
dpctl  help   link      noecho       pingpairfull  quit    time  \n\
dump   intfs  links     pingall      ports         sh      wait  \n\
exit   iperf  net       pingallfull  px            source  x     \n\
\n\
You may also send a command to a node using:\n  <node> command {args}\n\
For example:\n  mininet> h1 ifconfig\n\
\n\
The interpreter automatically substitutes IP addresses\n\
for node names when a node is the first arg, so commands\n\
like\n  mininet> h2 ping h3\n\
should work.\n\
\n\
Some character-oriented interactive commands require\n\
noecho:\n  mininet> noecho h2 vi foo.py\n\
However, starting up an xterm/gterm is generally better:\n  mininet> xterm h2\n\
\n";

/// Golden: `nodes net links dump ports intfs sh help exit` on single,3 —
/// captured from a real Python CLI session on the same net (prompts
/// stripped).
#[test]
fn golden_single3_listing_commands_and_help() {
    let mut net = single3();
    let script = "nodes\nnet\nlinks\ndump\nports\nintfs\nsh echo hello-from-sh\nhelp\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "");
    let expected = concat!(
        "available nodes are: \nh1 h2 h3 s1\n",
        "h1 h1-eth0:s1-eth1\nh2 h2-eth0:s1-eth2\nh3 h3-eth0:s1-eth3\n",
        "s1 lo:  s1-eth1:h1-eth0 s1-eth2:h2-eth0 s1-eth3:h3-eth0\n",
        // links: Python output(link, status, '\n') -> trailing space per line
        "h1-eth0<->s1-eth1 (MISSING MISSING) \n",
        "h2-eth0<->s1-eth2 (MISSING MISSING) \n",
        "h3-eth0<->s1-eth3 (MISSING MISSING) \n",
        "<Host h1: h1-eth0:10.0.0.1 pid=None> \n",
        "<Host h2: h2-eth0:10.0.0.2 pid=None> \n",
        "<Host h3: h3-eth0:10.0.0.3 pid=None> \n",
        "<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None,s1-eth3:None pid=None> \n",
        "s1 lo:0 s1-eth1:1 s1-eth2:2 s1-eth3:3 \n",
        "h1: h1-eth0\nh2: h2-eth0\nh3: h3-eth0\ns1: lo,s1-eth1,s1-eth2,s1-eth3\n",
        "hello-from-sh\n",
    );
    assert_eq!(out, format!("{expected}{HELP_GOLDEN}"));
}

/// Golden: error-level commands on single,3 — captured from a real Python
/// CLI session run with the logger on stderr at level 'error'.
#[test]
fn golden_single3_error_commands() {
    let mut net = single3();
    let script = "bogus arg1\n\
                  help bogus\n\
                  route\n\
                  iperf h1\n\
                  iperf h1 nope\n\
                  iperfudp h1 h2\n\
                  dpctl\n\
                  xterm\n\
                  xterm nope\n\
                  source\n\
                  source a b\n\
                  exit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    // `help bogus` writes to Cmd's stdout (Python: "*** No help on bogus").
    assert_eq!(out, "*** No help on bogus\n");
    assert_eq!(
        err,
        concat!(
            "*** Unknown command: bogus arg1\n",
            // cli.py has no do_route: Python prints the unknown-command line
            "*** Unknown command: route\n",
            "invalid number of args: iperf src dst\n",
            "node 'nope' not in network\n",
            "invalid number of args: iperfudp bw src dst\n",
            "bw examples: 10M\n",
            "usage: dpctl command [arg1] [arg2] ...\n",
            "usage: xterm node1 node2 ...\n",
            "node 'nope' not in network\n",
            "usage: source <file>\n",
            "usage: source <file>\n",
        )
    );
}

/// Golden: the ping family + listings on two isolated hosts (no intfs), where
/// Python's ping() never calls cmd() — fully deterministic non-root.
/// `ping h1 h2` is the task-listed convenience (stock cli.py would print
/// "*** Unknown command: ping h1 h2"); it runs mn.ping([h1, h2]).
#[test]
fn golden_isolated_ping_family_and_listings() {
    let mut net = isolated();
    let script =
        "pingall\npingall 2\npingpair\nping h1 h2\nnodes\nnet\nlinks\ndump\nintfs\nports\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "");
    let pingall =
        "*** Ping: testing ping reachability\nh1 -> X \nh2 -> X \n*** Warning: No packets sent\n";
    let pair = "h1 -> X \nh2 -> X \n*** Warning: No packets sent\n";
    let expected = format!(
        "{pingall}{pingall}{pair}{pair}available nodes are: \nh1 h2\nh1\nh2\n<Host h1:  pid=None> \n<Host h2:  pid=None> \nh1: \nh2: \n"
    );
    assert_eq!(out, expected);
}

/// Golden: EOF (no `exit` line) behaves like the `EOF` command: do_EOF prints
/// a newline via output() and exits.
#[test]
fn golden_eof_prints_newline_and_stops() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "nodes\n");
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(out, "available nodes are: \nh1 h2 h3 s1\n\n");
}

/// Golden: `source` runs a file through the dispatch and — like Python's
/// do_source, which ignores onecmd's return — an `exit` inside the file does
/// not stop the outer loop. Also: unreadable file -> error line, loop
/// continues (Python additionally crashes on inputFile.close(); we don't).
#[test]
fn golden_source_runs_file_and_ignores_inner_exit() {
    let dir = std::env::temp_dir().join(format!("mn-repl-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let inner = dir.join("inner.txt");
    std::fs::write(&inner, "bogus\nnodes\nexit\n").unwrap();
    let script = format!("source {}\nnodes\nexit\n", inner.display());
    let mut net = single3();
    let (out, err, code) = session(&mut net, &script);
    assert_eq!(code, 0);
    assert_eq!(err, "*** Unknown command: bogus\n");
    assert_eq!(
        out,
        "available nodes are: \nh1 h2 h3 s1\navailable nodes are: \nh1 h2 h3 s1\n"
    );

    let mut net = single3();
    let script = "source /nonexistent-mn-repl-file.txt\nnodes\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "error reading file /nonexistent-mn-repl-file.txt\n");
    assert_eq!(out, "available nodes are: \nh1 h2 h3 s1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Golden: `precmd` strips `#` comments (even from `sh`), empty lines do
/// nothing — captured from Python.
#[test]
fn golden_comments_and_empty_lines() {
    let mut net = single3();
    let (out, err, code) = session(
        &mut net,
        "nodes # trailing comment\n# full comment line\n\nexit\n",
    );
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(out, "available nodes are: \nh1 h2 h3 s1\n");
}

/// Golden: `help <cmd>` / `?cmd` docstrings — captured from Python (CPython
/// 3.13+ dedents docstrings, which is what `help <cmd>` prints).
#[test]
fn golden_help_topics() {
    let mut net = single3();
    let script = "help nodes\nhelp iperf\nhelp sh\nhelp exit\n?pingall\nhelp bogus\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(
        out,
        concat!(
            "List all nodes.\n",
            "Simple iperf TCP test between two (optionally specified) hosts.\n",
            "Usage: iperf node1 node2\n",
            "Run an external shell command\n",
            "Usage: sh [cmd args]\n",
            "Exit\n",
            "Ping between all hosts.\n",
            "*** No help on bogus\n",
        )
    );
}

/// `xterm`/`gterm` with no `$DISPLAY` (task 10d): cli.py-exact
/// usage/not-in-network errors in arg order, then term.py's
/// `Error: Cannot connect to display` per valid node (captured from a real
/// Python CLI session with DISPLAY unset).
#[test]
fn golden_xterm_gterm_no_display() {
    let mut net = single3();
    let script = "xterm\nxterm nope\nxterm h1 nope\ngterm h1\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        concat!(
            "usage: xterm node1 node2 ...\n",
            "node 'nope' not in network\n",
            "Error: Cannot connect to display\n",
            "node 'nope' not in network\n",
            "Error: Cannot connect to display\n",
        )
    );
}

/// `dpctl status` prints the per-switch header (exact Python format:
/// `'*** ' + name + ' ' + '-'*72 + '\n'`) on `out`; on a dry-run net the
/// switch has no shell/datapath, so the dpctl call itself fails (content of
/// that failure is environment-dependent — ovs-ofctl may or may not exist —
/// so only the header is goldened here; the root test covers the live path).
#[test]
fn golden_dpctl_header_and_iperf_dry_run_error() {
    let mut net = single3();
    let (out, _, code) = session(&mut net, "dpctl status\nexit\n");
    assert_eq!(code, 0);
    let header = format!("*** s1 {}\n", "-".repeat(72));
    assert!(
        out.starts_with(&header),
        "out should start with the dpctl header {header:?}, got: {out:?}"
    );

    // iperf on a dry-run net: the server host has no shell -> NotRunning.
    let mut net = single3();
    let (out, err, code) = session(&mut net, "iperf h1 h2\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "*** iperf failed: node error: node \"h2\" is not running (shell exited or never started)\n"
    );
}

/// `pingallfull`/`pingpairfull` on a dry-run net: host cmd() fails and the
/// error text goes through Python's "could not parse ping output" path
/// (Python itself would crash with TypeError on a shell-less node, so this
/// dry-run rendering is a documented deviation; on a started net both print
/// real rtt lines).
#[test]
fn golden_pingfull_dry_run_error_path() {
    let mut net = isolated();
    let (out, err, code) = session(&mut net, "pingallfull\npingpairfull\nexit\n");
    assert_eq!(code, 0);
    let results = "*** Results: \n h1->h2: 1/0, rtt min/avg/max/mdev 0.000/0.000/0.000/0.000 ms\n h2->h1: 1/0, rtt min/avg/max/mdev 0.000/0.000/0.000/0.000 ms\n";
    let expected_out = format!(
        "*** Ping: testing ping reachability\nh1 -> X \nh2 -> X \n{results}h1 -> X \nh2 -> X \n{results}"
    );
    assert_eq!(out, expected_out);
    let one = "*** Error: could not parse ping output: node \"h1\" is not running (shell exited or never started)\n*** Error: could not parse ping output: node \"h2\" is not running (shell exited or never started)\n";
    assert_eq!(err, format!("{one}{one}"));
}

/// Convenience listings (10b format): sorted names per node kind.
#[test]
fn golden_hosts_switches_controllers_listings() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "hosts\nswitches\ncontrollers\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(out, "h1 h2 h3\ns1\n\n");
}

/// Unknown commands -> Python's `*** Unknown command: <line>` (incl. `!`
/// lines, which have no do_shell here).
#[test]
fn golden_unknown_commands() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "bogus arg1\n!ls\nroute\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "*** Unknown command: bogus arg1\n*** Unknown command: !ls\n*** Unknown command: route\n"
    );
}

/// The `mininet> ` prompt is written only when stdin is a tty (task spec;
/// CPython's input() would print it on pipes too — documented deviation).
#[test]
fn prompt_only_when_tty() {
    let mut net = single3();
    let (out, _, code) = session_tty(&mut net, "exit\n", false);
    assert_eq!(code, 0);
    assert_eq!(out, "");

    let mut net = single3();
    let (out, _, code) = session_tty(&mut net, "nodes\nexit\n", true);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "mininet> available nodes are: \nh1 h2 h3 s1\nmininet> "
    );
}

/// `quit` behaves like `exit`: no output, exit code 0.
#[test]
fn quit_exits_silently() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "quit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(err, "");
}

/// `sh` with an empty command runs an empty shell command silently
/// (Python: call('', shell=True)).
#[test]
fn golden_sh_empty_command() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "sh\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(err, "");
}
