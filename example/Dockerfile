FROM rust:1-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends python3-dev pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY apps apps
COPY bindings bindings
COPY example example
RUN cargo build --release -p experiment-runner -p mnexec -p mnr

FROM debian:bookworm
ARG SOURCE_REVISION
ARG SOURCE_DIRTY
ENV EXPERIMENT_SOURCE_REVISION=${SOURCE_REVISION} EXPERIMENT_SOURCE_DIRTY=${SOURCE_DIRTY}
RUN apt-get update && apt-get install -y --no-install-recommends \
    frr iproute2 iputils-ping iperf3 tcpdump procps coreutils util-linux \
    openvswitch-switch ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/experiment-runner /usr/local/bin/
COPY --from=builder /src/target/release/mnexec /usr/local/bin/
COPY --from=builder /src/target/release/mnr /usr/local/bin/
COPY example/scenarios /scenarios
COPY example/entrypoint.sh /usr/local/bin/mininet-research-entrypoint.sh
RUN chmod +x /usr/local/bin/mininet-research-entrypoint.sh
WORKDIR /runs
ENTRYPOINT ["/usr/local/bin/mininet-research-entrypoint.sh"]
