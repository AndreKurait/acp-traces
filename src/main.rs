mod acp;
mod spans;
mod telemetry;

use anyhow::{Context, Result};
use clap::Parser;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

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

    /// OTLP protocol: grpc or http
    #[arg(long, default_value = "grpc")]
    otlp_protocol: String,

    /// OTel service name
    #[arg(long, default_value = "acp-agent")]
    service_name: String,

    /// Record message content (gen_ai.input/output.messages) — contains sensitive data
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

    let (cmd, args) = cli.command.split_first().context("no command specified")?;
    tracing::info!(cmd = %cmd, args = ?args, "spawning agent");

    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to spawn: {cmd}"))?;

    let child_stdin = child.stdin.take().context("no child stdin")?;
    let child_stdout = child.stdout.take().context("no child stdout")?;

    let parent_stdin = tokio::io::stdin();
    let parent_stdout = tokio::io::stdout();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(acp::Direction, String)>();

    let tx_editor = tx.clone();
    let editor_to_agent = tokio::spawn(async move {
        let mut reader = BufReader::new(parent_stdin);
        let mut writer = child_stdin;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            let _ = tx_editor.send((acp::Direction::EditorToAgent, line.trim_end().to_string()));
            writer.write_all(line.as_bytes()).await?;
            writer.flush().await?;
        }
        anyhow::Ok(())
    });

    let tx_agent = tx;
    let agent_to_editor = tokio::spawn(async move {
        let mut reader = BufReader::new(child_stdout);
        let mut writer = parent_stdout;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            let _ = tx_agent.send((acp::Direction::AgentToEditor, line.trim_end().to_string()));
            writer.write_all(line.as_bytes()).await?;
            writer.flush().await?;
        }
        anyhow::Ok(())
    });

    // Process intercepted messages — owns span_mgr, no shared state
    let tp_clone = tracer_provider.clone();
    let processor = tokio::spawn(async move {
        let mut mgr = span_mgr;
        while let Some((direction, line)) = rx.recv().await {
            mgr.process_message(direction, &line);
        }
        mgr.shutdown();
        // Flush immediately so the root span is exported before process exit
        let _ = tp_clone.force_flush();
    });

    let status = tokio::select! {
        s = child.wait() => s?,
        _ = editor_to_agent => {
            // stdin EOF — kill child so we can shut down cleanly
            child.kill().await.ok();
            child.wait().await?
        }
    };
    // Abort the agent_to_editor task to drop its tx sender, closing the channel
    agent_to_editor.abort();
    let _ = processor.await;

    telemetry::shutdown(tracer_provider, meter_provider);

    tracing::info!(code = ?status.code(), "agent exited");
    std::process::exit(status.code().unwrap_or(0));
}
