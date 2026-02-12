use anyhow::Result;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource};

pub fn init(
    endpoint: &str,
    protocol: &str,
    service_name: &str,
) -> Result<(SdkTracerProvider, SdkMeterProvider)> {
    let resource = Resource::builder()
        .with_attribute(KeyValue::new("service.name", service_name.to_string()))
        .build();

    let tracer_provider = match protocol {
        "http" | "http-json" => {
            let mut builder = SpanExporter::builder().with_http().with_endpoint(endpoint);
            if protocol == "http-json" {
                builder = builder.with_protocol(Protocol::HttpJson);
            }
            let exporter = builder.build()?;
            SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(exporter)
                .build()
        }
        _ => {
            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?;
            SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(exporter)
                .build()
        }
    };

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    let meter_provider = SdkMeterProvider::builder().with_resource(resource).build();
    opentelemetry::global::set_meter_provider(meter_provider.clone());

    tracing::info!(endpoint = %endpoint, protocol = %protocol, "OTel initialized");
    Ok((tracer_provider, meter_provider))
}

pub fn shutdown(tracer_provider: SdkTracerProvider, meter_provider: SdkMeterProvider) {
    if let Err(e) = tracer_provider.force_flush() {
        tracing::warn!(error = %e, "tracer flush error");
    }
    if let Err(e) = tracer_provider.shutdown() {
        tracing::warn!(error = %e, "tracer shutdown error");
    }
    if let Err(e) = meter_provider.shutdown() {
        tracing::warn!(error = %e, "meter shutdown error");
    }
}
