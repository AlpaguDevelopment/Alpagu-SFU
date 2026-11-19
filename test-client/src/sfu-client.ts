// Signaling and two PeerConnections (pub/sub). Contract: ../../docs/protocol.md
export const PROTOCOL = 2;

export type Peer = { id: string; name: string; mic: boolean; cam: boolean; deaf: boolean };
export type Kind = 'audio' | 'video';
type Track = { mid: string; peer: string; kind: Kind };
type PcName = 'pub' | 'sub';

type ServerMsg =
  | { type: 'joined'; id: string; protocol: number; server: string; peers: Peer[] }
  | { type: 'peer_joined'; peer: Peer }
  | { type: 'peer_left'; id: string }
  | { type: 'peer_state'; id: string; mic: boolean; cam: boolean; deaf: boolean }
  | { type: 'pub_answer'; sdp: string }
  | { type: 'sub_offer'; sdp: string; tracks: Track[] }
  | { type: 'ice'; pc: PcName; candidate: string; sdpMid: string | null; sdpMLineIndex: number | null }
  | { type: 'pong' }
  | { type: 'error'; message: string; fatal: boolean };

export type Events = {
  published(): void;
  joined(selfId: string, server: string, peers: Peer[]): void;
  peerJoined(peer: Peer): void;
  peerLeft(id: string): void;
  peerState(id: string, s: { mic: boolean; cam: boolean; deaf: boolean }): void;
  /** A null track removes that media kind for the peer. */
  remoteTrack(peerId: string, kind: Kind, track: MediaStreamTrack | null): void;
  pcState(pc: PcName, state: RTCPeerConnectionState): void;
  closed(reason: string): void;
  log(line: string): void;
};

const RTC_CONFIG: RTCConfiguration = { iceServers: [], bundlePolicy: 'max-bundle' };
const VIDEO_MAX_BITRATE = 1_200_000;

export class SfuClient {
  private ws: WebSocket | null = null;
  private pub: RTCPeerConnection | null = null;
  private sub: RTCPeerConnection | null = null;
  private audioSender: RTCRtpSender | null = null;
  private videoSender: RTCRtpSender | null = null;
  private subChain: Promise<void> = Promise.resolve();
  private tracks: Track[] = [];
  /** "peer:kind" -> currently assigned track. */
  private assigned = new Map<string, MediaStreamTrack>();
  private pingTimer: number | undefined;
  private closedOnce = false;
  private publishReady = false;
  private ev: Events;

  constructor(ev: Events) { this.ev = ev; }

  canPublishVideo() { return this.publishReady && !this.closedOnce; }

  join(name: string, micTrack: MediaStreamTrack | null) {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    this.ws = ws;
    ws.onopen = () => {
      this.ev.log('ws open -> join');
      this.send({ type: 'join', name, protocol: PROTOCOL });
      this.pingTimer = window.setInterval(() => this.send({ type: 'ping' }), 15_000);
    };
    ws.onmessage = (e) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(String(e.data));
      } catch {
        this.ev.log('Invalid message: ' + String(e.data).slice(0, 120));
        return;
      }
      this.handle(msg, micTrack).catch((err) => this.ev.log('Error: ' + errText(err)));
    };
    ws.onerror = () => this.ev.log('WebSocket error');
    ws.onclose = (e) => this.shutdown(`Signaling closed (${e.code}${e.reason ? ' ' + e.reason : ''})`);
  }

  leave() {
    this.send({ type: 'leave' });
    this.shutdown('Left room');
  }

  sendState(s: { mic: boolean; cam: boolean; deaf: boolean }) {
    this.send({ type: 'state', ...s });
  }

  async setMic(track: MediaStreamTrack | null) {
    await this.audioSender?.replaceTrack(track);
  }

  async setCamera(track: MediaStreamTrack | null) {
    if (!this.videoSender || !this.canPublishVideo()) throw new Error('Publication is not ready');
    await this.videoSender.replaceTrack(track);
    if (!track) return;
    try {
      const p = this.videoSender.getParameters();
      if (p.encodings?.length) {
        p.encodings[0].maxBitrate = VIDEO_MAX_BITRATE;
        await this.videoSender.setParameters(p);
      }
    } catch (err) {
      this.ev.log('Could not set bitrate limit: ' + errText(err));
    }
  }

  private async handle(msg: ServerMsg, micTrack: MediaStreamTrack | null) {
    switch (msg.type) {
      case 'joined':
        if (msg.protocol !== PROTOCOL) {
          this.shutdown(`Protocol mismatch: client ${PROTOCOL}, server ${msg.protocol}`);
          return;
        }
        this.ev.joined(msg.id, msg.server, msg.peers);
        await this.startPublish(micTrack);
        return;
      case 'peer_joined':
        return this.ev.peerJoined(msg.peer);
      case 'peer_left':
        return this.ev.peerLeft(msg.id);
      case 'peer_state':
        return this.ev.peerState(msg.id, msg);
      case 'pub_answer':
        await this.pub?.setRemoteDescription({ type: 'answer', sdp: msg.sdp });
        this.publishReady = true;
        this.ev.published();
        this.ev.log('pub answer applied');
        return;
      case 'sub_offer':
        // Process offers sequentially, completing each before starting the next.
        this.subChain = this.subChain.then(() => this.answerSub(msg.sdp, msg.tracks)).catch((err) => {
          this.ev.log('sub negotiation error: ' + errText(err));
        });
        return;
      case 'ice': {
        const pc = msg.pc === 'pub' ? this.pub : this.sub;
        await pc?.addIceCandidate({ candidate: msg.candidate, sdpMid: msg.sdpMid, sdpMLineIndex: msg.sdpMLineIndex });
        return;
      }
      case 'pong':
        return;
      case 'error':
        this.ev.log(`Server error: ${msg.message}${msg.fatal ? ' (fatal)' : ''}`);
        return;
    }
  }

  private async startPublish(micTrack: MediaStreamTrack | null) {
    const pc = this.makePc('pub');
    this.pub = pc;
    this.audioSender = pc.addTransceiver('audio', { direction: 'sendonly' }).sender;
    this.videoSender = pc.addTransceiver('video', { direction: 'sendonly' }).sender;
    if (micTrack) await this.audioSender.replaceTrack(micTrack);
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    this.send({ type: 'pub_offer', sdp: pc.localDescription!.sdp });
    this.ev.log('pub offer sent');
  }

  private async answerSub(sdp: string, tracks: Track[]) {
    if (!this.sub) this.sub = this.makePc('sub');
    const pc = this.sub;
    this.tracks = tracks;
    await pc.setRemoteDescription({ type: 'offer', sdp });
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    this.send({ type: 'sub_answer', sdp: pc.localDescription!.sdp });
    this.ev.log(`sub answer sent (${tracks.length} tracks)`);
    this.syncRemoteTracks();
  }

  /** Reconcile transceivers against the server's complete track mapping. */
  private syncRemoteTracks() {
    if (!this.sub) return;
    const next = new Map<string, MediaStreamTrack>();
    for (const t of this.sub.getTransceivers()) {
      if (!t.mid) continue;
      const dir = t.currentDirection ?? t.direction;
      if (dir !== 'recvonly' && dir !== 'sendrecv') continue;
      const info = this.tracks.find((x) => x.mid === t.mid);
      if (!info) continue;
      next.set(`${info.peer}:${info.kind}`, t.receiver.track);
    }
    for (const [key, track] of this.assigned) {
      if (next.get(key) !== track) {
        const [peer, kind] = splitKey(key);
        this.ev.remoteTrack(peer, kind, null);
      }
    }
    for (const [key, track] of next) {
      if (this.assigned.get(key) !== track) {
        const [peer, kind] = splitKey(key);
        this.ev.remoteTrack(peer, kind, track);
      }
    }
    this.assigned = next;
  }

  private makePc(name: PcName) {
    const pc = new RTCPeerConnection(RTC_CONFIG);
    pc.onicecandidate = (e) => {
      if (!e.candidate) return;
      this.send({
        type: 'ice',
        pc: name,
        candidate: e.candidate.candidate,
        sdpMid: e.candidate.sdpMid,
        sdpMLineIndex: e.candidate.sdpMLineIndex,
      });
    };
    pc.onconnectionstatechange = () => {
      this.ev.log(`${name} connection: ${pc.connectionState}`);
      this.ev.pcState(name, pc.connectionState);
    };
    pc.oniceconnectionstatechange = () => this.ev.log(`${name} ice: ${pc.iceConnectionState}`);
    return pc;
  }

  private send(obj: Record<string, unknown>) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(obj));
  }

  private shutdown(reason: string) {
    if (this.closedOnce) return;
    this.closedOnce = true;
    this.publishReady = false;
    window.clearInterval(this.pingTimer);
    this.pub?.close();
    this.sub?.close();
    const ws = this.ws;
    this.ws = null;
    if (ws && ws.readyState <= WebSocket.OPEN) ws.close();
    this.ev.log(reason);
    this.ev.closed(reason);
  }
}

function splitKey(key: string): [string, Kind] {
  const i = key.lastIndexOf(':');
  return [key.slice(0, i), key.slice(i + 1) as Kind];
}

export function errText(err: unknown) {
  return err instanceof Error ? `${err.name}: ${err.message}` : String(err);
}
