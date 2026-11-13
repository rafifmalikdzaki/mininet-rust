//! Command-line surface of `mn-tui` (task 13).
//!
//! Modes:
//! - `--json`: dump the [`GraphView`](mnr_tui::GraphView) of the dry-run plan
//!   as JSON and exit (also the payload the MCP server will serve).
//! - `--frames N`: render N frames headless into a `StringGrid` and print
//!   them to stdout (CI / non-tty), then exit.
//! - otherwise: interactive ratatui TUI. As root the net is built **and
//!   started** and live status is refreshed every second; non-root renders
//!   the dry-run plan read-only.

use clap::Parser;
use mn_net::{ControllerKind, MininetOptions};

/// OMO-style live graph TUI for the emulated mininet network.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "mn-tui",
    about = "OMO-style live graph TUI for the emulated network: nodes as boxes, links as edges, live status",
    disable_version_flag = true
)]
pub struct Args {
    /// Topology spec in build_topo format (single,3 / linear,4 / minimal / tree,2,2 / torus,3,3 / reversed,N)
    #[arg(long, value_name = "SPEC", default_value = "single,3")]
    pub topo: String,

    /// Dump the GraphView as JSON and exit (non-interactive)
    #[arg(long)]
    pub json: bool,

    /// Render N frames headless to stdout and exit (CI / non-tty)
    #[arg(long, value_name = "N")]
    pub frames: Option<usize>,

    /// Frame size for --frames, as WxH
    #[arg(long, value_name = "WxH", default_value = "80x24")]
    pub size: String,

    /// --switch spec passed through to mn-net (default|ovs|lxbr|user|...)
    #[arg(long, default_value = "default")]
    pub switch: String,

    /// --link spec passed through to mn-net (default|tc,bw=10,...)
    #[arg(long, default_value = "default")]
    pub link: String,

    /// --controller spec passed through to mn-net (default|none)
    #[arg(long, default_value = "default")]
    pub controller: String,

    /// --ipbase passed through to mn-net
    #[arg(long, default_value = "10.0.0.0/8")]
    pub ipbase: String,
}

impl Args {
    /// Parse `--size WxH` into `(width, height)` (defaults to 80x24).
    pub fn size(&self) -> (usize, usize) {
        let mut parts = self.size.splitn(2, ['x', 'X']);
        let w = parts.next().and_then(|s| s.parse().ok()).unwrap_or(80);
        let h = parts.next().and_then(|s| s.parse().ok()).unwrap_or(24);
        (w.max(10), h.max(6))
    }

    /// Whether headless export was requested (`--json` or `--frames`).
    pub fn headless(&self) -> bool {
        self.json || self.frames.is_some()
    }

    /// The [`MininetOptions`] built from the passthrough flags.
    pub fn options(&self) -> MininetOptions {
        MininetOptions {
            switch_spec: self.switch.clone(),
            link_spec: self.link.clone(),
            ip_base: self.ipbase.clone(),
            controller: match self.controller.as_str() {
                "none" => None,
                _ => Some(ControllerKind::Default),
            },
            ..Default::default()
        }
    }

    /// Title bar text for the topo + mode.
    pub fn title(&self, live: bool) -> String {
        let mode = if live { "live" } else { "dry-run plan" };
        format!("mn-tui — topo {} ({mode})", self.topo)
    }
}
