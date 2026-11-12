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
//! Ported commands (handlers below): the full `cli.py` command set —
//! `help`, `exit`/`quit`/`EOF`, `nodes`, `net`, `links`, `dump`, `ports`,
//! `intfs`, `pingall`, `pingallfull`, `pingpair`, `pingpairfull`, `iperf`,
//! `iperfudp`, `dpctl`, `sh`, `source`, `link`, `switch`, `wait`, `time`,
//! `noecho`, `py`, `px`, `x`, `xterm`/`gterm` (real `term.py` spawns via
//! [`crate::term`]), plus the `default()` node passthrough (`h1 ifconfig`),
//! plus the 10b convenience commands `hosts`/`switches`/`controllers`/
//! `ping <a> <b>` (not in stock `cli.py` — see "Deviations" below).
//! Unknown commands produce Python's `*** Unknown command: <line>` error
//! line via `default()`.
//!
//! Task 10d notes:
//! - `link` runs `configLinkStatus` in the repl (scope is `apps/mn/**`):
//!   per connecting link, `ifconfig <intf> <up|down>` via the owning node's
//!   `cmd()` — exactly what Python's `Intf.ifconfig` does. On a non-started
//!   node Python's `cmd()` warns `(<node> exited - ignoring cmd(...))`
//!   (warn level → `out`) and returns None; the same warn line is emitted
//!   here when `cmd()` fails (documented deviation: Rust reports the error
//!   object's message only where Python would crash).
//! - `switch` reproduces cli.py byte-exactly, including its quirk of
//!   printing `args[1]` (the command, not the switch) in
//!   `invalid switch: %s`.
//! - `wait` is `mn.waitConnected()` with `timeout=None` (wait forever);
//!   its `info`/`warn` chatter goes to real stderr (same convention as
//!   `*** Starting CLI`), so the golden surface (`out`/`err`) stays empty
//!   when all switches are already connected (e.g. a switch-less net).
//! - `time` re-dispatches through `onecmd` semantics (`parseline` *without*
//!   `precmd`, empty line → `emptyline()` no-op, and the inner stop flag is
//!   ignored — `time exit` does not exit), then prints
//!   `*** Elapsed time: %0.6f secs` on `out`.
//! - `noecho` toggles `stty -echo`/`stty echo` only on a tty, then runs
//!   `default(line)`.
//! - `py`/`px` exec the line in a `python3 -c` subprocess with a JSON
//!   summary of the net in scope (`net` + one name per node — the safe
//!   subset sanctioned by the task; cli.py's module globals are not
//!   available). The `py` eval-result printing (`None` → nothing, `str` →
//!   `result + '\n'`, else `repr(result) + '\n'`) and the
//!   `output(str(e) + '\n')` exception path are reproduced by the wrapper,
//!   byte-exact vs the live-captured Python CLI (goldens). The `py`/`px`
//!   locals dict persists across calls within a session via a **JSON**
//!   state file inside a private 0700 per-session directory under the
//!   temp dir (`px x = 5` then `px print(x)` → `5`, like Python's
//!   `self.locals`). JSON (never pickle) + a `create_dir`-created 0700
//!   dir mean a tampered state file cannot execute code and cannot be
//!   planted by another user (security review fix). `Node.cmd()` inside
//!   the sandbox runs `mnexec -da <pid>
//!   sh -c ...` on a started net; on a non-started net it emits Python's
//!   `(<node> exited - ignoring cmd(...))` warn line and returns None.
//! - `xterm`/`gterm`/`x` spawn real terminals/tunnels via
//!   [`crate::term`] (root-gated at runtime: the interactive loop only
//!   runs on a started, root-owned net). With no `$DISPLAY`, Python prints
//!   `Error: Cannot connect to display` — reproduced (the resolved display
//!   is threaded through `run_with_io`'s `display` param so tests are
//!   deterministic). `x <unknown-node>` crashes Python with `KeyError`;
//!   here it prints `node '<name>' not in network` (documented deviation).
//! - The `default()` passthrough substitutes each arg that is a node name
//!   with its default intf's IP (`updateIP() or arg`; here the model IP —
//!   on a started net it equals the live IP), splits/joins on single
//!   spaces like Python's `args.split(' ')`/`' '.join(...)`, then
//!   `sendCmd(rest)` + `waitForNode` (here `send_cmd` + `wait_output`,
//!   output on `out`). Interactive key forwarding into a running node
//!   command (tty-only in Python) is not ported. On a non-started node
//!   Python's `sendCmd` asserts and crashes the CLI; here the error is
//!   reported as `*** <node> cmd failed: <err>` on `err` (documented).
//!
//! Deviations from CPython (all deliberate, documented):
//! - Prompt only on tty (see above; CPython prints it always).
//! - `do_sh` pipes the child's stdout/stderr and forwards them to `out`/`err`
//!   instead of inheriting the process fds, so scripted-stdin tests can
//!   capture the output (`subprocess.call(line, shell=True)` blocks either
//!   way, so non-interactive content is identical).
//! - `xterm`/`gterm`/`x` validate args exactly like `cli.py` (usage line /
//!   `node 'x' not in network`) and then spawn via `term.rs` on a started
//!   net; `x <unknown>` reports `node '<name>' not in network` where Python
//!   crashes with `KeyError`.
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
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use mn_net::{Mininet, NetSwitch, NodeRef};

use crate::term;

/// `CLI.prompt` in `mininet/cli.py`.
pub const PROMPT: &str = "mininet> ";

/// The repl's mutable IO context, threaded through `dispatch`/`default`:
/// `out`/`err` are the `output()`/`error()` streams, `tty` models
/// `isatty(stdin)`, `display` is the resolved `$DISPLAY` (None = unset —
/// threaded explicitly so tests are deterministic), `terms` holds spawned
/// terminal/tunnel children (Python's `mn.terms`), and `py_state` is the
/// private per-session state **directory** (0700, created with O_EXCL —
/// see [`py_state_dir`]) holding the JSON `py`/`px` locals file
/// [`PY_STATE_FILE`].
struct Io<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    tty: bool,
    display: Option<&'a str>,
    terms: Vec<Child>,
    py_state: Option<PathBuf>,
}

/// Run the interactive loop on the real stdin/stdout until
/// `exit`/`quit`/EOF. Returns the exit code (0; `bin/mn` stops the net after
/// the CLI returns, which `runner` does).
pub fn run(net: &mut Mininet, out: &mut dyn Write) -> i32 {
    // Python CLI.__init__: info( '*** Starting CLI:\n' ) — info chatter on
    // stderr in real mn (not part of golden command output).
    eprintln!("*** Starting CLI:");
    let stdin = std::io::stdin();
    let tty = stdin.is_terminal();
    let display = std::env::var("DISPLAY").ok();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    run_with_io(net, &mut lock, out, &mut err, tty, display.as_deref())
}

/// Testable core of [`run`]: read command lines from `stdin`, write
/// `output()`-level text to `out`, `error()`-level text to `err`, and the
/// `mininet> ` prompt to `out` only when `tty` is true. `display` is the
/// resolved `$DISPLAY` for the `xterm`/`x` commands (None = no display).
/// Returns 0 when the loop ends (`exit`/`quit`/`EOF`).
pub fn run_with_io(
    net: &mut Mininet,
    stdin: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    tty: bool,
    display: Option<&str>,
) -> i32 {
    let mut io = Io {
        out,
        err,
        tty,
        display,
        terms: Vec::new(),
        py_state: None,
    };
    loop {
        if io.tty {
            let _ = write!(io.out, "{PROMPT}");
            let _ = io.out.flush();
        }
        let mut buf = String::new();
        match stdin.read_line(&mut buf) {
            Ok(0) => {
                // EOF: cmdloop maps it to the 'EOF' command -> do_EOF:
                // output('\n') then exit.
                let _ = writeln!(io.out);
                break;
            }
            Ok(_) => {}
            Err(_) => {
                let _ = writeln!(io.out);
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
            None => default(net, &line, &mut io),
            Some(c) => dispatch(net, c, &arg, &line, &mut io),
        };
        if stop {
            break;
        }
    }
    drop_py_state(&mut io);
    0
}

/// Remove the private `py`/`px` state dir (and the JSON state file inside
/// it) when the loop ends.
fn drop_py_state(io: &mut Io) {
    if let Some(dir) = io.py_state.take() {
        let _ = std::fs::remove_dir_all(dir);
    }
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

/// `CLI.default`: the node passthrough (`h1 ifconfig`) plus the
/// `*** Unknown command: <line>` fallthrough. `parseline` is re-applied to
/// the full line exactly like `cmd.Cmd.onecmd` → `default(line)`.
fn default(net: &mut Mininet, line: &str, io: &mut Io) -> bool {
    let (first, args, _line) = parseline(line);
    if let Some(first) = first {
        if net.contains(&first) {
            if args.is_empty() {
                let _ = writeln!(io.err, "*** Please enter a command for node: {first} <cmd>");
                return false;
            }
            // Substitute IP addresses for node names in the rest of the
            // command: Python splits/joins on single spaces and uses
            // `self.mn[arg].defaultIntf().updateIP() or arg` per token.
            let rest: Vec<String> = args
                .split(' ')
                .map(|tok| {
                    if !net.contains(tok) {
                        return tok.to_string();
                    }
                    match net.get(tok).and_then(|n| n.node().default_intf().cloned()) {
                        Some(intf) => match intf.ip {
                            Some(ip) => ip.to_string(),
                            None => tok.to_string(), // updateIP() or arg
                        },
                        None => {
                            // Python's defaultIntf() warns then returns None
                            // and `.updateIP()` crashes the CLI with
                            // AttributeError; here the same warn line is
                            // emitted (warn level -> out) and the node name
                            // is kept (documented deviation: no crash).
                            let _ = writeln!(
                                io.out,
                                "*** defaultIntf: warning: {tok} has no interfaces"
                            );
                            tok.to_string()
                        }
                    }
                })
                .collect();
            let rest = rest.join(" ");
            // node.sendCmd( rest ); self.waitForNode( node )
            let name = first.clone();
            if let Some(mut node) = net.get_mut(&name) {
                let n = node.node_mut();
                match n.send_cmd(&rest, false) {
                    Ok(()) => match n.wait_output(None) {
                        // waitForNode: output( data ) as it arrives.
                        Ok(data) => {
                            let _ = io.out.write_all(data.as_bytes());
                        }
                        Err(e) => {
                            let _ = writeln!(io.err, "*** {name} waitOutput failed: {e}");
                        }
                    },
                    // Python's sendCmd asserts `self.shell and not
                    // self.waiting` and crashes the CLI on a non-started
                    // node; here the error is reported (documented).
                    Err(e) => {
                        let _ = writeln!(io.err, "*** {name} cmd failed: {e}");
                    }
                }
            }
            return false;
        }
    }
    let _ = writeln!(io.err, "*** Unknown command: {line}");
    false
}

/// `CLI.onecmd`: dispatch `cmd` with `arg` (the stripped rest of `line`).
/// Returns true when the loop should stop (`exit`/`quit`/`EOF`).
fn dispatch(net: &mut Mininet, cmd: &str, arg: &str, line: &str, io: &mut Io) -> bool {
    match cmd {
        "help" => do_help(arg, io.out),
        "exit" | "quit" => return true,
        "EOF" => {
            // do_EOF: output('\n') then exit
            let _ = writeln!(io.out);
            return true;
        }
        "nodes" => do_nodes(net, io.out),
        "net" => do_net(net, io.out),
        "links" => do_links(net, io.out),
        "dump" => do_dump(net, io.out),
        "ports" => do_ports(net, io.out),
        "intfs" => do_intfs(net, io.out),
        "hosts" => do_list(net.hosts.keys(), io.out),
        "switches" => do_list(net.switches.keys(), io.out),
        "controllers" => do_list(net.controllers.keys(), io.out),
        "ping" => do_ping(net, arg, io),
        "pingall" => do_pingall(net, arg, io),
        "pingallfull" => ping_full(net, None, io),
        "pingpair" => do_ping_pair(net, io),
        "pingpairfull" => {
            let names: Vec<String> = net.hosts.keys().take(2).map(str::to_string).collect();
            ping_full(net, Some(names), io);
        }
        "iperf" => do_iperf(net, arg, io, false),
        "iperfudp" => do_iperf(net, arg, io, true),
        "dpctl" => do_dpctl(net, arg, io),
        "sh" => do_sh(arg, io),
        "source" => {
            do_source(net, arg, io);
        }
        "link" => do_link(net, arg, io),
        "switch" => do_switch(net, arg, io),
        "wait" => do_wait(net),
        "time" => do_time(net, arg, io),
        "noecho" => do_noecho(net, arg, io),
        "py" => do_py(net, arg, io),
        "px" => do_px(net, arg, io),
        "x" => do_x(net, arg, io),
        "xterm" => do_xterm(net, arg, io, "xterm"),
        "gterm" => do_xterm(net, arg, io, "gterm"),
        _ => {
            default(net, line, io);
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
fn do_pingall(net: &mut Mininet, arg: &str, io: &mut Io) {
    // Python: pingAll(line) with line='' is falsy -> no '-W' option.
    let timeout = if arg.is_empty() { None } else { Some(arg) };
    match net.ping_all(timeout) {
        Ok(r) => {
            let _ = write!(io.out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(io.err, "*** pingall failed: {e}");
        }
    }
}

/// `do_pingpair`: `self.mn.pingPair()`.
fn do_ping_pair(net: &mut Mininet, io: &mut Io) {
    match net.ping_pair() {
        Ok(r) => {
            let _ = write!(io.out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(io.err, "*** pingpair failed: {e}");
        }
    }
}

/// Convenience `ping <a> <b>` (not in stock cli.py): `mn.ping([a, b])` with
/// `do_iperf`-style node validation.
fn do_ping(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args = match parse_host_pair(arg) {
        Ok(a) => a,
        Err(line) => {
            let _ = write!(io.err, "{line}");
            return;
        }
    };
    let mut bad = false;
    for a in &args {
        if !net.contains(a) {
            let _ = writeln!(io.err, "node '{a}' not in network");
            bad = true;
        }
    }
    if bad {
        return;
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match net.ping(Some(refs), None) {
        Ok(r) => {
            let _ = write!(io.out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(io.err, "*** ping failed: {e}");
        }
    }
}

/// `do_iperf` / `do_iperfudp`.
fn do_iperf(net: &mut Mininet, arg: &str, io: &mut Io, udp: bool) {
    let (hosts, udp_bw) = match parse_iperf_args(arg, udp) {
        Ok(v) => v,
        Err(line) => {
            let _ = write!(io.err, "{line}");
            return;
        }
    };
    if let Some((a, b)) = &hosts {
        let mut bad = false;
        for x in [a, b] {
            if !net.contains(x) {
                let _ = writeln!(io.err, "node '{x}' not in network");
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
            let _ = write!(io.out, "{}", r.output);
        }
        Err(e) => {
            let _ = writeln!(io.err, "*** iperf failed: {e}");
        }
    }
}

/// `do_dpctl`: per switch, `output( '*** ' + name + ' ' + '-'*72 + '\n' )`
/// then `output( sw.dpctl( *args ) )` (no added newline).
fn do_dpctl(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args: Vec<String> = arg.split_whitespace().map(str::to_string).collect();
    if args.is_empty() {
        let _ = writeln!(io.err, "usage: dpctl command [arg1] [arg2] ...");
        return;
    }
    let names: Vec<String> = net.switches.keys().map(str::to_string).collect();
    for name in &names {
        let _ = writeln!(io.out, "*** {name} {}", "-".repeat(72));
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
                let _ = write!(io.out, "{r}");
            }
            Err(e) => {
                let _ = writeln!(io.err, "*** dpctl failed on {name}: {e}");
            }
        }
    }
}

/// `do_sh`: `call( line, shell=True )` — run via `/bin/sh -c`. The child's
/// stdout/stderr are piped and forwarded to `out`/`err` (so scripted-stdin
/// tests capture them); the exit status is ignored, like `call()`.
fn do_sh(arg: &str, io: &mut Io) {
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
                let _ = io.out.write_all(&o.stdout);
                let _ = io.err.write_all(&o.stderr);
            }
            Err(_) => {
                let _ = writeln!(io.err, "*** sh failed for {arg:?}");
            }
        },
        Err(e) => {
            let _ = writeln!(io.err, "*** sh failed to spawn: {e}");
        }
    }
}

/// `do_source`: run each line of a file through the same dispatch. Python
/// ignores `onecmd`'s return value here, so `exit` inside a sourced file does
/// NOT stop the outer loop.
fn do_source(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.len() != 1 {
        let _ = writeln!(io.err, "usage: source <file>");
        return;
    }
    let path = args[0];
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            let _ = writeln!(io.err, "error reading file {path}");
            return;
        }
    };
    for raw in content.lines() {
        let (cmd, sarg, line) = parseline(precmd(raw));
        if line.is_empty() {
            continue;
        }
        let stop = match cmd.as_deref() {
            None => default(net, &line, io),
            Some(c) => dispatch(net, c, &sarg, &line, io),
        };
        let _ = stop; // do_source ignores the stop signal
    }
}

// ---------------------------------------------------------------------
// Task 10d handlers: link / switch / wait / time / noecho / py / px / x /
// xterm (cli.py) — term spawning lives in crate::term (term.py port)
// ---------------------------------------------------------------------

/// `do_link` (cli.py): arg validation is byte-exact, then
/// `mn.configLinkStatus( *args )` (ported below, since the scope of this
/// task is `apps/mn/**` only).
fn do_link(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.len() != 3 {
        let _ = writeln!(io.err, "invalid number of args: link end1 end2 [up down]");
        return;
    }
    if args[2] != "up" && args[2] != "down" {
        let _ = writeln!(io.err, "invalid type: link end1 end2 [up down]");
        return;
    }
    config_link_status(net, args[0], args[1], args[2], io);
}

/// Port of `Mininet.configLinkStatus` (net.py): for every link between
/// `src` and `dst`, `intf.ifconfig( status )` on both ends — i.e.
/// `node.cmd('ifconfig', <intf>, <status>)` in the owning node (exactly
/// what Python's `Intf.ifconfig` runs). Error lines are byte-exact
/// (`src`/`dst` render as node names via `Node.__str__`).
fn config_link_status(net: &mut Mininet, src: &str, dst: &str, status: &str, io: &mut Io) {
    if !net.contains(src) {
        let _ = writeln!(io.err, "src not in network: {src}");
        return;
    }
    if !net.contains(dst) {
        let _ = writeln!(io.err, "dst not in network: {dst}");
        return;
    }
    let idxs = net.links_between(src, dst);
    if idxs.is_empty() {
        // Python: error( 'src and dst not connected: %s %s' % ( src, dst ) )
        // with src/dst re-bound to the Node objects — '%s' uses __str__ = name.
        let _ = writeln!(io.err, "src and dst not connected: {src} {dst}");
        return;
    }
    for idx in idxs {
        // connectionsTo(src, dst): (srcIntf, dstIntf) per link, src's end first.
        let (s_intf, d_intf) = {
            let l = &net.links[idx];
            if l.node1 == src {
                (l.intf1.clone(), l.intf2.clone())
            } else {
                (l.intf2.clone(), l.intf1.clone())
            }
        };
        for (intf, side) in [(&s_intf, "src"), (&d_intf, "dst")] {
            let node_name = intf.node.clone();
            let ifconfig = format!("ifconfig {} {status}", intf.name);
            let Some(mut node) = net.get_mut(&node_name) else {
                continue;
            };
            match node.node_mut().cmd(&ifconfig) {
                Ok(result) => {
                    if !result.is_empty() {
                        let _ = writeln!(io.err, "link {side} status change failed: {result}");
                    }
                }
                Err(_) => {
                    // Non-started node: Python's cmd() warns
                    // '(<node> exited - ignoring cmd%s)' (warn level -> out)
                    // and returns None, so no "status change failed" line.
                    let _ = writeln!(
                        io.out,
                        "({node_name} exited - ignoring cmd('ifconfig', '{}', '{status}'))",
                        intf.name
                    );
                }
            }
        }
    }
}

/// `do_switch` (cli.py): start/stop a switch. Byte-exact errors — including
/// cli.py's quirk of printing `args[1]` (the command) in `invalid switch:`.
fn do_switch(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.len() != 2 {
        let _ = writeln!(
            io.err,
            "invalid number of args: switch <switch name>{{start, stop}}"
        );
        return;
    }
    let sw = args[0];
    let command = args[1];
    if !net.contains(sw) || net.net_switch(sw).is_none() {
        let _ = writeln!(io.err, "invalid switch: {}", args[1]);
        return;
    }
    let name = sw.to_string();
    match command {
        "start" => {
            // self.mn.get( sw ).start( self.mn.controllers ) — disjoint field
            // borrows: controllers (immutable) + switches (mutable).
            let controllers: Vec<_> = net.controllers.values().collect();
            let sw = net.switches.get_mut(&name).expect("checked above");
            if let Err(e) = sw.start(&controllers) {
                // Python crashes here on a non-started net (no golden);
                // reported instead (documented).
                let _ = writeln!(io.err, "*** switch {name} start failed: {e}");
            }
        }
        "stop" => {
            // self.mn.get( sw ).stop( deleteIntfs=False )
            let sw = net.switches.get_mut(&name).expect("checked above");
            if let Err(e) = sw.stop(false) {
                let _ = writeln!(io.err, "*** switch {name} stop failed: {e}");
            }
        }
        _ => {
            let _ = writeln!(
                io.err,
                "invalid command: switch <switch name> {{start, stop}}"
            );
        }
    }
}

/// `do_wait` (cli.py): `self.mn.waitConnected()` — timeout=None, i.e. wait
/// until every switch reports connected. The `info`/`warn` chatter goes to
/// real stderr (same convention as `*** Starting CLI`), so the golden
/// `out`/`err` surface only carries it on the real terminal.
fn do_wait(net: &mut Mininet) {
    eprintln!("*** Waiting for switches to connect");
    let delay = 0.5; // Python's delay=.5
    let mut remaining: Vec<String> = net.switches.keys().map(str::to_string).collect();
    loop {
        let mut still: Vec<String> = Vec::new();
        for name in &remaining {
            let s = net.switches.get_mut(name).expect("listed above");
            if s.connected() {
                // Python: info( '%s ' % switch ) — str(switch) is its name.
                eprint!("{name} ");
            } else {
                still.push(name.clone());
            }
        }
        remaining = still;
        if remaining.is_empty() {
            eprintln!();
            return; // Python: info( '\n' ); return True
        }
        // timeout=None -> wait forever (Python's waitConnected() default)
        std::thread::sleep(Duration::from_secs_f64(delay));
    }
}

/// `do_time` (cli.py): time any command. The inner `onecmd(line)` applies
/// `parseline` WITHOUT `precmd`, an empty line is `emptyline()` (no-op),
/// and the inner stop flag is discarded (`time exit` does not exit); then
/// `*** Elapsed time: %0.6f secs` is written to Cmd's stdout (`out`).
fn do_time(net: &mut Mininet, arg: &str, io: &mut Io) {
    let start = Instant::now();
    let (cmd, sarg, line) = parseline(arg);
    if !line.is_empty() {
        match cmd.as_deref() {
            None => {
                default(net, &line, io);
            }
            Some(c) => {
                let _ = dispatch(net, c, &sarg, &line, io); // stop flag ignored
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let _ = writeln!(io.out, "*** Elapsed time: {elapsed:.6} secs");
}

/// `do_noecho` (cli.py): `stty -echo` around `default(arg)` on a tty only
/// (`onecmd` passes the arg after `noecho`, and `default` re-parses it).
fn do_noecho(net: &mut Mininet, arg: &str, io: &mut Io) {
    if io.tty {
        term::quiet_run("stty -echo");
    }
    default(net, arg, io);
    if io.tty {
        term::quiet_run("stty echo");
    }
}

/// `do_xterm` / `do_gterm` (cli.py): validate args exactly (usage line /
/// `node 'x' not in network`, in order), then `makeTerms([node], term=term)`
/// — a real spawn via [`term::make_term`] (root-gated: the loop only runs
/// on a started net).
fn do_xterm(net: &mut Mininet, arg: &str, io: &mut Io, term: &str) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.is_empty() {
        let _ = writeln!(io.err, "usage: {term} node1 node2 ...");
        return;
    }
    for a in &args {
        if !net.contains(a) {
            let _ = writeln!(io.err, "node '{a}' not in network");
        } else {
            let name = (*a).to_string();
            if let Some(mut node) = net.get_mut(&name) {
                term::make_term(
                    &mut node,
                    "Node",
                    term,
                    io.display,
                    "bash",
                    io.err,
                    &mut io.terms,
                );
            }
        }
    }
}

/// `do_x` (cli.py): `runX11( node, cmd )`. Python looks the node up with
/// `self.mn[args[0]]` and crashes with `KeyError` on an unknown name; here
/// the `node '<name>' not in network` line is printed instead (documented).
fn do_x(net: &mut Mininet, arg: &str, io: &mut Io) {
    let args: Vec<&str> = arg.split_whitespace().collect();
    if args.is_empty() {
        let _ = writeln!(io.err, "usage: x node [cmd args]...");
        return;
    }
    if !net.contains(args[0]) {
        let _ = writeln!(io.err, "node '{}' not in network", args[0]);
        return;
    }
    let name = args[0].to_string();
    let cmd: Vec<String> = args[1..].iter().map(|s| s.to_string()).collect();
    if let Some(mut node) = net.get_mut(&name) {
        term::run_x11(&mut node, &cmd, io.display, io.err, &mut io.terms);
    }
}

/// `do_py` (cli.py): evaluate a Python expression with `net` (and every
/// node name) in scope. Implemented as a `python3 -c` subprocess fed a JSON
/// summary of the net (the task-sanctioned safe subset — see module docs);
/// the wrapper reproduces eval-result printing and the `output(str(e))`
/// exception path byte-exact.
fn do_py(net: &Mininet, arg: &str, io: &mut Io) {
    run_py(net, arg, true, io);
}

/// `do_px` (cli.py): execute a Python statement with `net` in scope.
fn do_px(net: &Mininet, arg: &str, io: &mut Io) {
    run_py(net, arg, false, io);
}

/// Run `line` in a `python3 -c` wrapper (`PY_WRAPPER`) with mode `py`
/// (eval) or `px` (exec). `MN_NET_JSON` carries the net summary,
/// `MN_PY_LINE` the user line, and `MN_PY_STATE` the JSON persistent
/// locals file — `py_state_dir().join(PY_STATE_FILE)` inside the private
/// 0700 per-session state dir, created lazily on first use and removed
/// when the loop ends ([`drop_py_state`]). If the dir cannot be created,
/// the wrapper simply runs without persistent state.
fn run_py(net: &Mininet, line: &str, eval_mode: bool, io: &mut Io) {
    if io.py_state.is_none() {
        io.py_state = py_state_dir();
    }
    let state = io.py_state.as_ref().map(|dir| dir.join(PY_STATE_FILE));
    let summary = net_summary_json(net);
    let mode = if eval_mode { "py" } else { "px" };
    let mut cmd = std::process::Command::new("python3");
    cmd.arg("-c")
        .arg(PY_WRAPPER)
        .arg(mode)
        .env("MN_PY_LINE", line)
        .env("MN_NET_JSON", &summary);
    if let Some(state) = &state {
        cmd.env("MN_PY_STATE", state);
    }
    let spawned = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    match spawned {
        Ok(c) => match c.wait_with_output() {
            Ok(o) => {
                let _ = io.out.write_all(&o.stdout);
                let _ = io.err.write_all(&o.stderr);
            }
            Err(e) => {
                let _ = writeln!(io.err, "*** {mode} failed: {e}");
            }
        },
        // Python always has its own interpreter; a missing python3 binary
        // is reported here (documented).
        Err(e) => {
            let _ = writeln!(io.err, "*** {mode} failed to spawn python3: {e}");
        }
    }
}

/// Name of the JSON locals file inside the per-session state dir.
const PY_STATE_FILE: &str = "state.json";

/// Create the private per-session `py`/`px` state directory under the
/// system temp dir: `mn-repl-py-<pid>-<nanos>` with mode **0700**.
///
/// Security (design-review fix): the wrapper loads the state as **JSON**
/// (never pickle — `pickle.load` on an attacker-controlled file is
/// arbitrary code execution), and the directory is created with
/// `DirBuilder::create`, which is O_EXCL (fails if the path already
/// exists), so a name collision can never point the repl at a
/// pre-existing, possibly attacker-planted directory; mode 0700 keeps
/// every other user out. Returns `None` if creation fails — the wrapper
/// then runs without persistent state.
fn py_state_dir() -> Option<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!(
        "mn-repl-py-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;
    Some(dir)
}

/// The `python3 -c` wrapper for `py`/`px`. argv[1] is the mode (`py`/`px`);
/// `MN_PY_LINE` is the user line, `MN_NET_JSON` the net summary, and
/// `MN_PY_STATE` (optional) a **JSON** file holding the persistent locals
/// dict across `py`/`px` calls (Python's `CLI.locals`, which is seeded
/// with `net` + node names on every `getLocals()`). JSON cannot carry
/// code, so loading the state file is safe even if it were tampered with
/// (the repl additionally keeps it in a private 0700 dir, see
/// [`py_state_dir`]).
const PY_WRAPPER: &str = r#"
import json, os, sys
_mode = sys.argv[1]
_line = os.environ["MN_PY_LINE"]
_state = os.environ.get("MN_PY_STATE")
class _Node:
    def __init__(self, d):
        self.name = d["name"]; self.kind = d["kind"]; self.pid = d["pid"]
        self.ip = d["ip"]; self.intfs = d["intfs"]
    def __repr__(self):
        intfs = ",".join("%s:%s" % (i["name"], i["ip"]) for i in self.intfs)
        return "<%s %s: %s pid=%s> " % (self.kind, self.name, intfs, self.pid)
    def IP(self, intf=None):
        if intf is None:
            return self.ip
        for i in self.intfs:
            if i["name"] == intf:
                return i["ip"]
        return None
    def intfNames(self):
        return [i["name"] for i in self.intfs]
    def cmd(self, *args):
        # On a started net run the command in the node's namespace via
        # mnexec (Python's Node.cmd); on a non-started net reproduce
        # Python's warn line + None.
        if self.pid:
            import subprocess
            _p = subprocess.run(
                ["mnexec", "-da", str(self.pid), "sh", "-c", " ".join(args)],
                capture_output=True, text=True)
            sys.stdout.write(_p.stdout)
            return _p.stdout
        sys.stdout.write("(%s exited - ignoring cmd%s)\n" % (self.name, args))
        return None
class _Net:
    def __init__(self, nodes, hosts, switches, controllers):
        self._nodes = {n["name"]: _Node(n) for n in nodes}
        self.hosts = [self._nodes[n] for n in hosts]
        self.switches = [self._nodes[n] for n in switches]
        self.controllers = [self._nodes[n] for n in controllers]
    def __iter__(self):
        return iter(list(self._nodes))
    def __len__(self):
        return len(self._nodes)
    def __contains__(self, key):
        return key in self._nodes
    def __getitem__(self, key):
        return self._nodes[key]
    def keys(self):
        return list(self._nodes)
    def values(self):
        return list(self._nodes.values())
# Persistent locals: a plain-JSON object in MN_PY_STATE (inside the
# repl's private 0700 state dir). json.load can only produce JSON
# values — it never executes code the way Python's pickle module does.
_ns = {}
if _state and os.path.exists(_state):
    try:
        with open(_state, "r", encoding="utf-8") as _f:
            _loaded = json.load(_f)
        if isinstance(_loaded, dict):
            _ns.update(_loaded)
    except (OSError, ValueError):
        _ns = {}
_summary = json.loads(os.environ["MN_NET_JSON"])
net = _Net(_summary["nodes"], _summary["hosts"], _summary["switches"],
           _summary["controllers"])
# getLocals(): self.locals.update( self.mn ) — net + node names, always.
_ns["net"] = net
for _n in _summary["nodes"]:
    _ns[_n["name"]] = net[_n["name"]]
try:
    if _mode == "py":
        _r = eval(_line, globals(), _ns)
        if _r is None:
            pass
        elif isinstance(_r, str):
            sys.stdout.write(_r + "\n")
        else:
            sys.stdout.write(repr(_r) + "\n")
    else:
        exec(_line, globals(), _ns)
except Exception as _e:
    sys.stdout.write(str(_e) + "\n")
if _state:
    # Persist only JSON-serializable, non-shim values: the _Net/_Node
    # shims are re-created from MN_NET_JSON on every call, and values
    # json cannot represent (functions, class instances) are skipped —
    # they simply do not persist across py/px calls.
    _persist = {}
    for _k, _v in _ns.items():
        if isinstance(_v, (_Node, _Net)):
            continue
        try:
            json.dumps(_v)
        except (TypeError, ValueError):
            continue
        _persist[_k] = _v
    try:
        with open(_state, "w", encoding="utf-8") as _f:
            json.dump(_persist, _f)
    except OSError:
        pass
"#;

/// Minimal JSON string escaper (names/IPs are simple, but escape properly).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// JSON rendering of an `Option<String>` (`null` when None).
fn json_opt(s: Option<String>) -> String {
    s.map(|v| json_str(&v))
        .unwrap_or_else(|| "null".to_string())
}

/// The net summary passed to the `py`/`px` wrapper as `MN_NET_JSON`:
/// `{"nodes": [{"name", "kind", "pid", "ip", "intfs": [{"name","ip"}]}],
///   "hosts": [...], "switches": [...], "controllers": [...]}`.
/// Pure (unit-tested below).
fn net_summary_json(net: &Mininet) -> String {
    let mut nodes = Vec::new();
    for name in net.keys() {
        let node = net.get(&name).expect("keys in sync");
        let n = node.node();
        let intfs: Vec<String> = node_intfs(node)
            .into_iter()
            .map(|(iname, _)| {
                let ip = match &node {
                    NodeRef::Switch(s) => s
                        .control_intf
                        .as_ref()
                        .filter(|lo| lo.name == iname)
                        .map(|lo| {
                            if lo.name == "lo" {
                                // Python Intf.__init__: lo -> 127.0.0.1
                                Some("127.0.0.1".parse().expect("valid IP"))
                            } else {
                                lo.ip
                            }
                        })
                        .unwrap_or(None),
                    _ => n.intf(Some(&iname)).ok().and_then(|i| i.ip),
                };
                format!(
                    "{{\"name\": {}, \"ip\": {}}}",
                    json_str(&iname),
                    json_opt(ip.map(|a| a.to_string()))
                )
            })
            .collect();
        // Python's defaultIntf(): Switch returns its control intf (lo ->
        // 127.0.0.1), others the lowest-port intf.
        let ip = match node {
            NodeRef::Switch(s) => match &s.control_intf {
                Some(lo) if lo.name == "lo" => Some("127.0.0.1".to_string()),
                Some(lo) => lo.ip.map(|a| a.to_string()),
                None => s
                    .node()
                    .default_intf()
                    .and_then(|i| i.ip)
                    .map(|a| a.to_string()),
            },
            _ => n.default_intf().and_then(|i| i.ip).map(|a| a.to_string()),
        };
        nodes.push(format!(
            "{{\"name\": {}, \"kind\": {}, \"pid\": {}, \"ip\": {}, \"intfs\": [{}]}}",
            json_str(&name),
            json_str(n.kind()),
            match n.pid() {
                Some(p) => p.to_string(),
                None => "null".to_string(),
            },
            json_opt(ip),
            intfs.join(", ")
        ));
    }
    let names = |keys: Vec<String>| {
        keys.iter()
            .map(|k| json_str(k))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{{\"nodes\": [{}], \"hosts\": [{}], \"switches\": [{}], \"controllers\": [{}]}}",
        nodes.join(", "),
        names(net.hosts.keys().map(str::to_string).collect()),
        names(net.switches.keys().map(str::to_string).collect()),
        names(net.controllers.keys().map(str::to_string).collect()),
    )
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
fn ping_full(net: &mut Mininet, hosts: Option<Vec<String>>, io: &mut Io) {
    let names: Vec<String> = match hosts {
        Some(v) if !v.is_empty() => v,
        _ => {
            let _ = writeln!(io.out, "*** Ping: testing ping reachability");
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
        let _ = write!(io.out, "{src} -> ");
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
            let (sent, received, rttmin, rttavg, rttmax, rttdev) = parse_ping_full(&result, io.err);
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
                let _ = write!(io.out, "{dst} ");
            } else {
                let _ = write!(io.out, "X ");
            }
        }
        let _ = writeln!(io.out);
    }
    let _ = writeln!(io.out, "*** Results: ");
    for (src, dst, sent, received, rttmin, rttavg, rttmax, rttdev) in &all_outputs {
        let _ = write!(io.out, " {src}->{dst}: {sent}/{received}, ");
        let _ = writeln!(
            io.out,
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
    fn net_summary_json_shape_for_single3() {
        let net = Mininet::dry_run(
            "single,3",
            mn_net::MininetOptions {
                controller: None,
                ..Default::default()
            },
        )
        .unwrap();
        let j = net_summary_json(&net);
        assert_eq!(
            j,
            "{\"nodes\": [\
             {\"name\": \"h1\", \"kind\": \"Host\", \"pid\": null, \"ip\": \"10.0.0.1\", \"intfs\": [{\"name\": \"h1-eth0\", \"ip\": \"10.0.0.1\"}]}, \
             {\"name\": \"h2\", \"kind\": \"Host\", \"pid\": null, \"ip\": \"10.0.0.2\", \"intfs\": [{\"name\": \"h2-eth0\", \"ip\": \"10.0.0.2\"}]}, \
             {\"name\": \"h3\", \"kind\": \"Host\", \"pid\": null, \"ip\": \"10.0.0.3\", \"intfs\": [{\"name\": \"h3-eth0\", \"ip\": \"10.0.0.3\"}]}, \
             {\"name\": \"s1\", \"kind\": \"OVSSwitch\", \"pid\": null, \"ip\": \"127.0.0.1\", \"intfs\": [{\"name\": \"lo\", \"ip\": \"127.0.0.1\"}, {\"name\": \"s1-eth1\", \"ip\": null}, {\"name\": \"s1-eth2\", \"ip\": null}, {\"name\": \"s1-eth3\", \"ip\": null}]}], \
             \"hosts\": [\"h1\", \"h2\", \"h3\"], \"switches\": [\"s1\"], \"controllers\": []}"
        );
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("h1"), "\"h1\"");
        assert_eq!(json_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(json_opt(None), "null");
        assert_eq!(json_opt(Some("10.0.0.1".to_string())), "\"10.0.0.1\"");
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

    // -----------------------------------------------------------------
    // py/px persistent state (security: JSON in a private 0700 dir,
    // never pickle in shared /tmp)
    // -----------------------------------------------------------------

    fn dry_net() -> Mininet {
        Mininet::dry_run(
            "single,3",
            mn_net::MininetOptions {
                controller: None,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn python3_available() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn test_io<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>, state: Option<PathBuf>) -> Io<'a> {
        Io {
            out,
            err,
            tty: false,
            display: None,
            terms: Vec::new(),
            py_state: state,
        }
    }

    /// Serializes the py-state tests: they all create/remove
    /// `mn-repl-py-<pid>-*` dirs in the shared temp dir, and the
    /// run_with_io cleanup test scans that dir, so they must not overlap.
    static PY_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn state_dir_names() -> Vec<String> {
        let prefix = format!("mn-repl-py-{}-", std::process::id());
        std::fs::read_dir(std::env::temp_dir())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(&prefix))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn py_state_dir_is_private_and_exclusive() {
        let _g = PY_STATE_LOCK.lock().unwrap();
        let dir = py_state_dir().expect("temp dir is writable here");
        let meta = std::fs::metadata(&dir).unwrap();
        assert!(meta.is_dir());
        assert!(dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("mn-repl-py-"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o700,
                "state dir must be 0700 (other users must not read/plant it)"
            );
        }
        // O_EXCL: creating the same path again must fail, so a name
        // collision can never clobber/point at an existing directory.
        use std::os::unix::fs::DirBuilderExt;
        let again = std::fs::DirBuilder::new().mode(0o700).create(&dir);
        assert!(again.is_err(), "create on an existing dir must fail");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drop_py_state_removes_the_private_dir() {
        let _g = PY_STATE_LOCK.lock().unwrap();
        let dir = py_state_dir().unwrap();
        std::fs::write(dir.join(PY_STATE_FILE), "{}").unwrap();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let mut io = test_io(&mut out, &mut err, Some(dir.clone()));
        drop_py_state(&mut io);
        assert!(io.py_state.is_none());
        assert!(!dir.exists(), "state dir must be removed with the loop");
    }

    #[test]
    fn py_px_persist_locals_through_json_state_file() {
        if !python3_available() {
            return; // same assumption as the py/px golden tests
        }
        let _g = PY_STATE_LOCK.lock().unwrap();
        let net = dry_net();
        let dir = py_state_dir().unwrap();
        let state = dir.join(PY_STATE_FILE);
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        {
            let mut io = test_io(&mut out, &mut err, Some(dir.clone()));
            run_py(&net, "x = 5", false, &mut io); // px x = 5
            run_py(&net, "print(x)", false, &mut io); // px print(x) -> 5
            run_py(&net, "x + 1", true, &mut io); // py x + 1 -> 6
        }
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, "5\n6\n", "err: {}", String::from_utf8_lossy(&err));
        // the state file is plain JSON, not a pickle
        let content = std::fs::read_to_string(&state).unwrap();
        assert!(
            content.trim_start().starts_with('{'),
            "state must be a JSON object, got {content:?}"
        );
        assert!(content.contains("\"x\": 5"), "state: {content:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn py_state_file_is_json_and_never_loads_pickle() {
        if !python3_available() {
            return;
        }
        let _g = PY_STATE_LOCK.lock().unwrap();
        // the wrapper must not import or call pickle anymore
        for needle in ["import pickle", "pickle.load", "pickle.dump"] {
            assert!(!PY_WRAPPER.contains(needle), "wrapper still uses {needle}");
        }
        let net = dry_net();
        let dir = py_state_dir().unwrap();
        let state = dir.join(PY_STATE_FILE);
        // attacker-planted pickle payload: under pickle.load this runs
        // `echo pwned` (os.system); under json.load it is just invalid JSON
        std::fs::write(&state, b"cos\nsystem\n(S'echo pwned'\ntR.").unwrap();
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        {
            let mut io = test_io(&mut out, &mut err, Some(dir.clone()));
            run_py(&net, "print('safe')", false, &mut io);
        }
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("safe"),
            "out: {out:?} err: {}",
            String::from_utf8_lossy(&err)
        );
        assert!(
            !out.contains("pwned"),
            "pickle payload executed! out: {out:?}"
        );
        // ... and the file is rewritten as plain JSON
        let content = std::fs::read_to_string(&state).unwrap();
        assert!(
            content.trim_start().starts_with('{'),
            "state rewritten as JSON, got {content:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn run_with_io_py_px_session_and_cleanup() {
        if !python3_available() {
            return;
        }
        let _g = PY_STATE_LOCK.lock().unwrap();
        let before = state_dir_names();
        let mut net = dry_net();
        let script = "px x = 5\npy x\npx print(x)\nexit\n";
        let mut stdin = std::io::Cursor::new(script.as_bytes().to_vec());
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = run_with_io(&mut net, &mut stdin, &mut out, &mut err, false, None);
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "5\n5\n",
            "err: {}",
            String::from_utf8_lossy(&err)
        );
        // the loop removed its private state dir on exit: no leftover
        // mn-repl-py-<our pid>-* entry that did not exist before the run
        let new_leftovers: Vec<String> = state_dir_names()
            .into_iter()
            .filter(|n| !before.contains(n))
            .collect();
        assert!(
            new_leftovers.is_empty(),
            "state dirs left behind after exit: {new_leftovers:?}"
        );
    }
}
