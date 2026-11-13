//! `mn` — Rust port of Mininet's `bin/mn` runner.
//!
//! Task 10b: the clap CLI ([`mnr::cli`]) is wired to `mn-net`
//! ([`mnr::runner`]): `--topo <spec>` builds the net, `--dump` prints the
//! plan/dump/links sections (non-root via the dry-run path), `--test` runs
//! the built-in tests (root), `--clean` cleans up (root), and with no
//! `--test`/`--xterms` it enters the interactive loop iff stdin is a tty,
//! else dump+stop. `--version` prints the bare Mininet version and exits 0.

use clap::Parser;

use mnr::cli::MnArgs;
use mnr::runner;

fn main() {
    let args = MnArgs::parse();
    std::process::exit(runner::run(&args));
}
