# Signaling protocol v1

This document describes the current contract between `core` and `test-client`. Version changes to the wire contract must update both implementations and the protocol version.

## Transport and connection model

Connect using WebSocket at `/ws`. Each text message is one JSON object with a required `type`. The core listens on IPv4 TCP 7880 by default. Media uses one shared UDP socket on port 7882; ICE credentials and remote addresses demultiplex connections.

The test client's HTTPS development server proxies `/ws` to `ws://127.0.0.1:7880/ws`. Browsers send media directly to the address advertised by `--ip`. The server uses ICE-lite and embeds its host candidate in SDP. Trickle ICE from clients is optional and ignored; incoming STUN establishes the remote address.

Each participant has two PeerConnections:

- **pub:** the client offers, the server answers. The test client offers one audio and one video `sendonly` transceiver. v0.1 negotiates Opus audio only; the video media line may be rejected/inactive. The client's camera is a separate local preview and is not attached to the publication sender.
- **sub:** the server offers, the client answers. v0.1 creates one server-side `sendonly` audio media line for every other active audio publisher. Departed publishers become inactive; free audio media lines can be reused. Offers wait for the previous answer, accumulating membership changes.

v0.1 forwards encoded Opus without transcoding. VP8 forwarding and video keyframe requests are planned for v0.2, not implemented by v0.1. The video fields in protocol v1 reserve the representation for those later features.

## Client to server

| Type | Fields | Meaning |
|---|---|---|
| `join` | `name: string`, `protocol: 1` | Required first message; name has 1-32 Unicode characters, is not whitespace-only, and contains no control characters |
| `pub_offer` | `sdp: string` | One publication offer per connection in v0.1 |
| `sub_answer` | `sdp: string` | Answer to the outstanding subscription offer |
| `ice` | `pc: "pub" or "sub"`, `candidate: string`, `sdpMid: string or null`, `sdpMLineIndex: number or null` | Optional; currently ignored by the server |
| `state` | `mic: boolean`, `cam: boolean`, `deaf: boolean` | Display state; not server-enforced permissions |
| `leave` | none | Leave, then close the socket; socket closure also removes the participant |
| `ping` | none | Application heartbeat; test client sends every 15 seconds |

## Server to client

| Type | Fields | Meaning |
|---|---|---|
| `joined` | `id: string`, `protocol: 1`, `server: string`, `peers: Peer[]` | Core version and existing peers, excluding self |
| `peer_joined` | `peer: Peer` | Another participant joined |
| `peer_left` | `id: string` | Participant removed |
| `peer_state` | `id: string`, `mic`, `cam`, `deaf` | Updated boolean state fields |
| `pub_answer` | `sdp: string` | Publication answer |
| `sub_offer` | `sdp: string`, `tracks: Track[]` | Complete current track mapping, not a delta |
| `ice` | same candidate fields as client `ice` | Reserved; v0.1 does not send this |
| `pong` | none | Response to `ping` |
| `error` | `message: string`, `fatal: boolean` | Fatal errors are followed by socket closure |

```typescript
type Peer = { id: string; name: string; mic: boolean; cam: boolean; deaf: boolean };
type Track = { mid: string; peer: string; kind: 'audio' | 'video' };
```

IDs are opaque strings scoped to a server run, not authenticated identities. Clients reconcile their subscription transceivers with the full mapping after each answer.

## Sequence

1. Connect and send `join`.
2. Receive `joined`; existing participants receive `peer_joined`.
3. Send `pub_offer`, receive `pub_answer`.
4. Receive `sub_offer` when other audio publications become available; send `sub_answer`.
5. Further membership changes trigger serialized offers. Leave or close the WebSocket to remove the participant and update subscriptions.

## Limits and local controls

Messages and frames are limited to 64 KiB. The room holds at most 16 participants; at most 32 signaling connections are admitted. Each connection allows 60 incoming frames per one-second window. Handshake timeout is 5 seconds, first join timeout is 10 seconds, and signaling writes time out after 2 seconds. Outbound signaling queues hold 32 messages; slow consumers are disconnected. Invalid messages may close the connection without a JSON error response.

Publication SDP allows one audio line and at most one video line. Subscription answers allow up to 15 audio lines. SDP is limited to 1024 lines. v0.1 does not support publication renegotiation.

Deafen is client-side: mute remote audio and disable the microphone; restore its previous state when undeafening. `state.deaf` is informational. Likewise `state.cam` can indicate a local preview, not a remotely available video track. WebSocket closure is cleaned up; media-only disconnection timeouts and automatic reconnection are future work.
