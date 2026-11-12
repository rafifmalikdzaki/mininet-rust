//! End-to-end protocol tests for the mn-mcp server: drive
//! [`Server::run`] with in-memory pipes (newline-delimited JSON-RPC 2.0 on
//! stdio, exactly what `apps/mn-mcp` wires to stdin/stdout).

use std::io::{BufReader, Cursor};

use mnr_mcp::{registry, Server};
use serde_json::{json, Value};

/// Whether root/namespace integration tests should actually run (same
/// convention as `mn-net`: `#[ignore]` + `MN_RUST_ROOT_TESTS=1`).
fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

/// Feed `input` (raw stdio bytes) through the server, return the parsed
/// response messages (one per output line) plus the raw output bytes.
fn drive(server: &mut Server, input: &str) -> (Vec<Value>, String) {
    let reader = BufReader::new(Cursor::new(input.as_bytes().to_vec()));
    let mut out: Vec<u8> = Vec::new();
    server
        .run(reader, &mut out)
        .expect("in-memory io cannot fail");
    let raw = String::from_utf8(out).expect("server writes utf-8 json");
    let msgs = raw
        .lines()
        .map(|l| serde_json::from_str(l).expect("each output line is one json message"))
        .collect();
    (msgs, raw)
}

/// One `tools/call` request line.
fn call(id: i64, name: &str, arguments: Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }))
        .unwrap()
    )
}

/// Extract the text of content[0] from a tools/call result.
fn text_of(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

#[test]
fn initialize_handshake_bytes() {
    let mut server = Server::new();
    let req = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0\"}}}\n";
    let (msgs, raw) = drive(&mut server, req);
    // exactly one response line, newline-terminated
    assert!(raw.ends_with('\n'));
    assert_eq!(raw.lines().count(), 1);
    assert_eq!(
        msgs[0],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "mn-mcp", "version": env!("CARGO_PKG_VERSION")},
            }
        })
    );
}

#[test]
fn notifications_and_ping() {
    let mut server = Server::new();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n",
    );
    let (msgs, _) = drive(&mut server, input);
    // the notification produces no response; ping answers {}
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0], json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
}

#[test]
fn notifications_are_never_answered_even_unknown_methods() {
    let mut server = Server::new();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":1}}\n",
        // unknown method WITHOUT id is still a notification: no reply
        "{\"jsonrpc\":\"2.0\",\"method\":\"no/such\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n",
    );
    let (msgs, _) = drive(&mut server, input);
    assert_eq!(msgs.len(), 1, "only the ping request may be answered");
    assert_eq!(msgs[0], json!({"jsonrpc": "2.0", "id": 9, "result": {}}));
}

#[test]
fn tools_list_contains_all_tools_with_schemas() {
    let mut server = Server::new();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\"}\n";
    let (msgs, _) = drive(&mut server, input);
    let tools = msgs[0]["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
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
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
    }
    // registry() agrees with what tools/list served
    assert_eq!(names.len(), registry().len());
}

#[test]
fn tools_call_topo_plan_single3_matches_fixture() {
    let mut server = Server::new();
    let (msgs, _) = drive(
        &mut server,
        &call(1, "topo_plan", json!({"spec": "single,3"})),
    );
    assert!(msgs[0].get("error").is_none(), "{}", msgs[0]);
    let g: Value = serde_json::from_str(&text_of(&msgs[0])).unwrap();
    assert_eq!(g["spec"], "single,3");
    assert_eq!(g["class"], "SingleSwitchTopo");
    // nodes match fixtures/topo_single3.txt: h1..h3 hosts with IPs, s1 switch
    // (canonical GraphView shape: "id" key, lowercase kind/status)
    let nodes: Vec<(String, String, String, Value)> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            (
                n["id"].as_str().unwrap().to_string(),
                n["kind"].as_str().unwrap().to_string(),
                n["status"].as_str().unwrap().to_string(),
                n["ip"].clone(),
            )
        })
        .collect();
    assert_eq!(
        nodes,
        vec![
            (
                "h1".into(),
                "host".into(),
                "planned".into(),
                json!("10.0.0.1")
            ),
            (
                "h2".into(),
                "host".into(),
                "planned".into(),
                json!("10.0.0.2")
            ),
            (
                "h3".into(),
                "host".into(),
                "planned".into(),
                json!("10.0.0.3")
            ),
            ("s1".into(), "switch".into(), "planned".into(), Value::Null),
        ]
    );
    // edges match the fixture's links(): hN-eth0 <-> s1-ethN
    let edges: Vec<(String, String, i64, i64, String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["a"].as_str().unwrap().to_string(),
                e["b"].as_str().unwrap().to_string(),
                e["port1"].as_i64().unwrap(),
                e["port2"].as_i64().unwrap(),
                e["intf1"].as_str().unwrap().to_string(),
                e["intf2"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        vec![
            (
                "h1".into(),
                "s1".into(),
                0,
                1,
                "h1-eth0".into(),
                "s1-eth1".into()
            ),
            (
                "h2".into(),
                "s1".into(),
                0,
                2,
                "h2-eth0".into(),
                "s1-eth2".into()
            ),
            (
                "h3".into(),
                "s1".into(),
                0,
                3,
                "h3-eth0".into(),
                "s1-eth3".into()
            ),
        ]
    );
}

#[test]
fn tools_call_topo_plan_bad_spec_is_tool_error_not_rpc_error() {
    let mut server = Server::new();
    let (msgs, _) = drive(
        &mut server,
        &call(1, "topo_plan", json!({"spec": "bogus,1"})),
    );
    let res = &msgs[0]["result"];
    assert_eq!(res["isError"], true);
    assert!(text_of(&msgs[0]).contains("Invalid topo name"));
    assert!(msgs[0].get("error").is_none());
}

#[test]
fn protocol_errors() {
    let mut server = Server::new();
    let input = concat!(
        "this is not json\n",                                      // -32700
        "{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"ping\"}\n",    // -32600
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"no/such\"}\n", // -32601
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"nope\"}}\n", // -32602
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"arguments\":{}}}\n", // -32602 no name
        "[1,2,3]\n", // -32600 not an object
    );
    let (msgs, _) = drive(&mut server, input);
    assert_eq!(msgs.len(), 6);
    assert_eq!(msgs[0]["error"]["code"], -32700);
    assert_eq!(msgs[0]["id"], Value::Null);
    assert_eq!(msgs[1]["error"]["code"], -32600);
    assert_eq!(msgs[1]["id"], 1);
    assert_eq!(msgs[2]["error"]["code"], -32601);
    assert_eq!(msgs[3]["error"]["code"], -32602);
    assert!(msgs[3]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown tool 'nope'"));
    assert_eq!(msgs[4]["error"]["code"], -32602);
    assert_eq!(msgs[5]["error"]["code"], -32600);
    assert_eq!(msgs[5]["id"], Value::Null);
}

#[test]
fn session_state_across_calls_build_graph_release() {
    let mut server = Server::new();
    // build a net into the session (dry-run, non-root) -> graph shows the
    // model (not running) -> dump/links in cli.py format -> release ->
    // graph then fails as a tool error
    let mut buf = String::new();
    buf.push_str(&call(
        1,
        "net_build",
        json!({"spec": "single,3", "opts": {"controller": "none"}}),
    ));
    buf.push_str(&call(2, "graph", json!({})));
    buf.push_str(&call(3, "dump", json!({})));
    buf.push_str(&call(4, "links", json!({})));
    buf.push_str(&call(5, "net_release", json!({})));
    buf.push_str(&call(6, "graph", json!({})));
    let (msgs, _) = drive(&mut server, &buf);
    assert_eq!(msgs.len(), 6);

    // net_build returns the graph JSON of the built model
    let g: Value = serde_json::from_str(&text_of(&msgs[0])).unwrap();
    assert_eq!(g["spec"], "single,3");
    assert_eq!(g["running"], false);
    assert_eq!(g["nodes"].as_array().unwrap().len(), 4); // h1..h3, s1 (no c0)

    // graph after build: same nodes, all planned (dry-run: pid None in detail)
    let g: Value = serde_json::from_str(&text_of(&msgs[1])).unwrap();
    assert_eq!(g["running"], false);
    for n in g["nodes"].as_array().unwrap() {
        assert_eq!(n["detail"]["pid"], Value::Null);
        assert_eq!(n["status"], "planned");
    }
    assert_eq!(g["edges"].as_array().unwrap().len(), 3);
    // canonical GraphView fields are present
    assert_eq!(g["edges"][0]["intf1"], "h1-eth0");
    assert_eq!(g["edges"][0]["port2"], 1);
    assert_eq!(g["edges"][0]["label"], "h1-eth0<->s1-eth1");

    // dump: exact cli.py format (compare with mn-net's observe tests)
    let dump = text_of(&msgs[2]);
    assert!(
        dump.contains("<Host h1: h1-eth0:10.0.0.1 pid=None>"),
        "{dump}"
    );
    assert!(dump.contains("<OVSSwitch s1: lo:127.0.0.1"), "{dump}");

    // links: exact cli.py format
    let links = text_of(&msgs[3]);
    assert_eq!(
        links,
        "h1-eth0<->s1-eth1 (MISSING MISSING)\n\
         h2-eth0<->s1-eth2 (MISSING MISSING)\n\
         h3-eth0<->s1-eth3 (MISSING MISSING)\n"
    );

    // release ok, then graph is an isError tool result
    assert!(msgs[4]["result"]["isError"].is_null());
    assert_eq!(msgs[5]["result"]["isError"], true);
    assert!(text_of(&msgs[5]).contains("net_build"));
}

#[test]
fn non_root_net_start_is_requires_root_tool_error() {
    if mn_util::is_root() {
        return; // only meaningful unprivileged
    }
    let mut server = Server::new();
    let mut buf = String::new();
    buf.push_str(&call(1, "net_build", json!({"spec": "single,2"})));
    buf.push_str(&call(2, "net_start", json!({})));
    let (msgs, _) = drive(&mut server, &buf);
    assert_eq!(msgs[1]["result"]["isError"], true);
    assert!(text_of(&msgs[1]).contains("requires root"));
}

/// Live round-trip on a privileged host: build, start, pingall, graph,
/// stop, release. `#[ignore]` + `MN_RUST_ROOT_TESTS=1` (same convention as
/// mn-net's root tests).
#[test]
#[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-mcp -- --ignored --test-threads=1"]
fn root_live_net_start_pingall_stop_release() {
    if !root_tests_enabled() {
        return;
    }
    let mut server = Server::new();
    let mut buf = String::new();
    // minimal = h1-h2 direct link, no switch/controller binaries needed
    // lxbr = kernel LinuxBridge: works without the OVS kernel datapath
    // module (which privileged CI containers cannot load).
    buf.push_str(&call(
        1,
        "net_build",
        json!({"spec": "minimal", "opts": {"controller": "none", "switch": "lxbr"}}),
    ));
    buf.push_str(&call(2, "net_start", json!({})));
    buf.push_str(&call(3, "pingall", json!({})));
    buf.push_str(&call(4, "ping", json!({"a": "h1", "b": "h2"})));
    buf.push_str(&call(5, "graph", json!({})));
    buf.push_str(&call(
        6,
        "host_cmd",
        json!({"host": "h1", "cmd": "echo hello-from-h1"}),
    ));
    buf.push_str(&call(7, "net_stop", json!({})));
    buf.push_str(&call(8, "net_release", json!({})));
    let (msgs, _) = drive(&mut server, &buf);
    assert_eq!(msgs.len(), 8);
    for (i, m) in msgs.iter().enumerate() {
        assert!(m.get("error").is_none(), "msg {i}: {m}");
        assert_ne!(
            m["result"]["isError"],
            true,
            "msg {i} tool error: {}",
            text_of(m)
        );
    }
    // pingall: 1 packet sent, 0 loss between h1 and h2
    let p: Value = serde_json::from_str(&text_of(&msgs[2])).unwrap();
    assert_eq!(p["ploss"], 0.0, "{}", p["output"]);
    let p: Value = serde_json::from_str(&text_of(&msgs[3])).unwrap();
    assert_eq!(p["ploss"], 0.0);
    // graph while running: both hosts are up with pids (in detail)
    let g: Value = serde_json::from_str(&text_of(&msgs[4])).unwrap();
    assert_eq!(g["running"], true);
    assert_eq!(g["live"], true);
    for n in g["nodes"].as_array().unwrap() {
        assert_eq!(n["status"], "up", "{n}");
        // hosts have a shell pid; the root-ns lxbr switch has none
        if n["kind"] == "host" {
            assert!(n["detail"]["pid"].is_number(), "{n}");
        }
    }
    // host_cmd ran inside h1's namespace
    assert!(text_of(&msgs[5]).contains("hello-from-h1"));
}
