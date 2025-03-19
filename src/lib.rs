#![doc = include_str!("../README.md")]
#![allow(clippy::doc_markdown)]
#![allow(clippy::implicit_hasher)]
pub use error::PrometheusError;
pub use ser::{to_prometheus_text, write_prometheus_text};

mod error;
mod ser;
