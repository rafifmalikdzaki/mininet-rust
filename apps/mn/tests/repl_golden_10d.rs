//! Golden-output tests for task 10d — the remaining `mininet/cli.py`
//! commands ported in `apps/mn/src/repl.rs` (`link`, `switch`, `wait`,
//! `time`, `noecho`, `py`, `px`, `x`, `xterm`/`gterm` real spawns, and the
//! `default()` node passthrough) plus the `mininet/term.py` port in
//! `apps/mn/src/term.rs`.
//!
//! Expected strings were captured live from `python3` sessions driving
//! `mininet.cli.CLI` against a non-started (shell-stubbed) `Mininet` — the
//! same capture convention as `repl_golden.rs` (out = Cmd stdout +
//! OUTPUT/WARNING logger records, err = ERROR records, prompts stripped,
//! `use_rawinput=0`). Commands whose Python behavior is a crash on a
//! non-started net (node passthrough `h1 ping h2` → `sendCmd` assert,
//! `switch s1 start` on a dead OVS, `x nope` → `KeyError`) have no Python
//! golden; the Rust behavior there is asserted directly and documented as
//! a deviation in `repl.rs`. Real xterm/passthrough execution is root-gated
//! in `root_run.rs`.

use mn::repl;
use mn_net::{Mininet, MininetOptions};
use mn_topo::Attrs;

/// `single,3` dry-run net, no controller (matches the Python captures).
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

/// Drive the repl with `script` as stdin, no display; returns `(out, err, code)`.
fn session(net: &mut Mininet, script: &str) -> (String, String, i32) {
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = repl::run_with_io(net, &mut input, &mut out, &mut err, false, None);
    (
        String::from_utf8(out).expect("out is UTF-8"),
        String::from_utf8(err).expect("err is UTF-8"),
        code,
    )
}

/// Golden (captured): `link` arg validation + unknown-node/not-connected
/// errors on single,3, plus the dry-run `ifconfig` warn lines for a real
/// link (`link s1 h1 down`).
#[test]
fn golden_10d_link_errors_and_dry_run_warns() {
    let mut net = single3();
    let script = "link\n\
                  link h1\n\
                  link h1 h2\n\
                  link h1 h2 up extra\n\
                  link h1 h2 sideways\n\
                  link h1 nope up\n\
                  link nope h2 up\n\
                  link h1 h3 up\n\
                  link s1 h1 down\n\
                  exit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(
        err,
        concat!(
            "invalid number of args: link end1 end2 [up down]\n",
            "invalid number of args: link end1 end2 [up down]\n",
            "invalid number of args: link end1 end2 [up down]\n",
            "invalid number of args: link end1 end2 [up down]\n",
            "invalid type: link end1 end2 [up down]\n",
            "dst not in network: nope\n",
            "src not in network: nope\n",
            "src and dst not connected: h1 h3\n",
        )
    );
    // `link s1 h1 down` on a non-started net: Python's Intf.ifconfig ->
    // node.cmd warns "(<node> exited - ignoring cmd(...))" (warn -> out),
    // src end first, no error line (cmd returned None).
    assert_eq!(
        out,
        concat!(
            "(s1 exited - ignoring cmd('ifconfig', 's1-eth1', 'down'))\n",
            "(h1 exited - ignoring cmd('ifconfig', 'h1-eth0', 'down'))\n",
        )
    );
}

/// Golden (captured): `link h1 s1 up` runs ifconfig on h1's end first;
/// `link h1 h2 up` reports the not-connected error. `links` output is
/// unchanged (Python's Intf.status() only checks existence).
#[test]
fn golden_10d_link_up_and_links_unchanged() {
    let mut net = single3();
    let script = "links\nlink h1 h2 up\nlink h1 s1 up\nlinks\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "src and dst not connected: h1 h2\n");
    let links = "h1-eth0<->s1-eth1 (MISSING MISSING) \n\
                 h2-eth0<->s1-eth2 (MISSING MISSING) \n\
                 h3-eth0<->s1-eth3 (MISSING MISSING) \n";
    assert_eq!(
        out,
        format!(
            "{links}(h1 exited - ignoring cmd('ifconfig', 'h1-eth0', 'up'))\n\
             (s1 exited - ignoring cmd('ifconfig', 's1-eth1', 'up'))\n{links}"
        )
    );
}

/// Golden (captured): `switch` error paths on single,3 — including cli.py's
/// quirk of printing the *command* (`args[1]`) in `invalid switch:`.
#[test]
fn golden_10d_switch_errors() {
    let mut net = single3();
    let script = "switch\n\
                  switch s1\n\
                  switch s1 start stop\n\
                  switch nope start\n\
                  switch h1 start\n\
                  switch s1 bogus\n\
                  exit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        concat!(
            "invalid number of args: switch <switch name>{start, stop}\n",
            "invalid number of args: switch <switch name>{start, stop}\n",
            "invalid number of args: switch <switch name>{start, stop}\n",
            "invalid switch: start\n",
            "invalid switch: start\n",
            "invalid command: switch <switch name> {start, stop}\n",
        )
    );
}

/// Deviation (no Python golden — the Python capture crashes on a
/// non-started OVS switch only because `checkSetup` was stubbed; a real
/// non-root Python never reaches the CLI): `switch s1 start`/`stop` on a
/// dry-run net are silent (vsctl errors are swallowed into discarded output,
/// like Python's `cmd()` on a dead shell; `stop` is model-only).
#[test]
fn deviation_10d_switch_start_stop_dry_run_silent() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "switch s1 start\nswitch s1 stop\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(err, "");
}

/// Golden (captured): `noecho` on a non-tty stdin just runs
/// `default(arg)` — `noecho nodes` is `*** Unknown command: nodes` (the
/// arg, not the full line, reaches `default`).
#[test]
fn golden_10d_noecho_non_tty_is_default() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "noecho bogus\nnoecho nodes\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "*** Unknown command: bogus\n*** Unknown command: nodes\n"
    );
}

/// Golden (captured): `py`/`px` against the JSON-summary sandbox —
/// eval-result printing, repr for non-strings, exception text on `out`,
/// statement exec, persistent locals (`px x = 5` then `px print(x)`), and
/// the dry-run `node.cmd` warn line.
#[test]
fn golden_10d_py_px() {
    let mut net = single3();
    let script = "py len(net)\n\
                  py net.keys()\n\
                  py h1\n\
                  py h1.IP()\n\
                  py 1+1\n\
                  py \"str\"\n\
                  py None\n\
                  py [1, 2]\n\
                  py bogus_name\n\
                  py h1.cmd('ls')\n\
                  px print(\"hi\")\n\
                  px x = 5\n\
                  px print(x)\n\
                  px 1+\n\
                  px for h in net.hosts: print(h.name)\n\
                  exit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "");
    assert_eq!(
        out,
        concat!(
            "4\n",
            "['h1', 'h2', 'h3', 's1']\n",
            "<Host h1: h1-eth0:10.0.0.1 pid=None> \n",
            "10.0.0.1\n",
            "2\n",
            "str\n",
            // py None -> no output
            "[1, 2]\n",
            "name 'bogus_name' is not defined\n",
            "(h1 exited - ignoring cmd('ls',))\n",
            "hi\n",
            "5\n",
            "invalid syntax (<string>, line 1)\n",
            "h1\nh2\nh3\n",
        )
    );
}

/// `time` (captured shape; the elapsed value is nondeterministic so it is
/// pattern-matched): `time nodes` runs the command then prints
/// `*** Elapsed time: %0.6f secs`; `time` alone just prints the line;
/// `time boguscmd` errors on err but still prints the line; `time exit`
/// does NOT stop the loop (cli.py's do_time discards onecmd's return).
#[test]
fn golden_10d_time_wraps_onecmd() {
    let mut net = single3();
    let script = "time nodes\ntime\ntime boguscmd\ntime exit\nnodes\nexit\n";
    let (out, err, code) = session(&mut net, script);
    assert_eq!(code, 0);
    assert_eq!(err, "*** Unknown command: boguscmd\n");
    let lines: Vec<&str> = out.lines().collect();
    // nodes (2 lines), elapsed x4 (incl. 'time exit', which printed its
    // elapsed line but did NOT stop the loop), nodes (2 lines).
    assert_eq!(lines.len(), 8, "out: {out:?}");
    assert_eq!(lines[0], "available nodes are: ");
    assert_eq!(lines[1], "h1 h2 h3 s1");
    for line in &lines[2..6] {
        assert_elapsed_line(line);
    }
    assert_eq!(lines[6], "available nodes are: ");
    assert_eq!(lines[7], "h1 h2 h3 s1");
}

/// Assert `line` is a `*** Elapsed time: <f64 with 6 decimals> secs` line.
fn assert_elapsed_line(line: &str) {
    let Some(rest) = line.strip_prefix("*** Elapsed time: ") else {
        panic!("not an elapsed line: {line:?}");
    };
    let Some(num) = rest.strip_suffix(" secs") else {
        panic!("not an elapsed line: {line:?}");
    };
    let (int_part, frac) = num.split_once('.').expect("has a dot");
    assert!(!int_part.is_empty() && int_part.chars().all(|c| c.is_ascii_digit()));
    assert_eq!(frac.len(), 6, "%%0.6f -> 6 decimals: {line:?}");
    assert!(frac.chars().all(|c| c.is_ascii_digit()));
    num.parse::<f64>().expect("parses as f64");
}

/// Golden (captured): the `default()` passthrough prefix errors — a bare
/// node name and an unknown command.
#[test]
fn golden_10d_passthrough_prefix_errors() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "h1\nnope cmd\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "*** Please enter a command for node: h1 <cmd>\n*** Unknown command: nope cmd\n"
    );
}

/// Deviation (no Python golden — `sendCmd` asserts and crashes the Python
/// CLI on a non-started net): `h1 ping h2` reports the send failure on
/// `err` and the loop continues.
#[test]
fn deviation_10d_passthrough_dry_run_reports_error() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "h1 ping h2\nnodes\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "available nodes are: \nh1 h2 h3 s1\n");
    assert_eq!(
        err,
        "*** h1 cmd failed: node \"h1\" is not running (shell exited or never started)\n"
    );
}

/// Golden (captured with DISPLAY unset): `xterm`/`gterm`/`x` usage and
/// not-in-network errors in arg order, then term.py's
/// `Error: Cannot connect to display` per valid node.
#[test]
fn golden_10d_xterm_gterm_x_no_display() {
    let mut net = single3();
    let script = "xterm\nxterm nope\nxterm h1 nope\ngterm\ngterm h1\nx\nx h1\nexit\n";
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
            "usage: gterm node1 node2 ...\n",
            "Error: Cannot connect to display\n",
            "usage: x node [cmd args]...\n",
            "Error: Cannot connect to display\n",
        )
    );
}

/// Deviation (Python crashes with `KeyError` on `self.mn[args[0]]`): `x
/// nope` prints the standard not-in-network line.
#[test]
fn deviation_10d_x_unknown_node() {
    let mut net = single3();
    let (out, err, code) = session(&mut net, "x nope\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(err, "node 'nope' not in network\n");
}

/// Golden (captured): `wait` on a switch-less net returns immediately; the
/// `info` chatter goes to real stderr (not the golden `out`/`err` surface).
#[test]
fn golden_10d_wait_no_switches_returns_immediately() {
    let mut net = isolated();
    let (out, err, code) = session(&mut net, "wait\nexit\n");
    assert_eq!(code, 0);
    assert_eq!(out, "");
    assert_eq!(err, "");
}
