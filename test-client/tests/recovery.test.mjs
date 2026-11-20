import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL, SfuClient } from '../src/sfu-client.ts';

function setup(t) {
  const timers = new Map();
  let nextTimer = 0;
  function arm(fn, ms) {
    timers.set(++nextTimer, { fn, ms });
    return nextTimer;
  }
  const pcs = [];
  const sent = [];
  const closures = [];
  let ws;
  class Socket {
    static OPEN = 1;
    readyState = 1;
    constructor() { ws = this; }
    send(text) { sent.push(JSON.parse(text)); }
    close() { this.readyState = 3; }
  }
  class Pc {
    constructor() { pcs.push(this); }
    addTransceiver() { return { sender: {} }; }
    async createOffer() { return { type: 'offer', sdp: 'offer' }; }
    async createAnswer() { return { type: 'answer', sdp: 'answer' }; }
    async setLocalDescription(sdp) { this.localDescription = sdp; }
    async setRemoteDescription(sdp) { this.remoteDescription = sdp; }
    getTransceivers() { return []; }
    close() { this.connectionState = 'closed'; }
  }
  for (const [key, value] of Object.entries({
    window: { setTimeout: arm, setInterval: arm, clearTimeout: (id) => timers.delete(id), clearInterval: (id) => timers.delete(id) },
    location: { protocol: 'https:', host: 'localhost' }, WebSocket: Socket, RTCPeerConnection: Pc,
  })) {
    const previous = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    t.after(() => previous ? Object.defineProperty(globalThis, key, previous) : delete globalThis[key]);
  }
  const client = new SfuClient({
    closed: (...args) => closures.push(args), log() {}, published() {}, joined() {}, pcState() {},
  });
  client.join('Peer', null);
  ws.onopen();
  async function message(msg) {
    ws.onmessage({ data: JSON.stringify(msg) });
    await new Promise((done) => setImmediate(done));
  }
  return { client, ws, sent, timers, closures, pcs, message };
}

test('a disconnected transport requests one restart and clears its deadline on recovery', async (t) => {
  const { client, sent, timers, closures, pcs, message } = setup(t);
  await message({ type: 'joined', protocol: PROTOCOL, peers: [] });
  await message({ type: 'pub_answer', sdp: 'initial' });
  await message({ type: 'sub_offer', sdp: 'subscription', tracks: [] });
  for (const [i, pc] of pcs.entries()) {
    pc.iceConnectionState = 'disconnected';
    pc.oniceconnectionstatechange();
    assert.equal(sent.filter((m) => m.type === 'restart_ice').length, i);
    pc.iceConnectionState = 'failed';
    pc.oniceconnectionstatechange();
    const name = i === 0 ? 'pub' : 'sub';
    assert.deepEqual(sent.filter((m) => m.pc === name), [{ type: 'restart_ice', pc: name }]);
    const type = i === 0 ? 'pub_restart_offer' : 'sub_offer';
    await message({ type, sdp: 'restart', tracks: [] });
    assert.equal(pc.remoteDescription.sdp, 'restart');
    assert.equal(sent.at(-1).type, i === 0 ? 'pub_restart_answer' : 'sub_answer');
    pc.iceConnectionState = 'connected';
    pc.oniceconnectionstatechange();
    assert.equal([...timers.values()].some((timer) => timer.ms === 12_000), false);
  }
  client.leave();
  assert.deepEqual(closures, [['Left room', false]]);
  assert.equal(timers.size, 0);
});

test('brief ICE loss recovers in place while sustained loss requests a restart', async (t) => {
  const { client, timers, sent, pcs, message } = setup(t);
  await message({ type: 'joined', protocol: PROTOCOL, peers: [] });
  await message({ type: 'pub_answer', sdp: 'initial' });
  const [pc] = pcs;
  function state(value) {
    pc.iceConnectionState = value;
    pc.oniceconnectionstatechange();
  }
  const grace = () => [...timers.values()].find((timer) => timer.ms === 2000);
  state('disconnected');
  assert.ok(grace());
  state('completed');
  assert.equal(grace(), undefined);
  assert.ok(!sent.some((m) => m.type === 'restart_ice'));
  state('disconnected');
  grace().fn();
  state('failed');
  assert.deepEqual(sent.filter((m) => m.type === 'restart_ice'), [{ type: 'restart_ice', pc: 'pub' }]);
  state('connected');
  state('disconnected');
  client.leave();
  assert.equal(timers.size, 0);
});

test('a silent socket times out and late events cannot recreate the old session', async (t) => {
  const { timers, closures, message, pcs, ws } = setup(t);
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  const heartbeat = [...timers.values()].find((timer) => timer.ms === 4000).fn;
  heartbeat();
  now += 12_000;
  heartbeat();
  ws.onclose({ code: 1006 });
  await message({ type: 'joined', protocol: PROTOCOL, peers: [] });
  assert.deepEqual(closures, [['Signaling heartbeat timed out', true]]);
  assert.equal(pcs.length, 0);
  assert.equal(timers.size, 0);
});

test('duplicate name is a final rejection with a visible reason', async (t) => {
  const { message, ws, closures } = setup(t);
  await message({ type: 'error', fatal: true, message: 'Name is already in use' });
  ws.onclose({ code: 1000 });
  assert.deepEqual(closures, [['Name is already in use', false]]);
});
