//! Root-gated end-to-end tests for the wired `mn` (tasks 10b/10c): a real
//! `single,3` run with `--test pingall`, and the interactive `cli.py` loop
//! (`mnr::repl`) driven with a scripted stdin against the started net, on a
//! privileged host.
//!
//! Run on a root Linux host with netns support and OVS installed:
//!   MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1
//!
//! (Same convention as mn-net/mn-node/mn-link: `#[ignore]` **and** the
//! MN_RUST_ROOT_TESTS=1 gate, so the normal non-root `cargo test --workspace`
//! never attempts it.)

mod support;

use support::run_mn;

/// Root-gated interactive-loop test (task 10c): build + start a real
/// `single,3` net, then drive `mnr::repl::run_with_io` with a scripted stdin
/// (`nodes`/`dump`/`links`/`pingall`/`ping h1 h2`/`exit`) and check the
/// cli.py-format output — real pids in `dump`, `(OK OK)` links, and a 0%-loss
/// `pingall`.
#[test]
#[ignore = "requires root + netns + OVS on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_repl_interactive_loop_single3() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    use mn_net::{Mininet, MininetOptions};

    let mut net = Mininet::dry_run(
        "single,3",
        MininetOptions {
            controller: None,
            listen_port: Some(6654),
            ..Default::default()
        },
    )
    .unwrap();
    net.start().unwrap();
    // `pingall 2` bounds each ping with -W 2s so a blackholed datapath fails
    // fast instead of hanging the test step (Python has the same option).
    let script = "nodes\ndump\nlinks\npingall 2\nping h1 h2\nexit\n";
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = mnr::repl::run_with_io(&mut net, &mut input, &mut out, &mut err, false, None);
    let out = String::from_utf8(out).expect("out is UTF-8");
    let err = String::from_utf8(err).expect("err is UTF-8");
    let stop = net.stop();
    assert_eq!(code, 0);
    stop.unwrap();
    assert!(
        out.starts_with("available nodes are: \nh1 h2 h3 s1\n"),
        "out: {out:?}"
    );
    // dump carries real pids on a started net
    assert!(
        out.contains("<Host h1: h1-eth0:10.0.0.1 pid="),
        "out: {out:?}"
    );
    assert!(!out.contains("pid=None"), "out: {out:?}");
    // links report OK after attach
    assert!(out.contains("h1-eth0<->s1-eth1 (OK OK) \n"), "out: {out:?}");
    // pingall: 6 pings on single,3, 0% loss; ping h1 h2: 2 pings, 0% loss
    assert!(
        out.contains("*** Results: 0% dropped (6/6 received)"),
        "out: {out:?}"
    );
    assert!(
        out.contains("*** Results: 0% dropped (2/2 received)"),
        "out: {out:?}"
    );
    assert_eq!(err, "", "err: {err:?}");
}

#[test]
#[ignore = "requires root + netns + OVS on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_single3_pingall_end_to_end() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    // mn --topo single,3 --controller none --test pingall: build, start
    // (OVSSwitch via the SwitchKind seam), pingall, stop.
    let output = run_mn(&[
        "--topo",
        "single,3",
        "--controller",
        "none",
        "--test",
        "pingall",
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "mn --topo single,3 --test pingall failed (rc={:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("*** Ping: testing ping reachability"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("*** Results: 0% dropped (6/6 received)"),
        "pingall should be 0% loss on single,3 (6 pings), stdout: {stdout}"
    );
}

#[test]
#[ignore = "requires root + netns + OVS on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_single3_dump_real_start_shows_pids_and_ok_links() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    // Root --dump: real start -> dump has real pids, links report OK OK.
    let output = run_mn(&["--topo", "single,3", "--controller", "none", "--dump"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "rc={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("<OVSSwitch s1: lo:127.0.0.1,") && !stdout.contains("pid=None"),
        "root dump should have real pids, stdout: {stdout}"
    );
    assert!(
        stdout.contains("h1-eth0<->s1-eth1 (OK OK)"),
        "root links should be OK after attach, stdout: {stdout}"
    );
}

/// Root-gated (task 10d): the `default()` node passthrough on a *started*
/// net — `h1 ping -c 1 h2` substitutes h2's IP (10.0.0.2) and streams the
/// node's output to `out`; a bare node name and an unknown command hit the
/// golden error lines. Also `link h1 s1 down`/`up` (real ifconfig in h1's
/// namespace) and `wait` on a switch-less started net.
#[test]
#[ignore = "requires root + netns + OVS on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_10d_passthrough_link_wait_on_started_net() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    use mn_net::{Mininet, MininetOptions};

    let mut net = Mininet::dry_run(
        "single,3",
        MininetOptions {
            controller: None,
            listen_port: Some(6654),
            ..Default::default()
        },
    )
    .unwrap();
    net.start().unwrap();
    let script =
        "h1 ping -c 1 -W 2 h2\nh1\nnope cmd\nlink h1 s1 down\nlink h1 s1 up\nlinks\nexit\n";
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = mnr::repl::run_with_io(&mut net, &mut input, &mut out, &mut err, false, None);
    let out = String::from_utf8(out).expect("out is UTF-8");
    let err = String::from_utf8(err).expect("err is UTF-8");
    let stop = net.stop();
    assert_eq!(code, 0);
    stop.unwrap();
    // passthrough: h2 substituted by its IP, real ping output on `out`
    assert!(
        out.contains("ping -c 1 -W 2 10.0.0.2") || out.contains("PING 10.0.0.2"),
        "passthrough should run 'ping -c 1 -W 2 10.0.0.2' on h1, out: {out:?}"
    );
    assert!(
        out.contains("1 received") || out.contains("1 packets received"),
        "out: {out:?}"
    );
    // error lines from the passthrough prefix
    assert!(
        err.contains("*** Please enter a command for node: h1 <cmd>\n"),
        "err: {err:?}"
    );
    assert!(
        err.contains("*** Unknown command: nope cmd\n"),
        "err: {err:?}"
    );
    // link down/up ran real ifconfig (no warn lines, no failure lines)
    assert!(!err.contains("status change failed"), "err: {err:?}");
    assert!(!out.contains("exited - ignoring"), "out: {out:?}");
    // links still render OK OK (Python's Intf.status() only checks existence)
    assert!(out.contains("h1-eth0<->s1-eth1 (OK OK) \n"), "out: {out:?}");
}

/// Root-gated (task 10d): `wait` on a started net with no switches returns
/// immediately (info chatter on real stderr, golden surface empty); `xterm`
/// without a display prints term.py's error line and spawns nothing.
#[test]
#[ignore = "requires root + netns on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_10d_wait_and_xterm_no_display() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    use mn_net::{Mininet, MininetOptions};
    use mn_topo::Attrs;

    let mut net = Mininet::new(MininetOptions {
        build: false,
        controller: None,
        ..Default::default()
    })
    .unwrap();
    net.add_host("h1", &Attrs::new()).unwrap();
    net.add_host("h2", &Attrs::new()).unwrap();
    net.build().unwrap();
    net.start().unwrap();
    let script = "wait\nxterm h1\nx h1 echo hi\nexit\n";
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = mnr::repl::run_with_io(&mut net, &mut input, &mut out, &mut err, false, None);
    let out = String::from_utf8(out).expect("out is UTF-8");
    let err = String::from_utf8(err).expect("err is UTF-8");
    let stop = net.stop();
    assert_eq!(code, 0);
    stop.unwrap();
    assert_eq!(out, "");
    // no switches -> wait returns immediately; no DISPLAY -> term.py error
    assert_eq!(
        err,
        "Error: Cannot connect to display\nError: Cannot connect to display\n"
    );
}

/// Root-gated (task 10d): `--xterms` on a started net without $DISPLAY
/// prints startTerms' error line on stderr and still runs the test.
#[test]
#[ignore = "requires root + netns on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1"]
fn root_10d_xterms_flag_no_display_errors_cleanly() {
    if !std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1") {
        return;
    }
    // Clear DISPLAY so start_terms takes term.py's no-display error path.
    let output = std::process::Command::new(support::mn_bin())
        .args([
            "--topo",
            "single,3",
            "--controller",
            "none",
            "--xterms",
            "--test",
            "none",
        ])
        .env_remove("DISPLAY")
        .output()
        .expect("failed to spawn mn");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error starting terms: Cannot connect to display"),
        "stderr: {stderr}"
    );
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
