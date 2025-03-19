use super::error::PrometheusError;

use serde::Serialize;
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use std::collections::{HashMap, HashSet};
use std::io;
use strum_macros::{AsRefStr, Display as DisplayStr, EnumString};

#[derive(Debug, Clone, Copy, EnumString, AsRefStr, DisplayStr, Default, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum MetricType {
    #[default]
    Untyped,
    Counter,
    Gauge,
    Histogram,
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

/// A custom serializer that flattens structs into Prometheus metrics.
pub struct PrometheusSerializer<'s, W: io::Write> {
    /// The output buffer where we accumulate Prometheus text.
    output: W,
    /// Current prefix (path) being processed. Nested fields append `_field_name`.
    current_prefix: String,
    /// Metric metadata (help, type, labels) keyed by metric name.
    metadata: &'s HashMap<&'s str, MetricDescriptor<'s>>,
    /// A set of metric names we've already written # HELP / # TYPE for.
    seen_metrics: HashSet<String>,
    /// Default descriptor for metrics without explicit metadata.
    default_desc: MetricDescriptor<'s>,
    /// Optional namespace to prefix all metric names.
    namespace: Option<&'s str>,
}

impl<'s, W> PrometheusSerializer<'s, W>
where
    W: io::Write,
{
    /// Create a new serializer.
    pub fn new(
        output: W,
        namespace: Option<&'s str>,
        metadata: &'s HashMap<&'s str, MetricDescriptor>,
    ) -> Self {
        PrometheusSerializer {
            output,
            current_prefix: String::new(),
            metadata,
            seen_metrics: HashSet::new(),
            default_desc: MetricDescriptor::default(),
            namespace,
        }
    }

    /// Utility to escape label values by replacing `\"` and `\\`.
    fn escape_label_value(val: &str) -> String {
        // minimal escaping for quotes and backslashes
        let mut escaped = String::with_capacity(val.len());
        for c in val.chars() {
            match c {
                '\\' => escaped.push_str("\\\\"),
                '\"' => escaped.push_str("\\\""),
                _ => escaped.push(c),
            }
        }
        escaped
    }

    /// Writes a metric line for the current prefix with the given numeric value.
    fn write_metric(&mut self, value: &str) -> Result<(), PrometheusError> {
        let metric_name = &self.current_prefix;
        let full_metric_name = if let Some(ns) = self.namespace {
            format!("{ns}_{metric_name}")
        } else {
            metric_name.clone()
        };
        let desc = self.metadata.get(metric_name.as_str()).unwrap_or_else(|| {
            self.namespace
                .and_then(|_| self.metadata.get(full_metric_name.as_str()))
                .unwrap_or(&self.default_desc)
        });

        // Only write # HELP / # TYPE once per metric name
        if !self.seen_metrics.contains(metric_name) {
            // # HELP <metric_name> <help text>
            self.output.write_all(b"# HELP ")?;
            self.output.write_all(full_metric_name.as_bytes())?;
            self.output.write_all(b" ")?;
            if desc.help.is_empty() {
                self.output.write_all(b"?")?;
            } else {
                self.output.write_all(desc.help.as_bytes())?;
            }
            self.output.write_all(b"\n")?;
            // # TYPE <metric_name> <type>
            self.output.write_all(b"# TYPE ")?;
            self.output.write_all(full_metric_name.as_bytes())?;
            self.output.write_all(b" ")?;
            self.output
                .write_all(desc.metric_type.as_ref().as_bytes())?;
            self.output.write_all(b"\n")?;
            self.seen_metrics.insert(metric_name.clone());
        }

        // metric_name{labels} value
        self.output.write_all(full_metric_name.as_bytes())?;
        if !desc.labels.is_empty() {
            self.output.write_all(b"{")?;
            for (i, (k, v)) in desc.labels.iter().enumerate() {
                if i > 0 {
                    self.output.write_all(b",")?;
                }
                self.output.write_all(k.as_bytes())?;
                self.output.write_all(b"=\"")?;
                self.output
                    .write_all(Self::escape_label_value(v).as_bytes())?;
                self.output.write_all(b"\"")?;
            }
            self.output.write_all(b"}")?;
        }

        // Write the value
        self.output.write_all(b" ")?;
        self.output.write_all(value.as_bytes())?;
        self.output.write_all(b"\n")?;
        Ok(())
    }
}

/// Primary helper to convert a `T: Serialize` into a Prometheus text string.
///
/// # Errors
/// Returns a `PrometheusError` if serialization fails.
///
pub fn to_prometheus_text<T: Serialize>(
    value: &T,
    namespace: Option<&'_ str>,
    metadata: &HashMap<&'_ str, MetricDescriptor>,
) -> Result<String, PrometheusError> {
    let mut buf = Vec::new();
    let mut serializer = PrometheusSerializer::new(&mut buf, namespace, metadata);
    value.serialize(&mut serializer)?;
    String::from_utf8(buf).map_err(|e| PrometheusError::Custom(e.to_string()))
}

/// Primary helper to write a `T: Serialize` into a n output stream as Prometheus text.
/// This is useful for writing directly to a file or network stream.
///
/// # Errors
/// Returns a `PrometheusError` if serialization fails.
///
pub fn write_prometheus_text<T: Serialize, W: io::Write>(
    value: &T,
    writer: &mut W,
    namespace: Option<&'_ str>,
    metadata: &HashMap<&'static str, MetricDescriptor>,
) -> Result<(), PrometheusError> {
    let mut serializer = PrometheusSerializer::new(writer, namespace, metadata);
    value.serialize(&mut serializer)?;
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
            self.write_metric("1")
        } else {
            self.write_metric("0")
        }
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.write_metric(&v.to_string())
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

    fn serialize_element<T: ?Sized + Serialize>(&mut self, _value: &T) -> Result<(), Self::Error> {
        // skip
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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    #[test]
    fn serialize_nested() {
        #[derive(Serialize)]
        struct Inner {
            value: f64,
            threshold: u32,
            unknown: u32,
            note: String,
        }

        #[derive(Serialize)]
        struct Outer {
            requests: u64,
            errors: u64,
            status: String,
            inner: Inner,
        }

        let my_metrics = Outer {
            requests: 1024,
            errors: 4,
            status: "OK".to_string(),
            inner: Inner {
                value: 3.42,
                threshold: 100,
                unknown: 55,
                note: "important".to_string(),
            },
        };

        let mut meta = HashMap::new();
        meta.insert(
            "requests",
            MetricDescriptor {
                metric_type: MetricType::Counter,
                help: "Total number of requests processed",
                labels: vec![],
                rename: None,
            },
        );
        meta.insert(
            "my_errors",
            MetricDescriptor {
                metric_type: MetricType::Counter,
                help: "Total number of errors",
                labels: vec![("endpoint", "login")],
                rename: None,
            },
        );
        meta.insert(
            "inner_value",
            MetricDescriptor {
                metric_type: MetricType::Gauge,
                help: "Current value from inner struct",
                labels: vec![],
                rename: None,
            },
        );
        meta.insert(
            "inner_threshold",
            MetricDescriptor {
                metric_type: MetricType::Gauge,
                help: "Threshold value from inner struct",
                labels: vec![],
                rename: None,
            },
        );

        let expected = indoc! {"
            # HELP my_requests Total number of requests processed
            # TYPE my_requests counter
            my_requests 1024
            # HELP my_errors Total number of errors
            # TYPE my_errors counter
            my_errors{endpoint=\"login\"} 4
            # HELP my_inner_value Current value from inner struct
            # TYPE my_inner_value gauge
            my_inner_value 3.42
            # HELP my_inner_threshold Threshold value from inner struct
            # TYPE my_inner_threshold gauge
            my_inner_threshold 100
            # HELP my_inner_unknown ?
            # TYPE my_inner_unknown untyped
            my_inner_unknown 55
        "};

        let output = to_prometheus_text(&my_metrics, Some("my"), &meta).unwrap();
        assert_eq!(output, expected);
    }
}
