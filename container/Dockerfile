# mnr-rootless — run the Rust mininet port (mnr) inside a container with NO
# root/sudo on the host. Rootless podman/docker map the container's root into
# a user namespace (subuid/subgid), and `--privileged` then grants
# CAP_NET_ADMIN *inside that namespace only* — enough to create the netns /
# veth pairs mininet needs, without any privilege on the host.
#
#   podman build -t mnr-rootless:latest -f container/Dockerfile .
#   sh scripts/mnr-container.sh --topo single,3 --test pingall
#
# See scripts/mnr-container.sh and docs/migration/DECISIONS.md (D25).

# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS builder
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
        python3 python3-dev pkg-config libssl-dev >/dev/null \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY apps apps
COPY bindings bindings
COPY legacy/mnexec.c ./
RUN cargo build --workspace --release \
    && cc -O2 -Wall -DVERSION='"2.3.1b4"' legacy/mnexec.c -o mnexec

FROM ubuntu:24.04
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
        ca-certificates iproute2 net-tools bridge-utils ethtool iptables \
        iputils-ping openvswitch-switch openvswitch-testcontroller \
        python3 python3-packaging kmod \
    && rm -rf /var/lib/apt/lists/*
# Runtime binaries from the builder stage.
COPY --from=builder /src/target/release/mnr    /usr/local/bin/mnr
COPY --from=builder /src/target/release/mnexec /usr/local/bin/mnexec
COPY --from=builder /src/target/release/mnr-tui /usr/local/bin/mnr-tui
COPY --from=builder /src/target/release/mnr-mcp /usr/local/bin/mnr-mcp
COPY container/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
# `controller` shim for the default controller (same as CI workflows).
RUN printf '#!/bin/sh\nargs=""\nfor a in "$@"; do [ "$a" = "-v" ] && continue; args="$args $a"; done\nexec ovs-testcontroller $args\n' \
        > /usr/local/bin/controller && chmod +x /usr/local/bin/controller
WORKDIR /w
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["mnr"]
