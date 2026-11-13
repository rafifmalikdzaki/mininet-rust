//! Golden tests for `mn --topo <spec> --dump` (task 10b): the binary must
//! exit 0 non-root (dry-run path) and print the `=== topo ===` /
//! `=== dump ===` / `=== links ===` sections in the exact Python format.
//!
//! The `=== topo ===` section is compared against the Python-captured
//! baseline fixtures (`fixtures/topo_single3.txt`, `topo_linear4.txt`,
//! `topo_tree.txt`); the dump/links sections against checked-in goldens.

mod support;

use std::path::PathBuf;

use support::run_mn;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Parse a `fixtures/topo_*.txt` capture into
/// `(spec, class, nodes, hosts, switches, links)` where `links` are
/// `src-eth<port1><->dst-eth<port2>` strings.
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
            links.push(parse_fixture_link(line));
        }
    }
    (spec, class, nodes, hosts, switches, links)
}

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

fn parse_fixture_link(line: &str) -> String {
    let body = line.trim().strip_prefix("('").expect("tuple start");
    let (src, rest) = body.split_once("', '").expect("src");
    let (dst, rest) = rest.split_once("', ").expect("dst");
    let port1 = rest_after(rest, "'port1': ").parse::<i64>().expect("port1");
    let port2 = rest_after(rest, "'port2': ").parse::<i64>().expect("port2");
    format!("{src}-eth{port1}<->{dst}-eth{port2}")
}

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

/// Extract one `=== <name> ===` section (header + body lines) from the mn
/// stdout; the body runs up to the next `\n=== ` marker.
fn section(stdout: &str, name: &str) -> String {
    let header = format!("=== {name} ===");
    let start = stdout
        .find(&header)
        .unwrap_or_else(|| panic!("missing section {header} in:\n{stdout}"));
    let rest = &stdout[start..];
    let body_end = rest[header.len()..]
        .find("\n=== ")
        .map(|i| header.len() + i + 1) // keep the newline before the next header
        .unwrap_or(rest.len());
    rest[..body_end].to_string()
}

fn run_dump(spec: &str) -> String {
    let output = run_mn(&[
        "--topo",
        spec,
        "--switch",
        "default",
        "--host",
        "proc",
        "--controller",
        "none",
        "--link",
        "default",
        "--ipbase",
        "10.0.0.0/8",
        "--dump",
    ]);
    assert!(
        output.status.success(),
        "mn --topo {spec} --dump must exit 0 non-root, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

/// For each fixture: `mn --topo <spec> --dump` (non-root) must produce a
/// `=== topo ===` section identical in structure to the Python capture.
#[test]
fn dump_topo_sections_match_python_fixtures() {
    for fixture in ["topo_single3.txt", "topo_linear4.txt", "topo_tree.txt"] {
        let (spec, class, nodes, hosts, switches, links) = parse_fixture(fixture);
        let stdout = run_dump(&spec);
        let topo = section(&stdout, "topo");
        let mut sorted_links = links.clone();
        sorted_links.sort_by_key(|s| mn_util::natural(s));
        let expected = format!(
            "=== topo ===\n\
             spec: {spec}\n\
             class: {class}\n\
             nodes: {}\n\
             hosts: {}\n\
             switches: {}\n\
             links: {}\n",
            nodes.join(" "),
            hosts.join(" "),
            switches.join(" "),
            sorted_links.join(" ")
        );
        assert_eq!(topo, expected, "topo section mismatch for {fixture}");
    }
}

/// Full golden output for `mn --topo single,3 --controller none --dump`
/// (non-root dry-run): exact plan+dump+links text, OVSSwitch repr included.
#[test]
fn dump_single3_full_golden() {
    let stdout = run_dump("single,3");
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
    assert_eq!(stdout, expected);
}

#[test]
fn dump_linear4_and_tree22_golden_dump_links() {
    let stdout = run_dump("linear,4");
    let dump = section(&stdout, "dump");
    assert!(
        dump.contains("<OVSSwitch s4: lo:127.0.0.1,s4-eth1:None,s4-eth2:None pid=None> \n"),
        "{dump}"
    );
    assert!(
        dump.contains("<Host h4: h4-eth0:10.0.0.4 pid=None> \n"),
        "{dump}"
    );
    let links = section(&stdout, "links");
    assert!(
        links.contains("s4-eth2<->s3-eth3 (MISSING MISSING)\n"),
        "{links}"
    );

    let stdout = run_dump("tree,2,2");
    let links = section(&stdout, "links");
    assert!(
        links.contains("s1-eth1<->s2-eth3 (MISSING MISSING)\n"),
        "{links}"
    );
    assert!(
        links.contains("s3-eth2<->h4-eth0 (MISSING MISSING)\n"),
        "{links}"
    );
}

/// Custom `--ipbase` flows through to host IPs in the dump.
#[test]
fn dump_respects_ipbase() {
    let output = run_mn(&[
        "--topo",
        "single,2",
        "--controller",
        "none",
        "--ipbase",
        "192.168.0.0/16",
        "--dump",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("<Host h1: h1-eth0:192.168.0.1 pid=None> "),
        "{stdout}"
    );
    assert!(
        stdout.contains("<Host h2: h2-eth0:192.168.0.2 pid=None> "),
        "{stdout}"
    );
}
