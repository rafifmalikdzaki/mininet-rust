# justfile — Rust mininet (`mnr`) developer commands. Run `just` to list.
# Root-gated work runs in a privileged container (no sudo on the host needed
# for your machine beyond docker/podman; see scripts/mnr-container.sh and
# docs/migration/DECISIONS.md D25).

root := `git rev-parse --show-toplevel`
image := "mnr-rootless:latest"
research_image := "mininet-research:latest"

# default recipe: list available commands
default:
    @just --list

# --- build / quality gates -----------------------------------------------------

# build everything (debug)
build:
    cargo build --workspace

# build everything (release, used by the harnesses)
build-release:
    cargo build --workspace --release

# format check + clippy -D warnings (workspace + research monitor)
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --manifest-path example/tui/Cargo.toml -- --check
    cargo clippy --manifest-path example/tui/Cargo.toml --all-targets -- -D warnings

# apply formatting
fmt:
    cargo fmt --all
    cargo fmt --manifest-path example/tui/Cargo.toml

# clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --manifest-path example/tui/Cargo.toml --all-targets -- -D warnings

# non-root test suite (workspace + research monitor)
test:
    cargo test --workspace --no-fail-fast
    cargo test --manifest-path example/tui/Cargo.toml

# one crate or a test-name filter: just test-filter mn-link netem
test-filter what:
    cargo test --workspace --no-fail-fast -- {{what}}

# --- root-gated tests ----------------------------------------------------------
# run the root-gated suites directly (needs a root Linux host)
root-tests-host:
    MN_RUST_ROOT_TESTS=1 cargo test --workspace -- --ignored --test-threads=1

# root-gated suites in a privileged docker container (repeat runs are fast;
# uses a separate Cargo target directory from host builds)
# gcc/libc6-dev provide rustc's linker (cc + crt); no C sources in tree.
root-tests-docker:
    docker run --rm --privileged \
        -v {{root}}:/w -w /w \
        -v mnr-cargo-home:/root/.cargo \
        ubuntu:24.04 bash -c '\
            export DEBIAN_FRONTEND=noninteractive; \
            export CARGO_TARGET_DIR=/root/.cargo/mnr-container-target; \
            apt-get update -qq >/dev/null 2>&1; \
            apt-get install -y -qq --no-install-recommends \
                ca-certificates curl python3 python3-packaging iproute2 \
                net-tools bridge-utils ethtool iptables iputils-ping \
                gcc libc6-dev >/dev/null 2>&1; \
            test -d /root/.cargo/bin || curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable >/dev/null 2>&1; \
            export PATH=$PATH:/root/.cargo/bin; \
            cargo build -p mnexec --no-default-features; \
            install -m 0755 "$CARGO_TARGET_DIR/debug/mnexec" /usr/local/bin/mnexec; \
            MN_RUST_ROOT_TESTS=1 cargo test --workspace --no-fail-fast \
                -- --ignored --test-threads=1'

# --- differential harness + benchmarks ------------------------------------------

# Python-vs-Rust differential dump diff for one topo spec (needs root):
#   just diff single,3
diff spec:
    sudo sh scripts/diff_topology.sh {{spec}}

# benchmark ladder, Python mn vs mnr (needs root; real OVS by default)
bench runs="5":
    sudo env BENCH_RUNS={{runs}} sh scripts/bench_scenarios.sh

# rootless benchmark ladder inside the mnr-rootless image (build/load it first)
bench-rootless runs="3":
    podman run --rm --privileged --network none -v "{{root}}:/w" -w /w \
        -e MN_RUST_BIN=/usr/local/bin/mnr -e BENCH_RUNS={{runs}} \
        mnr-rootless:latest sh scripts/bench_scenarios.sh

# full privileged one-shot (tests + diff ladder + summary)
root-tests:
    sudo sh scripts/run_root_tests.sh

# --- research experiments ------------------------------------------------------

# build the research image (FRRouting + experiment runner)
research-build:
    docker build -f example/Dockerfile -t {{research_image}} .

# run one preset: just research bgp bgp-001
# scenarios: ospf isis bgp vlan multi-tenant multi-tenant-fault
# real-switch variants: vlan-ovs multi-tenant-ovs multi-tenant-fault-ovs
#                      ospf-ovs dual-home-ovs clos-l2-ovs ring-isis clos-ebgp
research scenario="ospf" run="research-run":
    mkdir -p "{{root}}/runs"
    docker run --rm --init --privileged --network none \
        -v "{{root}}/runs:/runs" {{research_image}} "/scenarios/{{scenario}}.json" "/runs/{{run}}"

# rootless research run (image must be built/loaded into Podman)
research-rootless scenario="ospf" run="research-run":
    mkdir -p "{{root}}/runs"
    podman run --rm --init --privileged --network none \
        -v "{{root}}/runs:/runs" {{research_image}} "/scenarios/{{scenario}}.json" "/runs/{{run}}"

# live read-only monitor for a completed or running experiment
research-monitor run:
    cargo run --manifest-path example/tui/Cargo.toml -- {{root}}/runs/{{run}}

# --- containers (rootless, no sudo needed) ---------------------------------------

# build (or rebuild) the mnr-rootless image
container-build:
    sh scripts/mnr-container.sh --build

# run mnr in a rootless container (any mnr args pass through):
#   just container-run --topo single,3 --test pingall
container-run *args:
    sh scripts/mnr-container.sh {{args}}

# shell inside the mnr image (debugging)
container-shell:
    sh scripts/mnr-container.sh sh

# local container smoke test
[private]
container-smoke:
    docker build -t mnr-rootless:ci -f container/Dockerfile .
    MNR_IMAGE=mnr-rootless:ci MNR_RUNTIME=docker sh scripts/mnr-container.sh --topo single,3 --test pingall

# --- docs / cleanup --------------------------------------------------------------

# build the Sphinx docs under docs/_build
docs:
    python3 -m pip install -q -r docs/requirements.txt 2>/dev/null || true
    python3 -m sphinx -b html docs docs/_build/html

# serve the built docs locally
docs-serve:
    python3 -m http.server 8000 -d docs/_build/html

# remove build artifacts (kernel leftovers: run `mnr --clean` instead)
clean:
    cargo clean
