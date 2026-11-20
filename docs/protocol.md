# Signaling protocol v3

This document describes the current contract between `core` and `test-client`. Version changes to the wire contract must update both implementations and the protocol version.

## Transport and connection model

Connect using WebSocket at `/ws`. Each text message is one JSON object with a required `type`. The core listens on IPv4 TCP 7880 by default. Media uses one shared UDP socket on port 7882; ICE credentials and remote addresses demultiplex connections.

The test client's HTTPS development server proxies `/ws` to `ws://127.0.0.1:7880/ws`. Browsers send media directly to the address advertised by `--ip`. The server uses ICE-lite and embeds its host candidate in SDP. Trickle ICE from clients is optional and ignored; incoming STUN establishes the remote address.

Each participant has two PeerConnections:

- **pub:** the client offers the initial publication and the server answers. The server offers subsequent ICE restarts. The test client offers one audio and one video `sendonly` transceiver. v0.3 negotiates Opus and VP8 (with RTX). Camera capture is attached with replaceTrack after the publication answer; toggling does not require a new offer.
- **sub:** the server offers, the client answers. v0.3 creates server-side `sendonly` audio/video media lines for other negotiated publishers. Departed tracks become inactive and release their send streams and retransmission state. Once that offer is answered, a new publisher of the same kind can use the media line with fresh SSRCs. The MID and receiver track may be reused, so clients must use the complete mapping rather than track identity to identify participants. Offers wait for the previous answer, accumulating membership changes.

v0.3 forwards encoded Opus and VP8 without transcoding. Video subscriptions wait for a keyframe. PLI/FIR feedback is routed to the source publisher and coalesced to at most one upstream request per 500 ms. NACK/RTX recovery runs independently on the publication and subscription transports. Upgrade both endpoints together.

## Client to server

| Type | Fields | Meaning |
|---|---|---|
| `join` | `name: string`, `protocol: 3` | Required first message; name has 1-32 Unicode characters, is not whitespace-only, and contains no control characters |
| `pub_offer` | `sdp: string` | One initial publication offer per connection |
| `restart_ice` | `pc: "pub" or "sub"` | Request an ICE restart for an existing transport. The server offers and keeps its local candidate. Subscription changes and restarts share the same offer queue. |
| `pub_restart_answer` | `sdp: string` | Answer to the outstanding publication ICE restart offer |
| `sub_answer` | `sdp: string` | Answer to the outstanding subscription offer |
| `ice` | `pc: "pub" or "sub"`, `candidate: string`, `sdpMid: string or null`, `sdpMLineIndex: number or null` | Optional; currently ignored by the server |
| `state` | `mic: boolean`, `cam: boolean`, `deaf: boolean` | Display state; not server-enforced permissions |
| `leave` | none | Leave, then close the socket; socket closure also removes the participant |
| `ping` | none | Application heartbeat; test client probes every four seconds |

## Server to client

| Type | Fields | Meaning |
|---|---|---|
| `joined` | `id: string`, `protocol: 3`, `server: string`, `peers: Peer[]` | Core version and existing peers, excluding self |
| `peer_joined` | `peer: Peer` | Another participant joined |
| `peer_left` | `id: string` | Participant removed |
| `peer_state` | `id: string`, `mic`, `cam`, `deaf` | Updated boolean state fields |
| `pub_restart_offer` | `sdp: string` | Server offer for ICE restart on the existing publication |
| `pub_answer` | `sdp: string` | Publication answer |
| `sub_offer` | `sdp: string`, `tracks: Track[]` | Complete current track mapping, not a delta |
| `ice` | same candidate fields as client `ice` | Reserved; v0.3 does not send this |
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
4. Receive `sub_offer` when other media publications become available; send `sub_answer`.
5. Further membership changes trigger serialized offers. Leave or close the WebSocket to remove the participant and update subscriptions.

## Limits and local controls

WebSocket messages and frames are limited to 64 KiB. Forwarded media frames are limited to 512 KiB. The room holds at most 16 participants; at most 32 signaling connections are admitted. Each connection allows 60 incoming frames per one-second window. Handshake timeout is 5 seconds, first join timeout is 10 seconds, and signaling writes time out after 2 seconds. Outbound signaling queues hold 32 messages; slow consumers are disconnected. Invalid messages may close the connection without a JSON error response.

Publication SDP allows one audio line and at most one video line. Subscription answers allow up to 15 audio and 15 video lines (30 total), including inactive lines. SDP is limited to 1024 lines. The initial publication layout stays fixed across ICE restarts.

Deafen is client-side: mute remote audio and disable the microphone; restore its previous state when undeafening. `state.deaf` is informational. `state.cam` reports whether local camera publication is enabled; it is not an authorization control. Closing the WebSocket removes the participant immediately.

## Disconnect and recovery

Names are unique within the room after trimming surrounding whitespace. Comparison is case-sensitive. A duplicate join receives a fatal error and the original session continues. A fresh join receives a new ID and the current roster.

The publication must establish ICE and DTLS within 15 seconds of joining. The same deadline starts for the subscription when its first offer is sent. Each outstanding server offer also has a 15-second answer deadline. An unused subscription has no media deadline. An established transport with no incoming traffic for 15 seconds expires even when signaling still works. Losing either required transport removes the whole participant. WebSocket control pings also detect a silently lost signaling path.

The test client gives a disconnected ICE transport two seconds to recover. If it recovers, the current transport continues. If it stays disconnected or enters the failed state, the client requests an ICE restart over the existing socket. If the session closes unexpectedly it tries one fresh join when online and the page is visible. A disconnect in the background reserves this attempt until the user returns to the page. After a failed attempt the user can select **Reconnect**. A full rejoin starts with microphone and camera off. An intentional leave cancels recovery.
