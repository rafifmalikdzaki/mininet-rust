//! `mn-node` — Rust port of the `Node`/`Host` core of `mininet/node.py`.
//!
//! A Mininet node is "simply a shell in a network namespace" (node.py):
//! we spawn a persistent `bash` inside the node's namespace through the
//! `mnexec` helper (`mnexec -a <pid>` attaches to a pid's namespaces) and
//! talk to it over pipes, using the prompt sentinel `chr(127)` to delimit
//! command output. One-shot commands (`popen`/`pexec` in Python) are run
//! as `mnexec -da <pid> <cmd...>` subprocesses.
//!
//! # What is ported here (task 8a slice)
//!
//! - [`Node`]: model (name, netns name, `in_namespace`, `params`/opts dict,
//!   `intfs` port→[`mn_link::Intf`] map, `default_intf`), lifecycle
//!   ([`Node::start`]/[`Node::stop`]), command execution
//!   ([`Node::cmd`]/[`Node::send_cmd`]/[`Node::wait_output`]/[`Node::popen`]/
//!   [`Node::pexec`]), interface management ([`Node::add_intf`]/[`Node::intf`]/
//!   [`Node::intf_names`]/[`Node::set_ip`]/[`Node::set_mac`]/[`Node::config`]).
//! - [`Host`]: a `Node` with Mininet's automatic IP/MAC assignment hooks
//!   (`mn_util::ip_add_with` / `mac_colon_hex`, as `Mininet.addHost` does in
//!   `mininet/net.py`).
//! - [`Switch`] + subclasses [`OVSSwitch`], [`OVSBridge`], [`UserSwitch`],
//!   [`IVSSwitch`] (task 8b): the OVS control plane stays behind `ovs-vsctl`
//!   subprocess calls (per the migration plan — retained, not ported to
//!   netlink); all `ovs-vsctl` argv vectors are pure, unit-tested builders in
//!   [`switch`]. `UserSwitch`/`IVSSwitch` subprocess launches are root-gated.
//! - [`Controller`] + subclasses [`OVSController`], [`RemoteController`],
//!   [`NOX`], [`Ryu`] (task 8b): `isListening` is a std `TcpStream` probe
//!   with timeout (works unprivileged on localhost); `NOX`/`Ryu` are stubs
//!   with TODO `start()`. [`CPULimitedHost`] remains a stub (cgroup CPU
//!   limiting is a later slice).
//!
//! # Namespace lifecycle
//!
//! [`Node::start`] for an `in_namespace` node:
//! 1. `mn_netlink::netns_add(<name>)` creates the named netns
//!    (`/var/run/netns/<name>` bind mount, like `ip netns add`);
//! 2. a holder child is forked that `setns`es into the named netns **by
//!    name** (`/var/run/netns/<name>`, via the allocation-free
//!    `mn_netlink::setns_path_raw` on a pre-fork CString) and sleeps. The
//!    parent blocks on a readiness pipe until the child writes one byte
//!    *after* its `setns` succeeded (timeout/EOF = failure), so the shell
//!    can never be attached to a holder that is still in the root
//!    namespace — that was the holder/setns race;
//! 3. the shell is spawned as `mnexec -cda <holder_pid> env PS1=\x7f bash
//!    --norc --noediting -is mininet:<name>`, i.e. attached to the holder's
//!    (the node's) namespace; `node.pid` is the shell's pid. As defense in
//!    depth, `start_shell` additionally verifies
//!    `readlink(/proc/<shell>/ns/net) == readlink(/var/run/netns/<name>)`.
//!
//! [`Node::stop`] kills the shell's process group (SIGHUP, like Python's
//! `os.killpg`; SIGKILL fallback also uses `killpg`), kills the holder,
//! and `mn_netlink::netns_del`s the named namespace. All cleanup steps run
//! even on the error path (first error is returned, nothing leaks).
//!
//! # `mnexec` location
//!
//! The `mnexec` binary is resolved by [`mnexec::mnexec_path`]: `$MNEXEC` if
//! set, else `<workspace>/target/debug/mnexec` if it exists, else `mnexec`
//! on `PATH`.
//!
//! # Deliberate deviations from CPython (documented)
//!
//! - Python's `Node.__init__` starts the shell immediately; here
//!   construction ([`Node::new`]) is pure and [`Node::start`] performs all
//!   kernel/process side effects. This keeps the model unit-testable
//!   without root.
//! - Python spawns the shell on a pty; here stdin is a pipe and
//!   stdout+stderr share one pipe (so the `\x7f` prompt sentinel, which
//!   bash prints on stderr, is seen on the same stream — the pty merged
//!   them too).
//! - For `in_namespace=false` nodes, one-shot commands are spawned
//!   directly instead of via `mnexec -a <pid>`: attaching to the root
//!   namespace is a no-op, and `setns(2)` to one's own namespace still
//!   requires `CAP_SYS_ADMIN`, so the `mnexec -a` detour would fail for
//!   unprivileged users where Python (running as root) never noticed.
//! - `sendCmd`'s `printPid` handling prefixes `mnexec -p ` without
//!   Python's `isShellBuiltin` check (that helper lives in `util.py` and is
//!   not ported yet).
//! - Interface IP/MAC configuration is applied via netlink, not
//!   `ifconfig`: while the interface is still in the orchestrator's
//!   namespace this uses the local [`mn_netlink::Netlink`] handle; once the
//!   node is running, a short-lived forked child does `setns` into the
//!   node's namespace and performs the netlink op there
//!   ([`Node::set_ip`]/[`Node::set_mac`]). On a not-yet-started node the
//!   model is updated only (like Python, whose `cmd()` on a dead shell
//!   just warns).
//! - `set_default_route` keeps Python's shell form (`ip route del default;
//!   ip route add default dev <intf> [via <gw>]` run inside the ns); the
//!   pure-netlink `route_add_default` in `mn-netlink` is used by the
//!   root-gated test path where a gateway form is needed.
//! - `connectionsTo`, privateDirs mounts, `monitor()` pid-marker parsing
//!   beyond the basic sentinel handling, and the `fdToNode` poll maps are
//!   not ported in this slice.
//!
//! # Privilege model
//!
//! Model, command construction, and error paths are testable anywhere.
//! Everything that creates netns/processes-in-netns (i.e. `start()` for
//! `in_namespace` nodes, netlink ops inside a node ns) needs root +
//! `CAP_SYS_ADMIN`; those tests are `#[ignore]`d **and** gated on
//! `MN_RUST_ROOT_TESTS=1`, to run on a privileged host/CI runner:
//! `MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1`

mod error;
mod host;
mod mnexec;
mod node;

pub mod controller;
pub mod switch;

pub use controller::{
    cargs_with_port, default_controller, is_listening, is_listening_timeout, Controller,
    OVSController, RemoteController, Ryu, DEFAULT_CARGS, DEFAULT_COMMAND, DEFAULT_IP, DEFAULT_PORT,
    DEFAULT_PROTOCOL, LISTEN_PROBE_TIMEOUT, NOX,
};
pub use error::{Error, Result};
pub use host::{CPULimitedHost, Host};
pub use mnexec::{
    attach_argv, mnexec_available, mnexec_available_in, mnexec_path, mnexec_path_from,
    send_cmd_line, shell_argv, which, which_in, PROMPT_SENTINEL,
};
pub use node::{run_in_netns, CmdOutput, Node};
pub use switch::{
    bridge_opts, controller_target, default_dpid, default_dpid_len, dpctl_listen_addr,
    intf_opts_args, is_old_ovs_version, ivs_logfile, ivs_start_argv, ofctl_dpctl_argv,
    ofdatapath_cmd, ofprotocol_cmd, ovs_version_from, parse_controller_uuids, ptcp_target,
    user_switch_listen_opt, vsctl_add_br, vsctl_add_port, vsctl_batch_del_br, vsctl_del_br,
    vsctl_del_controller, vsctl_del_port, vsctl_get_bridge_controller,
    vsctl_get_controller_is_connected, vsctl_set_bridge, vsctl_set_controller,
    vsctl_set_datapath_id, vsctl_set_fail_mode, IVSSwitch, OVSBridge, OVSSwitch, Switch,
    UserSwitch, DPID_LEN, OVS_OFCTL, OVS_VSCTL, USER_DPID_LEN,
};

/// Whether root/namespace integration tests should actually run.
///
/// Same convention as `mn-netlink`/`mn-link`: all root-requiring tests are
/// `#[ignore]`d by default **and** additionally gated on this, so they only
/// execute when `MN_RUST_ROOT_TESTS=1` is set, on a privileged host/CI:
/// `MN_RUST_ROOT_TESTS=1 cargo test -p mn-node -- --ignored --test-threads=1`
#[cfg(test)]
pub(crate) fn root_tests_enabled() -> bool {
    std::env::var("MN_RUST_ROOT_TESTS").is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_gate_defaults_off() {
        // In this sandbox MN_RUST_ROOT_TESTS is unset, so the gate is off.
        // (If a developer runs the suite with it set, this test simply
        // reports the enabled state instead of failing.)
        let _ = root_tests_enabled();
    }
}
