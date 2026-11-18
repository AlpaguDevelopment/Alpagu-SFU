import test from 'node:test';
import assert from 'node:assert/strict';
import { LocalCamera } from '../src/local-camera.ts';

function capture(video = true) {
  const track = { readyState: 'live', stop() { this.readyState = 'ended'; } };
  return { track, getTracks: () => [track], getVideoTracks: () => video ? [track] : [] };
}

test('ten preview cycles release every camera track', async () => {
  const camera = new LocalCamera();
  for (let i = 0; i < 10; i++) {
    const stream = capture();
    assert.equal(await camera.start(async () => stream), stream.track);
    assert.equal(stream.track.readyState, 'live');
    camera.stop();
    assert.equal(stream.track.readyState, 'ended');
  }
});

test('leaving during permission prompt releases the late capture', async () => {
  const camera = new LocalCamera();
  let resolve;
  const pending = camera.start(() => new Promise((r) => { resolve = r; }));
  camera.stop();
  const stream = capture();
  resolve(stream);
  assert.equal(await pending, null);
  assert.equal(stream.track.readyState, 'ended');
});

test('late old capture cannot replace a new session camera', async () => {
  const camera = new LocalCamera();
  let resolve;
  const pending = camera.start(() => new Promise((r) => { resolve = r; }));
  camera.stop();
  const current = capture();
  await camera.start(async () => current);
  const stale = capture();
  resolve(stale);
  assert.equal(await pending, null);
  assert.equal(stale.track.readyState, 'ended');
  assert.equal(current.track.readyState, 'live');
  camera.stop();
  assert.equal(current.track.readyState, 'ended');
});

test('permission denial allows a later retry', async () => {
  const camera = new LocalCamera();
  await assert.rejects(camera.start(async () => { throw new Error('Permission denied'); }));
  const stream = capture();
  assert.equal(await camera.start(async () => stream), stream.track);
  camera.stop();
  assert.equal(stream.track.readyState, 'ended');
});

test('invalid camera stream releases all acquired tracks', async () => {
  const camera = new LocalCamera();
  const stream = capture(false);
  await assert.rejects(camera.start(async () => stream), /no video track/);
  assert.equal(stream.track.readyState, 'ended');
});
