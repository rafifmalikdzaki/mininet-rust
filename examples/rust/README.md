# Rust-backed examples (run on the `mnr` core via the `mininet_rust` Python bindings)

These mirror the classic `examples/*.py`, but drive the **Rust** mininet core
through the PyO3 bindings (`bindings/python`), so Python code gets the
netlink-based, differentially-verified engine:

```console
$ pip install maturin && maturin develop -m bindings/python/Cargo.toml   # build module
$ python3 examples/rust/emptynet.py            # dry-run plan, no root
$ sudo python3 examples/rust/emptynet.py       # real emulation
```

| file | what it shows |
|------|---------------|
| `emptynet.py` | build single,3; dump/links; pingall when run as root |
| `external_nat.py` | **external connections**: NAT node, default routes, egress ping |
| `cybersec_testbed.py` | **cybersecurity testbed**: attacker/victims/defender + NAT; recon → probe → detection steps (`--plan` for no-root preview) |
| `simpleperf.py` | shaped link (`tc,bw=100,delay=1ms`) + iperf/ping between hosts |

The original Python examples in `examples/` keep working against the Python
mininet package — both can coexist; the Rust CLI binary is `mnr`.
