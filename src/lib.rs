#![doc = include_str!("../README.md")]

pub use error::PrometheusError;
pub use ser::{to_prometheus_text, write_prometheus_text};

mod error;
mod ser;
