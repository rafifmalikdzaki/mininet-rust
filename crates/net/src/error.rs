//! Error type for the `mn-net` crate.

use std::fmt;

/// Errors returned by [`crate::Mininet`] construction, build, start/stop,
/// lookups, and observability helpers.
#[derive(Debug)]
pub enum Error {
    /// Caller passed invalid input (unknown node name, bad IP base, bad
    /// port number, duplicate node, ...). Python raises `KeyError` /
    /// `ValueError` / `Exception` here.
    InvalidInput(String),
    /// Error from the `mn-topo` crate (`buildTopo`, topo model).
    Topo(mn_topo::TopoError),
    /// Error from the `mn-node` crate (node lifecycle, cmd, netlink ops).
    Node(mn_node::Error),
    /// Error from the `mn-link` crate (link model, attach/delete, tc).
    Link(mn_link::Error),
    /// Error from the `mn-netlink` crate (netns/veth/addr/route ops).
    Netlink(mn_netlink::Error),
    /// A pure `mn-util` helper rejected its input (bad `ipBase` string,
    /// unparseable MAC, ...); Python raises `ValueError`.
    Util(String),
    /// Any other operational failure.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Error::Topo(e) => write!(f, "topo error: {e}"),
            Error::Node(e) => write!(f, "node error: {e}"),
            Error::Link(e) => write!(f, "link error: {e}"),
            Error::Netlink(e) => write!(f, "netlink error: {e}"),
            Error::Util(msg) => write!(f, "util error: {msg}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Topo(e) => Some(e),
            Error::Node(e) => Some(e),
            Error::Link(e) => Some(e),
            Error::Netlink(e) => Some(e),
            Error::InvalidInput(_) | Error::Util(_) | Error::Other(_) => None,
        }
    }
}

impl From<mn_topo::TopoError> for Error {
    fn from(e: mn_topo::TopoError) -> Self {
        Error::Topo(e)
    }
}

impl From<mn_node::Error> for Error {
    fn from(e: mn_node::Error) -> Self {
        Error::Node(e)
    }
}

impl From<mn_link::Error> for Error {
    fn from(e: mn_link::Error) -> Self {
        Error::Link(e)
    }
}

impl From<mn_netlink::Error> for Error {
    fn from(e: mn_netlink::Error) -> Self {
        Error::Netlink(e)
    }
}

/// Convenient result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_formats_each_variant() {
        assert_eq!(
            format!("{}", Error::InvalidInput("no such node".to_string())),
            "invalid input: no such node"
        );
        assert!(format!("{}", Error::Util("bad ipBase".to_string())).contains("util error"));
        assert!(format!("{}", Error::Other("boom".to_string())).contains("boom"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<T: std::error::Error>(_: &T) {}
        assert_error(&Error::InvalidInput("x".to_string()));
    }
}
