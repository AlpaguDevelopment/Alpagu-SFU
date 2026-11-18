# Changelog

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
