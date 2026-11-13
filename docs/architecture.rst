Architecture
============

Runtime model (unchanged from Python Mininet): each host is a process in its
own Linux network namespace; links are veth pairs; switches are OVS (or a
kernel Linux bridge with ``--switch lxbr``); TCLink shapes traffic with netem.

What changed in Rust:

* **netlink instead of CLI tools** — namespace/veth/address/route/qdisc/bridge
  operations go through ``rtnetlink`` (``crates/netlink``), not ``ip``/``tc``.
  Only iptables (NAT), sysctl, and the OVS control plane remain subprocesses.
* **``mnexec`` in Rust** — the C helper became ``apps/mnexec`` (``setns`` /
  ``unshare`` / cgroup / ``execvp``, byte-identical usage text).
* **Pure model, explicit lifecycle** — building a topology is side-effect
  free (enables non-root dry-run ``--dump``); ``start()``/``stop()`` own all
  kernel and process operations, with full cleanup on error paths.
* **Fork safety** — readiness-pipe handshake around ``setns``, pre-fork
  allocation, runtime built before ``fork()`` (see ``docs/migration/DECISIONS.md``
  D6).

Testing tiers and the migration ledger live in :doc:`testing` and
:doc:`migration`.
