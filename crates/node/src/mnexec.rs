//! Locating the `mnexec` binary and building the exact argv vectors that
//! Python's `mininet/node.py` passes to it.
//!
//! Python call sites being ported:
//!
//! - `Node.startShell`: `mnexec -cd[n] env PS1=\x7f bash --norc --noediting
//!   -is mininet:<name>` (we attach with `-a <holder pid>` instead of
//!   unsharing with `-n`, so the shell lands in the node's *named* netns);
//! - `Node.popen`: `mnexec -da <node.pid> <cmd...>`;
//! - `Node.sendCmd` with `printPid`: prefix `mnexec -p `.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The prompt sentinel: Python sets `PS1=chr(127)` so the interactive
/// shell's prompt is a single `DEL` character, which [`crate::Node`] uses
/// to detect command completion in the output stream.
pub const PROMPT_SENTINEL: char = '\x7f';

/// Environment variable overriding the `mnexec` binary location.
pub const MNEXEC_ENV: &str = "MNEXEC";

/// Resolve the `mnexec` binary: `$MNEXEC` if set, else the workspace's
/// `target/debug/mnexec` build if it exists, else plain `mnexec` (relying
/// on `PATH` lookup at spawn time).
pub fn mnexec_path() -> PathBuf {
    let debug_candidate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/mnexec");
    mnexec_path_from(
        std::env::var_os(MNEXEC_ENV).as_deref(),
        Some(&debug_candidate),
    )
}

/// Pure core of [`mnexec_path`], split out for unit testing.
///
/// `env_mnexec` is the value of `$MNEXEC` (if any); `debug_candidate` is
/// the workspace `target/debug/mnexec` path (if the caller computed one).
pub fn mnexec_path_from(env_mnexec: Option<&OsStr>, debug_candidate: Option<&Path>) -> PathBuf {
    if let Some(p) = env_mnexec {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(candidate) = debug_candidate {
        if candidate.is_file() {
            return candidate.to_path_buf();
        }
    }
    PathBuf::from("mnexec")
}

/// Whether a resolved [`mnexec_path`] is actually usable: an explicit path
/// must exist and be an executable file; a bare name (no path separator)
/// is resolved against `PATH` exactly like `execvp(3)` would at spawn
/// time. Tests use this to skip gracefully on clean machines where no
/// `mnexec` has been built or installed.
pub fn mnexec_available(path: &Path) -> bool {
    mnexec_available_in(path, std::env::var_os("PATH").as_deref())
}

/// Pure core of [`mnexec_available`], split out for unit testing.
///
/// `path_env` is the value of `$PATH` to search (pass `None` to simulate
/// an empty `PATH`).
pub fn mnexec_available_in(path: &Path, path_env: Option<&OsStr>) -> bool {
    let name = path.as_os_str();
    if name.to_string_lossy().contains(std::path::MAIN_SEPARATOR) {
        // Explicit path (absolute or relative): must be an executable file.
        return is_executable_file(path);
    }
    // Bare name: resolve against PATH like execvp(3).
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| is_executable_file(&dir.join(name)))
}

/// Is `path` a regular file with at least one executable bit set?
#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Non-Unix fallback: existence of a regular file.
#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Build the shell-spawning argv (Python `Node.startShell`).
///
/// With `attach_pid = Some(pid)` this is
/// `mnexec -cd -a <pid> env PS1=\x7f bash --norc --noediting -is mininet:<name>`
/// — the shell attaches to the holder pid's namespaces, i.e. runs inside
/// the node's named netns. With `attach_pid = None` it is the root-ns form
/// `mnexec -cd env PS1=\x7f bash ...` (no `-n`: the namespace was already
/// created by `netns_add` + holder, and `mnexec -n` would unshare a *new*
/// one).
pub fn shell_argv(mnexec: &Path, name: &str, attach_pid: Option<u32>) -> Vec<String> {
    let mut argv = vec![mnexec.to_string_lossy().into_owned(), "-cd".to_string()];
    if let Some(pid) = attach_pid {
        argv.push("-a".to_string());
        argv.push(pid.to_string());
    }
    argv.extend([
        "env".to_string(),
        format!("PS1={PROMPT_SENTINEL}"),
        "bash".to_string(),
        "--norc".to_string(),
        "--noediting".to_string(),
        "-is".to_string(),
        format!("mininet:{name}"),
    ]);
    argv
}

/// Build a one-shot in-namespace command argv (Python `Node.popen`'s
/// default `mncmd`): `mnexec -da <pid> <cmd...>`.
pub fn attach_argv(mnexec: &Path, pid: u32, cmd: &[String]) -> Vec<String> {
    let mut argv = vec![
        mnexec.to_string_lossy().into_owned(),
        "-da".to_string(),
        pid.to_string(),
    ];
    argv.extend(cmd.iter().cloned());
    argv
}

/// Port of `Node.sendCmd`'s command-line munging: empty commands become
/// `echo -n`, a trailing `&` appends the `^A<pid>\n` marker printf, and
/// `print_pid` prefixes `mnexec -p ` (without Python's `isShellBuiltin`
/// check — see crate docs).
pub fn send_cmd_line(cmd: &str, print_pid: bool) -> String {
    let mut cmd = cmd.to_string();
    // Python: `if not re.search(r'\w', cmd): cmd = 'echo -n'`
    if !cmd.chars().any(|c| c.is_alphanumeric() || c == '_') {
        cmd = "echo -n".to_string();
    }
    if cmd.ends_with('&') {
        // print ^A{pid}\n so monitor()/wait_output can recover lastPid
        cmd.push_str(" printf \"\\001%d\\012\" $! ");
    } else if print_pid {
        cmd = format!("mnexec -p {cmd}");
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnexec_path_prefers_env_var() {
        let p = mnexec_path_from(Some(OsStr::new("/opt/mn/mnexec")), Some(Path::new("/x")));
        assert_eq!(p, PathBuf::from("/opt/mn/mnexec"));
    }

    #[test]
    fn mnexec_path_falls_back_to_debug_binary_then_path() {
        let dir = std::env::temp_dir().join(format!("mn-node-mnexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("mnexec");
        std::fs::write(&bin, b"").unwrap();
        let p = mnexec_path_from(None, Some(&bin));
        assert_eq!(p, bin);
        let missing = dir.join("no-such-binary");
        let p = mnexec_path_from(None, Some(&missing));
        assert_eq!(p, PathBuf::from("mnexec"));
        // Empty env value is ignored, too.
        let p = mnexec_path_from(Some(OsStr::new("")), Some(&missing));
        assert_eq!(p, PathBuf::from("mnexec"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    fn make_executable(dir: &std::path::Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn mnexec_available_resolves_bare_names_against_path() {
        let dir = std::env::temp_dir().join(format!("mn-node-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = make_executable(&dir, "mnexec");
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let path_env = std::env::join_paths([&dir, &empty]).unwrap();
        // Bare name found on PATH:
        assert!(mnexec_available_in(Path::new("mnexec"), Some(&path_env)));
        // Bare name NOT on PATH (clean machine: no mnexec installed):
        let empty_env = std::env::join_paths([&empty]).unwrap();
        assert!(!mnexec_available_in(Path::new("mnexec"), Some(&empty_env)));
        // No PATH at all:
        assert!(!mnexec_available_in(Path::new("mnexec"), None));
        // Explicit executable path is available:
        assert!(mnexec_available_in(&bin, None));
        // Explicit path that is not executable (e.g. a source file):
        let plain = dir.join("mnexec.c");
        std::fs::write(&plain, b"int main(){}").unwrap();
        assert!(!mnexec_available_in(&plain, None));
        // Explicit path that does not exist:
        assert!(!mnexec_available_in(&dir.join("nope"), None));
        // Relative path with a separator:
        assert!(!mnexec_available_in(
            Path::new("./no-such-mnexec"),
            Some(&path_env)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shell_argv_root_ns_form_matches_python_minus_n() {
        let argv = shell_argv(Path::new("mnexec"), "h1", None);
        assert_eq!(
            argv,
            [
                "mnexec",
                "-cd",
                "env",
                "PS1=\x7f",
                "bash",
                "--norc",
                "--noediting",
                "-is",
                "mininet:h1"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shell_argv_attach_form_adds_a_pid() {
        let argv = shell_argv(Path::new("/t/mnexec"), "s1", Some(4242));
        assert_eq!(
            argv,
            [
                "/t/mnexec",
                "-cd",
                "-a",
                "4242",
                "env",
                "PS1=\x7f",
                "bash",
                "--norc",
                "--noediting",
                "-is",
                "mininet:s1"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn attach_argv_matches_python_popen_mncmd() {
        let cmd = vec!["ip".to_string(), "link".to_string(), "show".to_string()];
        let argv = attach_argv(Path::new("mnexec"), 1234, &cmd);
        assert_eq!(
            argv,
            ["mnexec", "-da", "1234", "ip", "link", "show"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn send_cmd_line_handles_empty_and_background_and_pid() {
        assert_eq!(send_cmd_line("", false), "echo -n");
        assert_eq!(send_cmd_line("   ", false), "echo -n");
        assert_eq!(send_cmd_line("ls -l", false), "ls -l");
        assert_eq!(
            send_cmd_line("sleep 10 &", false),
            "sleep 10 & printf \"\\001%d\\012\" $! "
        );
        assert_eq!(send_cmd_line("iperf -s", true), "mnexec -p iperf -s");
        // '&' wins over print_pid, like Python's elif chain.
        assert_eq!(
            send_cmd_line("iperf -s &", true),
            "iperf -s & printf \"\\001%d\\012\" $! "
        );
    }
}
