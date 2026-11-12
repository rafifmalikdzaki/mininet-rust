//! `mn-mcp` — MCP (Model Context Protocol) server core for mininet-rust
//! (task 14).
//!
//! Exposes the mininet-rust graph + simulation to any MCP-capable AI agent
//! over stdio (newline-delimited JSON-RPC 2.0). Register with e.g.
//! `vibe mcp add mn-mcp --transport stdio --command target/release/mn-mcp`.
//!
//! # Layout
//!
//! - [`jsonrpc`]: JSON-RPC 2.0 envelope helpers and error codes
//!   (-32700/-32600/-32601/-32602/-32603).
//! - [`server`]: [`Server`] — `initialize` / `notifications/initialized` /
//!   `ping` / `tools/list` / `tools/call` dispatch, with
//!   `run(reader, writer)` so tests drive it with in-memory pipes.
//! - [`session`]: [`Session`] — per-connection state holding the optional
//!   [`mn_net::Mininet`] built via mn-net's dry-run path (non-root safe).
//! - [`graph`]: the graph JSON shared by `topo_plan`/`graph`/`net_build`
//!   — the canonical [`mn_tui::GraphView`] shape (`nodes:[{id,label,kind,
//!   ip,status,detail?}], edges:[{a,b,status,label,intf1,intf2,port1,
//!   port2}]`, lowercase kind/status) plus session-level `class`/`running`/
//!   `built` fields.
//! - [`tools`]: the tool registry (name → description + inputSchema +
//!   handler) and the twelve tools: `topo_list`, `topo_plan`, `graph`,
//!   `net_build`, `net_start`, `net_stop`, `net_release`, `pingall`,
//!   `ping`, `dump`, `links`, `host_cmd`.
//!
//! Every tool returns `{content: [{type: "text", text: ...}]}` per the MCP
//! spec; tool failures are `isError: true` content, protocol-level failures
//! are JSON-RPC error responses. Kernel-touching tools (`net_start`,
//! `net_stop`, `pingall`, `ping`, `host_cmd`) are root-gated and answer
//! `requires root` otherwise.
//!
//! # Tests
//!
//! Protocol and session tests live in `tests/mcp.rs` and run unprivileged.
//! The live `net_start` + `pingall` round-trip is `#[ignore]`d and gated on
//! `MN_RUST_ROOT_TESTS=1` (same convention as `mn-net`):
//! `MN_RUST_ROOT_TESTS=1 cargo test -p mn-mcp -- --ignored --test-threads=1`

pub mod graph;
pub mod jsonrpc;
pub mod server;
pub mod session;
pub mod tools;

pub use server::{Server, PROTOCOL_VERSION, SERVER_NAME};
pub use session::Session;
pub use tools::{registry, Tool};
