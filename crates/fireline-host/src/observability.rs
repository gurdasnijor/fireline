//! Host-level tracing and OpenTelemetry bootstrap.

use std::collections::HashMap;
use std::future;

use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

const DEFAULT_SERVICE_NAME: &str = "fireline";

#[derive(Debug)]
pub struct ObservabilityGuard {
    tracer_provider: SdkTracerProvider,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        let _ = self.tracer_provider.shutdown();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservabilityConfig {
    service_name: String,
    resource_attributes: Vec<(String, String)>,
    exporter: ExporterConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExporterConfig {
    Noop,
    Otlp {
        endpoint: String,
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Default)]
struct NoopSpanExporter;

impl SpanExporter for NoopSpanExporter {
    fn export(&self, _batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        future::ready(Ok(()))
    }
}

pub fn init_tracing() -> Result<ObservabilityGuard> {
    let config = ObservabilityConfig::from_env();
    let tracer_provider = build_tracer_provider(&config)?;
    let tracer = tracer_provider.tracer(config.service_name.clone());

    global::set_tracer_provider(tracer_provider.clone());

    let span_events = if std::env::var_os("FIRELINE_TRACE_SPANS").is_some() {
        FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().with_span_events(span_events).without_time())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .context("initialize tracing subscriber")?;

    Ok(ObservabilityGuard { tracer_provider })
}

fn build_tracer_provider(config: &ObservabilityConfig) -> Result<SdkTracerProvider> {
    let resource = Resource::builder()
        .with_attributes(
            config
                .resource_attributes
                .iter()
                .filter(|(key, _)| key != "service.name")
                .map(|(key, value)| KeyValue::new(key.clone(), value.clone())),
        )
        .with_service_name(config.service_name.clone())
        .build();

    let builder = SdkTracerProvider::builder().with_resource(resource);

    match &config.exporter {
        ExporterConfig::Noop => Ok(builder.with_simple_exporter(NoopSpanExporter).build()),
        ExporterConfig::Otlp { endpoint, headers } => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(endpoint.clone())
                .with_headers(headers.clone())
                .build()
                .context("build OTLP span exporter")?;
            Ok(builder.with_simple_exporter(exporter).build())
        }
    }
}

impl ObservabilityConfig {
    fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        let service_name = lookup("OTEL_SERVICE_NAME")
            .and_then(|value| non_empty(value))
            .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string());
        let resource_attributes = lookup("OTEL_RESOURCE_ATTRIBUTES")
            .map(|value| parse_key_value_pairs(&value))
            .unwrap_or_default();

        let exporter = match lookup("OTEL_EXPORTER_OTLP_ENDPOINT").and_then(non_empty) {
            Some(endpoint) => ExporterConfig::Otlp {
                endpoint,
                headers: lookup("OTEL_EXPORTER_OTLP_HEADERS")
                    .map(|value| parse_key_value_pairs(&value).into_iter().collect())
                    .unwrap_or_default(),
            },
            None => ExporterConfig::Noop,
        };

        Self {
            service_name,
            resource_attributes,
            exporter,
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_key_value_pairs(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ExporterConfig, ObservabilityConfig};
    use std::collections::HashMap;

    #[test]
    fn selects_noop_exporter_when_otlp_endpoint_is_missing() {
        let config = ObservabilityConfig::from_lookup(|key| match key {
            "OTEL_SERVICE_NAME" => Some("custom-fireline".to_string()),
            "OTEL_RESOURCE_ATTRIBUTES" => Some("deployment.environment=dev".to_string()),
            _ => None,
        });

        assert_eq!(config.service_name, "custom-fireline");
        assert_eq!(
            config.resource_attributes,
            vec![("deployment.environment".to_string(), "dev".to_string())]
        );
        assert_eq!(config.exporter, ExporterConfig::Noop);
    }

    #[test]
    fn selects_otlp_exporter_when_endpoint_is_present() {
        let config = ObservabilityConfig::from_lookup(|key| match key {
            "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://collector:4318/v1/traces".to_string()),
            "OTEL_EXPORTER_OTLP_HEADERS" => {
                Some("authorization=Bearer test,x-tenant=fireline".to_string())
            }
            _ => None,
        });

        let expected_headers = HashMap::from([
            ("authorization".to_string(), "Bearer test".to_string()),
            ("x-tenant".to_string(), "fireline".to_string()),
        ]);

        assert_eq!(config.service_name, "fireline");
        assert_eq!(
            config.exporter,
            ExporterConfig::Otlp {
                endpoint: "http://collector:4318/v1/traces".to_string(),
                headers: expected_headers,
            }
        );
    }
}
