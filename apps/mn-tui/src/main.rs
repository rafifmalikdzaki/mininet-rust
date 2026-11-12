//! `mn-tui` — OMO-style live graph TUI for the emulated network (task 13).
//!
//! The graph IS the UI: nodes as bordered boxes (green=up/ok,
//! red=missing/down, yellow=switch, cyan=controller), links as unicode
//! edges with status color, a detail pane for the selected node, a title
//! bar with the topo spec, and a footer with the keybindings.
//!
//! - `mn-tui --topo single,3` non-root: renders the dry-run plan, read-only.
//! - as root: builds + starts the net and refreshes live status every 1s
//!   (link status from `net.links()`, node status from pids); `q` stops
//!   the net.
//! - `mn-tui --topo single,3 --json`: dump the GraphView as JSON and exit.
//! - `mn-tui --topo single,3 --frames N [--size WxH]`: render N frames
//!   headless to stdout (CI / non-tty) and exit.

mod app;
mod cli;
mod headless;
mod tui;

use std::io::IsTerminal;

use clap::Parser;

use cli::Args;

fn main() {
    let args = Args::parse();
    let code = if args.headless() {
        match headless::run(&args) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("mn-tui: {e}");
                1
            }
        }
    } else if std::io::stdin().is_terminal() {
        tui::run(&args)
    } else {
        eprintln!(
            "mn-tui: interactive mode needs a TTY; use --json or --frames N for headless export"
        );
        2
    };
    std::process::exit(code);
}
