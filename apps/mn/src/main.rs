//! `mn` — Rust port of Mininet's `bin/mn` runner.
//!
//! Task 10 (first slice): PARSING ONLY. This binary parses the command-line
//! argument surface ported from `bin/mn` (see [`cli`]), handles `--help`
//! (clap) and `--version` (prints the Mininet version string, like
//! `bin/mn`'s `version()` callback), and reports "not yet implemented" —
//! exiting non-zero — for anything that would start actual emulation.

use clap::Parser;

use mn::cli::{MnArgs, VERSION};

fn main() {
    let args = MnArgs::parse();

    // `bin/mn --version` prints the bare version string and exits 0.
    if args.version {
        println!("{VERSION}");
        return;
    }

    // Everything else would create/configure a network in `bin/mn`
    // (including `--clean`, which mutates the system). Not wired up yet.
    eprintln!("mn: not yet implemented: this Rust port parses command-line arguments only;");
    eprintln!("mn: network emulation (topo/link/host/controller/netlink) is not wired up yet.");
    eprintln!("mn: parsed args: {args:?}");
    let topo = args.topo_spec();
    eprintln!(
        "mn: parsed --topo spec: name={:?}, positional params={:?}, key=value params={:?}",
        topo.name,
        topo.args(),
        topo.kwargs()
    );
    std::process::exit(1);
}
