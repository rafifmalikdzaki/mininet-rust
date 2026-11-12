//! Root-gated end-to-end test for the wired `mn` (task 10b): a real
//! `single,3` run with `--test pingall` on a privileged host.
//!
//! Run on a root Linux host with netns support and OVS installed:
//!   MN_RUST_ROOT_TESTS=1 cargo test -p mn --test root_run -- --ignored --test-threads=1
//!
//! (Same convention as mn-net/mn-node/mn-link: `#[ignore]` **and** the
//! MN_RUST_ROOT_TESTS=1 gate, so the normal non-root `cargo test --workspace`
//! never attempts it.)

mod support;

use support::run_mn;

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
