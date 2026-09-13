use std::fmt;

#[derive(Debug)]
pub enum Error {
    Db(rusqlite::Error),
    Io(std::io::Error),
    Usage(String),
    NotFound(String),
    NotInitialized,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(e) => write!(f, "database error: {}", e),
            Error::Io(e) => write!(f, "io error: {}", e),
            Error::Usage(msg) => write!(f, "{}", msg),
            Error::NotFound(key) => write!(f, "key not found: {}", key),
            Error::NotInitialized => write!(
                f,
                "agent-mem is not initialized. Run 'agent-mem init' first."
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Db(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Usage(_) | Error::NotFound(_) | Error::NotInitialized => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
