//! `mn` — Rust port of Mininet's `bin/mn` runner.
//!
//! Task 10b: the clap CLI ([`mn::cli`]) is wired to `mn-net`
//! ([`mn::runner`]): `--topo <spec>` builds the net, `--dump` prints the
//! plan/dump/links sections (non-root via the dry-run path), `--test` runs
//! the built-in tests (root), `--clean` cleans up (root), and with no
//! `--test`/`--xterms` it enters the interactive loop iff stdin is a tty,
//! else dump+stop. `--version` prints the bare Mininet version and exits 0.

use clap::Parser;

use mn::cli::MnArgs;
use mn::runner;

fn main() {
    let args = MnArgs::parse();
    std::process::exit(runner::run(&args));
}
