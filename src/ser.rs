use super::error::PrometheusError;

use indexmap::IndexMap;
use serde::Serialize;
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::io;
use strum_macros::{AsRefStr, Display as DisplayStr, EnumString};

/// Metric type (counter, gauge, histogram, summary, etc.)
#[derive(Debug, Clone, Copy, EnumString, AsRefStr, DisplayStr, Default, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum MetricType {
    /// Untyped metric (default)
    #[default]
    Untyped,
    /// Counter metric
    Counter,
    /// Gauge metric
    Gauge,
    /// Histogram metric
    Histogram,
    /// Summary metric
    Summary,
}

/// Metadata for each metric, including type, help text, and optional custom labels.
#[derive(Debug, Default)]
pub struct MetricDescriptor<'s> {
    /// e.g., "counter", "gauge", "histogram", "summary", etc.
    pub metric_type: MetricType,
    /// # HELP text
    pub help: &'s str,
    /// Static labels for this metric (key-value pairs)
    pub labels: Vec<(&'s str, &'s str)>,
    /// Optional custom name for the metric
    pub rename: Option<&'s str>,
}

#[derive(Debug)]
struct MetricFamily {
    header: String,
    samples: IndexMap<String, String>,
}

/// A custom serializer that flattens structs into Prometheus metrics.
pub struct PrometheusSerializer<'s, W>
where
    W: io::Write,
{
    /// The output buffer where we accumulate Prometheus text.
    output: W,
    /// Current prefix (path) being processed. Nested fields append `_field_name`.
    current_prefix: String,
    /// Metric metadata (help, type, labels) keyed by metric name.
    metadata: &'s HashMap<&'s str, MetricDescriptor<'s>>,
    /// Default descriptor for metrics without explicit metadata.
    default_desc: MetricDescriptor<'s>,
    /// Optional namespace to prefix all metric names.
    namespace: Option<String>,
    /// Common labels to apply to all metrics.
    common_labels: Vec<(&'s str, &'s str)>,
    /// Stores metric families keyed by metric name.
    families: IndexMap<String, MetricFamily>,
}

impl<'s, W> PrometheusSerializer<'s, W>
where
    W: io::Write,
{
    /// Create a new serializer.
    pub fn new<L, Li>(
        output: W,
        namespace: Option<impl Into<String>>,
        metadata: &'s HashMap<&'s str, MetricDescriptor>,
        common_labels: L,
    ) -> Self
    where
        L: IntoIterator<Item = Li>,
        Li: Borrow<(&'s str, &'s str)>,
    {
        PrometheusSerializer {
            output,
            current_prefix: String::new(),
            metadata,
            default_desc: MetricDescriptor::default(),
            namespace: namespace.map(Into::into),
            common_labels: common_labels
                .into_iter()
                .map(|el| {
                    let (k, v) = el.borrow();
                    (*k, *v)
                })
                .collect(),
            families: IndexMap::new(),
        }
    }

    /// Finalizes the serializer by concatenating all buffered metric families.
    ///
    /// # Errors
    /// Returns a `PrometheusError` if writing to the output stream fails.
    pub fn finish(mut self) -> Result<W, PrometheusError> {
        let mut seen = false;
        for (_, family) in self.families {
            if seen {
                self.output.write_all(b"\n")?;
            }
            self.output.write_all(family.header.as_bytes())?;
            self.output.write_all(b"\n")?;
            for (key, value) in family.samples {
                self.output.write_all(key.as_bytes())?;
                self.output.write_all(b" ")?;
                self.output.write_all(value.as_bytes())?;
                self.output.write_all(b"\n")?;
            }
            seen = true;
        }
        Ok(self.output)
    }

    /// Utility to escape label values by replacing `\"` and `\\`.
    fn escape_label_value(val: &str) -> String {
        // minimal escaping for quotes and backslashes
        let mut escaped = String::with_capacity(val.len());
        for c in val.chars() {
            match c {
                '\\' => escaped.push_str("\\\\"),
                '"' => escaped.push_str("\\\""),
                '\n' => escaped.push_str("\\n"),
                _ => escaped.push(c),
            }
        }
        escaped
    }

    fn sample_key(&self, metric_name: &str, desc: &MetricDescriptor<'_>) -> String {
        let mut sample_line = metric_name.to_string();
        if !desc.labels.is_empty() || !self.common_labels.is_empty() {
            sample_line.push('{');
            for (i, (k, v)) in self
                .common_labels
                .iter()
                .chain(desc.labels.iter())
                .enumerate()
            {
                if i > 0 {
                    sample_line.push(',');
                }
                sample_line.push_str(k);
                sample_line.push_str("=\"");
                sample_line.push_str(&Self::escape_label_value(v));
                sample_line.push('"');
            }
            sample_line.push('}');
        }
        sample_line
    }

    /// Writes a metric line for the current prefix with the given numeric value.
    fn write_metric(&mut self, value: &str) {
        let metric_name = &self.current_prefix;
        let full_metric_name = if let Some(ns) = &self.namespace {
            format!("{ns}_{metric_name}")
        } else {
            metric_name.clone()
        };
        let desc = self.metadata.get(metric_name.as_str()).unwrap_or_else(|| {
            self.namespace
                .as_ref()
                .and_then(|_| self.metadata.get(full_metric_name.as_str()))
                .unwrap_or(&self.default_desc)
        });
        let metric_name = if let Some(rename) = desc.rename {
            rename
        } else {
            full_metric_name.as_str()
        };

        let sample_key = self.sample_key(metric_name, desc);

        let family = self
            .families
            .entry(metric_name.to_string())
            .or_insert_with(|| {
                let mut header = String::new();
                if !desc.help.is_empty() {
                    header.push_str("# HELP ");
                    header.push_str(metric_name);
                    header.push(' ');
                    header.push_str(desc.help);
                    header.push('\n');
                }
                header.push_str("# TYPE ");
                header.push_str(metric_name);
                header.push(' ');
                header.push_str(desc.metric_type.as_ref());
                MetricFamily {
                    header,
                    samples: IndexMap::new(),
                }
            });

        family.samples.insert(sample_key, value.to_owned());
    }
}

/// Primary helper to convert a `T: Serialize` into a Prometheus text string.
///
/// # Errors
/// Returns a `PrometheusError` if serialization fails.
///
pub fn to_prometheus_text<'s, T, L, Li>(
    value: &T,
    namespace: Option<&'s str>,
    metadata: &'s HashMap<&'s str, MetricDescriptor>,
    common_labels: L,
) -> Result<String, PrometheusError>
where
    T: ?Sized + Serialize,
    L: IntoIterator<Item = Li>,
    Li: Borrow<(&'s str, &'s str)>,
{
    let mut buf = Vec::new();
    let mut serializer = PrometheusSerializer::new(&mut buf, namespace, metadata, common_labels);
    value.serialize(&mut serializer)?;
    serializer.finish()?;
    String::from_utf8(buf).map_err(|e| PrometheusError::Custom(e.to_string()))
}

/// Primary helper to write a `T: Serialize` into a n output stream as Prometheus text.
/// This is useful for writing directly to a file or network stream.
///
/// # Errors
/// Returns a `PrometheusError` if serialization fails.
///
pub fn write_prometheus_text<'s, T, W, L, Li>(
    value: &T,
    writer: &mut W,
    namespace: Option<&'s str>,
    metadata: &'s HashMap<&'static str, MetricDescriptor>,
    common_labels: L,
) -> Result<(), PrometheusError>
where
    T: ?Sized + Serialize,
    W: io::Write,
    L: IntoIterator<Item = Li>,
    Li: Borrow<(&'s str, &'s str)>,
{
    let mut serializer = PrometheusSerializer::new(writer, namespace, metadata, common_labels);
    value.serialize(&mut serializer)?;
    serializer.finish()?;
    Ok(())
}

impl<W> Serializer for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    // We only care about struct-like serialization. We'll skip sequences, maps, etc.
    // or handle them similarly if needed. For completeness, we can still implement them.

    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        if v {
            self.write_metric("1");
        } else {
            self.write_metric("0");
        }
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string());
        Ok(())
    }

    fn serialize_char(self, _v: char) -> Result<Self::Ok, Self::Error> {
        // Not a numeric metric. Skip.
        Ok(())
    }

    fn serialize_str(self, _v: &str) -> Result<Self::Ok, Self::Error> {
        // We don't export strings as metrics. Skip.
        Ok(())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
        // Not numeric. Skip.
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        // No metric for None
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        // unit = no data
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        // We can skip this, or treat the variant as a label
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        // skip
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        // skip
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        // skip
        Ok(self)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(self)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(self)
    }
}

impl<W> SerializeSeq for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut **self)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeTuple for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, _value: &T) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeTupleStruct for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, _value: &T) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeTupleVariant for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, _value: &T) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeMap for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, _key: &T) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, _value: &T) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeStruct for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        field_name: &'static str,
        value: &T,
    ) -> Result<(), PrometheusError> {
        let old_prefix = self.current_prefix.clone();
        if !self.current_prefix.is_empty() {
            self.current_prefix.push('_');
        }
        self.current_prefix.push_str(field_name);
        value.serialize(&mut **self)?;
        self.current_prefix = old_prefix;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<W> SerializeStructVariant for &mut PrometheusSerializer<'_, W>
where
    W: io::Write,
{
    type Ok = ();
    type Error = PrometheusError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _field: &'static str,
        _value: &T,
    ) -> Result<(), PrometheusError> {
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}
