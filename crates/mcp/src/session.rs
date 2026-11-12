//! The MCP session: one optional [`Mininet`] per client connection.
//!
//! Building a net ([`Session::build`]) is a pure model build (the mn-net
//! dry-run path), so it works unprivileged; only `net_start`/`net_stop`/
//! `ping*`/`host_cmd` touch the kernel and are root-gated by the tools.

use mn_net::{ControllerKind, Mininet, MininetOptions};
use serde_json::{json, Value};

use crate::graph::net_graph;

/// Per-connection state handed to every tool handler.
#[derive(Debug, Default)]
pub struct Session {
    /// The built network (dry-run model until `net_start` runs).
    pub net: Option<Mininet>,
    /// Whether `net_start` succeeded and `net_stop` has not run yet.
    pub started: bool,
    /// The spec the current net was built from (`net_build`'s `spec`).
    pub spec: Option<String>,
}

impl Session {
    /// New empty session (no network).
    pub fn new() -> Self {
        Self::default()
    }

    /// The built net, or a tool error telling the agent to build one first.
    pub fn net(&self) -> Result<&Mininet, String> {
        self.net
            .as_ref()
            .ok_or_else(|| "no network in session; call net_build(spec) first".to_string())
    }

    /// Mutable access to the built net (same error as [`Session::net`]).
    pub fn net_mut(&mut self) -> Result<&mut Mininet, String> {
        self.net
            .as_mut()
            .ok_or_else(|| "no network in session; call net_build(spec) first".to_string())
    }

    /// `net_build`: build the net from a built-in topo spec as a dry-run
    /// model (non-root safe) and store it in the session, replacing any
    /// previous non-running net. Returns the graph JSON.
    pub fn build(&mut self, spec: &str, opts: &Value) -> Result<Value, String> {
        if self.net.is_some() && self.started {
            return Err(
                "a network is running; call net_stop and net_release before rebuilding".to_string(),
            );
        }
        let net = build_net(spec, opts)?;
        let graph = net_graph(&net, false);
        self.net = Some(net);
        self.started = false;
        self.spec = Some(spec.to_string());
        Ok(graph)
    }

    /// `net_release`: drop the net from the session (best-effort `stop()`
    /// first when it was started on a root host).
    pub fn release(&mut self) -> Result<Value, String> {
        let spec = self
            .spec
            .clone()
            .ok_or_else(|| "no network in session; nothing to release".to_string())?;
        if let Some(mut net) = self.net.take() {
            if self.started && mn_util::is_root() {
                let _ = net.stop();
            }
        }
        self.started = false;
        self.spec = None;
        Ok(json!({"released": spec}))
    }
}

/// Build a [`Mininet`] dry-run model from a topo spec plus the optional
/// `opts` object of the `net_build` tool (`link`, `switch`, `controller`,
/// `ipBase`, `autoSetMacs`, `autoStaticArp` — the `bin/mn` knobs that are
/// runtime strings/bools in mn-net).
pub fn build_net(spec: &str, opts: &Value) -> Result<Mininet, String> {
    let mut mo = MininetOptions {
        controller: Some(ControllerKind::Default),
        ..Default::default()
    };
    let str_opt = |v: &Value, key: &str| -> Result<String, String> {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("opts.{key} must be a string"))
    };
    let bool_opt = |v: &Value, key: &str| -> Result<bool, String> {
        v.as_bool()
            .ok_or_else(|| format!("opts.{key} must be a boolean"))
    };
    if let Some(v) = opts.get("link") {
        mo.link_spec = str_opt(v, "link")?;
    }
    if let Some(v) = opts.get("switch") {
        mo.switch_spec = str_opt(v, "switch")?;
    }
    if let Some(v) = opts.get("ipBase") {
        mo.ip_base = str_opt(v, "ipBase")?;
    }
    if let Some(v) = opts.get("autoSetMacs") {
        mo.auto_set_macs = bool_opt(v, "autoSetMacs")?;
    }
    if let Some(v) = opts.get("autoStaticArp") {
        mo.auto_static_arp = bool_opt(v, "autoStaticArp")?;
    }
    if let Some(v) = opts.get("controller") {
        let c = str_opt(v, "controller")?;
        mo.controller = match c.as_str() {
            "none" => None,
            "default" | "ref" => Some(ControllerKind::Default),
            other => {
                return Err(format!(
                    "unsupported opts.controller {other:?}; use default|ref|none"
                ))
            }
        };
    }
    Mininet::dry_run(spec, mo).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_then_release_roundtrip() {
        let mut s = Session::new();
        assert!(s.net().is_err());
        let g = s.build("single,3", &json!({})).unwrap();
        assert_eq!(g["spec"], "single,3");
        assert_eq!(g["nodes"].as_array().unwrap().len(), 5); // h1..h3, s1, c0
        assert!(s.net().is_ok());
        assert!(!s.started);
        let r = s.release().unwrap();
        assert_eq!(r["released"], "single,3");
        assert!(s.net().is_err());
        assert!(s.release().is_err());
    }

    #[test]
    fn build_with_opts_controller_none_drops_c0() {
        let mut s = Session::new();
        let g = s.build("single,2", &json!({"controller": "none"})).unwrap();
        let names: Vec<&str> = g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["h1", "h2", "s1"]);
    }

    #[test]
    fn build_rejects_bad_opts_and_specs() {
        let mut s = Session::new();
        assert!(s.build("single,2", &json!({"controller": 3})).is_err());
        assert!(s.build("single,2", &json!({"controller": "nox"})).is_err());
        assert!(s.build("bogus", &json!({})).is_err());
    }
}
