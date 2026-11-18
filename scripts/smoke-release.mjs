// Smoke-test the actual release executable without touching the normal LAN ports.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { createSocket } from 'node:dgram';
import { resolve } from 'node:path';

const executable = resolve(process.argv[2] ?? 'core/target/x86_64-pc-windows-msvc/release/sfu-core.exe');
assert.equal(execFileSync(executable, ['--version'], { encoding: 'utf8', windowsHide: true }).trim(), '0.1.0');
const tcp = createServer();
tcp.listen(0, '127.0.0.1');
await once(tcp, 'listening');
const port = tcp.address().port;
await new Promise((done) => tcp.close(done));
const udp = createSocket('udp4');
udp.bind(0, '127.0.0.1');
await once(udp, 'listening');
const udpPort = udp.address().port;
udp.close();
const server = spawn(executable, ['--ip', '127.0.0.1', '--port', String(port), '--udp-port', String(udpPort)], { windowsHide: true });
let output = '';
server.stdout.on('data', (chunk) => { output += chunk; });
server.stderr.on('data', (chunk) => { output += chunk; });
const sockets = [];
const timer = setTimeout(() => { server.kill(); throw new Error('Smoke test timed out'); }, 15000);
async function connect(name) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
  sockets.push(ws);
  const queue = [];
  const waiting = [];
  ws.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (waiting.length) waiting.shift()(message); else queue.push(message);
  });
  const next = () => queue.length ? Promise.resolve(queue.shift()) : new Promise((done) => waiting.push(done));
  await new Promise((done, fail) => { ws.addEventListener('open', done, { once: true }); ws.addEventListener('error', fail, { once: true }); });
  ws.send(JSON.stringify({ type: 'join', name, protocol: 1 }));
  const joined = await next();
  assert.equal(joined.type, 'joined');
  assert.equal(joined.server, '0.1.0');
  return { ws, next, joined };
}
try {
  while (!output.includes('SFU started')) {
    if (server.exitCode !== null) throw new Error('Server exited before startup');
    await new Promise((done) => setTimeout(done, 20));
  }
  const first = await connect('Smoke A');
  const second = await connect('Smoke B');
  assert.equal(second.joined.peers.length, 1);
  assert.equal((await first.next()).type, 'peer_joined');
  second.ws.send(JSON.stringify({ type: 'state', mic: true, cam: false, deaf: false }));
  assert.equal((await first.next()).mic, true);
  first.ws.send(JSON.stringify({ type: 'ping' }));
  assert.equal((await first.next()).type, 'pong');
  second.ws.send(JSON.stringify({ type: 'leave' }));
  assert.equal((await first.next()).type, 'peer_left');
  first.ws.send(JSON.stringify({ type: 'leave' }));
  while (!output.includes('participants=0')) await new Promise((done) => setTimeout(done, 20));
  console.log('Release smoke test passed: version, startup, two peers, state, ping, leave, zero participants');
} finally {
  clearTimeout(timer);
  for (const ws of sockets) ws.close();
  server.kill();
  await once(server, 'exit');
}
