//! Command-line argument surface of the Rust `mn` port.
//!
//! This is a parsing-only port of `bin/mn` (Python `optparse`, 451 lines):
//! every long option name, short flag, default, and help string of the
//! original is mirrored here with `clap` derive. Nothing is wired to
//! netlink/nodes yet — see `main.rs`.
//!
//! Option inventory (from `bin/mn`'s `MininetRunner.parseArgs`):
//!
//! * dict options (`addDictOption`): `--switch`, `--host`, `--controller`,
//!   `--link`, `--topo` — free-form `name[,param=value...]` strings whose
//!   value is split by [`parse_class_spec`] the way `mininet.util.splitArgs`
//!   splits `bin/mn`'s `topo,arg,kw=val` values.
//! * plain options: `--clean/-c`, `--custom`, `--test`, `--xterms/-x`,
//!   `--ipbase/-i`, `--mac`, `--arp`, `--verbosity/-v`, `--innamespace`,
//!   `--listenport`, `--nolistenport`, `--pre`, `--post`, `--pin`, `--nat`,
//!   `--version`, `--wait/-w`, `--twait/-t`, `--cluster`, `--placement`.

use clap::{ArgAction, Parser};

/// Mininet version, ported from `mininet/net.py` (`VERSION = "2.3.1b4"`).
///
/// `bin/mn --version` prints this string verbatim (via the `version()`
/// callback: `output("%s\n" % VERSION)`), so the Rust port must print the
/// bare version string too — not clap's `{name} {version}` format.
pub const VERSION: &str = "2.3.1b4";

// The `mn` utility: creates a Mininet network from the command line.
//
// Port of `bin/mn`'s `OptionParser` (`usage`/`description` in
// `MininetRunner.parseArgs`). Field order matches the Python option
// registration order so `--help` lists options in the same order.
// (Plain comments on purpose: clap derive would otherwise turn doc comments
// into long_help text.)
#[derive(Debug, Clone, Parser)]
#[command(
    name = "mnr",
    disable_version_flag = true,
    about = "The mn utility creates Mininet network from the command line. It can create parametrized topologies, invoke the Mininet CLI, and run tests."
)]
pub struct MnArgs {
    // default|ivs|lxbr|ovs|ovsbr|ovsk|user[,param=value...] user=UserSwitch ovs=OVSSwitch ovsbr=OVSBridge ovsk=OVSSwitch ivs=IVSSwitch lxbr=LinuxBridge default=OVSSwitch
    #[arg(
        long,
        value_name = "SWITCH",
        default_value = "default",
        help = "default|ivs|lxbr|ovs|ovsbr|ovsk|user[,param=value...] user=UserSwitch ovs=OVSSwitch ovsbr=OVSBridge ovsk=OVSSwitch ivs=IVSSwitch lxbr=LinuxBridge default=OVSSwitch"
    )]
    pub switch: String,

    // cfs|proc|rt[,param=value...] proc=Host rt=CPULimitedHost{'sched': 'rt'} cfs=CPULimitedHost{'sched': 'cfs'}
    #[arg(
        long,
        value_name = "HOST",
        default_value = "proc",
        help = "cfs|proc|rt[,param=value...] proc=Host rt=CPULimitedHost{'sched': 'rt'} cfs=CPULimitedHost{'sched': 'cfs'}"
    )]
    pub host: String,

    // default|none|nox|ovsc|ref|remote|ryu[,param=value...] ref=Controller ovsc=OVSController nox=NOX remote=RemoteController ryu=Ryu default=DefaultController none=NullController
    #[arg(
        long,
        value_name = "CONTROLLER",
        action = ArgAction::Append,
        help = "default|none|nox|ovsc|ref|remote|ryu[,param=value...] ref=Controller ovsc=OVSController nox=NOX remote=RemoteController ryu=Ryu default=DefaultController none=NullController"
    )]
    pub controller: Vec<String>,

    // default|ovs|tc|tcu[,param=value...] default=Link tc=TCLink tcu=TCULink ovs=OVSLink
    #[arg(
        long,
        value_name = "LINK",
        default_value = "default",
        help = "default|ovs|tc|tcu[,param=value...] default=Link tc=TCLink tcu=TCULink ovs=OVSLink"
    )]
    pub link: String,

    // linear|minimal|reversed|single|torus|tree[,param=value...] minimal=MinimalTopo linear=LinearTopo reversed=SingleSwitchReversedTopo single=SingleSwitchTopo tree=TreeTopo torus=TorusTopo
    #[arg(
        long,
        value_name = "TOPO",
        default_value = "minimal",
        help = "linear|minimal|reversed|single|torus|tree[,param=value...] minimal=MinimalTopo linear=LinearTopo reversed=SingleSwitchReversedTopo single=SingleSwitchTopo tree=TreeTopo torus=TorusTopo"
    )]
    pub topo: String,

    // clean and exit
    #[arg(short = 'c', long, action = ArgAction::SetTrue, help = "clean and exit")]
    pub clean: bool,

    // read custom classes or params from .py file(s)
    #[arg(
        long,
        value_name = "CUSTOM",
        help = "read custom classes or params from .py file(s)"
    )]
    pub custom: Option<String>,

    // pingall|pingpair|iperf|iperfudp|all|none|build
    #[arg(
        long,
        value_name = "TEST",
        action = ArgAction::Append,
        help = "pingall|pingpair|iperf|iperfudp|all|none|build"
    )]
    pub test: Vec<String>,

    // spawn xterms for each node
    #[arg(
        short = 'x',
        long,
        action = ArgAction::SetTrue,
        help = "spawn xterms for each node"
    )]
    pub xterms: bool,

    // print the topology plan + dump + links sections and exit; non-root
    // runs use the dry-run (no-kernel) path. Not in bin/mn's optparse —
    // this flag exists for scripts/diff_topology.sh, which probes
    // `mn --topo <spec> ... --dump` and expects exit 0.
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "print the planned topology (=== topo === / === dump === / === links === sections, as scripts/diff_topology.sh expects) and exit; works non-root via the dry-run path"
    )]
    pub dump: bool,

    // base IP address for hosts
    #[arg(
        short = 'i',
        long,
        value_name = "IPBASE",
        default_value = "10.0.0.0/8",
        help = "base IP address for hosts"
    )]
    pub ipbase: String,

    // automatically set host MACs
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "automatically set host MACs"
    )]
    pub mac: bool,

    // set all-pairs ARP entries
    #[arg(long, action = ArgAction::SetTrue, help = "set all-pairs ARP entries")]
    pub arp: bool,

    // debug|info|output|warning|warn|error|critical
    #[arg(
        short = 'v',
        long,
        value_name = "VERBOSITY",
        default_value = "info",
        value_parser = clap::builder::PossibleValuesParser::new([
            "debug",
            "info",
            "output",
            "warning",
            "warn",
            "error",
            "critical",
        ]),
        help = "debug|info|output|warning|warn|error|critical"
    )]
    pub verbosity: String,

    // sw and ctrl in namespace?
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "sw and ctrl in namespace?"
    )]
    pub innamespace: bool,

    // base port for passive switch listening
    #[arg(
        long,
        value_name = "LISTENPORT",
        default_value_t = 6654,
        help = "base port for passive switch listening"
    )]
    pub listenport: i64,

    // don't use passive listening port
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "don't use passive listening port"
    )]
    pub nolistenport: bool,

    // CLI script to run before tests
    #[arg(long, value_name = "PRE", help = "CLI script to run before tests")]
    pub pre: Option<String>,

    // CLI script to run after tests
    #[arg(long, value_name = "POST", help = "CLI script to run after tests")]
    pub post: Option<String>,

    // pin hosts to CPU cores (requires --host cfs or --host rt)
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "pin hosts to CPU cores (requires --host cfs or --host rt)"
    )]
    pub pin: bool,

    // [option=val...] adds a NAT to the topology that connects Mininet hosts to the physical network. Warning: This may route any traffic on the machine that uses Mininet's IP subnet into the Mininet network. If you need to change Mininet's IP subnet, see the --ipbase option.
    //
    // Mirrors `bin/mn`'s callback option: the flag takes no required value,
    // but a following non-option argument is consumed as NAT params
    // (`--nat` -> `Some("")`, `--nat foo=bar` -> `Some("foo=bar")`).
    #[arg(
        long,
        value_name = "OPTION=VAL...",
        num_args = 0..=1,
        default_missing_value = "",
        help = "[option=val...] adds a NAT to the topology that connects Mininet hosts to the physical network. Warning: This may route any traffic on the machine that uses Mininet's IP subnet into the Mininet network. If you need to change Mininet's IP subnet, see the --ipbase option."
    )]
    pub nat: Option<String>,

    // prints the version and exits
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "prints the version and exits"
    )]
    pub version: bool,

    // wait for switches to connect
    #[arg(
        short = 'w',
        long,
        action = ArgAction::SetTrue,
        help = "wait for switches to connect"
    )]
    pub wait: bool,

    // timed wait (s) for switches to connect
    #[arg(
        short = 't',
        long,
        value_name = "WAIT",
        help = "timed wait (s) for switches to connect"
    )]
    pub twait: Option<i64>,

    // run on multiple servers (experimental!)
    #[arg(
        long,
        value_name = "server1,server2...",
        help = "run on multiple servers (experimental!)"
    )]
    pub cluster: Option<String>,

    // node placement for --cluster (experimental!)
    #[arg(
        long,
        value_name = "block|random",
        default_value = "block",
        value_parser = clap::builder::PossibleValuesParser::new(["block", "random"]),
        help = "node placement for --cluster (experimental!)"
    )]
    pub placement: String,
}

impl MnArgs {
    /// `--topo` value split into `(name, [params])`, e.g.
    /// `tree,2,2` -> `("tree", ["2", "2"])`.
    pub fn topo_spec(&self) -> ClassSpec {
        parse_class_spec(&self.topo)
    }

    /// `--link` value split into `(name, [params])`, e.g.
    /// `tc,bw=10,delay=5ms` -> `("tc", ["bw=10", "delay=5ms"])`.
    pub fn link_spec(&self) -> ClassSpec {
        parse_class_spec(&self.link)
    }

    /// `--switch` value split into `(name, [params])`.
    pub fn switch_spec(&self) -> ClassSpec {
        parse_class_spec(&self.switch)
    }

    /// `--host` value split into `(name, [params])`.
    pub fn host_spec(&self) -> ClassSpec {
        parse_class_spec(&self.host)
    }

    /// Each `--controller` value split into `(name, [params])`.
    pub fn controller_specs(&self) -> Vec<ClassSpec> {
        self.controller
            .iter()
            .map(|c| parse_class_spec(c))
            .collect()
    }

    /// Whether `--nat` was given (`--nat` or `--nat option=val...`).
    pub fn nat_enabled(&self) -> bool {
        self.nat.is_some()
    }
}

/// A `name[,param...]` class/param spec, the first stage of
/// `mininet.util.splitArgs` as used by `bin/mn` for `--topo`, `--link`,
/// `--switch`, `--host`, `--controller` (e.g. `tree,2,2`,
/// `tc,bw=10,delay=5ms`, `remote,ip=127.0.0.1,port=6633`).
///
/// Numeric coercion of positional params (`makeNumeric` in `splitArgs`) is
/// deliberately left to the consumer for now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSpec {
    /// The class/topo/link name, e.g. `tree` or `tc`.
    pub name: String,
    /// Raw comma-separated params after the name, e.g. `["2", "2"]` or
    /// `["bw=10", "delay=5ms"]`.
    pub params: Vec<String>,
}

impl ClassSpec {
    /// Positional params (no `=`), mirroring `splitArgs`' `args`.
    pub fn args(&self) -> Vec<&str> {
        self.params
            .iter()
            .filter(|p| !p.contains('='))
            .map(String::as_str)
            .collect()
    }

    /// Keyword params split at the first `=`, mirroring `splitArgs`' `kwargs`.
    pub fn kwargs(&self) -> Vec<(&str, &str)> {
        self.params
            .iter()
            .filter_map(|p| p.split_once('='))
            .collect()
    }
}

/// Split a `name[,param...]` spec into `(name, [params])`.
///
/// Mirrors the first half of `mininet.util.splitArgs` (`argstr.split(',')`):
/// the first comma-separated token is the name, the rest are params.
pub fn parse_class_spec(spec: &str) -> ClassSpec {
    let mut parts = spec.split(',');
    let name = parts.next().unwrap_or_default().to_string();
    let params = parts.map(str::to_string).collect();
    ClassSpec { name, params }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Result<MnArgs, clap::Error> {
        MnArgs::try_parse_from(argv)
    }

    #[test]
    fn defaults_match_bin_mn() {
        let args = parse(&["mnr"]).expect("bare `mn` must parse");
        assert_eq!(args.switch, "default");
        assert_eq!(args.host, "proc");
        assert!(args.controller.is_empty());
        assert_eq!(args.link, "default");
        assert_eq!(args.topo, "minimal");
        assert!(!args.clean);
        assert!(args.custom.is_none());
        assert!(args.test.is_empty());
        assert!(!args.xterms);
        assert!(!args.dump); // harness-only flag (scripts/diff_topology.sh)
        assert_eq!(args.ipbase, "10.0.0.0/8");
        assert!(!args.mac);
        assert!(!args.arp);
        assert_eq!(args.verbosity, "info");
        assert!(!args.innamespace);
        assert_eq!(args.listenport, 6654);
        assert!(!args.nolistenport);
        assert!(args.pre.is_none());
        assert!(args.post.is_none());
        assert!(!args.pin);
        assert!(!args.nat_enabled());
        assert!(!args.version);
        assert!(!args.wait);
        assert!(args.twait.is_none());
        assert!(args.cluster.is_none());
        assert_eq!(args.placement, "block");
    }

    #[test]
    fn representative_topo_and_link_args() {
        let args = parse(&["mnr", "--topo", "tree,2,2", "--link", "tc,bw=10,delay=5ms"])
            .expect("representative argv must parse");
        let topo = args.topo_spec();
        assert_eq!(topo.name, "tree");
        assert_eq!(topo.params, vec!["2".to_string(), "2".to_string()]);
        assert_eq!(topo.args(), vec!["2", "2"]);
        assert!(topo.kwargs().is_empty());

        let link = args.link_spec();
        assert_eq!(link.name, "tc");
        assert_eq!(
            link.params,
            vec!["bw=10".to_string(), "delay=5ms".to_string()]
        );
        assert!(link.args().is_empty());
        assert_eq!(link.kwargs(), vec![("bw", "10"), ("delay", "5ms")]);
    }

    #[test]
    fn short_flags_and_values() {
        let args = parse(&[
            "mn",
            "-c",
            "-x",
            "-i",
            "10.1.0.0/16",
            "-v",
            "debug",
            "-w",
            "-t",
            "5",
        ])
        .expect("short flags must parse");
        assert!(args.clean);
        assert!(args.xterms);
        assert_eq!(args.ipbase, "10.1.0.0/16");
        assert_eq!(args.verbosity, "debug");
        assert!(args.wait);
        assert_eq!(args.twait, Some(5));
    }

    #[test]
    fn repeated_controller_and_test_append() {
        let args = parse(&[
            "mn",
            "--controller",
            "remote,ip=127.0.0.1,port=6633",
            "--controller",
            "ovsc",
            "--test",
            "pingall",
            "--test",
            "iperf",
        ])
        .expect("append options must parse");
        assert_eq!(
            args.controller,
            vec!["remote,ip=127.0.0.1,port=6633", "ovsc"]
        );
        let specs = args.controller_specs();
        assert_eq!(specs[0].name, "remote");
        assert_eq!(
            specs[0].kwargs(),
            vec![("ip", "127.0.0.1"), ("port", "6633")]
        );
        assert_eq!(specs[1].name, "ovsc");
        assert!(specs[1].params.is_empty());
        assert_eq!(args.test, vec!["pingall", "iperf"]);
    }

    #[test]
    fn nat_flag_optional_value() {
        // `--nat` alone: flag present, no params (bin/mn's setNat with empty rargs)
        let args = parse(&["mnr", "--nat"]).expect("--nat must parse");
        assert!(args.nat_enabled());
        assert_eq!(args.nat.as_deref(), Some(""));

        // `--nat foo=bar`: following non-option arg consumed as NAT params
        let args = parse(&["mnr", "--nat", "foo=bar"]).expect("--nat foo=bar must parse");
        assert_eq!(args.nat.as_deref(), Some("foo=bar"));

        // `--nat` followed by another option: not consumed
        let args = parse(&["mnr", "--nat", "--mac"]).expect("--nat --mac must parse");
        assert_eq!(args.nat.as_deref(), Some(""));
        assert!(args.mac);

        // absent
        let args = parse(&["mnr"]).expect("bare mn must parse");
        assert!(!args.nat_enabled());
        assert_eq!(args.nat, None);
    }

    #[test]
    fn version_flag_is_recognized() {
        let args = parse(&["mnr", "--version"]).expect("--version must parse");
        assert!(args.version);
    }

    #[test]
    fn choice_options_validate_like_optparse() {
        let args = parse(&["mnr", "--placement", "random", "-v", "warn"]).unwrap();
        assert_eq!(args.placement, "random");
        assert_eq!(args.verbosity, "warn");

        assert!(parse(&["mnr", "--placement", "bogus"]).is_err());
        assert!(parse(&["mnr", "-v", "bogus"]).is_err());
    }

    #[test]
    fn misc_value_options() {
        let args = parse(&[
            "mn",
            "--custom",
            "custom.py,other.py",
            "--listenport",
            "6653",
            "--nolistenport",
            "--pre",
            "pre.txt",
            "--post",
            "post.txt",
            "--pin",
            "--innamespace",
            "--arp",
            "--mac",
            "--cluster",
            "srv1,srv2",
        ])
        .unwrap();
        assert_eq!(args.custom.as_deref(), Some("custom.py,other.py"));
        assert_eq!(args.listenport, 6653);
        assert!(args.nolistenport);
        assert_eq!(args.pre.as_deref(), Some("pre.txt"));
        assert_eq!(args.post.as_deref(), Some("post.txt"));
        assert!(args.pin);
        assert!(args.innamespace);
        assert!(args.arp);
        assert!(args.mac);
        assert_eq!(args.cluster.as_deref(), Some("srv1,srv2"));
    }

    #[test]
    fn unknown_option_and_extra_positional_are_rejected() {
        // optparse: "no such option: --bogus" (exit 2); extra args: help + exit
        assert!(parse(&["mnr", "--bogus"]).is_err());
        assert!(parse(&["mnr", "extra-arg"]).is_err());
    }

    #[test]
    fn parse_class_spec_splits_name_and_params() {
        let spec = parse_class_spec("tree,2,2");
        assert_eq!(spec.name, "tree");
        assert_eq!(spec.params, vec!["2", "2"]);

        let spec = parse_class_spec("tc,bw=10,delay=5ms");
        assert_eq!(spec.name, "tc");
        assert_eq!(spec.params, vec!["bw=10", "delay=5ms"]);
        assert_eq!(spec.kwargs(), vec![("bw", "10"), ("delay", "5ms")]);

        let spec = parse_class_spec("single");
        assert_eq!(spec.name, "single");
        assert!(spec.params.is_empty());
        assert!(spec.args().is_empty());
        assert!(spec.kwargs().is_empty());

        // value containing '=' splits at the first '=' only (splitArgs semantics)
        let spec = parse_class_spec("remote,opts=a=b");
        assert_eq!(spec.kwargs(), vec![("opts", "a=b")]);
    }
}
