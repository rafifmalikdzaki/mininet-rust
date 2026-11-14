use mn_net::{Mininet, MininetOptions};
use mn_topo::{Attrs, Topo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    nodes: Vec<Node>,
    links: Vec<Link>,
    steps: Vec<Step>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    name: String,
    kind: Kind,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Host,
    Switch,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    a: String,
    b: String,
    port_a: Option<i64>,
    port_b: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    node: String,
    command: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
    #[serde(default)]
    retry_secs: u64,
    expect_stdout: Option<String>,
}
fn default_timeout() -> u64 {
    30
}

fn validate(s: &Scenario) -> Result<()> {
    if s.name.is_empty()
        || s.name.len() > 64
        || !s
            .name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        || s.nodes.is_empty()
    {
        return Err("scenario name must contain only ASCII letters, digits, underscores or hyphens (1–64 characters), and nodes must not be empty".into());
    }
    let mut nodes = HashMap::new();
    for node in &s.nodes {
        if node.name.is_empty()
            || node.name.len() > 9
            || !node
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(format!(
                "invalid node name {:?}: use 1–9 ASCII letters, digits or underscores",
                node.name
            )
            .into());
        }
        if nodes.insert(node.name.as_str(), node.kind).is_some() {
            return Err(format!("duplicate node {}", node.name).into());
        }
    }
    let mut ports = HashSet::new();
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for link in &s.links {
        if link.a == link.b
            || !nodes.contains_key(link.a.as_str())
            || !nodes.contains_key(link.b.as_str())
        {
            return Err(format!("invalid link {}--{}", link.a, link.b).into());
        }
        for (name, explicit) in [(&link.a, link.port_a), (&link.b, link.port_b)] {
            let count = counts.entry(name.as_str()).or_default();
            let base = i64::from(matches!(nodes[name.as_str()], Kind::Switch));
            let port = explicit.unwrap_or(*count + base);
            if !(base..=999).contains(&port) || !ports.insert((name.as_str(), port)) {
                return Err(format!("invalid or duplicate port {name}:{port}").into());
            }
            if name.len() + "-eth".len() + port.to_string().len() > 15 {
                return Err(format!(
                    "interface {name}-eth{port} exceeds the 15-character kernel limit; shorten the node name or use a lower port"
                )
                .into());
            }
            *count += 1;
        }
    }
    for step in &s.steps {
        if !nodes.contains_key(step.node.as_str())
            || step.command.is_empty()
            || step.timeout_secs == 0
        {
            return Err(format!("invalid step for node {}", step.node).into());
        }
    }
    Ok(())
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn open_directory(path: &Path) -> Result<fs::File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?)
}

fn inherit_owner(file: &fs::File, parent: &Path) -> Result<()> {
    let owner = open_directory(parent)?.metadata()?;
    if unsafe { libc::fchown(file.as_raw_fd(), owner.uid(), owner.gid()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn create_owned_dir(path: &Path) -> Result<()> {
    fs::create_dir(path)?;
    let dir = open_directory(path)?;
    inherit_owner(&dir, parent_dir(path))
}

fn artifact_file(path: &Path, append: bool) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).custom_flags(libc::O_NOFOLLOW);
    if append {
        options.create(true).append(true);
    } else {
        options.create_new(true);
    }
    let file = options.open(path)?;
    inherit_owner(&file, parent_dir(path))?;
    Ok(file)
}

fn write_artifact(path: &Path, contents: &[u8]) -> Result<()> {
    artifact_file(path, false)?.write_all(contents)?;
    Ok(())
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn event(dir: &Path, value: serde_json::Value) -> Result<()> {
    let line = serde_json::to_string(&value)?;
    let mut file = artifact_file(&dir.join("events.jsonl"), true)?;
    writeln!(file, "{line}")?;
    println!("{line}");
    Ok(())
}

#[derive(Serialize)]
struct StepResult<'a> {
    node: &'a str,
    command: &'a str,
    exit_code: Option<i32>,
    duration_ms: u128,
    attempt: u64,
    success: bool,
}

// Keep a reference to the namespace before Mininet removes its named mount;
// after stop, kill processes (notably daemonized FRR) still holding that namespace.
struct Network {
    net: Mininet,
    namespaces: Vec<PathBuf>,
}
impl Network {
    fn stop(&mut self) -> Result<()> {
        let result = self.net.stop().map_err(|e| e.to_string());
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            if pid == std::process::id() as i32 {
                continue;
            }
            if let Ok(ns) = fs::read_link(entry.path().join("ns/net")) {
                if self.namespaces.contains(&ns) {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
        self.namespaces.clear();
        result.map_err(Into::into)
    }
}
/// Keep only records belonging to `node` (plus loopback and records without
/// the scope key, e.g. blackhole routes). Switches share the container network
/// namespace, so an unfiltered sample would attribute every switch's ports to
/// whichever switch was sampled. Returns `(bytes, scoped)`; `scoped` is false
/// when the probe output is not a JSON array and is passed through unchanged.
fn scope_interfaces(node: &str, raw: &[u8], key: &str) -> (Vec<u8>, bool) {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_slice(raw) else {
        return (raw.to_vec(), false);
    };
    let prefix = format!("{node}-eth");
    let filtered: Vec<_> = items
        .into_iter()
        .filter(
            |item| match item.get(key).and_then(|value| value.as_str()) {
                Some(value) => value == node || value == "lo" || value.starts_with(&prefix),
                None => true,
            },
        )
        .collect();
    match serde_json::to_vec(&filtered) {
        Ok(bytes) => (bytes, true),
        Err(_) => (raw.to_vec(), false),
    }
}

fn sample(net: &Mininet, s: &Scenario, dir: &Path) -> Result<()> {
    let mut file = artifact_file(&dir.join("telemetry.jsonl"), true)?;
    for node in &s.nodes {
        let Some(n) = net.get(&node.name) else {
            continue;
        };
        if n.node().in_namespace && n.node().pid().is_none() {
            continue;
        }
        let pathspace = format!("mnr-{}-{}", s.name, node.name);
        let base = Path::new("/var/run/frr").join(&pathspace);
        let mut probes = vec![
            ("ip_link", "ip -j -s link"),
            ("ip_addr", "ip -j address"),
            ("ip_route", "ip -j route show table all"),
        ];
        if base.join("ospfd.conf").exists() {
            probes.extend([
                ("ospf_neighbor", "show ip ospf neighbor"),
                ("ospf_routes", "show ip ospf route"),
            ]);
        }
        if base.join("isisd.conf").exists() {
            probes.extend([
                ("isis_neighbor", "show isis neighbor"),
                ("isis_routes", "show isis route"),
            ]);
        }
        if base.join("bgpd.conf").exists() {
            probes.extend([
                ("bgp_summary", "show bgp summary"),
                ("bgp_routes", "show bgp ipv4 unicast"),
            ]);
        }
        for (probe, query) in probes {
            let mut cmd = Command::new("timeout");
            cmd.args(["--kill-after=1s", "5s"]);
            if n.node().in_namespace {
                cmd.arg("mnexec").arg("-a").arg(
                    n.node()
                        .pid()
                        .ok_or("node has no namespace process")?
                        .to_string(),
                );
            }
            if probe.starts_with("ip_") {
                cmd.arg("/bin/sh").arg("-c").arg(query);
            } else {
                cmd.args(["vtysh", "-N", &pathspace, "-c", query]);
            }
            let output = cmd.output()?;
            // Switches share the container network namespace, so their probes
            // must be narrowed to the sampled switch. Host probes already run
            // inside the host's namespace and must stay complete: scenarios may
            // create non-`<node>-eth` interfaces (gre, tun, bridges).
            let filtered =
                !n.node().in_namespace && matches!(probe, "ip_link" | "ip_addr" | "ip_route");
            let (stdout, scoped) = match probe {
                "ip_link" | "ip_addr" if filtered => {
                    scope_interfaces(&node.name, &output.stdout, "ifname")
                }
                "ip_route" if filtered => scope_interfaces(&node.name, &output.stdout, "dev"),
                _ => (output.stdout.clone(), false),
            };
            let record = json!({"timestamp_ms":timestamp(), "node":node.name, "probe":probe, "command":query,
                "stdout_scope": if scoped {"node"} else {"raw"},
                "exit_code":output.status.code(),
                "stdout":String::from_utf8_lossy(&stdout),
                "stderr":String::from_utf8_lossy(&output.stderr)});
            writeln!(file, "{}", serde_json::to_string(&record)?)?;
            if !output.status.success() {
                event(
                    dir,
                    json!({"event":"telemetry_error", "timestamp_ms":timestamp(), "node":node.name,
                    "probe":probe, "exit_code":output.status.code(), "stderr":String::from_utf8_lossy(&output.stderr)}),
                )?;
            }
        }
    }
    Ok(())
}

fn capture_frr(s: &Scenario, dir: &Path) -> Result<()> {
    create_owned_dir(&dir.join("frr"))?;
    for node in &s.nodes {
        let name = format!("mnr-{}-{}", s.name, node.name);
        let source = Path::new("/var/run/frr").join(&name);
        match fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(format!("FRR path is not a directory: {}", source.display()).into())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let destination = dir.join("frr").join(&name);
        create_owned_dir(&destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let mut source = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(entry.path())?;
                let permissions = source.metadata()?.permissions();
                let mut output = artifact_file(&destination.join(entry.file_name()), false)?;
                std::io::copy(&mut source, &mut output)?;
                output.set_permissions(permissions)?;
            }
        }
    }
    Ok(())
}

impl Drop for Network {
    fn drop(&mut self) {
        if !self.namespaces.is_empty() {
            let _ = self.stop();
        }
    }
}

fn run(s: &Scenario, dir: &Path, hold_secs: u64) -> Result<()> {
    let mut topo = Topo::new();
    for node in &s.nodes {
        match node.kind {
            Kind::Host => {
                topo.add_host(&node.name, Attrs::new());
            }
            Kind::Switch => {
                topo.add_switch(&node.name, Attrs::new());
            }
        }
    }
    for l in &s.links {
        topo.add_link(&l.a, &l.b, l.port_a, l.port_b, None, Attrs::new());
    }
    let options = MininetOptions {
        topo: Some(topo),
        build: false,
        controller: None,
        switch_spec: "lxbr".into(),
        ..Default::default()
    };
    let mut net = Network {
        net: Mininet::new(options)?,
        namespaces: Vec::new(),
    };
    let outcome = (|| -> Result<()> {
        let started = net.net.start();
        // Capture namespaces even on partial startup so cleanup covers failures.
        for node in &s.nodes {
            if let Some(n) = net.net.get(&node.name) {
                if n.node().in_namespace {
                    if let Some(pid) = n.node().pid() {
                        if let Ok(ns) = fs::read_link(format!("/proc/{pid}/ns/net")) {
                            net.namespaces.push(ns);
                        }
                    }
                }
            }
        }
        started?;
        sample(&net.net, s, dir)?;
        for (index, step) in s.steps.iter().enumerate() {
            let deadline = Instant::now() + Duration::from_secs(step.retry_secs);
            let mut attempt = 0;
            loop {
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return Err("interrupted".into());
                }
                attempt += 1;
                let node = net.net.get(&step.node).ok_or("node disappeared")?;
                let mut command = Command::new("timeout");
                command
                    .arg("--kill-after=2s")
                    .arg(format!("{}s", step.timeout_secs));
                if node.node().in_namespace {
                    command.arg("mnexec").arg("-a").arg(
                        node.node()
                            .pid()
                            .ok_or("node has no namespace process")?
                            .to_string(),
                    );
                }
                command.arg("/bin/sh").arg("-c").arg(&step.command);
                let start = Instant::now();
                let output = command.output()?;
                let success = output.status.success()
                    && step.expect_stdout.as_ref().is_none_or(|expected| {
                        String::from_utf8_lossy(&output.stdout).contains(expected)
                    });
                let result = StepResult {
                    node: &step.node,
                    command: &step.command,
                    exit_code: output.status.code(),
                    duration_ms: start.elapsed().as_millis(),
                    attempt,
                    success,
                };
                let prefix = dir
                    .join("steps")
                    .join(format!("{index:04}-attempt-{attempt:04}"));
                write_artifact(&prefix.with_extension("stdout"), &output.stdout)?;
                write_artifact(&prefix.with_extension("stderr"), &output.stderr)?;
                write_artifact(
                    &prefix.with_extension("json"),
                    &serde_json::to_vec_pretty(&result)?,
                )?;
                let mut data = serde_json::to_value(&result)?;
                data["event"] = json!("step");
                data["index"] = json!(index);
                data["timestamp_ms"] = json!(timestamp());
                event(dir, data)?;
                sample(&net.net, s, dir)?;
                if success {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "step {index} on {} failed (exit {:?})",
                        step.node,
                        output.status.code()
                    )
                    .into());
                }
                std::thread::sleep(
                    Duration::from_secs(1).min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
        let until = Instant::now() + Duration::from_secs(hold_secs);
        while Instant::now() < until {
            if INTERRUPTED.load(Ordering::SeqCst) {
                return Err("interrupted".into());
            }
            std::thread::sleep(
                Duration::from_secs(1).min(until.saturating_duration_since(Instant::now())),
            );
            sample(&net.net, s, dir)?;
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err("interrupted".into());
        }
        Ok(())
    })();
    let captured = capture_frr(s, dir);
    let stopped = net.stop();
    match (outcome, captured, stopped) {
        (Err(e), captured, stopped) => Err(format!(
            "{e}; capture: {}; cleanup: {}",
            captured.err().map_or("ok".into(), |e| e.to_string()),
            stopped.err().map_or("ok".into(), |e| e.to_string())
        )
        .into()),
        (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        _ => Ok(()),
    }
}

fn provenance_command(program: &str, args: &[&str]) -> serde_json::Value {
    match Command::new(program).args(args).output() {
        Ok(output) => json!({
            "program": program, "args": args,
            "status": if output.status.success() { "ok" } else { "nonzero" },
            "exit_code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "error": null
        }),
        Err(error) => json!({
            "program": program, "args": args, "status": "unavailable",
            "exit_code": null, "stdout": null, "stderr": null, "error": error.to_string()
        }),
    }
}

fn write_provenance(dir: &Path) -> Result<()> {
    let source_revision = std::env::var("EXPERIMENT_SOURCE_REVISION")
        .ok()
        .filter(|s| !s.is_empty());
    let source_dirty = std::env::var("EXPERIMENT_SOURCE_DIRTY")
        .ok()
        .filter(|s| !s.is_empty());
    let provenance = json!({
        "timestamp_ms": timestamp(),
        "runner_version": env!("CARGO_PKG_VERSION"),
        "source_revision": source_revision,
        "source_dirty": source_dirty,
        "commands": {
            "utc": provenance_command("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
            "uname": provenance_command("uname", &["-a"]),
            "frr_version": provenance_command("/usr/lib/frr/zebra", &["-v"]),
            "mnexec_version": provenance_command("mnexec", &["-v"]),
            "mnr_version": provenance_command("mnr", &["--version"])
        }
    });
    write_artifact(
        &dir.join("provenance.json"),
        &serde_json::to_vec_pretty(&provenance)?,
    )?;
    Ok(())
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("experiment-runner: {e}");
        std::process::exit(1);
    }
}
fn real_main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 && args.len() != 5 {
        return Err(
            "usage: experiment-runner SCENARIO.json NEW_RUN_DIRECTORY [--hold-secs N]".into(),
        );
    }
    let hold_secs = if args.len() == 5 {
        if args[3] != "--hold-secs" {
            return Err("expected --hold-secs N".into());
        }
        args[4]
            .to_str()
            .ok_or("invalid hold duration")?
            .parse::<u64>()?
    } else {
        0
    };
    let input = fs::read(&args[1])?;
    let scenario: Scenario = serde_json::from_slice(&input)?;
    validate(&scenario)?;
    let dir = Path::new(&args[2]);
    create_owned_dir(dir)?;
    create_owned_dir(&dir.join("steps"))?;
    write_artifact(&dir.join("scenario.json"), &input)?;
    write_provenance(dir)?;
    unsafe {
        libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, interrupt as *const () as libc::sighandler_t);
    }
    event(
        dir,
        json!({"event":"start", "timestamp_ms":timestamp(), "scenario":scenario.name}),
    )?;
    let result = run(&scenario, dir, hold_secs);
    event(
        dir,
        json!({"event":"finish", "timestamp_ms":timestamp(), "success":result.is_ok(), "error":result.as_ref().err().map(|e| e.to_string())}),
    )?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_arbitrary_host_switch_topology() {
        let s: Scenario = serde_json::from_str(r#"{"name":"test","nodes":[{"name":"r1","kind":"host"},{"name":"s1","kind":"switch"}],"links":[{"a":"r1","b":"s1"}],"steps":[{"node":"r1","command":"true"}]}"#).unwrap();
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn rejects_collision_between_explicit_and_automatic_ports() {
        let s: Scenario = serde_json::from_str(r#"{"name":"test","nodes":[{"name":"r1","kind":"host"},{"name":"r2","kind":"host"},{"name":"r3","kind":"host"}],"links":[{"a":"r1","b":"r2","port_a":1},{"a":"r1","b":"r3"}],"steps":[]}"#).unwrap();
        assert!(validate(&s).is_err());
    }

    #[test]
    fn rejects_interface_name_over_kernel_limit() {
        let s: Scenario = serde_json::from_str(r#"{"name":"test","nodes":[{"name":"abcdefghi","kind":"host"},{"name":"r2","kind":"host"}],"links":[{"a":"abcdefghi","b":"r2","port_a":999}],"steps":[]}"#).unwrap();
        let error = validate(&s).unwrap_err().to_string();
        assert!(error.contains("15-character"), "{error}");
    }

    #[test]
    fn accepts_interface_name_at_kernel_limit() {
        let s: Scenario = serde_json::from_str(r#"{"name":"test","nodes":[{"name":"abcdefgh","kind":"host"},{"name":"r2","kind":"host"}],"links":[{"a":"abcdefgh","b":"r2","port_a":999}],"steps":[]}"#).unwrap();
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn interface_scope_excludes_other_nodes() {
        let raw = br#"[{"ifname":"s1","x":1},{"ifname":"s2","x":2},{"ifname":"s3","x":3},{"ifname":"s1-eth1","x":4},{"ifname":"s2-eth1","x":5},{"ifname":"lo","x":6}]"#;
        let (bytes, scoped) = scope_interfaces("s1", raw, "ifname");
        assert!(scoped);
        let scoped_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let names: Vec<_> = scoped_json
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["ifname"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["s1", "s1-eth1", "lo"]);
    }

    #[test]
    fn interface_scope_keeps_unscoped_records() {
        let raw = br#"[{"dst":"10.0.0.0/8","dev":"s2"},{"dst":"10.1.0.0/16","dev":"s1"},{"dst":"192.0.2.0/24","type":"blackhole"}]"#;
        let (bytes, scoped) = scope_interfaces("s1", raw, "dev");
        assert!(scoped);
        let scoped_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let routes: Vec<_> = scoped_json
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["dst"].as_str().unwrap())
            .collect();
        assert_eq!(routes, ["10.1.0.0/16", "192.0.2.0/24"]);
    }

    #[test]
    fn interface_scope_preserves_unparseable_output() {
        let raw = b"not json\n";
        let (bytes, scoped) = scope_interfaces("s1", raw, "ifname");
        assert_eq!(bytes, raw);
        assert!(!scoped);
    }
}
