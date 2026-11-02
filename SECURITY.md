# Security and privacy

`acp-traces` is an experimental proof of concept. It runs an agent with the current user's permissions and observes that agent's stdio traffic. It is not a sandbox, a secret-redaction tool, or a production security boundary.

## Data handling

- Content recording is off by default. The proxy still exports session IDs, method names, agent/client metadata, tool titles, timestamps, and numeric error codes. Tool titles can contain sensitive paths or names.
- `--record-content` enables prompt/response text, tool payloads, and detailed error JSON. These can contain credentials or private source code. They are sent to the configured OTLP receiver without automatic secret redaction.
- The proxy inherits the agent's stderr. Do not assume stderr logs are sanitized.
- The default endpoint is unencrypted and local. TLS exporter features are not enabled in this build; use a trusted local collector to forward telemetry to a remote TLS/authenticated backend.
- Message size, opted-in content accumulation, and retained session state are not capped. Do not expose the proxy to untrusted or unbounded input streams.

See [README.md](README.md) for configuration and [DESIGN.md](DESIGN.md) for implementation limits.

## Reporting a vulnerability

Do not put credentials, private traces, or an unpatched security issue in a public issue. Email [andrekurait@gmail.com](mailto:andrekurait@gmail.com) with:

- A description of the issue and affected version/commit.
- A sanitized reproduction and the expected versus observed behavior.
- The potential impact and any suggested mitigation.

A private draft can also be used if GitHub's [Report a vulnerability](https://github.com/AndreKurait/acp-traces/security/advisories/new) option is available. Please agree on disclosure timing before publishing sensitive details.

There is no guaranteed response SLA or long-term support branch. Fixes will target current development and, when appropriate, a new release. Older releases may require upgrading. This project does not promise production suitability merely because no vulnerability has been reported.
