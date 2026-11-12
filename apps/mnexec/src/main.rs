//! `mnexec` — Rust port of `mnexec.c`, the execution utility for mininet.
//!
//! Starts up programs and does things that are slow or difficult in
//! Python: closing all fds except stdio, detaching from a controlling tty,
//! running in new network/mount namespaces, printing `^A` + pid, attaching
//! to a pid's namespaces and cgroup, setting RT scheduling, then execvp'ing
//! the command. Usage and option semantics mirror the C original exactly
//! (see `cli.rs`).

mod cli;
mod ops;

use std::io::Write;
use std::path::PathBuf;

/// Entry point: run with real stdio and exit with the returned code.
fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let code = run(&argv, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}

/// Run mnexec with captured output streams (split out for testability).
///
/// `argv` includes argv[0] (the program name), like C `main(argc, argv)`.
/// Returns the process exit code.
pub fn run<W: Write, V: Write>(argv: &[String], out: &mut W, err: &mut V) -> i32 {
    let name = argv.first().map(String::as_str).unwrap_or("mnexec");
    let parsed = match cli::parse(argv) {
        Ok(p) => p,
        Err(cli::ParseError::UnknownOption(c)) => {
            let _ = writeln!(err, "{name}: invalid option -- '{c}'");
            let _ = write!(out, "{}", cli::usage(name));
            return 1;
        }
        Err(cli::ParseError::MissingArg(c)) => {
            let _ = writeln!(err, "{name}: option requires an argument -- '{c}'");
            let _ = write!(out, "{}", cli::usage(name));
            return 1;
        }
    };

    // The C code captures the cwd at startup so `-a` can chdir back to it
    // after attaching to another process's mount namespace.
    let cwd: PathBuf = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    // Apply options in command-line order, exactly like the getopt loop in
    // the C main().
    for opt in &parsed.opts {
        let result: Result<(), String> = match opt {
            cli::Opt::Version => {
                let _ = writeln!(out, "{}", cli::VERSION);
                return 0;
            }
            cli::Opt::Help => {
                let _ = write!(out, "{}", cli::usage(name));
                return 0;
            }
            cli::Opt::CloseFds => {
                ops::close_fds_except_stdio();
                Ok(())
            }
            cli::Opt::Detach => match ops::detach_from_tty() {
                Ok(ops::Detach::ParentExit) => return 0,
                Ok(ops::Detach::Continued) => Ok(()),
                Err(e) => Err(e),
            },
            cli::Opt::NewNamespaces => ops::unshare_net_mount_ns(),
            cli::Opt::PrintPid => {
                ops::print_pid(out);
                Ok(())
            }
            cli::Opt::AttachPid(pid) => ops::attach_to_pid(*pid, &cwd),
            cli::Opt::Cgroup(group) => ops::add_to_cgroup(group),
            cli::Opt::RtPrio(prio) => ops::set_rt_priority(*prio),
        };
        if let Err(e) = result {
            let _ = writeln!(err, "{e}");
            return 1;
        }
    }

    if parsed.cmd.is_empty() {
        // C main(): falls through to usage() and returns 0.
        let _ = write!(out, "{}", cli::usage(name));
        return 0;
    }

    match ops::exec_cmd(&parsed.cmd) {
        Err(e) => {
            let _ = writeln!(err, "{e}");
            1
        }
        Ok(never) => match never {},
    }
}

/// Whether root/namespace-gated tests should actually run: only when
/// `MN_RUST_ROOT_TESTS=1` is set, on a privileged Linux host/CI runner.
#[cfg(test)]
pub(crate) fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_prints_usage_and_exits_zero() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["mnexec", "-h"]), &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(out).unwrap(), cli::usage("mnexec"));
        assert!(err.is_empty());
    }

    #[test]
    fn version_prints_version_and_exits_zero() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["mnexec", "-v"]), &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("{}\n", cli::VERSION)
        );
    }

    #[test]
    fn no_command_prints_usage_and_exits_zero() {
        // C main(): optind >= argc -> usage(); return 0;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["mnexec"]), &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(out).unwrap(), cli::usage("mnexec"));
    }

    #[test]
    fn unknown_option_prints_error_usage_and_exits_one() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["mnexec", "-z", "true"]), &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "mnexec: invalid option -- 'z'\n"
        );
        assert_eq!(String::from_utf8(out).unwrap(), cli::usage("mnexec"));
    }

    #[test]
    fn missing_option_argument_prints_error_usage_and_exits_one() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["mnexec", "-a"]), &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "mnexec: option requires an argument -- 'a'\n"
        );
    }

    #[test]
    fn print_pid_flag_emits_soh_pid_then_usage_when_no_cmd() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        // `-p` with no command: prints ^A<pid>, then usage, exit 0. We must
        // not exec anything, so the cmd list stays empty.
        let code = run(&args(&["mnexec", "-p"]), &mut out, &mut err);
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        let expected_pid = format!("\x01{}\n", std::process::id());
        assert!(
            text.starts_with(&expected_pid),
            "output should start with ^A<pid>, got {text:?}"
        );
        assert!(text.ends_with(&cli::usage("mnexec")));
    }

    #[test]
    fn program_name_comes_from_argv0() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args(&["/usr/bin/mnexec", "-h"]), &mut out, &mut err);
        assert_eq!(code, 0);
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("Usage: /usr/bin/mnexec [-cdnp]"));
    }

    #[test]
    fn root_gate_defaults_off() {
        let _ = root_tests_enabled();
    }
}
