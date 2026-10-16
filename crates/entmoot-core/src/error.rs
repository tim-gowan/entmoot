//! Typed errors for entmoot-core hashing and scan paths.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

impl From<&str> for CoreError {
    fn from(value: &str) -> Self {
        CoreError::Message(value.to_string())
    }
}

impl From<String> for CoreError {
    fn from(value: String) -> Self {
        CoreError::Message(value)
    }
}

impl From<Box<dyn std::error::Error>> for CoreError {
    fn from(value: Box<dyn std::error::Error>) -> Self {
        CoreError::Message(value.to_string())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for CoreError {
    fn from(value: Box<dyn std::error::Error + Send + Sync>) -> Self {
        CoreError::Message(value.to_string())
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
