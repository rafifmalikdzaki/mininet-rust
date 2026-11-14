# Rust Mininet research experiments

The runner constructs an arbitrary `mn-topo` graph and starts it with `mn-net`. Hosts (including routers) have separate Linux network namespaces; switch nodes are real Open vSwitch bridges by default (LinuxBridge remains available explicitly). Scenario commands execute as root. Do not run untrusted scenarios: commands are unrestricted shell code and a privileged container has substantial host privileges. Never mount sensitive host paths into it.

## Prerequisites and launch

Use Linux with network namespace, veth and OVS kernel support, Docker or rootless Podman, and permission to start a privileged container. Rootless Podman cannot load the OVS kernel module; the host must already provide it. The image includes FRRouting, Open vSwitch, iproute2, ping, iperf3, tcpdump, the Rust `mnexec`, `mnr`, and the experiment runner. The entrypoint starts OVS before the scenario. Build from the repository root:

```sh
docker build -f example/Dockerfile -t mininet-research .
mkdir -p runs
docker run --rm --init --privileged --network none -v "$PWD/runs:/runs" \
  mininet-research /scenarios/ospf.json /runs/ospf-001
```

Rootless Podman equivalent (the image can be built with Docker and loaded with `docker save ... | podman load`):

```sh
podman run --rm --init --privileged --network none -v "$PWD/runs:/runs" \
  mininet-research /scenarios/ospf.json /runs/ospf-001
```

Choose a **new** run directory for every invocation. To run a custom scenario, mount it read-only, e.g. `-v "$PWD/my-scenario.json:/scenario.json:ro"` and pass `/scenario.json /runs/my-run-001`. The container is disposable; its processes and network state are not intended to persist after the run. `--init` forwards signals and reaps orphaned daemon processes. Monitoring integration is in `example/monitoring/`. Container execution needs the host's kernel networking capabilities; failure to create namespaces or interfaces is an error, not a simulated run.

The new run directory and its artifacts inherit the numeric owner and group of the run directory's parent, so a bind-mounted `/runs` should be owned by the researcher who will read the results. Copied FRR files keep their original permission modes; new artifacts respect the process umask. Ownership is assigned as artifacts are created, including live event and telemetry files. If the container cannot assign the parent's numeric IDs (for example, due to user-namespace mappings), the runner fails rather than silently leaving inaccessible artifacts. Use a real directory, not a symlink, as the run parent.

Append `--hold-secs 120` after the run directory to retain the network for two minutes after the scenario checks, while collecting repeated telemetry. The default is zero; a run never holds indefinitely. Sampling also runs after startup and after each command attempt. A sample cycle can take longer than a second, and no background sample is taken during a long-running command.

## Presets

Choose a file under `/scenarios/` in the container:

| File | Research scenario |
|---|---|
| `ospf.json` | Two OSPF routers, adjacency, route installation and reachability |
| `isis.json` | Two IS-IS routers, adjacency, route installation and reachability |
| `bgp.json` | eBGP peering between two autonomous systems |
| `vlan.json` | Two VLANs with connectivity and isolation checks (legacy LinuxBridge) |
| `vlan-ovs.json` | Same VLAN checks with real OVS access ports |
| `multi-tenant-ovs.json` | Three sites/tenants on OVS, full isolation matrix |
| `multi-tenant-fault-ovs.json` | OVS multi-tenant trunk failure and recovery |
| `ospf-ovs.json` | OSPF adjacency and routes through an OVS switch |
| `dual-home-ovs.json` | Active-backup bond across two OVS ports |
| `clos-l2-ovs.json` | Redundant leaf-spine OVS fabric with STP |
| `multi-tenant.json` | Three sites and three isolated tenants using overlapping addresses |
| `multi-tenant-fault.json` | Multi-tenant topology with trunk failure and recovery |

The `switch` field selects the switch implementation for `switch` nodes. It defaults to `ovs,failMode=standalone`; use `"switch":"lxbr"` for legacy LinuxBridge commands. New scenarios should use OVS and configure ports with `ovs-vsctl` (VLAN tags/trunks) rather than `bridge vlan`.

For a live read-only terminal, start a run with `--hold-secs 120`, then run
`cargo run --manifest-path example/tui/Cargo.toml -- runs/ospf-001` from a
second terminal. The [monitoring guide](monitoring/README.md) covers the
optional Prometheus, Grafana, Alloy and Loki stack.

The broader validation report, scenario complexity map, research plans, and
real-switch/rootless evidence live under `docs/research/` (built into the
Sphinx docs as the “Research program” section). See `ROADMAP.md` for the next
designs and benchmarks.

## Scenario format

JSON object: `name`, optional `switch`, `nodes`, `links`, and `steps`. Nodes have `name` (1–9 ASCII letters, digits or underscores) and `kind` (`host` or `switch`). Links have endpoint names `a`, `b` and optional integer `port_a`, `port_b`. Host port numbering starts at 0, switch numbering at 1; interface names are `<node>-eth<port>` and must fit the kernel's 15-character limit, so a nine-character node name cannot use a three-digit port. Ports can be explicit; omitted ports use Mininet's allocation order. Duplicate names and ports, missing endpoints, self-links, and unknown fields are rejected.

The scenario name must be 1–64 ASCII letters, digits, underscores or hyphens. Built-in routing presets use FRR pathspaces named `mnr-<scenario>-<node>` under `/var/run/frr`; name your daemon config files `ospfd.conf`, `isisd.conf`, or `bgpd.conf` there to enable the corresponding protocol probes.

Each step has `node`, `command`, optional `timeout_secs` (default 30), optional `retry_secs` (default 0), and optional `expect_stdout` (literal substring). The command is passed to `/bin/sh -c`; for hosts it runs in the node's network namespace, while switch commands run in the container namespace. Zero exit status and, if specified, the stdout match are required. An unsuccessful step is retried until the retry window expires, with one-second intervals; each attempt is bounded by the timeout. A timeout is a failure. Commands share the container filesystem, so use distinct per-router paths for FRR configuration, PID files and zebra/VTY sockets. Redirect daemon output when starting background processes, so they do not keep capture pipes open. FRR and other namespace processes are terminated during teardown; graceful handling of SIGINT and SIGTERM is attempted. SIGKILL, host failure, or container-engine failure cannot be handled by the runner; `--rm` disposes of the container when it exits.

## Results

`provenance.json` records the runner package version, UTC epoch milliseconds, the output of `date -u`, `uname -a`, `/usr/lib/frr/zebra -v`, `mnexec -v`, and `mnr --version`. Each command records stdout, stderr, exit code and an `ok`, `nonzero`, or `unavailable` status; unsupported version flags are not presented as versions. The runner and package versions describe the container image; `uname` reports the shared host kernel and the container's UTS-namespace hostname, not necessarily the host hostname. The clock is the container's view of the host clock.

Source revision and dirty state are null unless supplied explicitly; they are user-provided build provenance, not independently verified. To embed them in the image when building from a Git checkout:

```sh
revision=$(git rev-parse HEAD)
if [ -n "$(git status --porcelain)" ]; then dirty=true; else dirty=false; fi
docker build -f example/Dockerfile -t mininet-research \
  --build-arg SOURCE_REVISION="$revision" --build-arg SOURCE_DIRTY="$dirty" .
```

For a native run, `EXPERIMENT_SOURCE_REVISION` and `EXPERIMENT_SOURCE_DIRTY` supply the same optional fields. Record the actual source context used to build the image, including uncommitted changes; the revision alone cannot identify dirty content.

The runner exits nonzero on invalid scenarios, failed commands, startup or teardown errors. It records a copy of the input in `scenario.json`, append-only lifecycle and per-attempt events in `events.jsonl` (also emitted to stdout), and `steps/NNNN-attempt-MMMM.stdout`, `.stderr`, and `.json` for every attempt. Step indices start at zero and attempt numbers at one; both have at least four digits. Step JSON includes node, command, exit code (`null` when signalled), duration in milliseconds, attempt number, and success. Events contain `timestamp_ms` as Unix epoch milliseconds; `start` includes the scenario name, `step` includes the step index and result, and `finish` includes success and any error. Preserve the complete run directory to reproduce and analyze an experiment. A missing `finish` indicates abrupt interruption or an output failure, not success.

`telemetry.jsonl` contains timestamped per-node command samples: `timestamp_ms`, `node`, `probe`, `command`, `stdout_scope`, `exit_code`, `stdout`, and `stderr`. Host probes run inside the host's network namespace and are recorded raw (`stdout_scope: "raw"`). Switches share the container namespace, so their JSON IP probes are narrowed to the sampled switch's bridge and `<node>-eth*` interfaces (`stdout_scope: "node"`); records without the scoping field (for example blackhole routes) are retained, and non-JSON output falls back to raw. Recognized FRR pathspaces add neighbor and protocol-route probes. A failed probe emits a `telemetry_error` event and retains its output; a probe failure is visible but does not itself fail a scenario. Raw files in each recognized FRR pathspace (including configs and logs) are copied to `frr/` before network teardown. For historical log collection, configure FRR `log file` directives; stdout redirection alone does not capture daemonized logging.
