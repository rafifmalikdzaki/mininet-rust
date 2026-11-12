//! Interactive command loop for `mn` — the task 10c port of
//! `mininet/cli.py`'s `CLI` class (a `cmd.Cmd` subclass).
//!
//! Ported machinery, mirroring `cli.py` + CPython `cmd.py`:
//!
//! - The loop reads lines from stdin and dispatches via `parseline`/`onecmd`
//!   semantics: `precmd` strips `#` comments, `?foo` maps to `help foo`,
//!   `!` lines and unrecognized prefixes fall to `default()`, an empty line
//!   does nothing (`emptyline()`), and EOF behaves like the `EOF` command
//!   (`do_EOF`: `output('\n')` then exit).
//! - The `mininet> ` prompt is written **only when stdin is a tty** (task
//!   spec; note CPython's `input(prompt)` prints it even on pipes — recorded
//!   deviation).
//! - `output()`-level text (nodes/net/links/dump/ports/intfs/ping/iperf/...)
//!   goes to `out`; `error()`-level lines (`*** Unknown command: ...`,
//!   `node 'x' not in network`, usage lines) go to `err`. In real `mn` both
//!   Python log streams land on the terminal (logger defaults to stderr;
//!   `scripts/diff_topology.sh` redirects the logger to stdout); this port
//!   follows the repo convention of the `runner`: command output on `out`
//!   (stdout), diagnostics on `err` (stderr).
//! - `help` reproduces `cmd.Cmd.do_help` exactly: the columnized
//!   "Documented commands" listing (all 29 `do_*` names of `cli.py`, with
//!   their docstrings for `help <cmd>`) plus `cli.py`'s `helpStr`.
//!
//! Ported commands (handlers below): `help`, `exit`/`quit`/`EOF`, `nodes`,
//! `net`, `links`, `dump`, `ports`, `intfs`, `pingall`, `pingallfull`,
//! `pingpair`, `pingpairfull`, `iperf`, `iperfudp`, `dpctl`, `sh`, `source`,
//! `xterm`/`gterm` (stub: clear not-ported error), plus the 10b convenience
//! commands `hosts`/`switches`/`controllers`/`ping <a> <b>` (not in stock
//! `cli.py` — see "Deviations" below). Unknown commands produce Python's
//! `*** Unknown command: <line>` error line via `default()`.
//!
//! Not ported yet (fall through to `*** Unknown command`, task 10d):
//! `link`, `switch`, `wait`, `time`, `noecho`, `py`, `px`, `x`, and the
//! `default()` node passthrough (`h1 ifconfig`). They still appear in the
//! `help` listing because `cli.py` defines them.
//!
//! Deviations from CPython (all deliberate, documented):
//! - Prompt only on tty (see above; CPython prints it always).
//! - `do_sh` pipes the child's stdout/stderr and forwards them to `out`/`err`
//!   instead of inheriting the process fds, so scripted-stdin tests can
//!   capture the output (`subprocess.call(line, shell=True)` blocks either
//!   way, so non-interactive content is identical).
//! - `xterm`/`gterm` validate args exactly like `cli.py` (usage line / `node
//!   'x' not in network`) but then emit a clear not-ported error instead of
//!   `makeTerms` (task spec: stub for now).
//! - `ping <a> <b>`, `hosts`, `switches`, `controllers` are conveniences not
//!   present in stock `cli.py` (where they would hit `default()` → `***
//!   Unknown command`); the task list and the 10b skeleton both define them,
//!   so they are kept: `ping a b` runs `Mininet.ping([a, b])` with `iperf`-style
//!   node validation, the listings print the sorted names (10b format).
//! - `route` has **no** `do_route` in `cli.py` (verified against the source
//!   and full git history, and a live capture shows `*** Unknown command:
//!   route`); the port reproduces that exactly via `default()`.
//! - `do_source` on an unreadable file prints `error reading file ...` like
//!   Python, but does not replicate Python 3.14's follow-on `AttributeError`
//!   crash (`self.inputFile.close()` on `None`).
//! - `do_source` applies `precmd` (comment stripping) to sourced-file lines,
//!   while CPython's `cmdloop` applies it only to interactive input, so a
//!   `# comment` line inside a sourced file is stripped here but prints
//!   `*** Unknown command: # ...` in Python (verified live on 3.14).
//! - `pingallfull`/`pingpairfull` need per-pair RTT parsing (`_parsePingFull`,
//!   not yet in `mn-net`), so it is ported here on top of the public `mn-net`
//!   API (scope is `apps/mn/**` only). On a non-started (dry-run) net the
//!   host `cmd()` fails; that error text is fed through the same
//!   "could not parse ping output" path Python uses for unparseable output
//!   (Python itself would crash with `TypeError` on a shell-less node, so no
//!   golden exists there — on a started net both produce real rtt lines).
//! - On a dry-run net, `ping`/`iperf`/`dpctl` fail with the node/shell error;
//!   the repl reports them as `*** <cmd> failed: <err>` on `err` (Python has
//!   no equivalent line — a shell-less net crashes its CLI).

use std::io::{BufRead, IsTerminal, Write};

use mn_net::{Mininet, NetSwitch, NodeRef};

/// `CLI.prompt` in `mininet/cli.py`.
pub const PROMPT: &str = "mininet> ";

/// Run the interactive loop on the real stdin/stdout until
/// `exit`/`quit`/EOF. Returns the exit code (0; `bin/mn` stops the net after
/// the CLI returns, which `runner` does).
pub fn run(net: &mut Mininet, out: &mut dyn Write) -> i32 {
    // Python CLI.__init__: info( '*** Starting CLI:\n' ) — info chatter on
    // stderr in real mn (not part of golden command output).
    eprintln!("*** Starting CLI:");
    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    run_with_io(net, &mut lock, out, &mut err, tty)
}

/// Testable core of [`run`]: read command lines from `stdin`, write
/// `output()`-level text to `out`, `error()`-level text to `err`, and the
/// `mininet> ` prompt to `out` only when `tty` is true. Returns 0 when the
/// loop ends (`exit`/`quit`/`EOF`).
pub fn run_with_io(
    net: &mut Mininet,
    stdin: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    tty: bool,
) -> i32 {
    loop {
        if tty {
            let _ = write!(out, "{PROMPT}");
            let _ = out.flush();
        }
        let mut buf = String::new();
        match stdin.read_line(&mut buf) {
            Ok(0) => {
                // EOF: cmdloop maps it to the 'EOF' command -> do_EOF:
                // output('\n') then exit.
                let _ = writeln!(out);
                break;
            }
            Ok(_) => {}
            Err(_) => {
                let _ = writeln!(out);
                break;
            }
        }
        // input() strips one trailing newline (and a preceding \r).
        let raw = buf
            .strip_suffix('\n')
            .map(|s| s.strip_suffix('\r').unwrap_or(s))
            .unwrap_or(&buf);
        let (cmd, arg, line) = parseline(precmd(raw));
        if line.is_empty() {
            continue; // emptyline(): don't repeat the last command
        }
        let stop = match cmd.as_deref() {
            None => default(&line, err),
            Some(c) => dispatch(net, c, &arg, &line, out, err),
        };
        if stop {
            break;
        }
    }
    0
}

// ---------------------------------------------------------------------
// cmd.Cmd line machinery (precmd / parseline / onecmd / default)
// ---------------------------------------------------------------------

/// `CLI.precmd`: allow comments — everything from the first `#` is dropped.
fn precmd(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// `cmd.Cmd.parseline`: returns `(cmd, arg, line)` where `cmd` is `None` for
/// `!`-prefixed lines (no `do_shell` here) or a non-identifier start, `arg`
/// is the stripped remainder, and `line` is the stripped input (`?x` is
/// rewritten to `help x`).
fn parseline(line: &str) -> (Option<String>, String, String) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return (None, String::new(), trimmed.to_string());
    }
    let owned;
    let line = if let Some(rest) = trimmed.strip_prefix('?') {
        owned = format!("help {rest}");
        owned.as_str()
    } else {
        trimmed
    };
    if line.starts_with('!') {
        return (None, String::new(), line.to_string());
    }
    // identchars = ASCII letters + digits + '_'
    let i = line
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let cmd = &line[..i];
    let arg = line[i..].trim();
    (
        if cmd.is_empty() {
            None
        } else {
            Some(cmd.to_string())
        },
        arg.to_string(),
        line.to_string(),
    )
}

/// `CLI.default` for unrecognized commands. (The `first in self.mn` node
/// passthrough of Python's `default()` is task 10d — not ported yet, so it
/// also lands here.)
fn default(line: &str, err: &mut dyn Write) -> bool {
    let _ = writeln!(err, "*** Unknown command: {line}");
    false
}

/// `CLI.onecmd`: dispatch `cmd` with `arg` (the stripped rest of `line`).
/// Returns true when the loop should stop (`exit`/`quit`/`EOF`).
fn dispatch(
    net: &mut Mininet,
    cmd: &str,
    arg: &str,
    line: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> bool {
    match cmd {
        "help" => do_help(arg, out),
        "exit" | "quit" => return true,
        "EOF" => {
            // do_EOF: output('\n') then exit
            let _ = writeln!(out);
            return true;
        }
        "nodes" => do_nodes(net, out),
        "net" => do_net(net, out),
        "links" => do_links(net, out),
        "dump" => do_dump(net, out),
        "ports" => do_ports(net, out),
        "intfs" => do_intfs(net, out),
        "hosts" => do_list(net.hosts.keys(), out),
        "switches" => do_list(net.switches.keys(), out),
        "controllers" => do_list(net.controllers.keys(), out),
        "ping" => do_ping(net, arg, out, err),
        "pingall" => do_pingall(net, arg, out, err),
        "pingallfull" => ping_full(net, None, out, err),
        "pingpair" => do_ping_pair(net, out, err),
        "pingpairfull" => {
            let names: Vec<String> = net.hosts.keys().take(2).map(str::to_string).collect();
            ping_full(net, Some(names), out, err);
        }
        "iperf" => do_iperf(net, arg, out, err, false),
        "iperfudp" => do_iperf(net, arg, out, err, true),
        "dpctl" => do_dpctl(net, arg, out, err),
        "sh" => do_sh(arg, out, err),
        "xterm" => do_xterm(net, arg, err, "xterm"),
        "gterm" => do_xterm(net, arg, err, "gterm"),
        "source" => {
            do_source(net, arg, out, err);
        }
        _ => {
            default(line, err);
        }
    }
    false
}

// ---------------------------------------------------------------------
// Handlers (cli.py do_* methods, in cli.py order)
// ---------------------------------------------------------------------

/// `do_help` + `cmd.Cmd.do_help`: `help` prints the columnized command
/// listing plus `helpStr`; `help <cmd>` prints the command's docstring (or
/// `*** No help on <cmd>`). Both go to Cmd's stdout (`out`).
fn do_help(arg: &str, out: &mut dyn Write) {
    if !arg.is_empty() {
        match COMMANDS.iter().find(|(name, _)| *name == arg) {
            Some((_, doc)) => {
                let _ = writeln!(out, "{doc}");
            }
            None => {
                let _ = writeln!(out, "*** No help on {arg}");
            }
        }
        return;
    }
    // Cmd.do_help with empty arg: doc_leader ("") + print_topics(...).
    let _ = write!(out, "\n{DOC_HEADER}\n{}\n", "=".repeat(DOC_HEADER.len()));
    let names: Vec<&str> = COMMANDS.iter().map(|(n, _)| *n).collect();
    let _ = write!(out, "{}", columnize(&names, 79));
    let _ = writeln!(out);
    // CLI.do_help: output( self.helpStr )
    let _ = write!(out, "{HELP_STR}");
}

/// `do_nodes`: `output( 'available nodes are: \n%s\n' % nodes )`.
fn do_nodes(net: &Mininet, out: &mut dyn Write) {
    let mut keys = net.keys();
    keys.sort();
    let _ = write!(out, "available nodes are: \n{}\n", keys.join(" "));
}

/// `do_net` → `util.dumpNodeConnections( mn.values() )`:
/// `output(node.name)`, then per intf `output(' %s:' % intf)` +
/// `output(remote_intf)` (or `output(' ')` when the intf has no link), then
/// `output('\n')`.
fn do_net(net: &Mininet, out: &mut dyn Write) {
    for name in net.keys() {
        let node = net.get(&name).expect("keys in sync");
        let _ = write!(out, "{}", node.node().name);
        for (iname, _) in node_intfs(node) {
            let _ = write!(out, " {iname}:");
            let remote = net
                .links
                .iter()
                .find(|l| l.intf1.name == iname || l.intf2.name == iname)
                .map(|l| {
                    if l.intf1.name == iname {
                        l.intf2.name.clone()
                    } else {
                        l.intf1.name.clone()
                    }
                });
            match remote {
                Some(r) => {
                    let _ = write!(out, "{r}");
                }
                None => {
                    let _ = write!(out, " ");
                }
            }
        }
        let _ = writeln!(out);
    }
}

/// `do_links`: `output( link, link.status(), '\n' )` — space-joined, so each
/// line is `<intf1><-><intf2> (<s1> <s2>) ` with a trailing space.
/// [`Mininet::links`] already renders `<intf1><-><intf2> (<s1> <s2>)` per
/// line; add the trailing space here (the `--dump` section keeps the
/// harness-normalized form).
fn do_links(net: &Mininet, out: &mut dyn Write) {
    for line in net.links().lines() {
        let _ = writeln!(out, "{line} ");
    }
}

/// `do_dump`: `output( '%s\n' % repr( node ) )` per node in `values()` order.
fn do_dump(net: &Mininet, out: &mut dyn Write) {
    let _ = write!(out, "{}", net.dump());
}

/// `do_ports` → `util.dumpPorts( mn.switches )`:
/// `output('%s ' % sw.name)`, per intf `output('%s:%d ' % (intf, port))`,
/// `output('\n')`.
fn do_ports(net: &Mininet, out: &mut dyn Write) {
    for name in net.switches.keys() {
        let sw = net.switches.get(name).expect("listed above");
        let s = sw.as_switch();
        let _ = write!(out, "{} ", s.node().name);
        for (iname, port) in node_intfs(NodeRef::Switch(s)) {
            let _ = write!(out, "{iname}:{} ", port.unwrap_or(0));
        }
        let _ = writeln!(out);
    }
}

/// `do_intfs`: `output( '%s: %s\n' % ( node.name, ','.join( node.intfNames() ) ) )`.
fn do_intfs(net: &Mininet, out: &mut dyn Write) {
    for name in net.keys() {
        let node = net.get(&name).expect("keys in sync");
        let names: Vec<String> = node_intfs(node).into_iter().map(|(n, _)| n).collect();
        let _ = writeln!(out, "{}: {}", node.node().name, names.join(","));
    }
}

/// Convenience listing (10b skeleton format): sorted names, space-joined.
fn do_list<'a>(keys: impl Iterator<Item = &'a str>, out: &mut dyn Write) {
    let mut v: Vec<&str> = keys.collect();
    v.sort_unstable();
    let _ = writeln!(out, "{}", v.join(" "));
}

/// `do_pingall`: `self.mn.pingAll( line )` — the arg is the ping timeout.
fn do_pingall(net: &mut Mininet, arg: &str, out: &mut dyn Write, err: &mut dyn Write) {
    // Python: pingAll(line) with line='' is falsy -> no '-W' option.
    let timeout = if arg.is_empty() { None } else { Some(arg) };
    match net.ping_all(timeout) {
        Ok(r) => {
            let _ = write!(out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(err, "*** pingall failed: {e}");
        }
    }
}

/// `do_pingpair`: `self.mn.pingPair()`.
fn do_ping_pair(net: &mut Mininet, out: &mut dyn Write, err: &mut dyn Write) {
    match net.ping_pair() {
        Ok(r) => {
            let _ = write!(out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(err, "*** pingpair failed: {e}");
        }
    }
}

/// Convenience `ping <a> <b>` (not in stock cli.py): `mn.ping([a, b])` with
/// `do_iperf`-style node validation.
fn do_ping(net: &mut Mininet, arg: &str, out: &mut dyn Write, err: &mut dyn Write) {
    let args = match parse_host_pair(arg) {
        Ok(a) => a,
        Err(line) => {
            let _ = write!(err, "{line}");
            return;
        }
    };
    let mut bad = false;
    for a in &args {
        if !net.contains(a) {
            let _ = writeln!(err, "node '{a}' not in network");
            bad = true;
        }
    }
    if bad {
        return;
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match net.ping(Some(refs), None) {
        Ok(r) => {
            let _ = write!(out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(err, "*** ping failed: {e}");
        }
    }
}

/// `do_iperf` / `do_iperfudp`.
fn do_iperf(net: &mut Mininet, arg: &str, out: &mut dyn Write, err: &mut dyn Write, udp: bool) {
    let (hosts, udp_bw) = match parse_iperf_args(arg, udp) {
        Ok(v) => v,
        Err(line) => {
            let _ = write!(err, "{line}");
            return;
        }
    };
    if let Some((a, b)) = &hosts {
        let mut bad = false;
        for x in [a, b] {
            if !net.contains(x) {
                let _ = writeln!(err, "node '{x}' not in network");
                bad = true;
            }
        }
        if bad {
            return;
        }
    }
    let l4 = if udp { "UDP" } else { "TCP" };
    let host_vec: Option<Vec<&str>> = hosts.as_ref().map(|(a, b)| vec![a.as_str(), b.as_str()]);
    // Python mn.iperf defaults: l4Type, udpBw='10M', seconds=5, port=5001.
    match net.iperf(host_vec, l4, udp_bw, 5, 5001) {
        Ok(r) => {
            let _ = write!(out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(err, "*** iperf failed: {e}");
        }
    }
}

/// `do_dpctl`: per switch, `output( '*** ' + name + ' ' + '-'*72 + '\n' )`
/// then `output( sw.dpctl( *args ) )` (no added newline).
fn do_dpctl(net: &mut Mininet, arg: &str, out: &mut dyn Write, err: &mut dyn Write) {
    let args: Vec<String> = arg.split_whitespace().map(str::to_string).collect();
    if args.is_empty() {
        let _ = writeln!(err, "usage: dpctl command [arg1] [arg2] ...");
        return;
    }
    let names: Vec<String> = net.switches.keys().map(str::to_string).collect();
    for name in &names {
        let _ = writeln!(out, "*** {name} {}", "-".repeat(72));
        let sw = net.net_switch_mut(name).expect("listed above");
        let res = match sw {
            NetSwitch::Ovs(s) => s.dpctl(&args),
            NetSwitch::Bridge(s) => s.ovs_mut().dpctl(&args),
            NetSwitch::User(s) => s.dpctl(&args),
            NetSwitch::Ivs(s) => s.dpctl(&args),
            NetSwitch::LxBr(s) => s.dpctl(&args),
        };
        match res {
            Ok(r) => {
                let _ = write!(out, "{r}");
            }
            Err(e) => {
                let _ = writeln!(err, "*** dpctl failed on {name}: {e}");
            }
        }
    }
}

/// `do_sh`: `call( line, shell=True )` — run via `/bin/sh -c`. The child's
/// stdout/stderr are piped and forwarded to `out`/`err` (so scripted-stdin
/// tests capture them); the exit status is ignored, like `call()`.
fn do_sh(arg: &str, out: &mut dyn Write, err: &mut dyn Write) {
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    match child {
        Ok(c) => match c.wait_with_output() {
            Ok(o) => {
                let _ = out.write_all(&o.stdout);
                let _ = err.write_all(&o.stderr);
            }
            Err(_) => {
                let _ = writeln!(err, "*** sh failed for {arg:?}");
            }
        },
        Err(e) => {
            let _ = writeln!(err, "*** sh failed to spawn: {e}");
        }
    }
}

/// `do_xterm` / `do_gterm`: validate args exactly like cli.py (usage line,
/// `node 'x' not in network`), then emit a clear not-ported error instead of
/// `makeTerms` (task spec: stub for now).
fn do_xterm(net: &Mininet, arg: &str, err: &mut dyn Write, term: &str) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.is_empty() {
        let _ = writeln!(err, "usage: {term} node1 node2 ...");
        return;
    }
    let mut valid: Vec<&str> = Vec::new();
    for a in &args {
        if net.contains(a) {
            valid.push(*a);
        } else {
            let _ = writeln!(err, "node '{a}' not in network");
        }
    }
    if !valid.is_empty() {
        let _ = writeln!(
            err,
            "*** {term}: not ported yet (mininet/term.py makeTerms TODO); \
             would spawn {term} for: {}",
            valid.join(" ")
        );
    }
}

/// `do_source`: run each line of a file through the same dispatch. Python
/// ignores `onecmd`'s return value here, so `exit` inside a sourced file does
/// NOT stop the outer loop.
fn do_source(net: &mut Mininet, arg: &str, out: &mut dyn Write, err: &mut dyn Write) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.len() != 1 {
        let _ = writeln!(err, "usage: source <file>");
        return;
    }
    let path = args[0];
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            let _ = writeln!(err, "error reading file {path}");
            return;
        }
    };
    for raw in content.lines() {
        let (cmd, sarg, line) = parseline(precmd(raw));
        if line.is_empty() {
            continue;
        }
        let stop = match cmd.as_deref() {
            None => default(&line, err),
            Some(c) => dispatch(net, c, &sarg, &line, out, err),
        };
        let _ = stop; // do_source ignores the stop signal
    }
}

// ---------------------------------------------------------------------
// pingFull (net.py pingFull/_parsePingFull — ported here because the scope
// of task 10c is apps/mn/** only; mn-net exposes ping/pingAll/pingPair)
// ---------------------------------------------------------------------

/// `(sent, received, rttmin, rttavg, rttmax, rttdev)` — the `_parsePingFull` tuple.
type PingFullTuple = (u64, u64, f64, f64, f64, f64);

/// One `*** Results:` row: `(src, dst, PingFullTuple)`.
type PingFullRow = (String, String, u64, u64, f64, f64, f64, f64);

/// Port of `Mininet.pingFull`: per ordered host pair run
/// `ping -c1 <opts> <dest.IP()>` and print `*** Results:` with per-pair rtt.
fn ping_full(
    net: &mut Mininet,
    hosts: Option<Vec<String>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) {
    let names: Vec<String> = match hosts {
        Some(v) if !v.is_empty() => v,
        _ => {
            let _ = writeln!(out, "*** Ping: testing ping reachability");
            net.hosts.keys().map(str::to_string).collect()
        }
    };
    // dest IP strings (Python: dest.IP(), which may render as 'None').
    let mut dests: Vec<(String, String)> = Vec::new();
    for n in &names {
        let ip = net
            .get(n)
            .map(|r| {
                r.node()
                    .ip(None)
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "None".to_string())
            })
            .unwrap_or_else(|| "None".to_string());
        dests.push((n.clone(), ip));
    }
    let mut all_outputs: Vec<PingFullRow> = Vec::new();
    for src in &names {
        let _ = write!(out, "{src} -> ");
        for (dst, ip) in &dests {
            if src == dst {
                continue;
            }
            // Python pingFull: node.cmd('ping -c1 %s %s' % (opts, dest.IP()))
            // — no LANG=C and no intfs check (unlike ping()).
            let result = match net.get_mut(src) {
                Some(mut r) => match r.node_mut().cmd(&format!("ping -c1  {ip}")) {
                    Ok(o) => o,
                    // Dry-run (no shell): feed the error text through the
                    // same unparseable-output path as Python (documented).
                    Err(e) => e.to_string(),
                },
                None => String::new(),
            };
            let (sent, received, rttmin, rttavg, rttmax, rttdev) = parse_ping_full(&result, err);
            all_outputs.push((
                src.clone(),
                dst.clone(),
                sent,
                received,
                rttmin,
                rttavg,
                rttmax,
                rttdev,
            ));
            if received > 0 {
                let _ = write!(out, "{dst} ");
            } else {
                let _ = write!(out, "X ");
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "*** Results: ");
    for (src, dst, sent, received, rttmin, rttavg, rttmax, rttdev) in &all_outputs {
        let _ = write!(out, " {src}->{dst}: {sent}/{received}, ");
        let _ = writeln!(
            out,
            "rtt min/avg/max/mdev {rttmin:.3}/{rttavg:.3}/{rttmax:.3}/{rttdev:.3} ms"
        );
    }
}

/// Port of `Mininet._parsePingFull` (error line included): the
/// `*** Error: could not parse ping output:` line goes to `err`.
fn parse_ping_full(output: &str, err: &mut dyn Write) -> PingFullTuple {
    let error_tuple: PingFullTuple = (1, 0, 0.0, 0.0, 0.0, 0.0);
    // r'[uU]nreachable' -> downed link
    if output.contains("unreachable") || output.contains("Unreachable") {
        return error_tuple;
    }
    // r'(\d+) packets transmitted, (\d+)( packets)? received'
    let Some((sent, received)) = parse_transmitted(output) else {
        let _ = writeln!(err, "*** Error: could not parse ping output: {output}");
        return error_tuple;
    };
    // r'rtt min/avg/max/mdev = (\d+\.\d+)/(\d+\.\d+)/(\d+\.\d+)/(\d+\.\d+) ms'
    match parse_rtt(output) {
        Some((mn, avg, mx, dev)) => (sent, received, mn, avg, mx, dev),
        None => {
            if received == 0 {
                error_tuple
            } else {
                let _ = writeln!(err, "*** Error: could not parse ping output: {output}");
                error_tuple
            }
        }
    }
}

/// The `(\d+) packets transmitted, (\d+)( packets)? received` half of the
/// `_parsePing`/`_parsePingFull` regexes.
fn parse_transmitted(output: &str) -> Option<(u64, u64)> {
    let marker = " packets transmitted, ";
    let pos = output.find(marker)?;
    let before = &output[..pos];
    let sent_start = before
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    let sent: u64 = before[sent_start..].parse().ok()?;
    let after = &output[pos + marker.len()..];
    let digits_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    let tail = &after[digits_end..];
    if !tail.starts_with(" received") && !tail.starts_with(" packets received") {
        return None;
    }
    let received: u64 = after[..digits_end].parse().ok()?;
    Some((sent, received))
}

/// The `rtt min/avg/max/mdev = <f>/<f>/<f>/<f> ms` regex (each `\d+\.\d+`).
fn parse_rtt(output: &str) -> Option<(f64, f64, f64, f64)> {
    let marker = "rtt min/avg/max/mdev = ";
    let pos = output.find(marker)? + marker.len();
    let mut rest = &output[pos..];
    let mut vals = [0.0f64; 4];
    for (idx, v) in vals.iter_mut().enumerate() {
        let num_end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        let num = &rest[..num_end];
        // regex group is \d+\.\d+: digits, a dot, digits
        if num.starts_with('.') || num.ends_with('.') || !num.contains('.') {
            return None;
        }
        *v = num.parse().ok()?;
        rest = &rest[num_end..];
        if idx < 3 {
            rest = rest.strip_prefix('/')?;
        } else if !rest.starts_with(" ms") {
            return None;
        }
    }
    Some((vals[0], vals[1], vals[2], vals[3]))
}

// ---------------------------------------------------------------------
// Arg parsing (unit-tested below)
// ---------------------------------------------------------------------

/// Parse `ping <a> <b>` args. Err is the exact error line (with `\n`).
fn parse_host_pair(arg: &str) -> Result<Vec<String>, String> {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.len() != 2 {
        return Err("invalid number of args: ping src dst\n".to_string());
    }
    Ok(args.iter().map(|s| s.to_string()).collect())
}

/// Parsed `iperf [a b]` / `iperfudp [bw a b]` args: `(hosts, udp_bw)`.
type IperfArgs<'a> = (Option<(String, String)>, &'a str);

/// Parse `iperf [a b]` / `iperfudp [bw a b]` args (cli.py do_iperf /
/// do_iperfudp). Err is the exact error line.
fn parse_iperf_args(arg: &str, udp: bool) -> Result<IperfArgs<'_>, String> {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if udp {
        if args.is_empty() {
            Ok((None, "10M"))
        } else if args.len() == 3 {
            Ok((Some((args[1].to_string(), args[2].to_string())), args[0]))
        } else {
            Err("invalid number of args: iperfudp bw src dst\nbw examples: 10M\n".to_string())
        }
    } else if args.is_empty() {
        Ok((None, "10M"))
    } else if args.len() == 2 {
        Ok((Some((args[0].to_string(), args[1].to_string())), "10M"))
    } else {
        Err("invalid number of args: iperf src dst\n".to_string())
    }
}

// ---------------------------------------------------------------------
// Node intf view (switch control intf 'lo' first, like Python's port-0 lo)
// ---------------------------------------------------------------------

/// `(name, port)` for the node's interfaces in `intfList()` order — for
/// root-namespace switches the control intf `lo` (port 0) comes first,
/// exactly like Python where `Switch.__init__` registers `Intf('lo', port=0)`.
fn node_intfs(node: NodeRef<'_>) -> Vec<(String, Option<u32>)> {
    let mut v: Vec<(String, Option<u32>)> = Vec::new();
    if let NodeRef::Switch(s) = node {
        if let Some(lo) = &s.control_intf {
            v.push((lo.name.clone(), lo.port));
        }
    }
    for i in node.node().intf_list() {
        v.push((i.name.clone(), i.port));
    }
    v
}

// ---------------------------------------------------------------------
// help machinery (cmd.Cmd do_help / print_topics / columnize)
// ---------------------------------------------------------------------

/// `Cmd.doc_header`.
const DOC_HEADER: &str = "Documented commands (type help <topic>):";

/// `CLI.helpStr` (cli.py), appended after a bare `help` listing.
/// (Explicit `\n` escapes: a `\` line-continuation in a Rust string literal
/// would strip the next line's leading spaces.)
const HELP_STR: &str = "You may also send a command to a node using:\n  <node> command {args}\nFor example:\n  mininet> h1 ifconfig\n\nThe interpreter automatically substitutes IP addresses\nfor node names when a node is the first arg, so commands\nlike\n  mininet> h2 ping h3\nshould work.\n\nSome character-oriented interactive commands require\nnoecho:\n  mininet> noecho h2 vi foo.py\nHowever, starting up an xterm/gterm is generally better:\n  mininet> xterm h2\n\n";

/// All `do_*` commands of `cli.py` (sorted by name, as `cmd.Cmd.get_names` +
/// `names.sort()` produces) with their docstrings — `help` lists these and
/// `help <cmd>` prints the doc, exactly like Python (docstrings are stored
/// dedented in CPython 3.13+, matching the captured output).
const COMMANDS: &[(&str, &str)] = &[
    ("EOF", "Exit"),
    (
        "dpctl",
        "Run dpctl (or ovs-ofctl) command on all switches.\nUsage: dpctl command [arg1] [arg2] ...",
    ),
    ("dump", "Dump node info."),
    ("exit", "Exit"),
    (
        "gterm",
        "Spawn gnome-terminal(s) for the given node(s).\nUsage: gterm node1 node2 ...",
    ),
    ("help", "Describe available CLI commands."),
    ("intfs", "List interfaces."),
    (
        "iperf",
        "Simple iperf TCP test between two (optionally specified) hosts.\nUsage: iperf node1 node2",
    ),
    (
        "iperfudp",
        "Simple iperf UDP test between two (optionally specified) hosts.\nUsage: iperfudp bw node1 node2",
    ),
    (
        "link",
        "Bring link(s) between two nodes up or down.\nUsage: link node1 node2 [up/down]",
    ),
    ("links", "Report on links"),
    ("net", "List network connections."),
    ("nodes", "List all nodes."),
    (
        "noecho",
        "Run an interactive command with echoing turned off.\nUsage: noecho [cmd args]",
    ),
    ("pingall", "Ping between all hosts."),
    ("pingallfull", "Ping between all hosts, returns all ping results."),
    ("pingpair", "Ping between first two hosts, useful for testing."),
    (
        "pingpairfull",
        "Ping between first two hosts, returns all ping results.",
    ),
    ("ports", "display ports and interfaces for each switch"),
    (
        "px",
        "Execute a Python statement.\nNode names may be used, e.g.: px print h1.cmd('ls')",
    ),
    (
        "py",
        "Evaluate a Python expression.\nNode names may be used, e.g.: py h1.cmd('ls')",
    ),
    ("quit", "Exit"),
    ("sh", "Run an external shell command\nUsage: sh [cmd args]"),
    ("source", "Read commands from an input file.\nUsage: source <file>"),
    ("switch", "Starts or stops a switch"),
    ("time", "Measure time taken for any command in Mininet."),
    ("wait", "Wait until all switches have connected to a controller"),
    (
        "x",
        "Create an X11 tunnel to the given node,\noptionally starting a client.\nUsage: x node [cmd args]",
    ),
    (
        "xterm",
        "Spawn xterm(s) for the given node(s).\nUsage: xterm node1 node2 ...",
    ),
];

/// Port of `cmd.Cmd.columnize` (CPython 3.14), used by `print_topics` with
/// `displaywidth = maxcol - 1 = 79`.
fn columnize(list: &[&str], displaywidth: usize) -> String {
    if list.is_empty() {
        return "<empty>\n".to_string();
    }
    let size = list.len();
    if size == 1 {
        return format!("{}\n", list[0]);
    }
    // Try every row count from 1 upwards; first one that fits wins
    // (for/else: fall back to a single column).
    let mut nrows = size;
    let mut ncols = 1;
    let mut colwidths: Vec<usize> = vec![0];
    for r in 1..size {
        let ncols_r = size.div_ceil(r);
        let mut cw: Vec<usize> = Vec::with_capacity(ncols_r);
        let mut totwidth: i64 = -2;
        for col in 0..ncols_r {
            let mut colwidth = 0;
            for row in 0..r {
                let i = row + r * col;
                if i >= size {
                    break;
                }
                colwidth = colwidth.max(list[i].len());
            }
            cw.push(colwidth);
            totwidth += colwidth as i64 + 2;
            if totwidth > displaywidth as i64 {
                break;
            }
        }
        if totwidth <= displaywidth as i64 {
            nrows = r;
            ncols = ncols_r;
            colwidths = cw;
            break;
        }
    }
    let mut out = String::new();
    for row in 0..nrows {
        let mut texts: Vec<&str> = Vec::with_capacity(ncols);
        for col in 0..ncols {
            let i = row + nrows * col;
            texts.push(if i >= size { "" } else { list[i] });
        }
        while texts.last() == Some(&"") {
            texts.pop();
        }
        let mut line = String::new();
        for (col, t) in texts.iter().enumerate() {
            if col > 0 {
                line.push_str("  ");
            }
            line.push_str(t);
            for _ in t.len()..colwidths[col] {
                line.push(' ');
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precmd_strips_comments_like_cli_py() {
        assert_eq!(precmd("nodes # hi"), "nodes ");
        assert_eq!(precmd("# full"), "");
        assert_eq!(precmd("sh echo a#b"), "sh echo a");
        assert_eq!(precmd("nodes"), "nodes");
    }

    #[test]
    fn parseline_mirrors_cmd_py() {
        assert_eq!(parseline(""), (None, String::new(), String::new()));
        assert_eq!(parseline("   "), (None, String::new(), String::new()));
        let (c, a, l) = parseline("nodes");
        assert_eq!(c.as_deref(), Some("nodes"));
        assert_eq!(a, "");
        assert_eq!(l, "nodes");
        let (c, a, l) = parseline("  ping h1 h2  ");
        assert_eq!(c.as_deref(), Some("ping"));
        assert_eq!(a, "h1 h2");
        assert_eq!(l, "ping h1 h2");
        // '?' -> help
        let (c, a, _) = parseline("?pingall");
        assert_eq!(c.as_deref(), Some("help"));
        assert_eq!(a, "pingall");
        // '!' -> no do_shell -> default(line)
        let (c, _, l) = parseline("!ls");
        assert_eq!(c, None);
        assert_eq!(l, "!ls");
        // non-identifier start -> cmd '' -> default
        let (c, _, l) = parseline(",foo");
        assert_eq!(c, None);
        assert_eq!(l, ",foo");
        // identchars are ASCII alnum + '_'
        let (c, a, _) = parseline("ping_all x");
        assert_eq!(c.as_deref(), Some("ping_all"));
        assert_eq!(a, "x");
    }

    #[test]
    fn parse_host_pair_validates_two_args() {
        assert_eq!(
            parse_host_pair("h1 h2").unwrap(),
            vec!["h1".to_string(), "h2".to_string()]
        );
        assert_eq!(
            parse_host_pair("  h1   h2 ").unwrap(),
            vec!["h1".to_string(), "h2".to_string()]
        );
        assert_eq!(
            parse_host_pair("h1").unwrap_err(),
            "invalid number of args: ping src dst\n"
        );
        assert_eq!(
            parse_host_pair("h1 h2 h3").unwrap_err(),
            "invalid number of args: ping src dst\n"
        );
        assert_eq!(
            parse_host_pair("").unwrap_err(),
            "invalid number of args: ping src dst\n"
        );
    }

    #[test]
    fn parse_iperf_args_matches_do_iperf_and_do_iperfudp() {
        // iperf: 0 args -> default pair (first/last host), 2 args -> pair
        assert_eq!(parse_iperf_args("", false).unwrap().0, None);
        let (hosts, bw) = parse_iperf_args("h1 h2", false).unwrap();
        assert_eq!(hosts, Some(("h1".to_string(), "h2".to_string())));
        assert_eq!(bw, "10M");
        assert_eq!(
            parse_iperf_args("h1", false).unwrap_err(),
            "invalid number of args: iperf src dst\n"
        );
        assert_eq!(
            parse_iperf_args("h1 h2 h3", false).unwrap_err(),
            "invalid number of args: iperf src dst\n"
        );
        // iperfudp: 0 args -> default UDP, 3 args -> (bw, pair)
        assert_eq!(parse_iperf_args("", true).unwrap().0, None);
        let (hosts, bw) = parse_iperf_args("20M h1 h2", true).unwrap();
        assert_eq!(hosts, Some(("h1".to_string(), "h2".to_string())));
        assert_eq!(bw, "20M");
        assert_eq!(
            parse_iperf_args("h1 h2", true).unwrap_err(),
            "invalid number of args: iperfudp bw src dst\nbw examples: 10M\n"
        );
    }

    #[test]
    fn commands_table_is_sorted_and_complete_like_python_get_names() {
        assert_eq!(COMMANDS.len(), 29);
        let mut sorted: Vec<&str> = COMMANDS.iter().map(|(n, _)| *n).collect();
        sorted.sort_unstable();
        let names: Vec<&str> = COMMANDS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, sorted, "COMMANDS must stay sorted (help listing)");
        // spot-check against cli.py's do_* set
        for c in [
            "EOF",
            "dpctl",
            "dump",
            "exit",
            "gterm",
            "help",
            "intfs",
            "iperf",
            "iperfudp",
            "link",
            "links",
            "net",
            "nodes",
            "noecho",
            "pingall",
            "pingallfull",
            "pingpair",
            "pingpairfull",
            "ports",
            "px",
            "py",
            "quit",
            "sh",
            "source",
            "switch",
            "time",
            "wait",
            "x",
            "xterm",
        ] {
            assert!(COMMANDS.iter().any(|(n, _)| *n == c), "missing {c}");
        }
    }

    #[test]
    fn columnize_matches_captured_python_help_listing() {
        // Captured from a real `python3` session driving mininet.cli.CLI
        // (`help` on a single,3 net), prompts stripped.
        let names: Vec<&str> = COMMANDS.iter().map(|(n, _)| *n).collect();
        let expected = "\
EOF    gterm  iperfudp  nodes        pingpair      py      switch  xterm\n\
dpctl  help   link      noecho       pingpairfull  quit    time  \n\
dump   intfs  links     pingall      ports         sh      wait  \n\
exit   iperf  net       pingallfull  px            source  x     \n";
        assert_eq!(columnize(&names, 79), expected);
    }

    #[test]
    fn columnize_edge_cases_match_cmd_py() {
        assert_eq!(columnize(&[], 79), "<empty>\n");
        assert_eq!(columnize(&["only"], 79), "only\n");
        // wide list falls back to one column when nothing fits
        let wide: Vec<String> = (0..30).map(|i| format!("cmd{i:02}")).collect();
        let wide_refs: Vec<&str> = wide.iter().map(String::as_str).collect();
        let out = columnize(&wide_refs, 10);
        assert_eq!(out.lines().count(), 30);
    }

    #[test]
    fn bare_help_output_matches_captured_python() {
        let mut out: Vec<u8> = Vec::new();
        do_help("", &mut out);
        let got = String::from_utf8(out).unwrap();
        let expected = "\n\
Documented commands (type help <topic>):\n\
========================================\n\
EOF    gterm  iperfudp  nodes        pingpair      py      switch  xterm\n\
dpctl  help   link      noecho       pingpairfull  quit    time  \n\
dump   intfs  links     pingall      ports         sh      wait  \n\
exit   iperf  net       pingallfull  px            source  x     \n\
\n\
You may also send a command to a node using:\n  <node> command {args}\n\
For example:\n  mininet> h1 ifconfig\n\
\n\
The interpreter automatically substitutes IP addresses\n\
for node names when a node is the first arg, so commands\n\
like\n  mininet> h2 ping h3\n\
should work.\n\
\n\
Some character-oriented interactive commands require\n\
noecho:\n  mininet> noecho h2 vi foo.py\n\
However, starting up an xterm/gterm is generally better:\n  mininet> xterm h2\n\
\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn help_topic_prints_docstring_or_no_help() {
        let mut out: Vec<u8> = Vec::new();
        do_help("iperf", &mut out);
        do_help("nodes", &mut out);
        do_help("bogus", &mut out);
        let got = String::from_utf8(out).unwrap();
        assert_eq!(
            got,
            "Simple iperf TCP test between two (optionally specified) hosts.\n\
             Usage: iperf node1 node2\n\
             List all nodes.\n\
             *** No help on bogus\n"
        );
    }

    #[test]
    fn parse_transmitted_and_rtt_match_python_regexes() {
        let out = "PING 10.0.0.2 (10.0.0.2) 56(84) bytes of data.\n\
                   64 bytes from 10.0.0.2: icmp_seq=1 ttl=64 time=0.042 ms\n\n\
                   --- 10.0.0.2 ping statistics ---\n\
                   1 packets transmitted, 1 received, 0% packet loss, time 0ms\n\
                   rtt min/avg/max/mdev = 0.042/0.050/0.061/0.007 ms\n";
        assert_eq!(parse_transmitted(out), Some((1, 1)));
        assert_eq!(parse_rtt(out), Some((0.042, 0.050, 0.061, 0.007)));
        let loss = "5 packets transmitted, 3 packets received, +2 errors, 40% packet loss\n";
        assert_eq!(parse_transmitted(loss), Some((5, 3)));
        assert_eq!(parse_rtt(loss), None);
        assert_eq!(parse_transmitted("garbage"), None);
    }

    #[test]
    fn parse_ping_full_error_tuple_behavior_like_python() {
        let mut err: Vec<u8> = Vec::new();
        // unreachable -> errorTuple, no error line
        let r = parse_ping_full("connect: Network is unreachable\n", &mut err);
        assert_eq!(r, (1, 0, 0.0, 0.0, 0.0, 0.0));
        assert!(err.is_empty());
        // unparseable -> error line + errorTuple
        let r = parse_ping_full("garbage", &mut err);
        assert_eq!(r, (1, 0, 0.0, 0.0, 0.0, 0.0));
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "*** Error: could not parse ping output: garbage\n"
        );
        // full parse
        let mut err: Vec<u8> = Vec::new();
        let out = "1 packets transmitted, 1 received, 0% packet loss\n\
                   rtt min/avg/max/mdev = 0.042/0.042/0.042/0.000 ms\n";
        let r = parse_ping_full(out, &mut err);
        assert_eq!(r, (1, 1, 0.042, 0.042, 0.042, 0.0));
        assert!(err.is_empty());
    }
}
