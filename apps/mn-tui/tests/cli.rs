//! CLI integration tests for the `mn-tui` binary (task 13), run headless
//! (non-root sandbox): `--json` must print a parseable GraphView with the
//! expected nodes/edges; `--frames N` must render N frames to stdout.

use std::process::Command;

fn mn_tui(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mn-tui"))
        .args(args)
        .output()
        .expect("run mn-tui")
}

#[test]
fn json_flag_parses_and_has_expected_nodes_edges() {
    let out = mn_tui(&["--topo", "single,3", "--json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is valid JSON");
    assert_eq!(v["spec"], "single,3");
    let nodes = v["nodes"].as_array().expect("nodes array");
    let ids: Vec<&str> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    for want in ["h1", "h2", "h3", "s1"] {
        assert!(ids.contains(&want), "nodes {ids:?} missing {want}");
    }
    let s1 = nodes.iter().find(|n| n["id"] == "s1").unwrap();
    assert_eq!(s1["kind"], "Switch");
    let h1 = nodes.iter().find(|n| n["id"] == "h1").unwrap();
    assert_eq!(h1["kind"], "Host");
    assert_eq!(h1["ip"], "10.0.0.1");
    let edges = v["edges"].as_array().expect("edges array");
    assert_eq!(edges.len(), 3);
    let pairs: Vec<(&str, &str)> = edges
        .iter()
        .map(|e| (e["a"].as_str().unwrap(), e["b"].as_str().unwrap()))
        .collect();
    for want in [("h1", "s1"), ("h2", "s1"), ("h3", "s1")] {
        assert!(pairs.contains(&want), "edges {pairs:?} missing {want:?}");
    }
    // every edge/node carries a status
    for e in edges {
        assert!(e["status"].is_string());
    }
}

#[test]
fn frames_flag_renders_n_headless_frames() {
    let out = mn_tui(&["--topo", "single,3", "--frames", "2", "--size", "60x18"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).expect("utf8");
    assert!(s.contains("=== frame 1 ==="), "output: {s}");
    assert!(s.contains("=== frame 2 ==="), "output: {s}");
    assert!(!s.contains("=== frame 3 ==="), "output: {s}");
    // the graph itself is in the frames
    assert!(s.contains("h1") && s.contains("s1"), "output: {s}");
    assert!(s.contains("q quit"), "footer missing: {s}");
    // 60x18 frames: each frame block is 18 rows
    let frame1 = s.split("=== frame 1 ===\n").nth(1).unwrap();
    let frame1 = frame1.split("=== frame 2 ===").next().unwrap();
    assert_eq!(frame1.trim_end_matches('\n').lines().count(), 18);
}

#[test]
fn bad_topo_spec_errors_nonzero() {
    let out = mn_tui(&["--topo", "bogus,1", "--json"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());
}

#[test]
fn non_tty_without_export_flags_refuses_with_hint() {
    // in the test harness stdin is not a tty
    let out = mn_tui(&["--topo", "single,3"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--frames"));
}
