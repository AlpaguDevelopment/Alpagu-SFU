import './style.css';
import { LocalCamera } from './local-camera';
import { SfuClient, errText, PROTOCOL, type Kind, type Peer } from './sfu-client';

const TEST_CLIENT_VERSION = '0.2.0';
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
  label.append(nameEl, badges);
  el.append(video, avatar, label);
  grid.append(el);
  const tile: Tile = { info, el, video, audio: null, avatar, badges, videoTrack: null };
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
  tile.badges.textContent = b.join(' · ');
  countEl.textContent = `${tiles.size} participants`;
}

function removeTile(id: string) {
  const tile = tiles.get(id);
  if (!tile) return;
  tile.audio?.remove();
  tile.el.remove();
  tiles.delete(id);
  countEl.textContent = `${tiles.size} participants`;
}

function playSafe(el: HTMLMediaElement) {
  el.play().catch((err) => {
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

function selfInfo(): Peer {
  return { id: SELF, name: nameInput.value.trim(), mic, cam, deaf };
}

function renderControls() {
  setBtn(btnMic, mic, 'Microphone on', 'Microphone off');
  setBtn(btnCam, cam, 'Camera on', 'Camera off');
  if (busy) btnCam.textContent = 'Updating camera…';
  setBtn(btnDeaf, !deaf, 'Deafen', 'Undeafen');
  btnMic.disabled = deaf || busy;
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

joinBtn.onclick = async () => {
  const name = nameInput.value.trim();
  if (!name) return showLobbyError('Enter a name.');
  lobbyError.hidden = true;
  try {
    localStorage.setItem(NAME_KEY, name);
  } catch {
    /* Ignore unavailable storage. */
  }
  joinBtn.disabled = true;

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
  cam = false;
  deaf = false;

  lobby.hidden = true;
  room.hidden = false;
  statusEl.textContent = 'Connecting…';
  tiles.set(SELF, makeTile(selfInfo(), true));
  renderControls();

  client = new SfuClient({
    published() { renderControls(); },
    joined(selfId, server, peers) {
      statusEl.textContent = `Connected · core ${server}`;
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
      if (state === 'failed') statusEl.textContent = `Media connection failed (${pc})`;
    },
    closed(reason) {
      backToLobby(reason);
    },
    log,
  });
  client.join(name, micTrack);
};

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

btnLeave.onclick = () => client?.leave();

function backToLobby(reason: string) {
  client = null;
  micTrack?.stop();
  localCamera.stop();
  busy = false;
  cameraStatus.textContent = '';
  micTrack = camTrack = null;
  mic = cam = deaf = false;
  for (const id of [...tiles.keys()]) removeTile(id);
  unlockBtn.hidden = true;
  room.hidden = true;
  lobby.hidden = false;
  joinBtn.disabled = false;
  if (reason !== 'Left room') showLobbyError('Connection closed. See the Log panel for details.');
}

// ---------- controls ----------
btnMic.onclick = async () => {
  if (deaf) return;
  if (!micTrack) {
    try {
      const s = await navigator.mediaDevices.getUserMedia({ audio: true });
      micTrack = s.getAudioTracks()[0];
      await client?.setMic(micTrack);
    } catch (err) {
      log('Could not acquire microphone: ' + errText(err));
      return;
    }
    mic = true;
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
