use std::{fmt::Display, io};

use thiserror::Error;

/// Error type for Prometheus serialization.
#[derive(Error, Debug)]
pub enum PrometheusError {
    /// Error when writing to output.
    #[error("failed to write to output: {0}")]
    Write(#[from] io::Error),
    /// Error when serializing.
    #[error("serde internal error: {0}")]
    Custom(String),
}

impl serde::ser::Error for PrometheusError {
    fn custom<T>(msg: T) -> Self
    where
        T: Display,
    {
        Self::Custom(msg.to_string())
    }
}
