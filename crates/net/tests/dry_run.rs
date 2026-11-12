//! Dry-run tests for `mn-net` (non-root): the planned topology built from a
//! spec via `Mininet::dry_run` must (a) match checked-in golden plans and
//! (b) match the topo-level baseline fixtures captured from the original
//! Python Mininet (`fixtures/topo_single3.txt`, `fixtures/topo_linear4.txt`,
//! `fixtures/topo_tree.txt`).

use std::path::PathBuf;

use mn_net::{ControllerKind, Mininet, MininetOptions};

fn opts() -> MininetOptions {
    MininetOptions {
        controller: Some(ControllerKind::None), // --controller none
        ..Default::default()
    }
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Parse a `fixtures/topo_*.txt` capture into
/// `(spec, class, nodes, hosts, switches, links)` where `links` are
/// `src-eth<port1><->dst-eth<port2>` strings (from the
/// `links(sort=True, withKeys=True, withInfo=True):` section).
#[allow(clippy::type_complexity)]
fn parse_fixture(
    name: &str,
) -> (
    String,
    String,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let text = std::fs::read_to_string(fixture_dir().join(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
    let mut spec = String::new();
    let mut class = String::new();
    let mut nodes = Vec::new();
    let mut hosts = Vec::new();
    let mut switches = Vec::new();
    let mut links = Vec::new();
    for line in text.lines() {
        if let Some(arg) = line.strip_prefix("buildTopo arg: ") {
            spec = arg.to_string();
        } else if let Some(cls) = line.strip_prefix("topo class: ") {
            class = cls.to_string();
        } else if let Some(rest) = line.strip_prefix("nodes(): ") {
            nodes = parse_repr_list(rest);
        } else if let Some(rest) = line.strip_prefix("hosts(): ") {
            hosts = parse_repr_list(rest);
        } else if let Some(rest) = line.strip_prefix("switches(): ") {
            switches = parse_repr_list(rest);
        } else if line.starts_with("   ('") {
            // `   ('h1', 's1', 1, {'node1': 'h1', 'node2': 's1', 'port1': 0, 'port2': 1})`
            links.push(parse_fixture_link(line));
        }
    }
    (spec, class, nodes, hosts, switches, links)
}

/// Parse a Python `repr` of a list of strings: `['h1', 'h2']`.
fn parse_repr_list(rest: &str) -> Vec<String> {
    let inner = rest
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| panic!("bad list repr {rest:?}"));
    if inner.is_empty() {
        return Vec::new();
    }
    inner
        .split("', '")
        .map(|s| s.trim_matches('\'').to_string())
        .collect()
}

/// Parse one fixture link line into `src-eth<port1><->dst-eth<port2>`.
fn parse_fixture_link(line: &str) -> String {
    let body = line.trim().strip_prefix("('").expect("tuple start");
    let (src, rest) = body.split_once("', '").expect("src");
    let (dst, rest) = rest.split_once("', ").expect("dst");
    let port1 = rest_after(rest, "'port1': ").parse::<i64>().expect("port1");
    let port2 = rest_after(rest, "'port2': ").parse::<i64>().expect("port2");
    format!("{src}-eth{port1}<->{dst}-eth{port2}")
}

/// Substring after `marker`, up to the next `,` or `}`.
fn rest_after<'a>(s: &'a str, marker: &str) -> &'a str {
    let start = s
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker} in {s:?}"))
        + marker.len();
    let end = s[start..]
        .find([',', '}'])
        .map(|i| start + i)
        .unwrap_or(s.len());
    s[start..end].trim()
}

/// The `=== topo ===` section of a plan as a map of `key -> value` lines.
fn topo_section_lines(plan: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_topo = false;
    for line in plan.lines() {
        if line == "=== topo ===" {
            in_topo = true;
            continue;
        }
        if in_topo {
            if line.starts_with("=== ") {
                break;
            }
            lines.push(line.to_string());
        }
    }
    lines
}

#[test]
fn dry_run_single3_golden() {
    let net = Mininet::dry_run("single,3", opts()).unwrap();
    let expected = "\
=== topo ===
spec: single,3
class: SingleSwitchTopo
nodes: h1 h2 h3 s1
hosts: h1 h2 h3
switches: s1
links: h1-eth0<->s1-eth1 h2-eth0<->s1-eth2 h3-eth0<->s1-eth3
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<Host h3: h3-eth0:10.0.0.3 pid=None> \n\
<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None,s1-eth3:None pid=None> \n\
=== links ===
h1-eth0<->s1-eth1 (MISSING MISSING)\n\
h2-eth0<->s1-eth2 (MISSING MISSING)\n\
h3-eth0<->s1-eth3 (MISSING MISSING)\n";
    assert_eq!(net.plan(), expected);
}

#[test]
fn dry_run_linear4_golden() {
    let net = Mininet::dry_run("linear,4", opts()).unwrap();
    let expected = "\
=== topo ===
spec: linear,4
class: LinearTopo
nodes: h1 h2 h3 h4 s1 s2 s3 s4
hosts: h1 h2 h3 h4
switches: s1 s2 s3 s4
links: h1-eth0<->s1-eth1 h2-eth0<->s2-eth1 h3-eth0<->s3-eth1 h4-eth0<->s4-eth1 s2-eth2<->s1-eth2 s3-eth2<->s2-eth3 s4-eth2<->s3-eth3
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<Host h3: h3-eth0:10.0.0.3 pid=None> \n\
<Host h4: h4-eth0:10.0.0.4 pid=None> \n\
<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None pid=None> \n\
<OVSSwitch s2: lo:127.0.0.1,s2-eth1:None,s2-eth2:None,s2-eth3:None pid=None> \n\
<OVSSwitch s3: lo:127.0.0.1,s3-eth1:None,s3-eth2:None,s3-eth3:None pid=None> \n\
<OVSSwitch s4: lo:127.0.0.1,s4-eth1:None,s4-eth2:None pid=None> \n\
=== links ===
h1-eth0<->s1-eth1 (MISSING MISSING)\n\
h2-eth0<->s2-eth1 (MISSING MISSING)\n\
h3-eth0<->s3-eth1 (MISSING MISSING)\n\
h4-eth0<->s4-eth1 (MISSING MISSING)\n\
s2-eth2<->s1-eth2 (MISSING MISSING)\n\
s3-eth2<->s2-eth3 (MISSING MISSING)\n\
s4-eth2<->s3-eth3 (MISSING MISSING)\n";
    assert_eq!(net.plan(), expected);
}

#[test]
fn dry_run_tree22_golden() {
    let net = Mininet::dry_run("tree,2,2", opts()).unwrap();
    let expected = "\
=== topo ===
spec: tree,2,2
class: TreeTopo
nodes: h1 h2 h3 h4 s1 s2 s3
hosts: h1 h2 h3 h4
switches: s1 s2 s3
links: s1-eth1<->s2-eth3 s1-eth2<->s3-eth3 s2-eth1<->h1-eth0 s2-eth2<->h2-eth0 s3-eth1<->h3-eth0 s3-eth2<->h4-eth0
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<Host h3: h3-eth0:10.0.0.3 pid=None> \n\
<Host h4: h4-eth0:10.0.0.4 pid=None> \n\
<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None pid=None> \n\
<OVSSwitch s2: lo:127.0.0.1,s2-eth1:None,s2-eth2:None,s2-eth3:None pid=None> \n\
<OVSSwitch s3: lo:127.0.0.1,s3-eth1:None,s3-eth2:None,s3-eth3:None pid=None> \n\
=== links ===
s1-eth1<->s2-eth3 (MISSING MISSING)\n\
s1-eth2<->s3-eth3 (MISSING MISSING)\n\
s2-eth1<->h1-eth0 (MISSING MISSING)\n\
s2-eth2<->h2-eth0 (MISSING MISSING)\n\
s3-eth1<->h3-eth0 (MISSING MISSING)\n\
s3-eth2<->h4-eth0 (MISSING MISSING)\n";
    assert_eq!(net.plan(), expected);
}

/// The dry-run plan's `=== topo ===` section must agree with the
/// Python-captured baseline fixtures (`fixtures/topo_*.txt`).
#[test]
fn dry_run_topo_sections_match_python_fixtures() {
    for fixture in ["topo_single3.txt", "topo_linear4.txt", "topo_tree.txt"] {
        let (spec, class, nodes, hosts, switches, links) = parse_fixture(fixture);
        let net = Mininet::dry_run(&spec, opts()).unwrap();
        let section = topo_section_lines(&net.plan());
        let mut sorted_links = links.clone();
        sorted_links.sort_by_key(|s| mn_util::natural(s));
        let expected = vec![
            format!("spec: {spec}"),
            format!("class: {class}"),
            format!("nodes: {}", nodes.join(" ")),
            format!("hosts: {}", hosts.join(" ")),
            format!("switches: {}", switches.join(" ")),
            format!("links: {}", sorted_links.join(" ")),
        ];
        assert_eq!(section, expected, "topo section mismatch for {fixture}");
    }
}

/// `--switch lxbr`: the seam holds a real `mn-node::nodelib::LinuxBridge`,
/// so the dry-run dump shows the `<LinuxBridge ...>` repr with the kind set
/// correctly — exactly like Python's `repr(LinuxBridge(...))`.
#[test]
fn dry_run_lxbr_switch_repr_golden() {
    let mut net = Mininet::dry_run(
        "single,2",
        MininetOptions {
            controller: Some(ControllerKind::None), // --controller none
            switch_spec: "lxbr".to_string(),        // --switch lxbr
            ..Default::default()
        },
    )
    .unwrap();
    let expected = "\
=== topo ===
spec: single,2
class: SingleSwitchTopo
nodes: h1 h2 s1
hosts: h1 h2
switches: s1
links: h1-eth0<->s1-eth1 h2-eth0<->s1-eth2
=== dump ===
<Host h1: h1-eth0:10.0.0.1 pid=None> \n\
<Host h2: h2-eth0:10.0.0.2 pid=None> \n\
<LinuxBridge s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None pid=None> \n\
=== links ===
h1-eth0<->s1-eth1 (MISSING MISSING)\n\
h2-eth0<->s1-eth2 (MISSING MISSING)\n";
    assert_eq!(net.plan(), expected);
    // the stored value is the LinuxBridge itself (seam wired), kind set
    assert!(matches!(
        net.net_switch("s1").unwrap(),
        mn_net::NetSwitch::LxBr(_)
    ));
    assert_eq!(net.switch("s1").unwrap().node().kind(), "LinuxBridge");
    // dry-run stop on the never-started LinuxBridge is a model-only no-op
    // (no netlink ops, no root needed)
    net.stop().unwrap();
    assert!(!net.built);
}

/// A default controller shows up in the dump in Python's Controller repr
/// format (`<Controller c0: 127.0.0.1:6653 pid=None> `).
#[test]
fn dry_run_with_default_controller_dump_line() {
    let net = Mininet::dry_run(
        "single,2",
        MininetOptions {
            controller: Some(ControllerKind::Default),
            ..Default::default()
        },
    )
    .unwrap();
    let dump = net.dump();
    assert!(
        dump.contains("<Controller c0: 127.0.0.1:6653 pid=None> \n"),
        "dump: {dump}"
    );
    // controllers come after hosts and switches in values() order
    let keys = net.keys();
    assert_eq!(keys, vec!["h1", "h2", "s1", "c0"]);
}

/// Dry-run never touches the kernel: building + planning + stop must work
/// with no privileges (this test runs in the non-root workspace gate).
#[test]
fn dry_run_is_pure_and_stoppable_non_root() {
    let mut net = Mininet::dry_run("tree,2,2", opts()).unwrap();
    assert!(net.built);
    assert!(!net.plan().is_empty());
    assert!(!net.dump().is_empty());
    assert!(!net.links().is_empty());
    net.stop().unwrap();
    assert!(!net.built);
}
