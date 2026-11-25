use std::fmt;

#[derive(Debug)]
pub enum Error {
    Usage(String),
    NotFound(String),
    /// A download or registry fetch failed (network, TLS, server error).
    Network(String),
    /// A downloaded file did not match its checksum.
    Verify(String),
    Io(String),
    Config(String),
    NeedsConfirmation(String),
    Aborted,
    Partial(String),
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// 0 ok, 1 failure, 2 usage, 3 not found, 5 confirmation required, 6 cancelled.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::NotFound(_) => 3,
            Error::NeedsConfirmation(_) => 5,
            Error::Aborted => 6,
            _ => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m)
            | Error::NotFound(m)
            | Error::Network(m)
            | Error::Verify(m)
            | Error::Io(m)
            | Error::Config(m)
            | Error::NeedsConfirmation(m)
            | Error::Partial(m)
            | Error::Failed(m) => f.write_str(m),
            Error::Aborted => f.write_str("Cancelled."),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}
