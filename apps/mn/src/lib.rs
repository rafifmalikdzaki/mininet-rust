//! Library target of the `mn` package: the command-line argument surface
//! ported from `bin/mn` (parsing only — see [`cli`]).
//!
//! Kept in a lib target (with a thin `main.rs` binary) so the ported parsing
//! API is public and unit-testable without dead-code warnings in the bin.

pub mod cli;
