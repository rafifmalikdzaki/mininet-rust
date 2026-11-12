//! Root-gated end-to-end tests for the wired `mn` (tasks 10b/10c): a real
//! `single,3` run with `--test pingall`, and the interactive `cli.py` loop
//! (`mn::repl`) driven with a scripted stdin against the started net, on a
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
/// `single,3` net, then drive `mn::repl::run_with_io` with a scripted stdin
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
    let script = "nodes\ndump\nlinks\npingall\nping h1 h2\nexit\n";
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = mn::repl::run_with_io(&mut net, &mut input, &mut out, &mut err, false);
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
