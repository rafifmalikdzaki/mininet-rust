//! Behavioral tests for the parsing-only slice of the Rust `mn`:
//! `--help`/`--version` work, and anything that would start emulation exits
//! non-zero with a clear "not yet implemented" message.

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
fn emulation_invocation_is_not_yet_implemented_and_exits_nonzero() {
    for argv in [
        vec!["mn"],
        vec!["mn", "--topo", "tree,2,2", "--link", "tc,bw=10,delay=5ms"],
        vec!["mn", "--test", "pingall"],
        vec!["mn", "--clean"],
    ] {
        let output = run_mn(&argv[1..]);
        assert!(
            !output.status.success(),
            "{argv:?} should exit non-zero (parsing-only slice)"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not yet implemented"),
            "{argv:?} should explain it is not yet implemented, stderr: {stderr}"
        );
    }
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
