#!/bin/sh
# Research-image entrypoint: start Open vSwitch, then run the experiment.
#
# OVS is started before the runner so scenarios with `"switch": "ovs,..."`
# work with controller-less standalone forwarding. The daemons live inside
# the container's network namespace. In rootless podman the container's root
# is mapped into the user namespace; OVS still gets CAP_NET_ADMIN for that
# namespace. `modprobe` is best-effort: the host may already have the kernel
# module (or the container may not be allowed to load it).
set -e

if command -v ovs-vswitchd >/dev/null 2>&1; then
    mkdir -p /etc/openvswitch /var/run/openvswitch /var/log/openvswitch
    if [ ! -f /etc/openvswitch/conf.db ]; then
        ovsdb-tool create /etc/openvswitch/conf.db \
            /usr/share/openvswitch/vswitch.ovsschema
    fi
    if ! ovs-vsctl show >/dev/null 2>&1; then
        ovsdb-server --remote=punix:/var/run/openvswitch/db.sock \
            --pidfile=/var/run/openvswitch/ovsdb-server.pid --detach \
            --log-file=/var/log/openvswitch/ovsdb-server.log
        ovs-vswitchd --pidfile=/var/run/openvswitch/ovs-vswitchd.pid --detach \
            --log-file=/var/log/openvswitch/ovs-vswitchd.log
    fi
    modprobe openvswitch 2>/dev/null || true
fi

exec /usr/local/bin/experiment-runner "$@"
