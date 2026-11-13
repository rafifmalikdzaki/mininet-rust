//! Golden test: the clap-generated `--help` of the Rust `mn` must match the
//! checked-in golden file `tests/golden/help.txt` (whitespace-normalized,
//! so terminal-width wrapping does not matter).
//!
//! Regenerate the golden file with:
//!   MN_UPDATE_GOLDEN=1 cargo test -p mn --test help_golden

mod support;

use support::{normalize_ws, read_test_file, tests_dir};

const GOLDEN_REL: &str = "golden/help.txt";

#[test]
fn help_matches_golden_file() {
    let output = support::run_mn(&["--help"]);
    assert!(
        output.status.success(),
        "mn --help should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).expect("help output must be UTF-8");

    let golden_path = tests_dir().join(GOLDEN_REL);
    let golden = read_test_file(GOLDEN_REL);

    if std::env::var("MN_UPDATE_GOLDEN").is_ok() {
        std::fs::write(&golden_path, &help).expect("failed to update golden file");
        eprintln!("updated {}", golden_path.display());
        return;
    }

    let actual_tokens = normalize_ws(&help);
    let golden_tokens = normalize_ws(&golden);

    assert_eq!(
        actual_tokens, golden_tokens,
        "mn --help output differs from {GOLDEN_REL} (set MN_UPDATE_GOLDEN=1 to regenerate)"
    );
}

#[test]
fn help_lists_all_bin_mn_options() {
    // Sanity anchor for the golden file: the option set ported from bin/mn.
    let output = support::run_mn(&["--help"]);
    let help = String::from_utf8(output.stdout).unwrap();
    let names = support::long_option_names(&help);
    let expected: Vec<&str> = vec![
        "arp",
        "clean",
        "cluster",
        "controller",
        "custom",
        "help",
        "host",
        "innamespace",
        "ipbase",
        "link",
        "listenport",
        "mac",
        "nat",
        "nolistenport",
        "pin",
        "placement",
        "post",
        "pre",
        "switch",
        "test",
        "topo",
        "twait",
        "verbosity",
        "version",
        "wait",
        "xterms",
    ];
    for name in expected {
        assert!(
            names.contains(name),
            "mn --help is missing --{name}; has: {names:?}"
        );
    }
}
