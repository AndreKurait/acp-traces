# Contributing to acp-traces

This is a proof of concept. Contributions should make its behavior easier to verify: reproducible bug reports, sanitized protocol fixtures, compatibility tests, and focused fixes are especially useful.

## Local setup

Install a stable Rust toolchain with `rustfmt` and `clippy`, then:

```bash
git clone https://github.com/AndreKurait/acp-traces.git
cd acp-traces
cargo build --locked
cargo run --locked -- --help
```

No agent account, cloud credentials, Docker daemon, or live OTLP service is needed for the test suite. Export tests start a receiver on a random loopback port. Subprocess tests use `sh` on Unix; Windows runs the remaining checks.

## Checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo package --locked --allow-dirty
python3 -B -m unittest discover -s tests -p 'test_release.py'
```

`cargo package` creates and verifies a local crate archive; it does not publish anything. `--allow-dirty` lets you run it while developing. CI uses a clean checkout and checks Linux, macOS, and Windows.

Commit `Cargo.lock` changes when updating dependencies. For changes to protocol handling, add a replay through the real `SpanManager` in `tests/span_integration.rs`; helper-only tests do not establish that the exported spans are correct. For forwarding/lifecycle changes, extend `tests/proxy_integration.rs`.

## Issues and pull requests

Use the [issue templates](https://github.com/AndreKurait/acp-traces/issues/new/choose). Include the proxy version/commit, OS, client/agent versions, and a minimal reproduction. Remove credentials, source code, private paths, prompts, and other sensitive data from logs and traces before sharing them. Report security-sensitive issues through [SECURITY.md](SECURITY.md).

Create a branch from `main`, keep the change focused, and explain the observed failure and resulting behavior in the PR. Include the checks you ran and any platforms you could not test. New protocol mappings should link their specification and include a small, sanitized fixture.

Large subsystems or compatibility guarantees need discussion first. The project currently stays focused on stdio forwarding, protocol observation, and OTLP export. See [DESIGN.md](DESIGN.md) for known limits, and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Releases (maintainers)

1. Update `Cargo.toml` and `Cargo.lock` to the intended version, and move the relevant `CHANGELOG.md` entries out of **Unreleased**.
2. Run the checks above and open a PR. Changes to the release workflow or Cargo manifests also run the six-target release build, with CLI smoke tests on native targets. A following job downloads the artifacts, checks the complete binary set, and smoke-tests the downloaded Linux binary. PR and manual runs exercise artifact transfer without publishing a release or changing Homebrew. Merge after these checks pass.
3. Create an annotated `v<version>` tag at that commit and push the tag. The tag must exactly match `Cargo.toml`; the workflow validates it before building.
4. Inspect all six binary artifacts and the generated `SHA256SUMS` in the release. Every build uses the validated source commit. Publication checks the complete set, refuses any existing release for the tag, and rechecks the remote tag before publishing the newly created draft by its exact ID. An upload failure leaves an unpublished draft; inspect and remove that draft before retrying against the unchanged tag. The workflow builds Linux/macOS/Windows targets but does not execute every cross-compiled binary. It does not publish to crates.io.
5. Confirm the Homebrew update job succeeds. It requires the repository's `HOMEBREW_TAP_TOKEN` secret with write access to `AndreKurait/homebrew-tap`.

For releases containing a checksum manifest, Linux users can run `sha256sum --check SHA256SUMS --ignore-missing` beside a downloaded binary; macOS users can compare its `shasum -a 256` output with the matching manifest entry. Existing releases before this workflow change have no checksum manifest.

Do not retag an existing release or replace its binaries silently. Fixes should receive a new version.
