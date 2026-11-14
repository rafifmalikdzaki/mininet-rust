# Research monitoring (optional)

This stack persists host-level Prometheus samples and Grafana dashboards, and
collects experiment lifecycle events and per-node telemetry records into Loki.
It runs independently of experiments; keep it running between runs. Prometheus
does not provide per-router or per-protocol metrics; those observations remain
in the runner's telemetry records. The complete raw stdout, stderr, command
metadata, and copied scenario remain in each run directory. Treat all artifacts
and logs as potentially sensitive.

## Prerequisites

Use a Linux Docker host with Docker Compose v2, sufficient disk space and access
to the image registries. The research container needs a host kernel supporting
network namespaces, veth, and the selected switch/datapath. It is deliberately
**privileged**: use a disposable isolated machine/VM and trusted scenarios only;
container root has powerful access to the host. Do not mount the Docker socket or
host network into it. Run artifacts must be readable by Alloy. Configure the
same absolute host runs directory for monitoring and the research container.

From the repository root, build the research image and prepare the run directory:

```sh
docker build -f example/Dockerfile -t mininet-research .
mkdir -p "$HOME/mininet-research-runs"
export RESEARCH_RUNS_DIR="$HOME/mininet-research-runs"
# Use a unique, strong secret. Do not commit or share it.
read -rs -p 'Grafana admin password: ' GRAFANA_ADMIN_PASSWORD; echo
export GRAFANA_ADMIN_PASSWORD
docker compose -f example/monitoring/compose.yaml up -d
```

Choose a non-existing run directory name and an existing scenario JSON; mount the
scenario read-only and persist `/runs`:

```sh
docker run --rm --init --privileged --network none \
  -v "$RESEARCH_RUNS_DIR:/runs" \
  -v "$(realpath scenario.json):/scenario.json:ro" \
  mininet-research /scenario.json /runs/run-001
```

See the research runner's own usage for the scenario format and available
presets. Bind-mounted paths use the Docker daemon host's filesystem; remote Docker
daemons need matching paths on that host. Ensure a unique run name for each run.

Open http://127.0.0.1:3000 and sign in as `admin` with the supplied password.
The provisioned **Mininet research overview** dashboard shows CPU, memory,
disk throughput, scrape health, and live run events. Prometheus is available
at http://127.0.0.1:9090. Alloy discovers `/runs/*/events.jsonl` and
`/runs/*/telemetry.jsonl` and tails appends; its `filename` label distinguishes runs.
Loki is reachable only inside
the Compose network. Prometheus scrapes every five seconds. CPU, memory and disk
metrics come from the host's `/proc` and `/sys`; network counters reflect the
exporter's network namespace, not the host or the emulated nodes.
Runner files are the authoritative per-run record, including short runs which may
finish between metric scrapes.

Grafana and Prometheus bind to IPv4 loopback only; Loki and exporters have no
published ports. Local users and processes can still access these interfaces,
and Loki has no authentication inside its Compose network. Never publish these
services on an untrusted network without authentication, TLS and network policy.
The Grafana password is set only on first initialization of its persistent data
volume; changing the environment variable later does not rotate it.

```sh
docker compose -f example/monitoring/compose.yaml logs -f
# Stop services but retain monitoring data and run artifacts:
docker compose -f example/monitoring/compose.yaml down
```

Prometheus retains samples for 30 days. Loki logs and named volumes persist until
manually removed; monitor disk usage and apply an appropriate retention policy
for your environment. `docker compose ... down -v` deletes **all** stack volumes
and its stored metrics, logs, Grafana state and tail positions, but not the bind
mounted run artifacts. On a clean Alloy position volume existing event files
may be ingested again.

Configuration references: [Prometheus configuration](https://prometheus.io/docs/prometheus/latest/configuration/configuration/),
[Grafana provisioning](https://grafana.com/docs/grafana/latest/administration/provisioning/),
[FRR container guidance](https://docs.frrouting.org/projects/dev-guide/en/latest/building-docker.html),
[Alloy file collection](https://grafana.com/docs/alloy/latest/reference/components/loki/loki.source.file/).
