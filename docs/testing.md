# Testing

Start the core and test client using the [README](../README.md). Use desktop and phone browsers on the same trusted network. Use headphones or separate rooms to avoid feedback.

## Automated checks

From the repository root:

```powershell
cargo fmt --manifest-path core/Cargo.toml --check
cargo clippy --manifest-path core/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path core/Cargo.toml --locked
npm --prefix test-client ci
npm --prefix test-client test
npm --prefix test-client run build
```

The nine Rust tests cover CLI/input/SDP limits, rate limiting, room capacity and cleanup, serialized offers, real local WebSocket signaling, and three simulated WebRTC peers exchanging encoded Opus over ICE/DTLS. Five client tests use mocked camera capture to cover ten open/close cycles, late permissions, session replacement, permission rejection, and invalid capture cleanup.

These tests do not measure audible quality or prove compatibility with every browser. Node.js 22.22 or newer is required for the client's TypeScript-based tests.

## Manual acceptance and regression checklist

1. **Two participants, once:** join from desktop and phone with different names. Confirm two tiles and core version `0.1.0`. Speak in each direction for at least 20 seconds. Press **Tap to enable audio** if shown.
2. **Microphone, five cycles on each device:** turn it off and on. Off must stop transmitted voice and show **Mic off** remotely; on must restore voice.
3. **Deafen, five cycles:** remote audio and the local microphone must both stop; the remote tile must show **Deafened**. Undeafen restores audio and the previous microphone state. Also try once with the microphone already off; it must stay off afterward.
4. **Three participants, once:** add a second desktop tab under a third name. All three tiles must appear. Speak from each participant and check the others receive audio. Mute unused tabs sharing the same physical microphone.
5. **Leave/rejoin, ten cycles:** keep two peers in the room while a third leaves and rejoins. Its tile disappears and returns with the correct name; audio resumes. Leave with everyone and check the final core log shows `participants=0`. This checks participant cleanup, not a memory benchmark.
6. **Close a tab, once:** join with two peers and close one tab directly. Its tile must disappear from the remaining peer.
7. **Local camera, five cycles on each device:** open and close preview. Expect **Opening camera...**, then **Camera on (local)** and your local video. Closing must remove the preview and release the device. Other peers must not receive video in v0.1. Audio must continue for at least 20 seconds while toggling preview.
8. **Camera cleanup, once:** leave with camera open. Its usage indicator must turn off. If an initial permission prompt is available, leave before granting permission, then grant it: capture must not remain active.

For each later release, repeat at least steps 1-3, 5 and 7-8 as regression checks. Close all old client tabs before testing a new version.

## Troubleshooting

- Phone cannot load the page: check the LAN address, port 5173, private-network firewall permissions, and Wi-Fi client isolation.
- Page loads but no audio: check microphone permission, autoplay unlock, the advertised `--ip`, UDP 7882, and `pub`/`sub` connection states in **Log**.
- HTTP camera/microphone errors: use HTTPS and accept the local test certificate warning.
- Connection closed: copy the **Log** panel (it remains visible after leaving) and core terminal output. A lone proxy `ECONNRESET` does not identify the cause; include what action preceded it.
- After changing the signaling port: set `SFU_CORE` for the Vite proxy as shown in the README.

Report the failed step, browser/OS versions, participant count, client logs from both devices, and core output. Remove private addresses or personal information before posting publicly. Never attach certificates or credentials.
