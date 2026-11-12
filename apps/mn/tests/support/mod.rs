//! Shared helpers for `apps/mn` integration tests.
//!
//! Not every test binary uses every helper, so dead-code warnings are
//! allowed for this shared module.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path to the compiled `mn` binary under test.
pub fn mn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mn")
}

/// Run `mn` with the given args, capturing output.
pub fn run_mn(args: &[&str]) -> Output {
    Command::new(mn_bin())
        .args(args)
        .output()
        .expect("failed to spawn mn binary")
}

/// Directory of this crate's `tests/` tree (via CARGO_MANIFEST_DIR).
pub fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

pub fn read_test_file(rel: &str) -> String {
    let path = tests_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Strip ANSI escape sequences (in case color is ever forced on).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // skip until the terminating 'm' of a CSI sequence
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whitespace normalization for golden comparisons: ANSI-stripped, then
/// tokenized on whitespace so terminal-width-dependent line wrapping of the
/// help text does not affect the comparison.
pub fn normalize_ws(s: &str) -> Vec<String> {
    strip_ansi(s)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Extract the set of long option names (`--foo` -> `foo`) from a help text.
///
/// Only recognizes option flags at the start of a help entry line (after
/// leading whitespace), so prose mentioning `--ipbase` inside another
/// option's help does not produce false positives unless it is itself a real
/// option name.
pub fn long_option_names(help: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in help.lines() {
        let mut rest = line.trim_start();
        loop {
            if let Some(long) = rest.strip_prefix("--") {
                let name: String = long
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                if !name.is_empty() {
                    names.insert(name);
                }
                break;
            } else if rest.starts_with('-')
                && rest.len() > 1
                && rest.as_bytes()[1].is_ascii_alphabetic()
            {
                // short flag, e.g. `-c`, `-i IPBASE, --ipbase=...`
                rest = &rest[2..];
                // skip an attached value/metavar up to the separating comma
                let skip = rest.find(',').unwrap_or(rest.len());
                rest = rest[skip..].trim_start();
                if let Some(r) = rest.strip_prefix(',') {
                    rest = r.trim_start();
                    continue;
                }
                break;
            } else {
                break;
            }
        }
    }
    names
}
