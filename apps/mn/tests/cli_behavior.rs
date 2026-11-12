//! Behavioral tests for the wired `mn` (task 10b): `--help`/`--version`
//! work, `--dump` works non-root via the dry-run path (exit 0), and anything
//! that would really start/stop a network is root-gated — as non-root it
//! exits non-zero with a clear "requires root" message.

mod support;

use support::run_mn;

#[test]
fn version_prints_bare_mininet_version_and_exits_zero() {
    let output = run_mn(&["--version"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "2.3.1b4\n");
}

#[test]
fn help_exits_zero() {
    let output = run_mn(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: mn"),
        "help should show usage:\n{stdout}"
    );
    assert!(
        stdout.contains("--topo"),
        "help should list --topo:\n{stdout}"
    );
}

#[test]
fn short_help_flag_works_like_optparse() {
    let output = run_mn(&["-h"]);
    assert!(output.status.success());
}

#[test]
fn real_start_stop_is_root_gated_non_root_gets_clear_error() {
    // This test runs in the non-root workspace gate: bare `mn`, a topo
    // build, `--test` and `--clean` must all exit non-zero with a clear
    // "requires root" message (Python's ensureRoot equivalent).
    for argv in [
        vec!["mn"],
        vec!["mn", "--topo", "tree,2,2", "--link", "tc,bw=10,delay=5ms"],
        vec!["mn", "--test", "pingall"],
        vec!["mn", "--clean"],
    ] {
        let output = run_mn(&argv[1..]);
        assert!(
            !output.status.success(),
            "{argv:?} should exit non-zero when non-root"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("requires root"),
            "{argv:?} should explain it requires root, stderr: {stderr}"
        );
    }
}

#[test]
fn dump_works_non_root_via_dry_run_and_exits_zero() {
    // scripts/diff_topology.sh probes exactly this: `mn --topo <spec> ... --dump`
    // must exit 0 non-root and print the plan/dump/links sections on stdout.
    for spec in ["single,3", "linear,4", "tree,2,2"] {
        let output = run_mn(&[
            "--topo",
            spec,
            "--switch",
            "default",
            "--host",
            "proc",
            "--controller",
            "none",
            "--link",
            "default",
            "--ipbase",
            "10.0.0.0/8",
            "--dump",
        ]);
        assert!(
            output.status.success(),
            "mn --topo {spec} --dump should exit 0 non-root, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.starts_with("=== topo ===\n"), "{spec}: {stdout}");
        assert!(stdout.contains("=== dump ===\n"), "{spec}: {stdout}");
        assert!(stdout.contains("=== links ===\n"), "{spec}: {stdout}");
    }
}

#[test]
fn unsupported_options_fail_with_clear_message() {
    for argv in [
        vec!["--custom", "foo.py"],
        vec!["--pre", "pre.txt"],
        vec!["--post", "post.txt"],
        vec!["--cluster", "srv1,srv2"],
        vec!["--innamespace"],
    ] {
        let output = run_mn(&argv);
        assert!(!output.status.success(), "{argv:?} should exit non-zero");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not ported") || stderr.contains("TODO"),
            "{argv:?} should explain it is not ported, stderr: {stderr}"
        );
    }
}

#[test]
fn bad_topo_spec_exits_nonzero_with_error() {
    let output = run_mn(&["--topo", "bogus", "--dump"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bogus"), "stderr: {stderr}");
}

#[test]
fn unknown_option_is_an_error() {
    let output = run_mn(&["--bogus"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--bogus"),
        "error should name the unknown option, stderr: {stderr}"
    );
}

#[test]
fn nat_flag_builds_nat0_in_dry_run_dump() {
    // --nat is wired: nat0 appears as a host connected to the first switch,
    // and hosts get defaultRoute via the NAT IP (visible in the dump repr).
    let output = run_mn(&[
        "--topo",
        "single,2",
        "--controller",
        "none",
        "--nat",
        "--dump",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("nat0"),
        "dump should contain nat0: {stdout}"
    );
    assert!(
        stdout.contains("nat0-eth0<->s1-eth"),
        "nat link missing: {stdout}"
    );
}
