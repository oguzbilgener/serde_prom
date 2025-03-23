use indoc::indoc;
use pretty_assertions::assert_eq;
use std::collections::HashMap;

use openmetrics_parser::prometheus::parse_prometheus;
use serde::Serialize;

use crate::{
    ser::{MetricDescriptor, MetricType},
    to_prometheus_text,
};

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
        my_requests{app=\"myapp\"} 1024

        # HELP my_errors Total number of errors
        # TYPE my_errors counter
        my_errors{app=\"myapp\",endpoint=\"login\"} 4

        # HELP my_inner_value Current value from inner struct
        # TYPE my_inner_value gauge
        my_inner_value{app=\"myapp\"} 3.42

        # HELP my_inner_threshold Threshold value from inner struct
        # TYPE my_inner_threshold gauge
        my_inner_threshold{app=\"myapp\"} 100

        # TYPE my_inner_unknown untyped
        my_inner_unknown{app=\"myapp\"} 55
    "};

    let labels = vec![("app", "myapp")];
    let output = to_prometheus_text(&my_metrics, Some("my"), &meta, labels).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn test_parse_simple() {
    #[derive(Serialize)]
    struct Sub {
        a: i32,
        b: i32,
    }

    #[derive(Serialize)]
    struct Data {
        one: i32,
        two: i32,
        three: i32,
        sub: Sub,
    }

    let mut meta = HashMap::new();
    meta.insert(
        "one",
        MetricDescriptor {
            metric_type: MetricType::Counter,
            help: "First one",
            labels: vec![],
            rename: Some("one_total"),
        },
    );
    meta.insert(
        "two",
        MetricDescriptor {
            metric_type: MetricType::Counter,
            help: "Second one",
            labels: vec![("thing", "stuff")],
            rename: Some("two_total"),
        },
    );
    meta.insert(
        "three",
        MetricDescriptor {
            metric_type: MetricType::Counter,
            help: "Third one",
            labels: vec![],
            rename: Some("three_total"),
        },
    );
    meta.insert(
        "sub_a",
        MetricDescriptor {
            metric_type: MetricType::Gauge,
            help: "Sub A",
            labels: vec![],
            rename: None,
        },
    );
    meta.insert(
        "sub_b",
        MetricDescriptor {
            metric_type: MetricType::Gauge,
            help: "Sub B",
            labels: vec![],
            rename: None,
        },
    );

    let inputs = vec![
        Data {
            one: 1,
            two: 2,
            three: 3,
            sub: Sub { a: 4, b: 5 },
        },
        Data {
            one: 4,
            two: 5,
            three: 6,
            sub: Sub { a: 5, b: 6 },
        },
        Data {
            one: 7,
            two: 8,
            three: 9,
            sub: Sub { a: 8, b: 9 },
        },
    ];

    let labels: Vec<(&str, &str)> = vec![];
    let output = to_prometheus_text(&inputs, Some("my"), &meta, &labels).unwrap();
    println!("output:\n{output}");
    let _parsed = parse_prometheus(&output).unwrap();
}
