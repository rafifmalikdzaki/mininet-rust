//! The MCP tool registry: name → {description, inputSchema, handler}.
//!
//! Every handler gets `&mut Session` and the tool's `arguments` object and
//! returns the MCP tool **result** value
//! (`{"content": [{"type": "text", "text": ...}]}`) on success; `Err(msg)`
//! becomes an `isError: true` result by the server (protocol-level problems
//! — unknown tool, bad params — are JSON-RPC errors instead, see
//! [`crate::server`]).
//!
//! Tools split into:
//! - pure/graph, non-root safe: `topo_list`, `topo_plan`, `graph`,
//!   `net_build`, `dump`, `links`, `net_release`;
//! - live/root-gated: `net_start`, `net_stop`, `pingall`, `ping`,
//!   `host_cmd` (they need a started net, which only root can create).

use mn_util::is_root;
use serde_json::{json, Value};

use crate::graph::{net_graph, plan_graph};
use crate::session::Session;

/// Handler signature: session + `arguments`, result or tool-error message.
pub type Handler = fn(&mut Session, &Value) -> Result<Value, String>;

/// One registered MCP tool.
pub struct Tool {
    /// MCP tool name (the `tools/call` `params.name`).
    pub name: &'static str,
    /// Human/LLM-readable description (sent in `tools/list`).
    pub description: &'static str,
    /// JSON Schema for the `arguments` object (sent as `inputSchema`).
    pub input_schema: Value,
    /// The handler.
    pub handler: Handler,
}

/// An MCP tool result with a single text content block.
pub fn text_result(text: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}]})
}

/// An MCP tool result flagged `isError: true` (tool-level failure).
pub fn error_content(text: impl Into<String>) -> Value {
    let mut v = text_result(text);
    v["isError"] = Value::Bool(true);
    v
}

/// Pretty-print a JSON value for a text content block.
fn pretty(v: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| format!("json encode failed: {e}"))
}

/// Extract a required string argument.
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or non-string required argument '{key}'"))
}

/// `requires root` tool error for the kernel-touching tools.
fn requires_root(what: &str) -> String {
    format!(
        "requires root: {what} needs euid 0 to create namespaces/links/processes; \
         the built model is a dry-run plan otherwise"
    )
}

// ---------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------

fn topo_list(_s: &mut Session, _args: &Value) -> Result<Value, String> {
    let topos = json!([
        {"name": "minimal", "spec": "minimal", "args": [],
         "description": "Two hosts h1-h2 connected directly, no switch"},
        {"name": "single", "spec": "single,k", "args": ["k"], "defaults": {"k": 2},
         "description": "One switch s1 with k hosts h1..hk"},
        {"name": "reversed", "spec": "reversed,k", "args": ["k"], "defaults": {"k": 2},
         "description": "SingleSwitchTopo with reversed port numbering"},
        {"name": "linear", "spec": "linear,k,n", "args": ["k", "n"], "defaults": {"k": 2, "n": 1},
         "description": "k switches in a line, each with n hosts"},
        {"name": "tree", "spec": "tree,depth,fanout", "args": ["depth", "fanout"],
         "defaults": {"depth": 1, "fanout": 2},
         "description": "Tree of switches with the given depth and fanout"},
        {"name": "torus", "spec": "torus,x,y,n", "args": ["x", "y", "n"], "defaults": {"n": 1},
         "description": "x by y switch torus, n hosts per switch (x,y >= 3)"}
    ]);
    Ok(text_result(pretty(&topos)?))
}

fn topo_plan(_s: &mut Session, args: &Value) -> Result<Value, String> {
    let spec = str_arg(args, "spec")?;
    let g = plan_graph(spec)?;
    Ok(text_result(pretty(&g)?))
}

fn graph(s: &mut Session, _args: &Value) -> Result<Value, String> {
    let net = s.net()?;
    let running = s.started;
    Ok(text_result(pretty(&net_graph(net, running))?))
}

fn net_build(s: &mut Session, args: &Value) -> Result<Value, String> {
    let choice = match (args.get("spec"), args.get("topology")) {
        (Some(Value::String(spec)), None) => Ok((Some(spec.as_str()), None)),
        (None, Some(topology)) => Ok((None, Some(topology))),
        (Some(_), None) => Err("'spec' must be a string"),
        _ => Err("provide exactly one of 'spec' or 'topology'"),
    }?;
    let opts = match args.get("opts") {
        None | Some(Value::Null) => json!({}),
        Some(o) if o.is_object() => o.clone(),
        Some(_) => return Err("'opts' must be an object".to_string()),
    };
    if let Some(key) = opts.as_object().and_then(|o| {
        o.keys().find(|k| {
            !matches!(
                k.as_str(),
                "link" | "switch" | "controller" | "ipBase" | "autoSetMacs" | "autoStaticArp"
            )
        })
    }) {
        return Err(format!("unknown opts field {key:?}"));
    }
    let g = if let Some(spec) = choice.0 {
        s.build(spec, &opts)?
    } else {
        let object = args.as_object().ok_or("arguments must be an object")?;
        if let Some(key) = object
            .keys()
            .find(|k| !matches!(k.as_str(), "topology" | "opts"))
        {
            return Err(format!("unknown net_build argument {key:?}"));
        }
        s.build_custom(choice.1.expect("topology selected"), &opts)?
    };
    Ok(text_result(pretty(&g)?))
}

fn net_start(s: &mut Session, _args: &Value) -> Result<Value, String> {
    if !is_root() {
        return Err(requires_root("net_start"));
    }
    let spec = s.spec.clone().unwrap_or_default();
    // Session::net_mut() already answers "no network in session; call
    // net_build(spec) first" when the session is empty.
    s.net_mut()?
        .start()
        .map_err(|e| format!("net_start failed: {e}"))?;
    s.started = true;
    let dump = s.net()?.dump();
    Ok(text_result(format!("started {spec}\n{dump}")))
}

fn net_stop(s: &mut Session, _args: &Value) -> Result<Value, String> {
    if !is_root() {
        return Err(requires_root("net_stop"));
    }
    let spec = s.spec.clone().unwrap_or_default();
    s.net_mut()?
        .stop()
        .map_err(|e| format!("net_stop failed: {e}"))?;
    s.started = false;
    Ok(text_result(format!("stopped {spec}")))
}

fn net_release(s: &mut Session, _args: &Value) -> Result<Value, String> {
    let r = s.release()?;
    Ok(text_result(pretty(&r)?))
}

fn require_running(s: &Session) -> Result<(), String> {
    s.net()?; // "no network in session; call net_build(spec) first"
    if !s.started {
        return Err(
            "network is built but not running; call net_start first (net_start requires root)"
                .to_string(),
        );
    }
    Ok(())
}

fn pingall(s: &mut Session, _args: &Value) -> Result<Value, String> {
    require_running(s)?;
    let net = s.net_mut()?;
    let r = net
        .ping_all(None)
        .map_err(|e| format!("pingall failed: {e}"))?;
    Ok(text_result(pretty(&json!({
        "ploss": r.ploss,
        "output": r.output,
    }))?))
}

fn ping(s: &mut Session, args: &Value) -> Result<Value, String> {
    require_running(s)?;
    let a = str_arg(args, "a")?;
    let b = str_arg(args, "b")?;
    let net = s.net_mut()?;
    let r = net
        .ping(Some(vec![a, b]), None)
        .map_err(|e| format!("ping {a} {b} failed: {e}"))?;
    Ok(text_result(pretty(&json!({
        "a": a,
        "b": b,
        "ploss": r.ploss,
        "output": r.output,
    }))?))
}

fn dump(s: &mut Session, _args: &Value) -> Result<Value, String> {
    let net = s.net()?;
    Ok(text_result(net.dump()))
}

fn links(s: &mut Session, _args: &Value) -> Result<Value, String> {
    let net = s.net()?;
    Ok(text_result(net.links()))
}

fn host_cmd(s: &mut Session, args: &Value) -> Result<Value, String> {
    if !is_root() {
        return Err(requires_root("host_cmd"));
    }
    require_running(s)?;
    let host = str_arg(args, "host")?;
    let cmd = str_arg(args, "cmd")?;
    let net = s.net_mut()?;
    let mut node = net
        .get_mut(host)
        .ok_or_else(|| format!("node '{host}' not in network"))?;
    let out = node
        .node_mut()
        .cmd(cmd)
        .map_err(|e| format!("cmd on {host} failed: {e}"))?;
    Ok(text_result(out))
}

// ---------------------------------------------------------------------
// registry
// ---------------------------------------------------------------------

const EMPTY_SCHEMA: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;

/// All tools the server exposes (order is the `tools/list` order).
pub fn registry() -> Vec<Tool> {
    let empty = || serde_json::from_str(EMPTY_SCHEMA).expect("static schema is valid JSON");
    vec![
        Tool {
            name: "topo_list",
            description: "List the built-in topology specs (minimal, single, reversed, linear, tree, torus) with their arguments and defaults",
            input_schema: empty(),
            handler: topo_list,
        },
        Tool {
            name: "topo_plan",
            description: "Plan a built-in topo spec (e.g. \"single,3\") and return its graph JSON (canonical GraphView shape: {nodes:[{id,label,kind,ip,status,detail?}], edges:[{a,b,status,label,intf1,intf2,port1,port2}]}) as a dry-run model (non-root safe, nothing is created)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "spec": {"type": "string", "description": "built-in topo spec, e.g. \"single,3\" (see topo_list)"}
                },
                "required": ["spec"],
                "additionalProperties": false
            }),
            handler: topo_plan,
        },
        Tool {
            name: "graph",
            description: "Graph JSON of the network in the session (canonical GraphView shape: nodes with id/kind/ip/status/detail, edges with label + intf/port; plus class/running/built); live status when the net is running",
            input_schema: empty(),
            handler: graph,
        },
        Tool {
            name: "net_build",
            description: "Build a built-in spec or a custom topology of named hosts, switches and links as a dry-run model; return its graph. Supply exactly one of spec or topology.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "spec": {"type": "string", "description": "built-in topo spec, e.g. \"single,3\""},
                    "topology": {
                        "type": "object",
                        "description": "Custom graph; links are pairs of existing node names. Parallel links are allowed; self-links are not. At most 256 nodes and 1024 links.",
                        "properties": {
                            "hosts": {"type": "array", "maxItems": 256, "items": {"type": "string", "pattern": "^[A-Za-z][A-Za-z0-9_]{0,6}$", "not": {"const": "c0"}}, "description": "Unique host names; c0 is reserved"},
                            "switches": {"type": "array", "maxItems": 256, "items": {"type": "string", "pattern": "^[A-Za-z][A-Za-z0-9_]{0,6}$", "not": {"const": "c0"}}, "description": "Unique switch names, distinct from hosts"},
                            "links": {
                                "type": "array", "maxItems": 1024,
                                "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "string"}},
                                "description": "Endpoint pairs, e.g. [[\"h1\", \"s1\"], [\"s1\", \"s2\"]]"
                            }
                        },
                        "required": ["hosts", "switches", "links"],
                        "additionalProperties": false
                    },
                    "opts": {
                        "type": "object",
                        "properties": {
                            "link": {"type": "string", "description": "link spec, e.g. \"default\" or \"tc,bw=10\""},
                            "switch": {"type": "string", "description": "switch spec, e.g. \"default\""},
                            "controller": {"type": "string", "enum": ["default", "ref", "none"]},
                            "ipBase": {"type": "string", "description": "base IP network, default \"10.0.0.0/8\""},
                            "autoSetMacs": {"type": "boolean"},
                            "autoStaticArp": {"type": "boolean"}
                        },
                        "additionalProperties": false
                    }
                },
                "oneOf": [{"required": ["spec"]}, {"required": ["topology"]}],
                "additionalProperties": false
            }),
            handler: net_build,
        },
        Tool {
            name: "net_start",
            description: "Start the session's network (controllers, switches, hosts, links) — requires root",
            input_schema: empty(),
            handler: net_start,
        },
        Tool {
            name: "net_stop",
            description: "Stop the session's running network — requires root",
            input_schema: empty(),
            handler: net_stop,
        },
        Tool {
            name: "net_release",
            description: "Release the session's network (stops it first if running) and clear the session",
            input_schema: empty(),
            handler: net_release,
        },
        Tool {
            name: "pingall",
            description: "Ping between all hosts of the running net; returns {ploss, output} — requires a started net (root)",
            input_schema: empty(),
            handler: pingall,
        },
        Tool {
            name: "ping",
            description: "Ping from host a to host b of the running net; returns {a, b, ploss, output} — requires a started net (root)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "a": {"type": "string", "description": "source host name"},
                    "b": {"type": "string", "description": "destination host name"}
                },
                "required": ["a", "b"],
                "additionalProperties": false
            }),
            handler: ping,
        },
        Tool {
            name: "dump",
            description: "cli.py-format dump of the session's net: one repr(node) per line",
            input_schema: empty(),
            handler: dump,
        },
        Tool {
            name: "links",
            description: "cli.py-format link list of the session's net: 'intf1<->intf2 (status1 status2)' per line",
            input_schema: empty(),
            handler: links,
        },
        Tool {
            name: "host_cmd",
            description: "Run a shell command inside a host of the running net and return its output — requires root",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "host": {"type": "string", "description": "host name, e.g. \"h1\""},
                    "cmd": {"type": "string", "description": "shell command, e.g. \"ip addr\""}
                },
                "required": ["host", "cmd"],
                "additionalProperties": false
            }),
            handler: host_cmd,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_tools_with_valid_schemas() {
        let reg = registry();
        let names: Vec<&str> = reg.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "topo_list",
                "topo_plan",
                "graph",
                "net_build",
                "net_start",
                "net_stop",
                "net_release",
                "pingall",
                "ping",
                "dump",
                "links",
                "host_cmd",
            ]
        );
        for t in &reg {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(!t.description.is_empty());
        }
    }

    #[test]
    fn non_root_live_tools_say_requires_root() {
        if is_root() {
            return; // on a root host the gate passes; nothing to assert here
        }
        let mut s = Session::new();
        s.build("single,2", &json!({})).unwrap();
        for h in [net_start as Handler, net_stop, host_cmd] {
            let err = h(&mut s, &json!({"host": "h1", "cmd": "true"})).unwrap_err();
            assert!(err.contains("requires root"), "{err}");
        }
        let err = pingall(&mut s, &json!({})).unwrap_err();
        assert!(err.contains("not running"), "{err}");
    }

    #[test]
    fn graph_without_net_is_tool_error() {
        let mut s = Session::new();
        let err = graph(&mut s, &json!({})).unwrap_err();
        assert!(err.contains("net_build"), "{err}");
    }

    #[test]
    fn no_network_error_comes_from_session_accessors() {
        // the single source of the "no network in session" message is
        // Session::net()/net_mut() — the handlers no longer duplicate it
        let mut s = Session::new();
        let want = "no network in session; call net_build(spec) first";
        assert_eq!(graph(&mut s, &json!({})).unwrap_err(), want);
        assert_eq!(links(&mut s, &json!({})).unwrap_err(), want);
        assert_eq!(dump(&mut s, &json!({})).unwrap_err(), want);
        assert_eq!(pingall(&mut s, &json!({})).unwrap_err(), want);
        assert_eq!(
            ping(&mut s, &json!({"a": "h1", "b": "h2"})).unwrap_err(),
            want
        );
        assert_eq!(s.net_mut().unwrap_err(), want);
    }
}
