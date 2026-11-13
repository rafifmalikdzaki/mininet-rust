//! Error type for the `mn-link` crate.

use std::fmt;

/// Errors returned by link-spec parsing and by the kernel-facing link ops.
#[derive(Debug)]
pub enum Error {
    /// Link spec / parameter parse error (bad value, unknown link type).
    Parse(String),
    /// Caller passed invalid input (bad interface name, missing interface).
    InvalidInput(String),
    /// Error from the `mn-netlink` attach/detach/address operations.
    Netlink(mn_netlink::Error),
    /// Error from a TrafficControl (qdisc) netlink exchange.
    Tc(String),
    /// Socket/runtime setup failure.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(msg) => write!(f, "link spec parse error: {msg}"),
            Error::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Error::Netlink(e) => write!(f, "netlink error: {e}"),
            Error::Tc(msg) => write!(f, "tc netlink error: {msg}"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Netlink(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Parse(_) | Error::InvalidInput(_) | Error::Tc(_) => None,
        }
    }
}

impl From<mn_netlink::Error> for Error {
    fn from(e: mn_netlink::Error) -> Self {
        Error::Netlink(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
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
            format!("{}", Error::Parse("bad bw".to_string())),
            "link spec parse error: bad bw"
        );
        assert_eq!(
            format!("{}", Error::InvalidInput("no intf".to_string())),
            "invalid input: no intf"
        );
        assert!(format!("{}", Error::Tc("boom".to_string())).contains("tc netlink error: boom"));
        assert!(format!("{}", Error::Io(std::io::Error::other("x"))).contains("io error"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<T: std::error::Error>(_: &T) {}
        assert_error(&Error::Parse("x".to_string()));
    }
}
