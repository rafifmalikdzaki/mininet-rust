//! Library target of the `mn` package: the command-line argument surface
//! ported from `bin/mn` ([`cli`]) and the wiring to `mn-net` ([`runner`],
//! with the minimal interactive loop in [`repl`]) — task 10b.
//!
//! Kept in a lib target (with a thin `main.rs` binary) so the ported API is
//! public and unit-testable without dead-code warnings in the bin.

pub mod cli;
pub mod repl;
pub mod runner;
