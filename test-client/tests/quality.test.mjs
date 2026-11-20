import assert from 'node:assert/strict';
import test from 'node:test';
import { measureQuality, monitorQuality } from '../src/quality.ts';

function report(...rows) {
  return new Map(rows.map((row) => [row.id, row]));
}
const audio = { type: 'inbound-rtp', id: 'a', ssrc: 1, bytesReceived: 1000, timestamp: 1000, packetsReceived: 100, packetsLost: 2 };
const video = { ...audio, id: 'v', ssrc: 2 };

test('receive rate adds stream rates and loss is weighted by packet counts', () => {
  const last = report(audio, video);
  const current = report(
    { ...audio, timestamp: 2000, bytesReceived: 3000, packetsReceived: 190, packetsLost: 12 },
    { ...video, timestamp: 3000, bytesReceived: 5000, packetsReceived: 200, packetsLost: 1 },
    { type: 'transport', id: 't', selectedCandidatePairId: 'selected' },
    { type: 'candidate-pair', id: 'selected', currentRoundTripTime: 0.2 },
    { type: 'candidate-pair', id: 'unused', currentRoundTripTime: 1 },
  );
  assert.deepEqual(measureQuality('sub', current, last), { bitrateKbps: 32, lossPct: 5, availableKbps: undefined, rttMs: 200 });
});

test('publication measures sent bytes separately from estimated capacity', () => {
  const out = { type: 'outbound-rtp', id: 'o', ssrc: 9, bytesSent: 0, timestamp: 500 };
  const current = report({ ...out, timestamp: 1500, bytesSent: 5000 }, audio,
    { type: 'transport', id: 't', selectedCandidatePairId: 'p' },
    { type: 'candidate-pair', id: 'p', availableOutgoingBitrate: 550000 });
  assert.equal(measureQuality('pub', current, report(out)).bitrateKbps, 40);
  assert.equal(measureQuality('pub', current).availableKbps, 550);
});

test('new streams, repeated reports and counter resets do not create bitrate spikes', () => {
  const last = report(audio);
  for (const row of [audio, { ...audio, ssrc: 2, timestamp: 2000 },
    { ...audio, bytesReceived: 0, timestamp: 2000 }]) {
    assert.equal(measureQuality('sub', report(row), last).bitrateKbps, undefined);
  }
  assert.equal(measureQuality('sub', last).lossPct, undefined);
  assert.equal(measureQuality('sub', report({ ...audio, timestamp: 2000 }), last).bitrateKbps, 0);
});

test('send loss uses fresh receiver feedback from active streams', () => {
  const out = { type: 'outbound-rtp', id: 'audio', remoteId: 'rr', ssrc: 9, timestamp: 19000, packetsSent: 50 };
  const last = report(out);
  const sending = { ...out, timestamp: 20000, packetsSent: 75 };
  const feedback = { type: 'remote-inbound-rtp', id: 'rr', timestamp: 19500, fractionLost: 0.125 };
  assert.equal(measureQuality('pub', report(sending, feedback), last).lossPct, 12.5);
  assert.equal(measureQuality('pub', report(sending, { ...feedback, timestamp: 5000 }), last).lossPct, undefined);
  assert.equal(measureQuality('pub', report({ ...sending, packetsSent: 50 }, feedback), last).lossPct, undefined);
});

test('a late stats result after leave cannot update the new room or reschedule polling', async (t) => {
  let finish;
  t.mock.method(globalThis, 'setTimeout', () => assert.fail('unexpected timer'));
  t.after(() => { delete globalThis.window; });
  globalThis.window = globalThis;
  const pending = new Promise((resolve) => { finish = resolve; });
  const stop = monitorQuality(() => pending, () => assert.fail('unexpected update'));
  stop();
  finish(report(audio));
  await new Promise((resolve) => setImmediate(resolve));
});
