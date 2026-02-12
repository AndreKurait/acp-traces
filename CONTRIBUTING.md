# Contributing to acp-traces

Thank you for your interest in contributing. This document explains how to get started and what we expect from contributions.

## Getting started

### Prerequisites

- Rust toolchain (stable): <https://rustup.rs/>
- `clippy` and `rustfmt`: `rustup component add clippy rustfmt`

### Build from source

```bash
git clone https://github.com/AndreKurait/acp-traces.git
cd acp-traces
cargo build --release
# Binary at target/release/acp-traces
```

### Run checks (required before submitting)

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

All CI runs these; please run them locally so your PR stays green.

## How to contribute

### Reporting bugs

Open an [issue](https://github.com/AndreKurait/acp-traces/issues) and use the **Bug report** template. Include:

- acp-traces version and OS
- Steps to reproduce
- Expected vs actual behavior
- Relevant logs or trace snippets if applicable

### Suggesting features

Use the **Feature request** template. Describe the use case and how it fits with ACP/OTel semantics. Check [DESIGN.md](DESIGN.md) for existing design notes.

### Pull requests

1. Fork the repo and create a branch from `main`.
2. Make your changes; keep commits focused and messages clear.
3. Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test`.
4. Open a PR with a short description and link any related issues.
5. We’ll review and may ask for small changes.

### Code style

- Follow `cargo fmt` and `cargo clippy` (we use `-D warnings`).
- Prefer existing patterns in the codebase (see `src/` and [DESIGN.md](DESIGN.md)).

### Scope

acp-traces aims to stay a thin proxy: parse ACP JSON-RPC, map to OTel GenAI semconv, export via OTLP. Changes that add optional flags or backends are fine; large new subsystems are better discussed in an issue first.

## Release process (maintainers)

Releases are driven by semver tags:

```bash
git tag -a v0.2.0 -m "Description"
git push origin v0.2.0
```

This triggers the release workflow: multi-platform builds, GitHub Release, and Homebrew formula update.

## Questions

Open a [Discussion](https://github.com/AndreKurait/acp-traces/discussions) or an issue if something is unclear. Please read the [Code of Conduct](CODE_OF_CONDUCT.md) before participating.
