import './style.css';
import { LocalCamera } from './local-camera';
import { monitorQuality } from './quality';
import { scheduleRejoin } from './rejoin';
import { SfuClient, errText, PROTOCOL, type Kind, type Peer } from './sfu-client';

const TEST_CLIENT_VERSION = '0.5.0';
const NAME_KEY = 'sfu-test-name';

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const lobby = $('lobby');
const room = $('room');
const nameInput = $<HTMLInputElement>('name');
const joinBtn = $<HTMLButtonElement>('join');
const lobbyError = $('lobby-error');
const grid = $('grid');
const statusEl = $('status');
const countEl = $('count');
const logEl = $('log');
const unlockBtn = $<HTMLButtonElement>('unlock-audio');
const btnMic = $<HTMLButtonElement>('btn-mic');
const btnCam = $<HTMLButtonElement>('btn-cam');
const btnDeaf = $<HTMLButtonElement>('btn-deaf');
const btnLeave = $<HTMLButtonElement>('btn-leave');
const cameraStatus = $('camera-status');
const localCamera = new LocalCamera();
const moderatorLogin = $<HTMLFormElement>('moderator-login');
const moderatorKey = $<HTMLInputElement>('moderator-key');
let moderator = false;
let selfServerMuted = false;

$('versions').textContent = `test-client ${TEST_CLIENT_VERSION} · protocol ${PROTOCOL}`;

// ---------- log ----------
const logLines: string[] = [];
function log(line: string) {
  const t = new Date().toLocaleTimeString('en-US', { hour12: false });
  logLines.push(`${t} ${line}`);
  if (logLines.length > 300) logLines.shift();
  logEl.textContent = logLines.join('\n');
  logEl.scrollTop = logEl.scrollHeight;
}

// ---------- tiles ----------
type Tile = {
  info: Peer;
  el: HTMLElement;
  video: HTMLVideoElement;
  audio: HTMLAudioElement | null;
  avatar: HTMLElement;
  badges: HTMLElement;
  muteButton: HTMLButtonElement;
  videoTrack: MediaStreamTrack | null;
};
const tiles = new Map<string, Tile>();
const SELF = '__self__';

function makeTile(info: Peer, isSelf: boolean): Tile {
  const el = document.createElement('div');
  el.className = 'tile' + (isSelf ? ' self' : '');
  const video = document.createElement('video');
  video.autoplay = true;
  video.playsInline = true;
  video.muted = true; // Keep video muted; audio uses a separate element.
  const avatar = document.createElement('div');
  avatar.className = 'avatar';
  const label = document.createElement('div');
  label.className = 'label';
  const nameEl = document.createElement('span');
  nameEl.textContent = info.name + (isSelf ? ' (you)' : '');
  const badges = document.createElement('span');
  badges.className = 'badges';
  const muteButton = document.createElement('button');
  muteButton.onclick = () => client?.setServerMute(info.id, !tile.info.serverMuted);
  label.append(nameEl, badges, muteButton);
  el.append(video, avatar, label);
  grid.append(el);
  const tile: Tile = { info, el, video, audio: null, avatar, badges, muteButton, videoTrack: null };
  renderTile(tile);
  return tile;
}

function renderTile(tile: Tile) {
  const { info } = tile;
  tile.avatar.textContent = (info.name.trim()[0] ?? '?').toLocaleUpperCase('en-US');
  const showVideo = info.cam && tile.videoTrack !== null;
  tile.el.classList.toggle('has-video', showVideo);
  const b: string[] = [];
  if (!info.mic) b.push('Mic off');
  if (info.deaf) b.push('Deafened');
  if (info.serverMuted) b.push('Server muted');
  tile.muteButton.hidden = !moderator || info.id === SELF;
  tile.muteButton.textContent = info.serverMuted ? 'Lift mute' : 'Server mute';
  tile.muteButton.setAttribute('aria-label', `${tile.muteButton.textContent} · ${info.name}`);
  tile.badges.textContent = b.join(' · ');
  countEl.textContent = `${tiles.size} participants`;
}

function removeTile(id: string) {
  const tile = tiles.get(id);
  if (!tile) return;
  tile.video.srcObject = null;
  if (tile.audio) tile.audio.srcObject = null;
  tile.audio?.remove();
  tile.el.remove();
  tiles.delete(id);
  countEl.textContent = `${tiles.size} participants`;
}

function playSafe(el: HTMLMediaElement) {
  el.play().catch((err) => {
    if (!el.isConnected || !el.srcObject || (err instanceof DOMException && err.name === 'AbortError')) return;
    log('Playback blocked: ' + errText(err));
    if (el instanceof HTMLAudioElement) unlockBtn.hidden = false;
  });
}

unlockBtn.onclick = () => {
  unlockBtn.hidden = true;
  for (const t of tiles.values()) if (t.audio) playSafe(t.audio);
};

// ---------- local state ----------
let client: SfuClient | null = null;
let micTrack: MediaStreamTrack | null = null;
let camTrack: MediaStreamTrack | null = null;
let mic = false;
let cam = false;
let deaf = false;
let micBeforeDeaf = false;
let busy = false;
let micPending = false;
let cancelRejoin: (() => void) | undefined;
let manualReconnect = false;
let autoRetryAvailable = true;
let stableTimer: number | undefined;
let stopQuality: (() => void) | undefined;
const metric = (value?: number, digits = 0) => value?.toFixed(digits) ?? '—';

function selfInfo(): Peer {
  return { id: SELF, name: nameInput.value.trim(), mic, cam, deaf, serverMuted: selfServerMuted };
}

function renderControls() {
  setBtn(btnMic, mic, 'Microphone on', 'Microphone off');
  setBtn(btnCam, cam, 'Camera on', 'Camera off');
  if (busy) btnCam.textContent = 'Updating camera…';
  setBtn(btnDeaf, !deaf, 'Deafen', 'Undeafen');
  btnMic.disabled = deaf || busy || micPending || !client?.canPublishVideo();
  btnDeaf.disabled = !client || micPending;
  btnCam.disabled = busy || !client?.canPublishVideo();
  const self = tiles.get(SELF);
  if (self) {
    self.info = selfInfo();
    self.videoTrack = camTrack;
    renderTile(self);
  }
  for (const t of tiles.values()) if (t.audio) t.audio.muted = deaf;
}

function setBtn(btn: HTMLButtonElement, on: boolean, onText: string, offText: string) {
  btn.textContent = on ? onText : offText;
  btn.classList.toggle('off', !on);
}

function pushState() {
  renderControls();
  client?.sendState({ mic, cam, deaf });
}

// ---------- join / leave ----------
try {
  nameInput.value = localStorage.getItem(NAME_KEY) ?? '';
} catch {
  /* Storage may be disabled. */
}

if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
  showLobbyError('Camera and microphone access requires HTTPS (https://<server-IP>:5173).');
  joinBtn.disabled = true;
}

function showLobbyError(text: string) {
  lobbyError.textContent = text;
  lobbyError.hidden = false;
}

nameInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') joinBtn.click();
});

joinBtn.onclick = () => {
  autoRetryAvailable = true;
  void enterRoom(manualReconnect);
};

async function enterRoom(reconnecting: boolean) {
  const name = nameInput.value.trim();
  if (!name) return showLobbyError('Enter a name.');
  lobbyError.hidden = true;
  try {
    localStorage.setItem(NAME_KEY, name);
  } catch {
    /* Ignore unavailable storage. */
  }
  joinBtn.disabled = true;

  if (!reconnecting) {
    try {
      const s = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      });
      micTrack = s.getAudioTracks()[0] ?? null;
      mic = micTrack !== null;
    } catch (err) {
      log('Could not acquire microphone: ' + errText(err));
      micTrack = null;
      mic = false;
    }
  }
  cam = false;
  deaf = false;

  lobby.hidden = true;
  room.hidden = false;
  statusEl.textContent = 'Connecting…';
  tiles.set(SELF, makeTile(selfInfo(), true));
  renderControls();

  let connectedStatus = '';
  let joined = false;
  let sessionId = '';
  const session = new SfuClient({
    moderatorGranted() {
      moderator = true;
      moderatorLogin.hidden = true;
      $('moderator-status').hidden = false;
      for (const tile of tiles.values()) renderTile(tile);
    },
    serverMute(id, muted) {
      if (id === sessionId) selfServerMuted = muted;
      const tile = tiles.get(id === sessionId ? SELF : id);
      if (tile) { tile.info.serverMuted = muted; renderTile(tile); }
    },
    published() { renderControls(); },
    joined(selfId, server, peers) {
      joined = true;
      sessionId = selfId;
      $('moderation').hidden = false;
      connectedStatus = `Connected · core ${server}`;
      stopQuality = monitorQuality((leg) => session.transportStats(leg), (up, down) => {
        $('upload-quality').textContent = `Send ${metric(up.bitrateKbps)} kbps · RTT ${metric(up.rttMs)} ms · peak stream loss ${metric(up.lossPct, 1)}% · estimated capacity ${metric(up.availableKbps)} kbps`;
        $('download-quality').textContent = `Receive ${metric(down.bitrateKbps)} kbps · RTT ${metric(down.rttMs)} ms · loss ${metric(down.lossPct, 1)}%`;
      });
      statusEl.textContent = connectedStatus;
      log(`Joined (id ${selfId}), ${peers.length} other peers`);
      for (const p of peers) tiles.set(p.id, makeTile(p, false));
      client?.sendState({ mic, cam, deaf });
      renderControls();
    },
    peerJoined(p) {
      log(`${p.name} joined`);
      if (!tiles.has(p.id)) tiles.set(p.id, makeTile(p, false));
      countEl.textContent = `${tiles.size} participants`;
    },
    peerLeft(id) {
      log(`${tiles.get(id)?.info.name ?? id} left`);
      removeTile(id);
    },
    peerState(id, s) {
      const t = tiles.get(id);
      if (!t) return;
      t.info = { ...t.info, ...s };
      renderTile(t);
    },
    remoteTrack(peerId, kind, track) {
      attachRemote(peerId, kind, track);
    },
    pcState(pc, state) {
      window.clearTimeout(stableTimer);
      if (session.mediaConnected()) {
        statusEl.textContent = connectedStatus;
        stableTimer = window.setTimeout(() => { autoRetryAvailable = true; }, 15_000);
      } else if (state === 'disconnected' || state === 'failed') {
        statusEl.textContent = `Restoring media (${pc})...`;
      }
    },
    closed(reason, retryable) {
      if (client !== session) return;
      resetSession();
      if (joined && retryable && autoRetryAvailable) {
        autoRetryAvailable = false;
        statusEl.textContent = 'Connection lost. Waiting to reconnect...';
        cancelRejoin = scheduleRejoin(() => {
          cancelRejoin = undefined;
          log('Rejoining automatically');
          void enterRoom(true);
        });
      } else {
        backToLobby(reason, retryable || reconnecting);
      }
    },
    log,
  });
  client = session;
  session.join(name, micTrack);
}

function attachRemote(peerId: string, kind: Kind, track: MediaStreamTrack | null) {
  const t = tiles.get(peerId);
  if (!t) {
    log(`Track received for unknown peer: ${peerId}/${kind}`);
    return;
  }
  if (kind === 'video') {
    t.videoTrack = track;
    t.video.srcObject = track ? new MediaStream([track]) : null;
    if (track) playSafe(t.video);
  } else {
    if (!t.audio) {
      t.audio = document.createElement('audio');
      t.audio.autoplay = true;
      document.body.append(t.audio);
    }
    t.audio.srcObject = track ? new MediaStream([track]) : null;
    t.audio.muted = deaf;
    if (track) playSafe(t.audio);
  }
  renderTile(t);
}

btnLeave.onclick = () => {
  if (client) client.leave();
  else backToLobby('Left room', false);
};

function resetSession() {
  moderator = selfServerMuted = false;
  moderatorLogin.reset();
  moderatorLogin.hidden = false;
  $('moderator-status').hidden = true;
  $('moderation').hidden = true;
  stopQuality?.();
  stopQuality = undefined;
  $('upload-quality').textContent = '';
  $('download-quality').textContent = '';
  window.clearTimeout(stableTimer);
  client = null;
  micTrack?.stop();
  localCamera.stop();
  busy = false;
  micPending = false;
  cameraStatus.textContent = '';
  micTrack = camTrack = null;
  mic = cam = deaf = false;
  for (const id of [...tiles.keys()]) removeTile(id);
  unlockBtn.hidden = true;
  renderControls();
}

function backToLobby(reason: string, reconnect: boolean) {
  cancelRejoin?.();
  cancelRejoin = undefined;
  room.hidden = true;
  lobby.hidden = false;
  manualReconnect = reconnect && reason !== 'Left room';
  joinBtn.textContent = manualReconnect ? 'Reconnect' : 'Join';
  joinBtn.disabled = false;
  if (reason !== 'Left room') showLobbyError(reason);
}

// ---------- controls ----------
moderatorLogin.onsubmit = (event) => {
  event.preventDefault();
  client?.claimModerator(moderatorKey.value);
  moderatorLogin.reset();
};

btnMic.onclick = async () => {
  const session = client;
  if (deaf || micPending || !session?.canPublishVideo()) return;
  if (!micTrack) {
    micPending = true;
    renderControls();
    let track: MediaStreamTrack | undefined;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      track = stream.getAudioTracks()[0];
      if (!track || client !== session) return;
      await session.setMic(track);
      if (client !== session) return;
      micTrack = track;
      mic = true;
    } catch (err) {
      log('Could not enable microphone: ' + errText(err));
    } finally {
      if (track !== micTrack) track?.stop();
      if (client === session) {
        micPending = false;
        renderControls();
      }
    }
    if (client !== session) return;
  } else {
    mic = !mic;
    micTrack.enabled = mic; // Disabled tracks send silence without renegotiation.
  }
  pushState();
};

btnCam.onclick = async () => {
  if (busy || !client) return;
  const session = client;
  cameraStatus.textContent = '';
  if (cam) {
    busy = true;
    localCamera.stop();
    camTrack = null;
    cam = false;
    const self = tiles.get(SELF);
    if (self) self.video.srcObject = null;
    log('Camera capture stopped');
    pushState();
    try { await session.setCamera(null); } catch (err) { log('Camera detach: ' + errText(err)); }
    finally { if (client === session) { busy = false; renderControls(); } }
    return;
  }
  busy = true;
  renderControls();
  try {
    // Capture ownership survives publication errors and late permission responses.
    const track = await localCamera.start(() => navigator.mediaDevices.getUserMedia({
      video: { width: { ideal: 640 }, height: { ideal: 480 }, frameRate: { ideal: 30 }, facingMode: 'user' },
    }), (track) => session.setCamera(track));
    if (!track || client !== session) return;
    camTrack = track;
    cam = true;
    const self = tiles.get(SELF);
    if (self) {
      self.video.srcObject = new MediaStream([track]);
      playSafe(self.video);
    }
    cameraStatus.textContent = 'Camera is being shared with the room.';
    log('Camera publication started');
  } catch (err) {
    if (client !== session) return;
    localCamera.stop();
    camTrack = null;
    cam = false;
    const self = tiles.get(SELF);
    if (self) self.video.srcObject = null;
    cameraStatus.textContent = 'Could not open the camera. Check permission and retry; see the Log panel for details.';
    log('Camera: ' + errText(err));
  } finally {
    if (client === session) {
      busy = false;
      pushState();
    }
  }
};

btnDeaf.onclick = () => {
  if (!deaf) {
    micBeforeDeaf = mic;
    deaf = true;
    mic = false;
  } else {
    deaf = false;
    mic = micBeforeDeaf && micTrack !== null;
  }
  if (micTrack) micTrack.enabled = mic;
  pushState();
};
