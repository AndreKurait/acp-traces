use std::process::Command;

#[test]
fn invalid_protocol_is_rejected_before_starting_an_agent() {
    let output = Command::new(env!("CARGO_BIN_EXE_acp-traces"))
        .args(["--otlp-protocol", "grcp", "--", "not-an-agent"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"));
    assert!(!stderr.contains("failed to spawn"));
}

#[test]
fn http_compression_settings_do_not_prevent_agent_startup() {
    const COMPRESSION_VARIABLES: [&str; 3] = [
        "OTEL_EXPORTER_OTLP_COMPRESSION",
        "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
        "OTEL_EXPORTER_OTLP_METRICS_COMPRESSION",
    ];
    for protocol in ["http", "http-json"] {
        for variable in COMPRESSION_VARIABLES {
            for compression in ["none", "gzip"] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_acp-traces"));
                for key in COMPRESSION_VARIABLES {
                    command.env_remove(key);
                }
                let output = command
                    .args([
                        "--otlp-protocol",
                        protocol,
                        "--otlp-endpoint",
                        "http://127.0.0.1:1",
                        "--",
                        "acp-traces-test-agent-that-does-not-exist",
                    ])
                    .env(variable, compression)
                    .env("OTEL_EXPORTER_OTLP_TIMEOUT", "100")
                    .output()
                    .unwrap();
                let stderr = String::from_utf8_lossy(&output.stderr);
                // The missing agent is intentional: reaching spawn proves both
                // exporters initialized without requiring any agent installation.
                assert!(
                    stderr.contains("failed to spawn: acp-traces-test-agent-that-does-not-exist"),
                    "{protocol}, {variable}={compression}: {stderr}"
                );
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::Write;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    fn proxy(script: &str) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_acp-traces"))
            .args([
                "--otlp-endpoint",
                "http://127.0.0.1:1",
                "--",
                "sh",
                "-c",
                script,
            ])
            .env("OTEL_EXPORTER_OTLP_TIMEOUT", "100")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn finish(mut child: std::process::Child) -> std::process::Output {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "proxy failed to exit: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn child_exit_drains_stdout_and_exits_even_when_editor_stdin_is_open() {
        let mut child = proxy("read line; printf '%s\\n' '{\"id\":1,\"result\":{}}'; exit 7");
        let mut editor = child.stdin.take().unwrap();
        editor
            .write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .unwrap();
        let output = finish(child);
        drop(editor);
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"{\"id\":1,\"result\":{}}\n");
    }

    #[test]
    fn editor_eof_allows_the_agent_to_send_a_final_response() {
        let mut child = proxy("cat >/dev/null; printf '%s\\n' '{\"id\":1,\"result\":{}}'");
        let mut editor = child.stdin.take().unwrap();
        editor
            .write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .unwrap();
        drop(editor);
        let output = finish(child);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"{\"id\":1,\"result\":{}}\n");
    }

    #[test]
    fn forwarding_preserves_unparseable_bytes_and_line_endings() {
        let mut child = proxy("cat");
        let mut editor = child.stdin.take().unwrap();
        let data = b"not json\r\ninvalid utf8: \xff\nunterminated";
        editor.write_all(data).unwrap();
        drop(editor);
        let output = finish(child);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, data);
    }
}
