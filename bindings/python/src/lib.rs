//! Python bindings for the Rust mininet core (`mnr`).
//!
//! ```python
//! import mininet_rust as mnr
//!
//! net = mnr.Mininet(topo="tree,2,2", controller="none")
//! net.build()
//! print(net.dump())
//! net.start()          # requires root
//! print(net.pingall())
//! net.cmd("h1", "ifconfig")
//! net.stop()
//!
//! print(mnr.plan("single,3"))   # dry-run plan text, no root needed
//! ```
//!
//! Thin layer over `mn-net` / `mn-topo` — all kernel work stays in Rust.

use mn_net::{Mininet as CoreNet, MininetOptions};
use mn_topo::Attrs;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

fn err<T>(r: mn_net::Result<T>) -> PyResult<T> {
    r.map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// A Mininet network backed by the Rust core.
#[pyclass(name = "Mininet")]
pub struct PyMininet {
    inner: CoreNet,
}

#[pymethods]
impl PyMininet {
    /// Mininet(topo="single,3", controller="none", link="default", switch="default",
    ///         ipbase="10.0.0.0/8", nat=False)
    #[new]
    #[pyo3(signature = (topo=None, controller=None, link=None, switch=None, ipbase=None, nat=false))]
    fn new(
        topo: Option<&str>,
        controller: Option<&str>,
        link: Option<&str>,
        switch: Option<&str>,
        ipbase: Option<&str>,
        nat: bool,
    ) -> PyResult<Self> {
        let mut opts = MininetOptions::default();
        if let Some(t) = topo {
            let built = mn_topo::build_topo(t)
                .map_err(|e| PyRuntimeError::new_err(format!("could not build topo {t:?}: {e}")))?;
            opts.topo_class = Some(built.class_name().to_string());
            opts.topo = Some(built.topo().clone());
            opts.topo_spec = Some(t.to_string());
        }
        if let Some(c) = controller {
            opts.controller = Some(if c == "none" {
                mn_net::ControllerKind::None
            } else {
                mn_net::ControllerKind::Default
            });
        }
        if let Some(l) = link {
            opts.link_spec = l.to_string();
        }
        if let Some(s) = switch {
            opts.switch_spec = s.to_string();
        }
        if let Some(ip) = ipbase {
            opts.ip_base = ip.to_string();
        }
        let mut inner = CoreNet::new(opts).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        inner
            .build()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        if nat {
            err(inner.add_nat("nat0", None, false, &Attrs::new()))?;
        }
        Ok(Self { inner })
    }

    /// Build/rebuild from the topo (idempotent).
    fn build(&mut self) -> PyResult<()> {
        err(self.inner.build())
    }

    /// Start the network (needs root).
    fn start(&mut self) -> PyResult<()> {
        err(self.inner.start())
    }

    /// Stop the network and clean up.
    fn stop(&mut self) -> PyResult<()> {
        err(self.inner.stop())
    }

    /// cli.py-format dump of all nodes.
    fn dump(&self) -> String {
        self.inner.dump()
    }

    /// cli.py-format links.
    fn links(&self) -> String {
        self.inner.links()
    }

    /// The dry-run plan as a JSON string (nodes/edges; no root needed).
    fn plan_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(&serde_json::json!({
            "topo": self.inner.topo_spec,
            "class": self.inner.topo_class,
        }))
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Host names (insertion order).
    fn hosts(&self) -> Vec<String> {
        self.inner.hosts.keys().map(str::to_string).collect()
    }

    /// Switch names.
    fn switches(&self) -> Vec<String> {
        self.inner.switches.keys().map(str::to_string).collect()
    }

    /// pingall() -> (loss_percent, output) like net.py's PingResult fields.
    fn pingall(&mut self) -> PyResult<(f64, String)> {
        let r = err(self.inner.ping_all(None))?;
        Ok((r.ploss, r.output))
    }

    /// ping(src, dst) -> (loss_percent, output).
    fn ping(&mut self, src: &str, dst: &str) -> PyResult<(f64, String)> {
        let r = err(self.inner.ping(Some(vec![src, dst]), None))?;
        Ok((r.ploss, r.output))
    }

    /// Run a command on a host, return its merged output (like Node.cmd).
    fn cmd(&mut self, host: &str, cmd: &str) -> PyResult<String> {
        let h = self
            .inner
            .host_mut(host)
            .ok_or_else(|| PyRuntimeError::new_err(format!("node {host:?} not in network")))?;
        h.cmd(cmd)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
}

/// plan(spec, **opts) -> dry-run plan text (same sections as `mnr --dump`;
/// no root needed).
#[pyfunction]
#[pyo3(signature = (spec, **_opts))]
fn plan(spec: &str, _opts: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<String> {
    let net = CoreNet::dry_run(spec, MininetOptions::default())
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(net.plan())
}

/// version() -> the mnr version string.
#[pyfunction]
fn version() -> String {
    "2.3.1b4 (rust)".to_string()
}

/// Module: mininet_rust
#[pymodule]
fn mininet_rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMininet>()?;
    m.add_function(wrap_pyfunction!(plan, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}
