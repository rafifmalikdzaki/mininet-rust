#!/bin/sh
# entrypoint for the mnr-rootless image.
# Starts the OVS daemons when present (for `--switch default/ovs`) and then
# runs mnr — or, if the first arg is not an mnr invocation, execs it as-is
# (so `... image bash` still works for debugging).
set -e

if command -v ovsdb-server >/dev/null 2>&1; then
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
    # Load the OVS kernel datapath when the host kernel provides it; harmless
    # when it does not (rootless containers usually cannot) — in that case
    # pass `--switch lxbr`, which scripts/mnr-container.sh does by default.
    modprobe openvswitch 2>/dev/null || true
fi

case "${1:-}" in
    mnr)
        shift
        exec mnr "$@"
        ;;
    --* | "")
        exec mnr "$@"
        ;;
    *)
        exec "$@"
        ;;
esac
