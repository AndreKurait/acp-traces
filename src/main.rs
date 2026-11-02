mod telemetry;

use acp_traces::{acp, spans};

use anyhow::{Context, Result};
use clap::Parser;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(
    name = "acp-traces",
    version,
    about = "OTel tracing proxy for Agent Client Protocol"
)]
struct Cli {
    /// OTLP endpoint
    #[arg(long, default_value = "http://localhost:4317")]
    otlp_endpoint: String,

    /// OTLP protocol
    #[arg(long, default_value = "grpc", value_parser = ["grpc", "http", "http-json"])]
    otlp_protocol: String,

    /// OTel service name
    #[arg(long, default_value = "acp-agent")]
    service_name: String,

    /// Record prompts, responses, tool payloads, and error details (may contain secrets)
    #[arg(long)]
    record_content: bool,

    /// Increase log verbosity (repeat for more: -v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Agent command and arguments
    #[arg(trailing_var_arg = true, required = true)]
    command: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .with_writer(std::io::stderr)
        .init();

    let (tracer_provider, meter_provider) =
        telemetry::init(&cli.otlp_endpoint, &cli.otlp_protocol, &cli.service_name)?;

    let tracer = opentelemetry::global::tracer("acp-traces");
    let meter = opentelemetry::global::meter("acp-traces");
    let span_mgr = spans::SpanManager::new(tracer, meter, cli.record_content);

    let result = run_proxy(&cli.command, span_mgr).await;
    telemetry::shutdown(tracer_provider, meter_provider);

    // Tokio's stdin adapter uses a blocking read that cannot be cancelled. Exit
    // explicitly after flushing telemetry, even if the editor still has stdin open.
    match result {
        Ok(status) => std::process::exit(exit_code(status)),
        Err(error) => {
            tracing::error!(error = %error, "proxy failed");
            std::process::exit(1);
        }
    }
}

async fn forward<R, W>(
    mut reader: R,
    mut writer: W,
    direction: acp::Direction,
    tx: tokio::sync::mpsc::Sender<(acp::Direction, String)>,
) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).await? == 0 {
            return Ok(());
        }
        // Observation must precede forwarding so a fast response cannot overtake
        // its request in the span manager. Malformed bytes still pass unchanged.
        if let Ok(message) = String::from_utf8(line.clone()) {
            let _ = tx.send((direction, message)).await;
        }
        writer.write_all(&line).await?;
        writer.flush().await?;
    }
}

async fn wait_for_shutdown(child: &mut Child) -> Result<ExitStatus> {
    match tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await {
        Ok(status) => Ok(status?),
        Err(_) => {
            tracing::warn!("agent did not exit within the shutdown grace period");
            child.kill().await?;
            Ok(child.wait().await?)
        }
    }
}

async fn run_proxy(command: &[String], span_mgr: spans::SpanManager) -> Result<ExitStatus> {
    let (cmd, args) = command.split_first().context("no command specified")?;
    tracing::info!(cmd = %cmd, "spawning agent");

    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn: {cmd}"))?;

    let child_stdin = child.stdin.take().context("no child stdin")?;
    let child_stdout = child.stdout.take().context("no child stdout")?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);

    let mut editor_to_agent = tokio::spawn(forward(
        BufReader::new(tokio::io::stdin()),
        child_stdin,
        acp::Direction::EditorToAgent,
        tx.clone(),
    ));
    let mut agent_to_editor = tokio::spawn(forward(
        BufReader::new(child_stdout),
        tokio::io::stdout(),
        acp::Direction::AgentToEditor,
        tx,
    ));
    let processor = tokio::spawn(async move {
        let mut mgr = span_mgr;
        while let Some((direction, line)) = rx.recv().await {
            mgr.process_message(direction, &line);
        }
        mgr.shutdown();
    });

    let mut input_finished = false;
    let mut output_finished = false;
    let result = async {
        tokio::select! {
            status = child.wait() => Ok(status?),
            input = &mut editor_to_agent => {
                input_finished = true;
                input.context("stdin forwarding task failed")??;
                // EOF closes the child's stdin. Give it time to emit its final response.
                wait_for_shutdown(&mut child).await
            }
            output = &mut agent_to_editor => {
                output_finished = true;
                output.context("stdout forwarding task failed")??;
                wait_for_shutdown(&mut child).await
            }
        }
    }
    .await;

    if result.is_err() {
        let _ = child.kill().await;
    }
    if !input_finished {
        editor_to_agent.abort();
        let _ = editor_to_agent.await;
    }

    // Drain buffered stdout after child exit instead of dropping the final messages.
    let output_result = if output_finished {
        Ok(())
    } else {
        match tokio::time::timeout(SHUTDOWN_GRACE, &mut agent_to_editor).await {
            Ok(result) => result
                .context("stdout forwarding task failed")
                .and_then(|r| r),
            Err(_) => {
                agent_to_editor.abort();
                let _ = agent_to_editor.await;
                Err(anyhow::anyhow!(
                    "agent stdout did not close after process exit"
                ))
            }
        }
    };
    processor.await.context("span processing task failed")?;
    output_result?;
    result
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(1)
    }
    #[cfg(not(unix))]
    1
}
