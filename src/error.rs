//!

use log::SetLoggerError;
use serde_json::Error as SerdeJsonError;

pub type ReplResult<T> = std::result::Result<T, ReplError>;

#[derive(Debug, displaydoc::Display, derive_more::From)]
pub enum ReplError {
    /// I/O error: {0}
    IoError(std::io::Error),
    /// Camino path conversion error: {0}
    Camino(camino::FromPathBufError),
    /// std::fmt::Error
    FmtErrror(std::fmt::Error),
    /// SetLoggerError: {0}
    SetLoggerError(SetLoggerError),
    /// SerdeJsonError: {0}
    SerdeJson(SerdeJsonError),
}
