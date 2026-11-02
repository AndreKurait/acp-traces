# Changelog

## Unreleased

### Development

- Update the OpenTelemetry crates together to 0.33.1 and adapt the integration-test exporter to the shared-reference export API.
- Enable HTTP gzip support and check both HTTP protocols with generic and signal-specific `none`/`gzip` compression settings before agent startup.
- Group future OpenTelemetry dependency updates to avoid incompatible partial upgrades.
- Update locked anyhow and clap patch versions.
- Update artifact upload/download actions and verify the complete downloaded binary set during release-preparation checks.

### Upgrade notes

- In this development source, OpenTelemetry 0.33 treats the configured CLI endpoint/protocol as final; OTLP environment variables no longer override them. Exporter timeout environment variables now use milliseconds rather than seconds.
- The updated SDK caps metric cardinality at 2,000 attribute sets per instrument by default, collecting excess sets in an overflow series.
- These changes are not in the published 0.3.1 binaries.

## 0.3.1

This project remains a proof of concept.

### Fixed

- Keep client and agent JSON-RPC request IDs in separate namespaces.
- Drain final agent output, allow graceful EOF handling, and exit when the agent exits even if client stdin remains open.
- Preserve forwarded bytes and line endings even when messages cannot be parsed.
- Suppress detailed error payloads and avoid retaining response text when content capture is disabled.
- Stop parenting new tool requests to a completed prompt.
- Export the advertised metrics and send OTLP/HTTP traces and metrics to their correct signal endpoints.
- Preserve collector URL path prefixes and query parameters when deriving HTTP signal endpoints.
- Reject unsupported protocol values instead of silently treating them as gRPC.
- Use a string array for `gen_ai.response.finish_reasons`.

### Development

- Add regression tests against the real span manager, subprocess transport, and a local HTTP receiver.
- Document POC status, privacy boundaries, unsupported cases, and unreleased-versus-installed behavior.
- Validate locked dependencies, package metadata, and release tag/version consistency; publish binary checksums for future releases.
- Build all six release targets on release-preparation PRs and manual checks without publishing; smoke-test binaries that match the runner architecture.
- Pin all release builds to the validated commit and publish only after uploading the complete, checksum-verified asset set to a new draft; reject existing releases and moved tags.

### Upgrade notes

- Existing CLI options and release binary names are unchanged. Unsupported `--otlp-protocol` values now fail instead of falling back to gRPC.
- `gen_ai.response.finish_reasons` is now an array of strings; consumers expecting a scalar need to adapt.
- Content capture remains off by default. Metadata can still contain sensitive information; see [SECURITY.md](SECURITY.md).

## 0.3.0 — 2026-02-12

See the [release and binary assets](https://github.com/AndreKurait/acp-traces/releases/tag/v0.3.0). The 0.3.1 fixes above are not included in these binaries.
