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

Fourteen Rust tests cover CLI/input/SDP limits, rate limiting, capacity and cleanup, serialized offers, local WebSocket signaling, and three simulated WebRTC peers exchanging Opus and VP8 over ICE/DTLS. Video tests cover PLI/FIR routing, keyframe gating, ten participant replacements with bounded SDP, request throttling, and recovery of a deliberately dropped RTP packet on each transport leg using NACK/RTX. VP8 fixtures test encoded-byte transport; they are not a visual decoder test.

Eight client tests use mocked capture/publication to cover camera cycles, late permissions, session replacement, rejection, and cleanup. These tests do not measure audiovisual quality or prove browser compatibility. Node.js 22.22 or newer is required.

## Manual acceptance and regression checklist

1. **Two participants, once:** join from desktop and phone with different names. Confirm two tiles and core version `0.2.0`. Speak in each direction for at least 20 seconds. Press **Tap to enable audio** if shown.
2. **Microphone, five cycles on each device:** turn it off and on. Off must stop transmitted voice and show **Mic off** remotely; on must restore voice.
3. **Deafen, five cycles:** remote audio and the local microphone must both stop; the remote tile must show **Deafened**. Undeafen restores audio and the previous microphone state. Also try once with the microphone already off; it must stay off afterward.
4. **Three participants, once:** add a second desktop tab under a third name. All three tiles must appear. Speak from each participant and check the others receive audio. Mute unused tabs sharing the same physical microphone.
5. **Leave/rejoin, ten cycles:** keep two peers in the room while a third leaves and rejoins. Its tile disappears and returns with the correct name; audio resumes. Leave with everyone and check the final core log shows `participants=0`. This checks participant cleanup, not a memory benchmark.
6. **Close a tab, once:** join with two peers and close one tab directly. Its tile must disappear from the remaining peer.
7. **Two-way camera, ten cycles on each device:** enable both cameras. Expect **Updating camera...**, then **Camera on**, local preview and moving remote video under the correct name. Disable/re-enable each camera ten times, waiting for video each time. Off must hide remote video and release capture; on must restore moving video. Speak throughout to check uninterrupted audio.
8. **Late viewer and replacement:** with camera already active, join from a third tab and confirm video appears. Then repeat step 5 with cameras enabled and distinct names; old video must not appear under a replacement participant's name.
9. **Phone background, five cycles:** background the browser for five seconds, then return. With the connection still alive, moving video must return in both directions. Record any disconnect separately: automatic reconnection is planned for v0.3.
10. **Camera cleanup, once per device:** leave with camera open; its usage indicator must turn off. Deny camera permission once, then allow and retry. If a permission prompt is available, leave before granting permission, then grant it: capture must not remain active.

Repeat all steps for v0.2 acceptance. Restart both processes and close/refresh all old tabs: protocol 1 and protocol 2 clients/servers cannot be mixed.

## v0.2 acceptance result

The tester confirmed desktop/phone camera, audio, mute/deafen, three-participant, participant replacement, and capture cleanup checks passed. Background camera suspension followed by recovery on return is expected on the tested phone. Longer background periods caused a disconnect; this remains a documented limitation, with recovery planned for v0.3. This acceptance does not claim uninterrupted mobile background operation or compatibility with all devices.

## Troubleshooting

- Phone cannot load the page: check the LAN address, port 5173, private-network firewall permissions, and Wi-Fi client isolation.
- Page loads but no audio: check microphone permission, autoplay unlock, the advertised `--ip`, UDP 7882, and `pub`/`sub` connection states in **Log**.
- HTTP camera/microphone errors: use HTTPS and accept the local test certificate warning.
- Connection closed: copy the **Log** panel (it remains visible after leaving) and core terminal output. A lone proxy `ECONNRESET` does not identify the cause; include what action preceded it.
- After changing the signaling port: set `SFU_CORE` for the Vite proxy as shown in the README.

Report the failed step, browser/OS versions, participant count, client logs from both devices, and core output. Remove private addresses or personal information before posting publicly. Never attach certificates or credentials.
