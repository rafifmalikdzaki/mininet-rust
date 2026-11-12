//! Wiring the `mn` CLI to `mn-net` — port of `bin/mn`'s `MininetRunner`
//! (`parseArgs` → `setup` → `begin`, tasks 10b).
//!
//! Flow (mirroring `MininetRunner.begin`):
//!
//! 1. `--version` prints the bare version and exits 0.
//! 2. `--clean` runs `mn_net::cleanup()` and exits (root-gated: cleanup
//!    mutates the system — pkill/ovs-vsctl/netns — like Python's
//!    `ensureRoot` gate).
//! 3. `--dump` builds the net and prints the `=== topo ===` / `=== dump ===`
//!    / `=== links ===` sections in the exact Python format
//!    (`scripts/diff_topology.sh` probes `mn --topo <spec> --dump`). Root:
//!    real `start()` + plan + `stop()`; **non-root: the dry-run path
//!    (pure model, no kernel) and exit 0**.
//! 4. Otherwise real start/stop is root-gated (Python's `Mininet.init` →
//!    `ensureRoot()`): non-root gets a clear error. Root: build → `start()`
//!    → `--xterms` spawns the terminals (`mininet/term.py` port,
//!    [`crate::term::start_terms`], after `start()` because Rust's deferred
//!    lifecycle starts the shells there — Python spawns them in `build()`
//!    since its shells already exist) → `--test` tests, or (no `--test`)
//!    the interactive loop iff stdin is a tty, else dump+links
//!    ("dump+stop") → `stop()` → `completed in ...`.
//!
//! Documented 10b decisions:
//! - `--custom`/`--pre`/`--post`/`--nat`/`--cluster`/`--innamespace` are
//!   not ported yet: clear error, exit 1 (10c/11 territory).
//! - Default controller is `DefaultController` (bin/mn's `findController()`
//!   availability probing is simplified; `--controller none` → no node).
//! - Errors exit 1 with `mn: <message>` on stderr (Python prints a traceback
//!   and runs global cleanup; we stop the net if it was started).

use std::io::Write;

use mn_net::{cleanup, is_root, ControllerKind, Mininet, MininetOptions, WaitConnected};
use mn_topo::Attrs;

use crate::cli::{parse_class_spec, MnArgs, VERSION};

/// Entry point: run `mn` with parsed args, return the process exit code.
pub fn run(args: &MnArgs) -> i32 {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stdin_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    run_with(args, &mut out, stdin_tty, is_root())
}

/// Testable core of [`run`]: `out` receives the `output()`-level text
/// (dump/links/ping/iperf sections — Python's logger redirected to stdout
/// by the diff harness), `stdin_tty` models `isatty(stdin)`, `root` models
/// euid == 0.
pub fn run_with(args: &MnArgs, out: &mut dyn Write, stdin_tty: bool, root: bool) -> i32 {
    if args.version {
        let _ = writeln!(out, "{VERSION}");
        return 0;
    }
    match begin(args, out, stdin_tty, root) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("mnr: {msg}");
            1
        }
    }
}

/// `MininetRunner.begin`.
fn begin(args: &MnArgs, out: &mut dyn Write, stdin_tty: bool, root: bool) -> Result<i32, String> {
    check_unsupported(args)?;

    if args.clean {
        if !root {
            return Err(
                "--clean mutates the system (pkill, ovs-vsctl del-br, netns del) \
                 and requires root (euid != 0)"
                    .to_string(),
            );
        }
        cleanup().map_err(|e| format!("cleanup failed: {e}"))?;
        return Ok(0);
    }

    if args.dump {
        return run_dump(args, out, root);
    }

    // Real start/stop touches netns/veth/processes: root-gated like
    // Python's Mininet.init -> ensureRoot().
    if !root {
        return Err("starting a network requires root (euid != 0); \
             use --dump for a non-root dry-run of the topology"
            .to_string());
    }

    let mut net = build_net(args)?;
    let start = std::time::Instant::now();
    if let Err(e) = net.start() {
        let _ = net.stop();
        return Err(format!("could not start network: {e}"));
    }
    // --xterms: Mininet.startTerms (net.py) — spawn one terminal per
    // controller/switch/host via the term.py port. Root-gated at runtime
    // by start() above; with no $DISPLAY it prints term.py's error line.
    let mut terms: Vec<std::process::Child> = Vec::new();
    if args.xterms {
        let display = std::env::var("DISPLAY").ok();
        let mut err = std::io::stderr();
        crate::term::start_terms(&mut net, display.as_deref(), &mut err, &mut terms);
    }
    // bin/mn: if opts.test: runTests(...) else: CLI(mn)
    let run_result: Result<i32, String> = if !args.test.is_empty() {
        run_tests(&mut net, &args.test, out)
    } else if enters_interactive_loop(args, stdin_tty) {
        Ok(crate::repl::run(&mut net, out))
    } else {
        // 10b decision: no --test and stdin not a tty -> print dump+links
        // and stop, instead of entering the interactive loop on a non-tty
        // stdin.
        dump_and_links(&net, out);
        Ok(0)
    };
    let stop_result = net.stop().map_err(|e| format!("error while stopping: {e}"));
    if args.xterms {
        // net.py stop() -> stopXterms(): kill the spawned terms + tunnels.
        crate::term::stop_xterms(&mut terms);
    }
    eprintln!(
        "*** completed in {:.3} seconds",
        start.elapsed().as_secs_f64()
    );
    run_result?;
    stop_result?;
    Ok(0)
}

/// Whether `mn` enters the interactive loop: no `--test` and stdin is a
/// tty (bin/mn enters `CLI(mn)` whenever there is no `--test`, with or
/// without `--xterms`; the 10b non-tty dump+stop deviation is kept).
pub fn enters_interactive_loop(args: &MnArgs, stdin_tty: bool) -> bool {
    args.test.is_empty() && stdin_tty
}

/// `--dump`: build the net and print the plan sections; root runs a real
/// start/stop around it, non-root uses the pure dry-run path (exit 0).
fn run_dump(args: &MnArgs, out: &mut dyn Write, root: bool) -> Result<i32, String> {
    let mut net = build_net(args)?;
    if root {
        if let Err(e) = net.start() {
            // Do not leak started hosts/switches/netns on a mid-start failure.
            let _ = net.stop();
            return Err(format!("could not start network: {e}"));
        }
        let plan = net.plan();
        let r = write!(out, "{plan}").map_err(|e| e.to_string());
        let stop = net.stop().map_err(|e| e.to_string());
        r?;
        stop?;
    } else {
        // Non-root dry-run path: pure model build, no kernel, no processes.
        write!(out, "{}", net.plan()).map_err(|e| e.to_string())?;
    }
    Ok(0)
}

/// Print the `=== dump ===` + `=== links ===` sections (cli.py do_dump /
/// do_links) — the non-tty "dump+stop" output.
fn dump_and_links(net: &Mininet, out: &mut dyn Write) {
    let _ = write!(out, "=== dump ===\n{}", net.dump());
    let _ = write!(out, "=== links ===\n{}", net.links());
}

/// Options not ported yet in this slice: fail with a clear message instead
/// of silently ignoring them.
fn check_unsupported(args: &MnArgs) -> Result<(), String> {
    if let Some(c) = &args.custom {
        return Err(format!(
            "--custom {c:?} is not ported (it exec()s Python files) — TODO"
        ));
    }
    if args.pre.is_some() {
        return Err("--pre CLI scripts are not ported yet (task 10c)".to_string());
    }
    if args.post.is_some() {
        return Err("--post CLI scripts are not ported yet (task 10c)".to_string());
    }
    // --nat is now wired (build_net adds nat0); nothing to reject here.
    if let Some(c) = &args.cluster {
        return Err(format!(
            "--cluster {c:?} (experimental cluster edition) is not ported"
        ));
    }
    if args.innamespace {
        return Err(
            "--innamespace needs MininetWithControlNet (configureControlNetwork) — TODO in mn-net"
                .to_string(),
        );
    }
    Ok(())
}

/// Build the [`MininetOptions`] from parsed args (bin/mn's option →
/// `Mininet(...)` constructor mapping in `begin()`).
pub fn options_from(args: &MnArgs) -> Result<MininetOptions, String> {
    // bin/mn: if opts.switch == 'user' and opts.link == 'default': opts.link = 'tcu'
    let mut link = args.link.clone();
    if args.switch_spec().name == "user" && parse_class_spec(&link).name == "default" {
        eprintln!("*** Using TCULink with UserSwitch");
        link = "tcu".to_string();
    }
    // bin/mn: if opts.nolistenport: opts.listenport = None
    let listen_port = if args.nolistenport {
        None
    } else {
        Some(
            u16::try_from(args.listenport)
                .map_err(|_| format!("--listenport {} out of u16 range", args.listenport))?,
        )
    };
    // bin/mn: if opts.test and opts.test != ['none'] and isinstance(opts.wait, bool):
    //     opts.wait = True
    let real_test = match args.test.as_slice() {
        [] => false,
        [only] => only != "none",
        _ => true,
    };
    let wait_connected = if let Some(t) = args.twait {
        WaitConnected::Timed(t as f64)
    } else if args.wait || real_test {
        WaitConnected::Forever
    } else {
        WaitConnected::Off
    };
    Ok(MininetOptions {
        topo: None, // set by build_net from --topo
        topo_spec: None,
        topo_class: None,
        build: false, // build_net adds controllers first, then builds
        xterms: args.xterms,
        cleanup: false,
        ip_base: args.ipbase.clone(),
        in_namespace: args.innamespace,
        auto_set_macs: args.mac,
        auto_static_arp: args.arp,
        auto_pin_cpus: args.pin,
        listen_port,
        wait_connected,
        link_spec: link,
        switch_spec: args.switch.clone(),
        controller: None, // controllers added explicitly in build_net
    })
}

/// `--controller` specs → kinds. Absent means `['default']` (bin/mn);
/// `default`/`ref` → DefaultController, `none` → NullController (adds no
/// node). Other classes (ovsc/nox/remote/ryu) are mn-node stubs and not
/// wired here yet.
pub fn controller_kinds(args: &MnArgs) -> Result<Vec<ControllerKind>, String> {
    if args.controller.is_empty() {
        return Ok(vec![ControllerKind::Default]);
    }
    args.controller
        .iter()
        .map(|c| {
            let name = parse_class_spec(c).name;
            match name.as_str() {
                "default" | "ref" => Ok(ControllerKind::Default),
                "none" => Ok(ControllerKind::None),
                other => Err(format!(
                    "--controller {other:?}: only default|ref|none are wired in the Rust port yet"
                )),
            }
        })
        .collect()
}

/// Build the net from `--topo` (like `buildTopo(TOPOS, opts.topo)` +
/// `Net(topo=..., ...)` + controllers + `build()`), purely in the model.
pub fn build_net(args: &MnArgs) -> Result<Mininet, String> {
    let mut opts = options_from(args)?;
    let built = mn_topo::build_topo(&args.topo)
        .map_err(|e| format!("could not build topo {:?}: {e}", args.topo))?;
    opts.topo_spec = Some(args.topo.clone());
    opts.topo_class = Some(built.class_name().to_string());
    opts.topo = Some(built.topo().clone());
    let mut net = Mininet::new(opts).map_err(|e| e.to_string())?;
    // bin/mn passes controller=[...] to Net(...); buildFromTopo adds a
    // default only when the list is empty — here we add them explicitly.
    for (i, kind) in controller_kinds(args)?.iter().enumerate() {
        net.add_controller(&format!("c{i}"), *kind, &Attrs::new())
            .map_err(|e| e.to_string())?;
    }
    net.build().map_err(|e| e.to_string())?;
    // bin/mn: if opts.nat: net.addNAT(**opts.nat) — after build, so the
    // first switch exists to connect to.
    if args.nat_enabled() {
        net.add_nat("nat0", None, false, &Attrs::new())
            .map_err(|e| format!("could not add NAT: {e}"))?;
    }
    Ok(net)
}

/// A built-in `--test` name (bin/mn's `TESTS` dict + `ALTSPELLING`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Test {
    /// `pingall` → `pingAll`
    PingAll,
    /// `pingpair` → `pingPair`
    PingPair,
    /// `iperf` → `iperf` (TCP)
    Iperf,
    /// `iperfudp` → `iperfUdp`
    IperfUdp,
    /// `all` → allTest (ping + iperf)
    All,
    /// `none`/`build` → nullTest (does nothing)
    Null,
}

/// bin/mn's `runTests`: each `--test` option splits on `+`, names are
/// lowercased and mapped through ALTSPELLING.
pub fn parse_tests(tests: &[String]) -> Result<Vec<Test>, String> {
    let mut out = Vec::new();
    for option in tests {
        for name in option.split('+') {
            out.push(match name.to_ascii_lowercase().as_str() {
                "pingall" => Test::PingAll,
                "pingpair" => Test::PingPair,
                "iperf" => Test::Iperf,
                "iperfudp" => Test::IperfUdp,
                "all" => Test::All,
                "none" | "build" => Test::Null,
                other => {
                    return Err(format!(
                        "Test {other} is unknown - please specify one of \
                         pingall|pingpair|iperf|iperfudp|all|none|build"
                    ))
                }
            });
        }
    }
    Ok(out)
}

/// bin/mn's `runTests(mn, options)` over the built-in tests.
fn run_tests(net: &mut Mininet, tests: &[String], out: &mut dyn Write) -> Result<i32, String> {
    for test in parse_tests(tests)? {
        match test {
            Test::Null => {} // nullTest: does nothing
            Test::PingAll => {
                let r = net.ping_all(None).map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
            }
            Test::PingPair => {
                let r = net.ping_pair().map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
            }
            Test::Iperf => {
                let r = net
                    .iperf(None, "TCP", "10M", 5, 5001)
                    .map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
            }
            Test::IperfUdp => {
                let r = net
                    .iperf(None, "UDP", "10M", 5, 5001)
                    .map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
            }
            // allTest: net.start(); net.ping(); net.iperf() — start already
            // happened in begin(), so ping + iperf.
            Test::All => {
                let r = net.ping_all(None).map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
                let r = net
                    .iperf(None, "TCP", "10M", 5, 5001)
                    .map_err(|e| e.to_string())?;
                write!(out, "{}", r.output).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::MnArgs;
    use clap::Parser;

    fn args(argv: &[&str]) -> MnArgs {
        MnArgs::try_parse_from(argv).expect("parses")
    }

    #[test]
    fn options_from_maps_bin_mn_defaults() {
        let opts = options_from(&args(&["mnr"])).unwrap();
        assert_eq!(opts.ip_base, "10.0.0.0/8");
        assert_eq!(opts.link_spec, "default");
        assert_eq!(opts.switch_spec, "default");
        assert_eq!(opts.listen_port, Some(6654));
        assert_eq!(opts.wait_connected, WaitConnected::Off);
        assert!(!opts.auto_set_macs && !opts.auto_static_arp && !opts.auto_pin_cpus);
        assert!(!opts.build);
        assert!(opts.controller.is_none());
    }

    #[test]
    fn options_from_maps_flags() {
        let opts = options_from(&args(&[
            "mnr",
            "--mac",
            "--arp",
            "--pin",
            "--nolistenport",
            "-i",
            "192.168.0.0/16",
            "--link",
            "tc,bw=10",
            "--switch",
            "ovsbr",
            "-w",
        ]))
        .unwrap();
        assert!(opts.auto_set_macs && opts.auto_static_arp && opts.auto_pin_cpus);
        assert_eq!(opts.listen_port, None);
        assert_eq!(opts.ip_base, "192.168.0.0/16");
        assert_eq!(opts.link_spec, "tc,bw=10");
        assert_eq!(opts.switch_spec, "ovsbr");
        assert_eq!(opts.wait_connected, WaitConnected::Forever);
    }

    #[test]
    fn twait_and_test_imply_wait_connected_like_bin_mn() {
        let opts = options_from(&args(&["mnr", "-t", "5"])).unwrap();
        assert_eq!(opts.wait_connected, WaitConnected::Timed(5.0));
        // bin/mn: tests (other than exactly ['none']) force wait=True
        let opts = options_from(&args(&["mnr", "--test", "pingall"])).unwrap();
        assert_eq!(opts.wait_connected, WaitConnected::Forever);
        let opts = options_from(&args(&["mnr", "--test", "none"])).unwrap();
        assert_eq!(opts.wait_connected, WaitConnected::Off);
        let opts = options_from(&args(&["mnr", "--test", "none", "--test", "pingall"])).unwrap();
        assert_eq!(opts.wait_connected, WaitConnected::Forever);
    }

    #[test]
    fn user_switch_with_default_link_switches_to_tcu_like_bin_mn() {
        let opts = options_from(&args(&["mnr", "--switch", "user"])).unwrap();
        assert_eq!(opts.link_spec, "tcu");
        let opts = options_from(&args(&["mnr", "--switch", "user", "--link", "tc,bw=5"])).unwrap();
        assert_eq!(opts.link_spec, "tc,bw=5");
    }

    #[test]
    fn controller_kinds_default_ref_none_and_rejection() {
        assert_eq!(
            controller_kinds(&args(&["mnr"])).unwrap(),
            vec![ControllerKind::Default]
        );
        assert_eq!(
            controller_kinds(&args(&["mnr", "--controller", "none"])).unwrap(),
            vec![ControllerKind::None]
        );
        assert_eq!(
            controller_kinds(&args(&[
                "mnr",
                "--controller",
                "ref",
                "--controller",
                "none"
            ]))
            .unwrap(),
            vec![ControllerKind::Default, ControllerKind::None]
        );
        assert!(controller_kinds(&args(&["mnr", "--controller", "ryu"])).is_err());
    }

    #[test]
    fn parse_tests_handles_plus_split_case_and_unknown() {
        assert_eq!(
            parse_tests(&["pingall+iperf".to_string(), "NONE".to_string()]).unwrap(),
            vec![Test::PingAll, Test::Iperf, Test::Null]
        );
        assert_eq!(
            parse_tests(&["build".to_string()]).unwrap(),
            vec![Test::Null]
        );
        assert_eq!(
            parse_tests(&["iperfudp".to_string()]).unwrap(),
            vec![Test::IperfUdp]
        );
        let err = parse_tests(&["bogus".to_string()]).unwrap_err();
        assert!(err.contains("Test bogus is unknown"), "{err}");
    }

    #[test]
    fn enters_interactive_loop_decision() {
        // no --test, stdin tty -> loop (bin/mn enters CLI(mn) regardless of
        // --xterms since 10d); otherwise dump+stop
        assert!(enters_interactive_loop(&args(&["mnr"]), true));
        assert!(enters_interactive_loop(&args(&["mnr", "-x"]), true));
        assert!(!enters_interactive_loop(&args(&["mnr"]), false));
        assert!(!enters_interactive_loop(
            &args(&["mnr", "--test", "none"]),
            true
        ));
    }

    #[test]
    fn build_net_single3_model_matches_dry_run() {
        let net = build_net(&args(&[
            "mnr",
            "--topo",
            "single,3",
            "--controller",
            "none",
        ]))
        .unwrap();
        assert!(net.built);
        assert_eq!(net.hosts.len(), 3);
        assert_eq!(net.switches.len(), 1);
        assert_eq!(net.controllers.len(), 0); // --controller none adds nothing
        assert_eq!(net.links.len(), 3);
        assert_eq!(net.keys(), vec!["h1", "h2", "h3", "s1"]);
        // default switch spec -> OVSSwitch seam
        assert_eq!(net.net_switch("s1").unwrap().node().kind(), "OVSSwitch");
    }

    #[test]
    fn build_net_default_controller_is_c0() {
        let net = build_net(&args(&["mnr", "--topo", "single,2"])).unwrap();
        assert!(net.contains("c0"));
        assert!(net
            .dump()
            .contains("<Controller c0: 127.0.0.1:6653 pid=None> "));
    }

    #[test]
    fn build_net_bad_topo_errors() {
        assert!(build_net(&args(&["mnr", "--topo", "bogus"])).is_err());
        assert!(build_net(&args(&["mnr", "--topo", "tree,x"])).is_err());
    }

    #[test]
    fn run_with_dump_non_root_prints_plan_and_exits_zero() {
        let mut buf: Vec<u8> = Vec::new();
        let code = run_with(
            &args(&[
                "mnr",
                "--topo",
                "single,3",
                "--controller",
                "none",
                "--dump",
            ]),
            &mut buf,
            false,
            false, // non-root
        );
        assert_eq!(code, 0);
        let text = String::from_utf8(buf).unwrap();
        assert!(text.starts_with("=== topo ===\nspec: single,3\n"), "{text}");
        assert!(text.contains("=== dump ===\n"), "{text}");
        assert!(
            text.contains(
                "<OVSSwitch s1: lo:127.0.0.1,s1-eth1:None,s1-eth2:None,s1-eth3:None pid=None> \n"
            ),
            "{text}"
        );
        assert!(
            text.contains("=== links ===\nh1-eth0<->s1-eth1 (MISSING MISSING)\n"),
            "{text}"
        );
    }

    #[test]
    fn run_with_version_prints_bare_version() {
        let mut buf: Vec<u8> = Vec::new();
        let code = run_with(&args(&["mnr", "--version"]), &mut buf, false, false);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(buf).unwrap(), "2.3.1b4\n");
    }

    #[test]
    fn run_with_non_root_real_start_is_a_clear_error() {
        for argv in [
            vec!["mn"],
            vec!["mnr", "--topo", "tree,2,2"],
            vec!["mnr", "--topo", "single,3", "--test", "pingall"],
            vec!["mnr", "--clean"],
        ] {
            let mut buf: Vec<u8> = Vec::new();
            let code = run_with(&args(&argv), &mut buf, false, false);
            assert_eq!(code, 1, "{argv:?} should exit 1 non-root");
        }
    }

    #[test]
    fn run_with_unsupported_options_error_clearly() {
        for argv in [
            vec!["mnr", "--custom", "foo.py"],
            vec!["mnr", "--pre", "pre.txt"],
            vec!["mnr", "--nat"],
            vec!["mnr", "--cluster", "srv1"],
            vec!["mnr", "--innamespace"],
        ] {
            let mut buf: Vec<u8> = Vec::new();
            let code = run_with(&args(&argv), &mut buf, false, false);
            assert_eq!(code, 1, "{argv:?} should exit 1");
        }
    }

    #[test]
    fn run_with_unknown_test_name_errors() {
        // root-gated start would happen first, so check parse_tests here
        assert!(parse_tests(&["bogus".to_string()]).is_err());
    }
}
