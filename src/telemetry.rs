use anyhow::{bail, Result};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource};

pub fn init(
    endpoint: &str,
    protocol: &str,
    service_name: &str,
) -> Result<(SdkTracerProvider, SdkMeterProvider)> {
    let resource = Resource::builder()
        .with_attribute(KeyValue::new("service.name", service_name.to_string()))
        .build();

    let (span_exporter, metric_exporter) = match protocol {
        "http" | "http-json" => {
            let protocol = if protocol == "http-json" {
                Protocol::HttpJson
            } else {
                Protocol::HttpBinary
            };
            (
                SpanExporter::builder()
                    .with_http()
                    .with_protocol(protocol)
                    .with_endpoint(http_endpoint(endpoint, "traces"))
                    .build()?,
                MetricExporter::builder()
                    .with_http()
                    .with_protocol(protocol)
                    .with_endpoint(http_endpoint(endpoint, "metrics"))
                    .build()?,
            )
        }
        "grpc" => (
            SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
            MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
        ),
        _ => bail!("unsupported OTLP protocol: {protocol}"),
    };
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_periodic_exporter(metric_exporter)
        .build();
    opentelemetry::global::set_meter_provider(meter_provider.clone());

    tracing::info!(endpoint = %endpoint, protocol = %protocol, "OTel initialized");
    Ok((tracer_provider, meter_provider))
}

fn http_endpoint(endpoint: &str, signal: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    // Accept the explicit trace path supported by earlier versions as well as a base URL.
    let base = base
        .strip_suffix("/v1/traces")
        .or_else(|| base.strip_suffix("/v1/metrics"))
        .unwrap_or(base);
    format!("{base}/v1/{signal}")
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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{
        metrics::MeterProvider,
        trace::{Span, Tracer, TracerProvider},
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_exports_both_traces_and_metrics_to_their_signal_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut requests = Vec::new();
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("mock receiver failed: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                let header = String::from_utf8(header).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                requests.push((
                    header.lines().next().unwrap().to_owned(),
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                ));
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            }
            requests
        });

        let (traces, metrics) = init(&endpoint, "http-json", "test-service").unwrap();
        traces.tracer("test").start("test-span").end();
        metrics
            .meter("test")
            .f64_histogram("test.duration")
            .build()
            .record(0.5, &[]);
        traces.force_flush().unwrap();
        metrics.force_flush().unwrap();
        shutdown(traces, metrics);
        stopped.store(true, Ordering::Relaxed);
        let requests = server.join().unwrap();
        assert!(requests
            .iter()
            .any(|(path, body)| path == "POST /v1/traces HTTP/1.1"
                && body.get("resourceSpans").is_some()));
        assert!(requests
            .iter()
            .any(|(path, body)| path == "POST /v1/metrics HTTP/1.1"
                && body.to_string().contains("test.duration")));
    }
}
