//! Error type for the `mn-node` crate.

use std::fmt;
use std::time::Duration;

/// Errors returned by node lifecycle, command execution, and interface
/// configuration.
#[derive(Debug)]
pub enum Error {
    /// Caller passed invalid input (bad node/netns/intf name, bad IP/MAC,
    /// no default interface, ...).
    InvalidInput(String),
    /// Socket/pipe/process setup failure.
    Io(std::io::Error),
    /// Error from an `mn-netlink` operation (netns, veth, addr, route).
    Netlink(mn_netlink::Error),
    /// Error from the `mn-link` model.
    Link(mn_link::Error),
    /// Operation requires a running shell/process, but the node is not
    /// started (Python warns "(node exited - ignoring cmd)" and returns
    /// `None`; here it is an explicit error for [`crate::Node::cmd`]).
    NotRunning(String),
    /// [`crate::Node::wait_output`] exceeded its timeout.
    TimedOut {
        /// The command that was waited on.
        cmd: String,
        /// The timeout that elapsed.
        timeout: Duration,
    },
    /// A subprocess could not be spawned.
    Spawn {
        /// The command line that failed to spawn.
        cmd: String,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// Any other operational failure.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Netlink(e) => write!(f, "netlink error: {e}"),
            Error::Link(e) => write!(f, "link error: {e}"),
            Error::NotRunning(name) => {
                write!(
                    f,
                    "node {name:?} is not running (shell exited or never started)"
                )
            }
            Error::TimedOut { cmd, timeout } => {
                write!(
                    f,
                    "timed out after {timeout:?} waiting for output of {cmd:?}"
                )
            }
            Error::Spawn { cmd, source } => write!(f, "failed to spawn {cmd:?}: {source}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Netlink(e) => Some(e),
            Error::Link(e) => Some(e),
            Error::Spawn { source, .. } => Some(source),
            Error::InvalidInput(_) | Error::NotRunning(_) | Error::TimedOut { .. } => None,
            Error::Other(_) => None,
        }
    }
}

impl From<mn_netlink::Error> for Error {
    fn from(e: mn_netlink::Error) -> Self {
        Error::Netlink(e)
    }
}

impl From<mn_link::Error> for Error {
    fn from(e: mn_link::Error) -> Self {
        Error::Link(e)
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
            format!("{}", Error::InvalidInput("bad name".to_string())),
            "invalid input: bad name"
        );
        assert!(format!("{}", Error::NotRunning("h1".to_string())).contains("h1"));
        assert!(format!(
            "{}",
            Error::TimedOut {
                cmd: "sleep 5".to_string(),
                timeout: Duration::from_millis(100)
            }
        )
        .contains("timed out"));
        assert!(format!(
            "{}",
            Error::Spawn {
                cmd: "nope".to_string(),
                source: std::io::Error::other("gone")
            }
        )
        .contains("failed to spawn"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<T: std::error::Error>(_: &T) {}
        assert_error(&Error::InvalidInput("x".to_string()));
    }
}
