# Research monitor

A read-only local terminal view of an experiment run. It never starts or stops a
run and does not invoke the privileged MCP command.

From the repository root, build the research image and start a run in another
terminal (the run directory must not already exist):

```sh
docker build -f example/Dockerfile -t mininet-research .
mkdir -p runs
docker run --rm --init --privileged --network none -v "$PWD/runs:/runs" mininet-research /scenarios/ospf.json /runs/ospf-run-001 --hold-secs 120
```

In a second terminal, with Rust installed:

```sh
cargo run --manifest-path example/tui/Cargo.toml -- runs/ospf-run-001
```

To include network receive byte counters from the monitoring stack's local
Prometheus, add `--prometheus 127.0.0.1:9090`. Prometheus is optional; without it
the display reports disconnected metrics. These are counters from the exporter's
network namespace, not per-emulated-node traffic. Prometheus instant-query timestamps are
evaluation times, so sample freshness is unknown.

The monitor reads `scenario.json`, `events.jsonl`, `telemetry.jsonl`, and
`steps/NNNN-attempt-NNNN.*` directly and refreshes every two seconds. The Nodes view shows the
latest recorded per-node interface counters and routing protocol probe output,
including command exit status and errors. The Logs view also shows the latest
probes for the selected step's node; those probes may be from a later attempt.
Probes are sampled after startup and step attempts, and approximately every
second during the optional hold (a probe cycle can take longer). Samples older
than 30 seconds are marked stale. Protocol probes are present only where the
runner has corresponding daemon configuration. `--hold-secs` keeps the network
available for observation after the steps; omit it to stop after the steps.

A start without a finish is not proof that the runner is alive; a quiet event
file older than 30 seconds is marked stale/status unknown, even if a long step
is still running. Missing, unreadable, malformed, or disconnected sources are
displayed rather than replaced with values. Step files and the event stream
preserve each attempt; step indices are zero-based and attempts are one-based.
The Logs view also shows copied FRR logs for the selected node, when available
after teardown. Event and probe timestamps are epoch milliseconds.

Refreshes read only appended bytes after the first load; the whole event and
telemetry files are read again only at startup or if a file shrinks. On a
1.8 MB multi-tenant run this measured about 60 ms for the initial load,
0.2 ms for a no-change refresh, and 0.5 ms to append 100 telemetry lines on the
development host.

Keys: `1`–`4` or Tab/Shift-Tab change views; `↑`/`↓` or `j`/`k` scroll;
Page Up/Down scroll faster; `←`/`→` or `h`/`l` select an attempt in Logs;
`r` refreshes, `p` pauses/resumes, `?` opens help, and `q` quits. Resize the
terminal at any time. Quitting leaves the experiment running.
