#![doc = include_str!("../README.md")]
#![allow(clippy::doc_markdown)]
#![allow(clippy::implicit_hasher)]
pub use error::PrometheusError;
pub use ser::{
    MetricDescriptor, MetricType, PrometheusSerializer, to_prometheus_text, write_prometheus_text,
};

mod error;
mod ser;
#[cfg(test)]
mod tests;
