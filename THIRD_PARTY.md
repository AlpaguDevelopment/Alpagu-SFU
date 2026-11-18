# Third-party software

Alpagu-SFU's own code is Apache-2.0. Dependencies retain their original licenses; this repository's license does not replace them.

Rust dependencies and exact versions are recorded in `core/Cargo.lock`. Client dependencies are recorded in `test-client/package-lock.json`. Direct Rust dependencies provide asynchronous networking (tokio), WebSocket transport (tokio-tungstenite, futures-util), JSON serialization (serde, serde_json), WebRTC (str0m), and logging (tracing, tracing-subscriber).

The Windows release archive contains `third-party/INDEX.md` and the license/notice files from the resolved Rust crates, including transitive/build dependencies. The archive distributes client source and its lockfile, not `node_modules` or a bundled client build; `npm ci` retrieves those packages and their license files separately.

To regenerate the Rust notice bundle, install Python 3 and run:

```powershell
python scripts/collect-licenses.py --output artifacts/third-party
```

The script invokes locked Cargo metadata, copies license and notice files without changing their text, and produces a package/license index. For crates that omit notices, it retrieves them from pinned upstream commits and records those URLs. It is a packaging aid, not a substitute for reviewing distribution obligations when dependencies change.
