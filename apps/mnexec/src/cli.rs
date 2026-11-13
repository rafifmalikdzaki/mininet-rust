//! Command-line parsing for `mnexec`, mirroring the getopt string
//! `"+cdnpa:g:r:vh"` of the C original (`mnexec.c`):
//!
//! - `+` (POSIXLY_CORRECT): stop option processing at the first
//!   non-option argument; the rest is the command to exec.
//! - `-c -d -n -p -v -h` are flags; `-a -g -r` take arguments (either glued
//!   to the flag, e.g. `-a123`, or as the next argv element, `-a 123`).
//! - `--` ends option processing.
//! - `-v` prints the version, `-h` prints usage; both exit immediately at
//!   their position in the option list, like the C switch.

/// One parsed command-line option, in the order it appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opt {
    /// `-c`: close all fds except stdin/out/error.
    CloseFds,
    /// `-d`: detach from tty via setsid().
    Detach,
    /// `-n`: run in new network and mount namespaces.
    NewNamespaces,
    /// `-p`: print `^A` + pid.
    PrintPid,
    /// `-a <pid>`: attach to pid's network and mount namespaces.
    AttachPid(i32),
    /// `-g <group>`: add to cgroup.
    Cgroup(String),
    /// `-r <rtprio>`: run with SCHED_RR.
    RtPrio(i32),
    /// `-v`: print version and exit 0.
    Version,
    /// `-h`: print usage and exit 0.
    Help,
}

/// Result of parsing argv (argv[0] = program name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Options in command-line order (side effects apply in this order).
    pub opts: Vec<Opt>,
    /// The command to execvp (everything after the options).
    pub cmd: Vec<String>,
}

/// Parse failure, mirroring getopt's error cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Unknown option character.
    UnknownOption(char),
    /// Option that requires an argument was given none.
    MissingArg(char),
}

/// Version string printed by `-v` (the C build substitutes a VERSION macro;
/// we use the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Usage text, byte-for-byte the same layout as `usage()` in `mnexec.c`.
pub fn usage(name: &str) -> String {
    format!(
        "Execution utility for Mininet\n\n\
         Usage: {name} [-cdnp] [-a pid] [-g group] [-r rtprio] cmd args...\n\n\
         Options:\n\
         \x20 -c: close all file descriptors except stdin/out/error\n\
         \x20 -d: detach from tty by calling setsid()\n\
         \x20 -n: run in new network and mount namespaces\n\
         \x20 -p: print ^A + pid\n\
         \x20 -a pid: attach to pid's network and mount namespaces\n\
         \x20 -g group: add to cgroup\n\
         \x20 -r rtprio: run with SCHED_RR (usually requires -g)\n\
         \x20 -v: print version\n"
    )
}

/// C `atoi(3)` semantics for `-a`/`-r` arguments: skip leading whitespace,
/// optional sign, take the leading digit run; anything else yields 0.
/// Overflow is clamped (C atoi overflow is UB; we pick saturating).
pub fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (sign, rest) = match t.strip_prefix('-') {
        Some(r) => (-1i64, r),
        None => (1i64, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0;
    }
    let n = digits.parse::<i64>().unwrap_or(i64::MAX);
    (sign.saturating_mul(n)).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Parse argv (including argv[0], the program name) like
/// `getopt(argc, argv, "+cdnpa:g:r:vh")`.
pub fn parse(argv: &[String]) -> Result<Parsed, ParseError> {
    let mut opts = Vec::new();
    let mut cmd = Vec::new();
    let mut i = 1usize; // skip program name
    while i < argv.len() {
        let arg = &argv[i];
        if !arg.starts_with('-') || arg == "-" {
            // First non-option: with '+' everything from here is the command.
            cmd = argv[i..].to_vec();
            break;
        }
        if arg == "--" {
            if i + 1 < argv.len() {
                cmd = argv[i + 1..].to_vec();
            }
            break;
        }
        let chars: Vec<char> = arg[1..].chars().collect();
        let mut j = 0usize;
        while j < chars.len() {
            let c = chars[j];
            match c {
                'c' => opts.push(Opt::CloseFds),
                'd' => opts.push(Opt::Detach),
                'n' => opts.push(Opt::NewNamespaces),
                'p' => opts.push(Opt::PrintPid),
                'v' => opts.push(Opt::Version),
                'h' => opts.push(Opt::Help),
                'a' | 'g' | 'r' => {
                    // Argument is the rest of this cluster, or the next argv.
                    let rest: String = chars[j + 1..].iter().collect();
                    let val = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        if i >= argv.len() {
                            return Err(ParseError::MissingArg(c));
                        }
                        argv[i].clone()
                    };
                    match c {
                        'a' => opts.push(Opt::AttachPid(atoi(&val))),
                        'g' => opts.push(Opt::Cgroup(val)),
                        'r' => opts.push(Opt::RtPrio(atoi(&val))),
                        _ => unreachable!(),
                    }
                    break; // rest of cluster consumed as the argument
                }
                _ => return Err(ParseError::UnknownOption(c)),
            }
            j += 1;
        }
        i += 1;
    }
    Ok(Parsed { opts, cmd })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_simple_flags_in_order() {
        let p = parse(&args(&["mnexec", "-c", "-d", "-n", "-p", "true"])).unwrap();
        assert_eq!(
            p.opts,
            vec![
                Opt::CloseFds,
                Opt::Detach,
                Opt::NewNamespaces,
                Opt::PrintPid
            ]
        );
        assert_eq!(p.cmd, vec!["true".to_string()]);
    }

    #[test]
    fn parses_clustered_flags() {
        let p = parse(&args(&["mnexec", "-cdnp", "echo", "hi"])).unwrap();
        assert_eq!(
            p.opts,
            vec![
                Opt::CloseFds,
                Opt::Detach,
                Opt::NewNamespaces,
                Opt::PrintPid
            ]
        );
        assert_eq!(p.cmd, vec!["echo".to_string(), "hi".to_string()]);
    }

    #[test]
    fn parses_option_args_glued_and_separate() {
        let p = parse(&args(&["mnexec", "-a1234", "-g", "h1", "-r5", "cmd"])).unwrap();
        assert_eq!(
            p.opts,
            vec![
                Opt::AttachPid(1234),
                Opt::Cgroup("h1".to_string()),
                Opt::RtPrio(5)
            ]
        );
        assert_eq!(p.cmd, vec!["cmd".to_string()]);
        let p = parse(&args(&["mnexec", "-a", "42", "-gh1", "-r", "99", "cmd"])).unwrap();
        assert_eq!(
            p.opts,
            vec![
                Opt::AttachPid(42),
                Opt::Cgroup("h1".to_string()),
                Opt::RtPrio(99)
            ]
        );
    }

    #[test]
    fn plus_semantics_stop_at_first_non_option() {
        // With '+' in the optstring, options after the first non-option are
        // part of the command, not parsed.
        let p = parse(&args(&["mnexec", "echo", "-c", "-n"])).unwrap();
        assert!(p.opts.is_empty());
        assert_eq!(
            p.cmd,
            vec!["echo".to_string(), "-c".to_string(), "-n".to_string()]
        );
    }

    #[test]
    fn double_dash_ends_options() {
        let p = parse(&args(&["mnexec", "-c", "--", "-d", "x"])).unwrap();
        assert_eq!(p.opts, vec![Opt::CloseFds]);
        assert_eq!(p.cmd, vec!["-d".to_string(), "x".to_string()]);
        let p = parse(&args(&["mnexec", "--"])).unwrap();
        assert!(p.opts.is_empty() && p.cmd.is_empty());
    }

    #[test]
    fn lone_dash_is_a_command() {
        let p = parse(&args(&["mnexec", "-"])).unwrap();
        assert!(p.opts.is_empty());
        assert_eq!(p.cmd, vec!["-".to_string()]);
    }

    #[test]
    fn unknown_option_is_an_error() {
        assert_eq!(
            parse(&args(&["mnexec", "-x", "true"])).unwrap_err(),
            ParseError::UnknownOption('x')
        );
        assert_eq!(
            parse(&args(&["mnexec", "-cx", "true"])).unwrap_err(),
            ParseError::UnknownOption('x')
        );
    }

    #[test]
    fn missing_option_argument_is_an_error() {
        assert_eq!(
            parse(&args(&["mnexec", "-a"])).unwrap_err(),
            ParseError::MissingArg('a')
        );
        assert_eq!(
            parse(&args(&["mnexec", "-c", "-g"])).unwrap_err(),
            ParseError::MissingArg('g')
        );
        assert_eq!(
            parse(&args(&["mnexec", "-r"])).unwrap_err(),
            ParseError::MissingArg('r')
        );
    }

    #[test]
    fn version_and_help_are_options() {
        let p = parse(&args(&["mnexec", "-v"])).unwrap();
        assert_eq!(p.opts, vec![Opt::Version]);
        let p = parse(&args(&["mnexec", "-h"])).unwrap();
        assert_eq!(p.opts, vec![Opt::Help]);
    }

    #[test]
    fn no_args_yields_no_opts_no_cmd() {
        let p = parse(&args(&["mnexec"])).unwrap();
        assert!(p.opts.is_empty() && p.cmd.is_empty());
    }

    #[test]
    fn atoi_mirrors_c_semantics() {
        assert_eq!(atoi("123"), 123);
        assert_eq!(atoi("-42"), -42);
        assert_eq!(atoi("  +7"), 7);
        assert_eq!(atoi("123abc"), 123);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(atoi(""), 0);
        assert_eq!(atoi("-"), 0);
        assert_eq!(atoi("99999999999999999999"), i32::MAX);
    }

    #[test]
    fn usage_text_matches_c_original() {
        let u = usage("mnexec");
        let expected = "Execution utility for Mininet\n\n\
            Usage: mnexec [-cdnp] [-a pid] [-g group] [-r rtprio] cmd args...\n\n\
            Options:\n\
            \x20 -c: close all file descriptors except stdin/out/error\n\
            \x20 -d: detach from tty by calling setsid()\n\
            \x20 -n: run in new network and mount namespaces\n\
            \x20 -p: print ^A + pid\n\
            \x20 -a pid: attach to pid's network and mount namespaces\n\
            \x20 -g group: add to cgroup\n\
            \x20 -r rtprio: run with SCHED_RR (usually requires -g)\n\
            \x20 -v: print version\n";
        assert_eq!(u, expected);
        // Program name is interpolated.
        assert!(usage("/usr/bin/mnexec").contains("Usage: /usr/bin/mnexec [-cdnp]"));
    }
}
