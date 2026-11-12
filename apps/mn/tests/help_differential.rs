//! Differential test: the set of long option names in the Rust `mn --help`
//! must equal the set in the captured `python3 bin/mn --help` output
//! (`tests/data/python_mn_help.txt`, captured from the reference
//! implementation on the `rust-migration` branch).

mod support;

use std::collections::BTreeSet;

use support::{long_option_names, read_test_file};

#[test]
fn same_option_names_as_python_bin_mn() {
    let rust_output = support::run_mn(&["--help"]);
    assert!(rust_output.status.success());
    let rust_help = String::from_utf8(rust_output.stdout).expect("UTF-8 help");

    let python_help = read_test_file("data/python_mn_help.txt");

    let rust_opts = long_option_names(&rust_help);
    let python_opts = long_option_names(&python_help);

    let missing_in_rust: Vec<_> = python_opts.difference(&rust_opts).collect();
    let extra_in_rust: Vec<_> = rust_opts.difference(&python_opts).collect();

    assert!(
        missing_in_rust.is_empty(),
        "options present in `python3 bin/mn --help` but missing from Rust mn --help: {missing_in_rust:?}\n\
         python help:\n{python_help}"
    );
    // The Rust port adds exactly one harness-only flag beyond bin/mn's
    // optparse surface: `--dump`, probed by scripts/diff_topology.sh
    // (`mn --topo <spec> ... --dump` must be accepted and exit 0).
    let allowed_extra: BTreeSet<&str> = ["dump"].into_iter().collect();
    let unexpected_extra: Vec<_> = extra_in_rust
        .iter()
        .filter(|o| !allowed_extra.contains(o.as_str()))
        .collect();
    assert!(
        unexpected_extra.is_empty(),
        "options present in Rust mn --help but not in `python3 bin/mn --help` \
         (beyond the allowed harness-only --dump): {unexpected_extra:?}"
    );

    // Guard against a broken extractor silently comparing two empty sets.
    assert!(
        python_opts.len() >= 25,
        "expected at least 25 options from python help, got {python_opts:?}"
    );
}

#[test]
fn python_version_matches_rust_version_constant() {
    // `python3 bin/mn --version` output captured in data/python_mn_version.txt.
    // (bin/mn prints via mininet.log.output, i.e. stderr; the Rust port prints
    // the same version string on stdout, so compare combined output.)
    let python_version = read_test_file("data/python_mn_version.txt");
    let output = support::run_mn(&["--version"]);
    assert!(output.status.success());
    let mut rust_version = String::from_utf8_lossy(&output.stdout).into_owned();
    rust_version.push_str(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        rust_version.trim_end(),
        python_version.trim_end(),
        "Rust `mn --version` must print the same version string as `bin/mn --version`"
    );
}
