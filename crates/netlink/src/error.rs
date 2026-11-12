//! Error type for the `mn-netlink` crate.

use std::fmt;

/// Errors returned by [`crate::Netlink`] and the namespace helpers.
#[derive(Debug)]
pub enum Error {
    /// Error reported by the rtnetlink request/response exchange.
    Rtnetlink(rtnetlink::Error),
    /// Error from a socket creation or tokio runtime setup.
    Io(std::io::Error),
    /// Error from a direct syscall wrapper (`nix`/`libc`).
    Errno(nix::errno::Errno),
    /// Caller passed invalid input (bad netns name, bad MAC, ...).
    InvalidInput(String),
    /// Any other operational failure (missing link, failed child, ...).
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rtnetlink(e) => write!(f, "netlink error: {e}"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Errno(e) => write!(f, "syscall error: {e}"),
            Error::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Rtnetlink(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Errno(e) => Some(e),
            Error::InvalidInput(_) | Error::Other(_) => None,
        }
    }
}

impl From<rtnetlink::Error> for Error {
    fn from(e: rtnetlink::Error) -> Self {
        Error::Rtnetlink(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<nix::errno::Errno> for Error {
    fn from(e: nix::errno::Errno) -> Self {
        Error::Errno(e)
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::Other(msg)
    }
}

/// Convenient result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_formats_each_variant() {
        let e = Error::InvalidInput("bad name".to_string());
        assert_eq!(format!("{e}"), "invalid input: bad name");
        let e = Error::Errno(nix::errno::Errno::EPERM);
        assert!(
            format!("{e}").contains("EPERM") || format!("{e}").contains("Operation not permitted")
        );
        let e = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        assert!(format!("{e}").contains("io error"));
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<T: std::error::Error>(_: &T) {}
        assert_error(&Error::InvalidInput("x".to_string()));
    }
}
