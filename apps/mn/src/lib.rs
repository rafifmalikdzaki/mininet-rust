//! Library target of the `mn` package: the command-line argument surface
//! ported from `bin/mn` ([`cli`]) and the wiring to `mn-net` ([`runner`],
//! with the interactive `mininet/cli.py` loop in [`repl`]) — tasks 10b/10c.
//!
//! Kept in a lib target (with a thin `main.rs` binary) so the ported API is
//! public and unit-testable without dead-code warnings in the bin.

pub mod cli;
pub mod repl;
pub mod runner;
pub mod term;
