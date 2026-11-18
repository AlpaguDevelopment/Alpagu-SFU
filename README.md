# Alpagu-SFU

Alpagu-SFU is an open-source WebRTC Selective Forwarding Unit (SFU) written in Rust, built on [str0m](https://github.com/algesten/str0m). It forwards media between participants without decoding or re-encoding it.

The project is developed using AI models **GPT-6 Astra, Claude Opus 5.5, and Claude Fable 5.1**, with human direction and manual testing.

## Status

**v0.1.0 is an experimental LAN audio release.** It supports one room, up to 16 participants, Opus audio, microphone/deafen state, and a browser test client. Each participant has separate publish and subscribe connections; all media shares one UDP port.

Camera in the test client is **local preview only**. Video forwarding, authentication, multiple rooms, TURN, signaling TLS in the core, and automatic reconnection are not implemented. Run this version on a trusted local network, not as a public Internet service.

## Run the Windows release

1. Download `Alpagu-SFU-v0.1.0-windows-x64.zip` from [Releases](https://github.com/AlpaguDevelopment/Alpagu-SFU/releases).
2. Extract it. Open PowerShell in the extracted directory.
3. Find your active LAN IPv4 address with `ipconfig`, then run:

```powershell
.\sfu-core.exe --ip <LAN-IP>
```

Replace `<LAN-IP>` with the actual address, without angle brackets. Wait for `SFU started`. Allow the server through Windows Firewall on **private networks**. Stop it with Ctrl+C.

The Windows x64 server does not require Rust or Node.js. The browser test client is separate and requires Node.js. The ZIP includes its source under `test-client/`, together with the documentation and dependency notices. This release is unsigned and was verified on the development Windows machine; it has not been tested across all Windows versions. No Linux or macOS binaries are provided yet.

## Run from source

Requirements: stable Rust (tested with 1.100.0), and the platform's native build tools. On Windows use the MSVC toolchain and Visual Studio Build Tools with C++ support. Node.js 22.22 or newer is needed for the test client.

```powershell
git clone https://github.com/AlpaguDevelopment/Alpagu-SFU.git
cd Alpagu-SFU
cargo run --manifest-path core/Cargo.toml --release --locked -- --ip <LAN-IP>
```

The initial build can take several minutes. For a binary without starting the server:

```powershell
cargo build --manifest-path core/Cargo.toml --release --locked
```

The output is `core/target/release/sfu-core.exe` on Windows. To reproduce the distributable archive with static CRT linkage, notices, checksum, and a binary smoke test, run `pwsh -File scripts/build-windows-release.ps1` (also requires Python 3, Node.js, and Git). Source builds on other platforms have not yet been validated for this release.

## Connect two browsers

Leave the server running. In a second terminal, from the repository or extracted release directory:

```powershell
cd test-client
npm ci
npm run dev
```

- Computer: open `https://localhost:5173`.
- Phone on the same network: open `https://<LAN-IP>:5173`.
- Accept the local development certificate warning and grant microphone access.
- Join with different names and speak in both directions. Use headphones to avoid feedback.
- If playback is blocked, press **Tap to enable audio**.

The HTTPS development server proxies `/ws` to the core. The core executable does not serve the browser UI. The client is a development/test tool, not a production application. `npm run build` produces static assets, but deploying those assets requires your own HTTPS server and WebSocket proxy.

## Ports and options

| Option | Default | Purpose |
|---|---|---|
| `--ip` | `127.0.0.1` | IPv4 address advertised to browsers; use the server's LAN address for phones |
| `--port` | `7880` | Signaling TCP port, bound to all IPv4 interfaces |
| `--udp-port` | `7882` | Shared media UDP port, bound to all IPv4 interfaces |
| `--version` | | Print `0.1.0` and exit |
| `--help` | | Show command-line usage |

The test client's HTTPS port is **5173/TCP**. Allow that port and **7882/UDP** on your private network. The client proxy connects locally to **7880/TCP** by default. If the core uses a different signaling port, set the proxy target before starting Vite:

```powershell
$env:SFU_CORE = 'ws://127.0.0.1:9000'
npm run dev
```

`--ip` changes the advertised media address, not the listening interface.

## Test and contribute

See [testing instructions](docs/testing.md) for automated checks, the desktop/phone checklist, and troubleshooting.

- [Protocol v1](docs/protocol.md)
- [Roadmap](docs/roadmap.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Third-party licensing](THIRD_PARTY.md)

## License

Copyright 2026 Alpagu Development contributors.

Licensed under the [Apache License 2.0](LICENSE). Third-party dependencies retain their respective licenses.
