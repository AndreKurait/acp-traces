use anyhow::{bail, Context, Result};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource};
use url::Url;

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
                    .with_endpoint(http_endpoint(endpoint, "traces")?)
                    .build()?,
                MetricExporter::builder()
                    .with_http()
                    .with_protocol(protocol)
                    .with_endpoint(http_endpoint(endpoint, "metrics")?)
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

    // Collector URLs may contain authentication query parameters.
    tracing::info!(protocol = %protocol, "OTel initialized");
    Ok((tracer_provider, meter_provider))
}

fn http_endpoint(endpoint: &str, signal: &str) -> Result<String> {
    let mut endpoint = Url::parse(endpoint).context("invalid OTLP HTTP endpoint URL")?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        bail!("OTLP HTTP endpoint must use http:// or https://");
    }
    let path = endpoint.path().trim_end_matches('/');
    // Change only the path so collector prefixes and authentication queries survive.
    // Accept either explicit signal path for compatibility with earlier configurations.
    let base = path
        .strip_suffix("/v1/traces")
        .or_else(|| path.strip_suffix("/v1/metrics"))
        .unwrap_or(path);
    endpoint.set_path(&format!("{base}/v1/{signal}"));
    Ok(endpoint.into())
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

    #[test]
    fn http_signal_paths_preserve_collector_prefixes_and_queries() {
        for (endpoint, signal, expected) in [
            (
                "http://localhost:4318/",
                "traces",
                "http://localhost:4318/v1/traces",
            ),
            (
                "https://collector/v1/traces?token=x",
                "metrics",
                "https://collector/v1/metrics?token=x",
            ),
            (
                "https://collector/otlp?token=a%2Fb&tenant=one",
                "traces",
                "https://collector/otlp/v1/traces?token=a%2Fb&tenant=one",
            ),
            (
                "https://collector/otlp/v1/metrics/?token=x",
                "traces",
                "https://collector/otlp/v1/traces?token=x",
            ),
            (
                "https://collector/tenant%2Fone/v1/traces?token=a%2Fb",
                "metrics",
                "https://collector/tenant%2Fone/v1/metrics?token=a%2Fb",
            ),
            (
                "http://[::1]:4318/otlp/v1/traces",
                "metrics",
                "http://[::1]:4318/otlp/v1/metrics",
            ),
        ] {
            assert_eq!(http_endpoint(endpoint, signal).unwrap(), expected);
        }
    }

    #[test]
    fn http_signal_paths_reject_invalid_urls_and_non_http_schemes() {
        for endpoint in [
            "collector/v1/traces",
            "file:///tmp/collector",
            "grpc://collector:4317",
        ] {
            assert!(http_endpoint(endpoint, "traces").is_err());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_exports_both_traces_and_metrics_to_their_signal_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/tenant%2Fone/v1/traces?token=test%2Ftoken&tenant=local",
            listener.local_addr().unwrap()
        );
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
        assert!(requests.iter().any(|(path, body)| path
            == "POST /tenant%2Fone/v1/traces?token=test%2Ftoken&tenant=local HTTP/1.1"
            && body.get("resourceSpans").is_some()));
        assert!(requests.iter().any(|(path, body)| path
            == "POST /tenant%2Fone/v1/metrics?token=test%2Ftoken&tenant=local HTTP/1.1"
            && body.to_string().contains("test.duration")));
    }
}
