# Changelog

## Unreleased

This project remains a proof of concept.

### Fixed

- Keep client and agent JSON-RPC request IDs in separate namespaces.
- Drain final agent output, allow graceful EOF handling, and exit when the agent exits even if client stdin remains open.
- Preserve forwarded bytes and line endings even when messages cannot be parsed.
- Suppress detailed error payloads and avoid retaining response text when content capture is disabled.
- Stop parenting new tool requests to a completed prompt.
- Export the advertised metrics and send OTLP/HTTP traces and metrics to their correct signal endpoints.
- Reject unsupported protocol values instead of silently treating them as gRPC.
- Use a string array for `gen_ai.response.finish_reasons`.

### Development

- Add regression tests against the real span manager, subprocess transport, and a local HTTP receiver.
- Document POC status, privacy boundaries, unsupported cases, and unreleased-versus-installed behavior.
- Validate locked dependencies, package metadata, and release tag/version consistency; publish binary checksums for future releases.

## 0.3.0 — 2026-02-12

See the [release and binary assets](https://github.com/AndreKurait/acp-traces/releases/tag/v0.3.0). The unreleased fixes above are not included in these binaries.
