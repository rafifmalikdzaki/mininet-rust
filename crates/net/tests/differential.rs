//! Differential test: build the same topology plans through the ORIGINAL
//! Python mininet code (driven by `tests/py_driver_plan.py`, pure topo/util —
//! no root needed) and through the Rust `mn-net` dry-run, and assert the
//! plan text (`Mininet::plan()`) is byte-identical.
//!
//! Protocol: specs (one per line, optionally `<spec> <ipbase>`) are sent to
//! the driver on stdin; it answers with `===CASE <spec>|<ipbase>===` blocks
//! containing the plan (or `===ERROR <class>===`) delimited by `===END===`.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use mn_net::{ControllerKind, Mininet, MininetOptions};

/// The specs compared against Python (task 9 requires single,3 / linear,4 /
/// tree,2,2; the rest are extra coverage of the built-in topos).
const SPECS: &[&str] = &[
    "single,3",
    "linear,4",
    "tree,2,2",
    "minimal",
    "reversed,3",
    "single,1",
    "linear,2",
    "tree,3,2",
    "torus,3,3",
    "single,3 192.168.0.0/16",
    "linear,3 10.1.0.0/16",
];

fn driver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/py_driver_plan.py")
}

fn run_driver(inputs: &[String]) -> String {
    let mut child = Command::new("python3")
        .arg(driver_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn python3 (is python3 on PATH?)");
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(inputs.join("\n").as_bytes())
        .expect("write specs to python3 stdin");
    let output = child.wait_with_output().expect("wait for python3");
    assert!(
        output.status.success(),
        "python3 driver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("driver output is utf-8")
}

/// Split the driver output into `(case-header, body)` blocks.
fn parse_blocks(driver_out: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut lines = driver_out.lines().peekable();
    while let Some(line) = lines.next() {
        let case = line
            .strip_prefix("===CASE ")
            .and_then(|s| s.strip_suffix("==="))
            .unwrap_or_else(|| panic!("expected ===CASE=== header, got {line:?}"));
        let mut body = String::new();
        for l in lines.by_ref() {
            if l == "===END===" {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        blocks.push((case.to_string(), body));
    }
    blocks
}

fn rust_plan(spec: &str, ipbase: &str) -> String {
    let net = Mininet::dry_run(
        spec,
        MininetOptions {
            controller: Some(ControllerKind::None), // --controller none
            ip_base: ipbase.to_string(),
            ..Default::default()
        },
    )
    .expect("dry_run succeeds");
    net.plan()
}

#[test]
fn dry_run_plans_match_python_byte_for_byte() {
    let inputs: Vec<String> = SPECS.iter().map(|s| s.to_string()).collect();
    let driver_out = run_driver(&inputs);
    let blocks = parse_blocks(&driver_out);
    assert_eq!(
        blocks.len(),
        SPECS.len(),
        "driver returned {} blocks for {} specs",
        blocks.len(),
        SPECS.len()
    );
    let mut checked = 0;
    for ((spec_line, body), input) in blocks.iter().zip(inputs.iter()) {
        let (spec, ipbase) = match spec_line.split_once('|') {
            Some((s, b)) => (s, b),
            None => panic!("bad case header {spec_line:?}"),
        };
        // the driver echoes `<spec>|<ipbase>`; rebuild it from the input line
        let (in_spec, in_ipbase) = match input.split_once(' ') {
            Some((s, b)) => (s, b),
            None => (input.as_str(), "10.0.0.0/8"),
        };
        assert_eq!(
            (in_spec, in_ipbase),
            (spec, ipbase),
            "driver case order mismatch: {input:?} vs {spec_line:?}"
        );
        assert!(
            !body.contains("===ERROR"),
            "python driver errored on {spec}: {body}"
        );
        let rust = rust_plan(spec, ipbase);
        assert_eq!(
            rust, *body,
            "plan mismatch for spec {spec:?} ipbase {ipbase:?}\n--- rust ---\n{rust}\n--- python ---\n{body}"
        );
        checked += 1;
    }
    assert_eq!(checked, SPECS.len());
}
