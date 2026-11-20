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

Rust tests cover CLI/input/SDP limits, rate limiting, capacity and cleanup, serialized offers, local WebSocket signaling, and three simulated WebRTC peers exchanging Opus and VP8 over ICE/DTLS. Video tests cover PLI/FIR routing, keyframe gating, ten participant replacements with bounded SDP, request throttling, and recovery of a deliberately dropped RTP packet on each transport leg using NACK/RTX. VP8 fixtures test encoded-byte transport; they are not a visual decoder test.

Recovery tests cover duplicate names, missing negotiation or transport traffic, an idle solo participant and ICE restart on both legs. Client tests cover camera cycles, late permissions, session replacement, terminal rejection, ICE restart and signaling timeout. These tests do not measure audiovisual quality or prove browser compatibility. Node.js 22.22 or newer is required.

## Manual acceptance and regression checklist

1. **Two participants, once:** join from desktop and phone with different names. Confirm two tiles and that the core and test-client versions match the build under test. Speak in each direction for at least 20 seconds. Press **Tap to enable audio** if shown.
2. **Microphone, five cycles on each device:** turn it off and on. Off must stop transmitted voice and show **Mic off** remotely; on must restore voice.
3. **Deafen, five cycles:** remote audio and the local microphone must both stop; the remote tile must show **Deafened**. Undeafen restores audio and the previous microphone state. Also try once with the microphone already off; it must stay off afterward.
4. **Three participants, once:** add a second desktop tab under a third name. All three tiles must appear. Speak from each participant and check the others receive audio. Mute unused tabs sharing the same physical microphone.
5. **Leave/rejoin, ten cycles:** keep two peers in the room while a third leaves and rejoins. Its tile disappears and returns with the correct name; audio resumes. Leave with everyone and check the final core log shows `participants=0`. This checks participant cleanup, not a memory benchmark.
6. **Close a tab, once:** join with two peers and close one tab directly. Its tile must disappear from the remaining peer.
7. **Two-way camera, ten cycles on each device:** enable both cameras. Expect **Updating camera...**, then **Camera on**, local preview and moving remote video under the correct name. Disable/re-enable each camera ten times, waiting for video each time. Off must hide remote video and release capture; on must restore moving video. Speak throughout to check uninterrupted audio.
8. **Late viewer and replacement:** with camera already active, join from a third tab and confirm video appears. Then repeat step 5 with cameras enabled and distinct names; old video must not appear under a replacement participant's name.
9. **Phone background, five cycles:** background the browser for five seconds, then return. With the connection still alive, moving video must return in both directions. If disconnected, verify recovery as described below.
10. **Camera cleanup, once per device:** leave with camera open; its usage indicator must turn off. Deny camera permission once, then allow and retry. If a permission prompt is available, leave before granting permission, then grant it: capture must not remain active.

## v0.5 audio moderation (development)

1. Before starting the development core, set the optional test moderator key in its PowerShell window with `$env:SFU_MODERATOR_KEY = Read-Host -MaskInput 'Test moderator key'` (PowerShell 7.1+, 16–128 UTF-8 bytes). Join from desktop and phone. Check core/client 0.5.0 and protocol 4.
2. On desktop open **Moderator controls**, enter the key and select **Unlock controls**. Select **Server mute** on the phone's tile. Speak into the phone, toggle its mic twice, and keep the camera moving. Both participants should see **Server muted**. The desktop should receive video and hear silence after buffered audio finishes. **Lift mute** should restore speech if the phone mic is on.
3. Turn both cameras off and speak continuously from desktop. On the phone switch **Deafen** on and off twice. After the playback buffer drains, expect incoming audio to stop and the receive bitrate to fall. Undeafening should restore incoming speech. Repeat once while the moderator keeps the phone muted. Its outgoing speech must stay blocked after undeafening.
4. Join a third tab while the phone is muted. The new tab should see the mute badge and receive no speech from the phone. Leave and rejoin as the moderator once. The new session must unlock moderator access again. For failures, record the step, both client logs and matching core output. Omit the moderator key.

## v0.4 network quality (development)

1. Run the development core and client, join from desktop and phone, and open **Network quality** on both. Turn on both cameras. Speak and move for 30 seconds. Record send/receive kbps, round-trip time (RTT), estimated upload capacity and packet loss on each device. Send loss shows the largest recent loss fraction reported by the SFU for an active outgoing stream. Receive loss combines the incoming streams over the sampling interval. A dash means there is no measurement yet.
2. With a packet shaper restricted to the phone's media UDP traffic on port 7882, introduce 100 ms of delay in each direction (200 ms added round trip) and 5% packet loss. Speak a sentence in each direction and keep moving for 30 seconds. Check whether the listener can repeat it and see motion. Remove the impairment and allow 60 seconds for recovery. Repeat with 15% loss. Run each profile twice.
3. Check whether the sender reduces its video rate under impairment and whether rate and picture recover with the link. Use comparable motion throughout. Record any freeze and its duration. The server logs `Transport quality` every five seconds per connected leg with the participant ID, cumulative transport bytes and available feedback.
4. Run the media and recovery regression checklists. For failures, include the profile, affected direction, both quality readings, browser **Log** and core output from the same time.

The publisher uses negotiated transport-wide congestion feedback and requests high priority for audio. The VP8 sender prefers to preserve frame rate. Opus advertises in-band FEC and the browser decides when to encode redundant information. After an audio packet is lost, the SFU waits up to 60 ms to reorder subsequent packets. The baseline keeps RED disabled so its additional bandwidth and audio benefit can be compared separately.

## v0.3 recovery acceptance

Use the v0.3.0 release for both processes and refresh both browsers. Check that the footer shows protocol 3. Also run the regression checklist above.

1. Join under the same name from a third tab once. Expect a visible rejection and uninterrupted media for the original participant. Close the original tab, then join under that name again. Expect exactly one tile for it.
2. Put the phone into airplane mode for 5 seconds and then for 30 seconds, twice each. Restore Wi-Fi on the same LAN. During a long outage the desktop should remove the tile within 15 seconds. On return expect one working session or **Reconnect** after one failed attempt. A full rejoin starts with camera and mic off. Turn them back on and check both directions.
3. Turn Wi-Fi off and back on twice. If both networks can route to the advertised server, also switch between them. Watch **Log** for ICE restart or rejoin. An isolated cellular network can only recover once the LAN is reachable again.
4. Stop the core with Ctrl+C with two browsers connected. Do this twice, restarting promptly on the first cycle and waiting for **Reconnect** on the second. Leave Vite running. Reconnect as needed and check two tiles, working media, and no repeated join attempts while the core is down.
5. Background the phone once for two minutes. Return to the page. If the session closed in the background, expect one automatic rejoin after returning online to the visible page. Check the log for a new join, with microphone and camera off. Use **Reconnect** if the attempt fails, then leave with both devices. Expect capture to stop and the core to report `participants=0`.

## v0.3 acceptance result

On 20 November 2026 the tester confirmed the desktop and phone recovery checklist and regression tests passed. On the tested phone, a session disconnected in the background and rejoined automatically when the page became visible. The microphone and camera were off after rejoining.

## v0.2 acceptance result

The tester confirmed desktop/phone camera, audio, mute/deafen, three-participant, participant replacement, and capture cleanup checks passed. Background camera suspension followed by recovery on return is expected on the tested phone. Longer background periods caused a disconnect; this remains a documented limitation, with recovery planned for v0.3. This acceptance does not claim uninterrupted mobile background operation or compatibility with all devices.

## Troubleshooting

- Phone cannot load the page: check the LAN address, port 5173, private-network firewall permissions, and Wi-Fi client isolation.
- Page loads but no audio: check microphone permission, autoplay unlock, the advertised `--ip`, UDP 7882, and `pub`/`sub` connection states in **Log**.
- HTTP camera/microphone errors: use HTTPS and accept the local test certificate warning.
- Connection closed: copy the **Log** panel (it remains visible after leaving) and core terminal output. A lone proxy `ECONNRESET` does not identify the cause; include what action preceded it.
- After changing the signaling port: set `SFU_CORE` for the Vite proxy as shown in the README.

Report the failed step, browser/OS versions, participant count, client logs from both devices, and core output. Remove private addresses or personal information before posting publicly. Never attach certificates or credentials.
