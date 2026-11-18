# Contributing

Alpagu-SFU is experimental. Please open an issue before proposing a large feature. Follow the [roadmap](docs/roadmap.md): reliability, correctness, simplicity, performance, then features.

## Development

- Target pull requests at `dev`. `main` tracks tagged releases.
- Use stable Rust, edition 2024. Keep `#![forbid(unsafe_code)]`.
- Do not use `unwrap`, `expect`, or `panic!` in production paths. Tests may use them.
- Bound untrusted input, queues, and resource use. Malformed traffic must not crash the process.
- Use `tracing`; never log credentials, SDP, or personal data unnecessarily.
- Keep dependencies small and explain new dependencies in the commit or PR description. Commit lockfiles.
- Keep protocol handling in `test-client/src/sfu-client.ts`, separate from UI code. Use strict TypeScript without a UI framework.
- Use English for source, comments, logs, UI, public documentation, and Conventional Commit messages.
- Update the protocol documentation first when changing the wire contract, and increment the protocol version for contract changes.

Run the checks in [testing.md](docs/testing.md). Add meaningful automated coverage for behavior changes, update the changelog, and describe manual desktop/phone validation. Release acceptance requires automated checks and manual regression tests.

## Developer Certificate of Origin

By contributing, certify your contribution under the [Developer Certificate of Origin 1.1](https://developercertificate.org/). Sign off your commits:

```sh
git commit -s
```

Contributions are accepted under Apache-2.0. Do not include secrets, private integration documents, generated build output, or local certificates.

AI-assisted contributions are welcome. Describe substantial AI assistance and the validation performed; contributors remain responsible for reviewing their changes and having the right to submit them.
