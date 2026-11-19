# Changelog

## 0.2.0 - 2026-11-19

- Forward VP8 camera frames alongside Opus without transcoding.
- Wait for a keyframe on new video subscriptions; route PLI/FIR to the correct publisher with a 500 ms request limit.
- Recover video packet loss with NACK/RTX on each transport leg.
- Reuse inactive media lines after negotiation, resetting SSRCs and retransmission buffers when a source leaves.
- Publish camera capture only after negotiation; release capture on publication failure or departure.
- Advance to protocol 2, allowing up to 15 audio and 15 video subscriptions. Upgrade server and client together.
- Add simulated VP8 transport, feedback, packet-loss recovery, churn, and camera lifecycle regression tests.

Automated validation passed: 14 Rust tests, 8 camera lifecycle tests, formatting, Clippy, client build, and Windows executable smoke tests. Desktop/phone acceptance and audio regression tests were confirmed by the tester, including camera toggles, three participants, participant replacement, and capture cleanup.

Known limitation: mobile browsers may suspend camera capture in the background. Video resumes on return if the connection survives; longer background periods can disconnect the session. Android stopping Chrome's media service was observed before one disconnect, but the initiating cause of the WebSocket closure was not conclusively isolated. Automatic recovery is planned for v0.3; rejoin manually in v0.2.

Windows x64 is the only prebuilt platform. Trusted LAN only; no authentication or production Internet deployment.

## 0.1.0 - 2026-11-19

Initial public release, licensed under Apache-2.0.

### Added

- Single-room Opus audio forwarding with separate publish/subscribe PeerConnections.
- ICE-lite and a shared UDP socket; configurable advertised IPv4 address and signaling/media ports.
- Protocol v1 WebSocket signaling, participant membership/state, ping/pong, and serialized subscription negotiation.
- Bounded messages, frame rate, connections, room capacity, SDP complexity, and signaling queues.
- English browser test client with microphone, local deafen, local camera preview, and persistent log panel.
- Nine Rust tests and five camera lifecycle tests.
- Windows x64 server release archive, test-client source, SHA-256 checksum, and dependency license notices.

### Fixed before publication

- Local camera preview no longer depends on unsupported video publication.
- Camera capture is released when closing the preview or leaving, including late permission responses.
- Participant count updates immediately when another participant joins.

### Validation and limitations

Automated checks passed. Two/three-participant audio, mute/deafen, join/leave cleanup, and the camera preview fix were manually confirmed on desktop and phone before publication. The publication update renames and translates the client; it does not add media features.

Audio only; camera video is not sent. One room, at most 16 participants. Automatic reconnection, media-failure timeout cleanup, authentication, and Internet deployment are future work. Windows x64 is the only prebuilt platform in this release.
