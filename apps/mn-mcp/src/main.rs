//! `mn-mcp` — MCP server binary exposing the mininet-rust graph + simulation
//! to AI tools natively (task 14).
//!
//! Speaks MCP (JSON-RPC 2.0, newline-delimited) on **stdin/stdout**; all
//! logging goes to **stderr** so the protocol stream stays clean. Register
//! with any MCP-capable agent, e.g.
//! `vibe mcp add mn-mcp --transport stdio --command target/release/mn-mcp`.
//!
//! Usage: `mn-mcp [--spec <topo-spec>]` — `--spec single,3` preloads a
//! dry-run network into the session at startup.

use std::io::{BufReader, Write};

use mnr_mcp::{Server, Session};
use serde_json::json;

const USAGE: &str = "usage: mn-mcp [--spec <topo-spec>]\n\
                     MCP server (JSON-RPC 2.0 over stdio) for mininet-rust.\n\
                     --spec single,3   preload a dry-run network into the session";

fn main() {
    let mut preload: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                return;
            }
            "--spec" => match args.next() {
                Some(spec) => preload = Some(spec),
                None => {
                    eprintln!("mn-mcp: --spec needs a value\n{USAGE}");
                    std::process::exit(2);
                }
            },
            other if other.starts_with("--spec=") => {
                preload = Some(other["--spec=".len()..].to_string());
            }
            other => {
                eprintln!("mn-mcp: ignoring unknown argument {other:?}");
            }
        }
    }

    let mut session = Session::new();
    if let Some(spec) = preload {
        match session.build(&spec, &json!({})) {
            Ok(_) => eprintln!("[mn-mcp] preloaded dry-run net from spec {spec:?}"),
            Err(e) => eprintln!("[mn-mcp] --spec {spec:?} preload failed: {e}"),
        }
    }
    eprintln!(
        "[mn-mcp] serving MCP (protocol {}) on stdio as {SERVER} v{}",
        mnr_mcp::PROTOCOL_VERSION,
        env!("CARGO_PKG_VERSION"),
        SERVER = mnr_mcp::SERVER_NAME,
    );

    let mut server = Server::with_session(session);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let code = match server.run(BufReader::new(stdin.lock()), stdout.lock()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[mn-mcp] io error on stdio: {e}");
            1
        }
    };
    let _ = std::io::stderr().flush();
    std::process::exit(code);
}
