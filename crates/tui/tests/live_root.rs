//! Root-gated live test: build + start a real net and check that
//! `GraphView::from_net(_, live=true)` reports live status (node pids,
//! link statuses) — the same data `net.links()` prints.
//!
//! Run on a privileged Linux host/CI:
//! `MN_RUST_ROOT_TESTS=1 cargo test -p mn-tui -- --ignored --test-threads=1`

use mn_net::{Mininet, MininetOptions};
use mn_tui::{EdgeStatus, GraphView, NodeStatus};

fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

#[test]
#[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-tui -- --ignored --test-threads=1"]
fn live_single3_graph_reports_up() {
    if !root_tests_enabled() {
        return;
    }
    // lxbr = netlink-backed LinuxBridge switch (no OVS needed); no controller.
    let opts = MininetOptions {
        controller: None,
        switch_spec: "lxbr".to_string(),
        ..Default::default()
    };
    let mut net = Mininet::dry_run("single,3", opts).unwrap();
    net.start().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let g = GraphView::from_net(&net, true);
        assert!(g.live);
        assert_eq!(g.nodes.len(), 4);
        for n in &g.nodes {
            assert_eq!(n.status, NodeStatus::Up, "node {} should be up", n.id);
            assert!(
                n.detail.as_ref().unwrap().pid.is_some(),
                "node {} should have a pid",
                n.id
            );
        }
        assert_eq!(g.edges.len(), 3);
        for e in &g.edges {
            assert_eq!(e.status, EdgeStatus::Up, "edge {} status", e.label);
        }
        // the TUI's live link status agrees with net.links() ("OK OK")
        let links = net.links();
        assert!(links.contains("(OK OK)"), "links: {links}");
        // pingall through the graph's edges
        let res = net.ping_all(None).unwrap();
        assert_eq!(res.ploss, 0.0, "ping output: {}", res.output);
    }));
    let stop = net.stop();
    assert!(stop.is_ok(), "stop: {stop:?}");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}
