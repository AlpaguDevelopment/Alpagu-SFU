# Roadmap

Build a small working media path first, then improve reliability, control, coverage, and measurement. Each milestone must pass automated checks and desktop/phone regression tests before the next begins. These are planned capabilities, not promises or current features.

**v0.3.0** adds disconnect cleanup and reconnection. Automated checks and desktop/phone acceptance passed. Mobile browsers can still suspend capture in the background. The next milestone is v0.4 network quality.

| Version | Milestone | Acceptance focus |
|---|---|---|
| 0.1 | Single-room Opus audio, ICE-lite, shared UDP, bounded signaling, local deafen | Two/three participants; mute/deafen; ten join/leave cycles; zero remaining participants |
| 0.2 | VP8 camera forwarding, keyframe requests, media-line reuse, NACK/RTX | Bidirectional video, ten camera toggles, correct tracks after participant replacement |
| 0.3 | Disconnect cleanup, media timeouts, reconnect policy, ICE restart | Tab closure, airplane mode, Wi-Fi changes, server restart without ghost peers |
| 0.4 | Congestion feedback and bandwidth estimation, audio FEC/RED evaluation, quality metrics | Understandable audio and recovering video under packet loss and delay |
| 0.5 | Server-enforced mute/deafen, bitrate limits, subscription permissions | Unauthorized media is never forwarded; client state cannot override server policy |
| 0.6 | Screen video and screen audio as separate tracks | Desktop sharing to a phone while camera/audio remain active |
| 0.7 | Signed access tokens, multiple rooms, management API, signed webhooks, metadata, active speaker, data messages | Access control, room isolation, management operations and webhook verification |
| 0.8 | Metrics, headless load client, Linux/Docker validation, comparative benchmarks | CPU, memory, latency, packet-loss behavior and reconnect measurements |
| 0.9 | Internet deployment, signaling TLS, NAT/TURN, mobile-network testing, security review and fuzzing | End-to-end operation beyond a trusted LAN |
| 1.0 | Reusable TypeScript client, web/Electron and React Native integration readiness | Documented integration contract and application compatibility |

The source is public from v0.1 onward. Production integration readiness is a later milestone.

Recording/egress, SIP, simulcast, end-to-end encryption, multi-node routing, and server-side audio processing are outside the current research scope.
