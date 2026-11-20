import test from 'node:test';
import assert from 'node:assert/strict';
import { scheduleRejoin } from '../src/rejoin.ts';

function setup(t) {
  const page = Object.assign(new EventTarget(), { visibilityState: 'hidden' });
  const network = { onLine: true };
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const browser = Object.assign(new EventTarget(), { setTimeout, clearTimeout });
  for (const [key, value] of Object.entries({ document: page, navigator: network, window: browser })) {
    const original = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { configurable: true, value });
    t.after(() => original ? Object.defineProperty(globalThis, key, original) : delete globalThis[key]);
  }
  return {
    show(visible) {
      page.visibilityState = visible ? 'visible' : 'hidden';
      page.dispatchEvent(new Event('visibilitychange'));
    },
    online(value) {
      network.onLine = value;
      browser.dispatchEvent(new Event(value ? 'online' : 'offline'));
    },
    wait() { t.mock.timers.tick(1500); },
  };
}

test('background loss reserves exactly one rejoin until both visible and online', (t) => {
  const page = setup(t);
  const join = t.mock.fn();
  scheduleRejoin(join);
  page.online(true);
  page.wait();
  page.online(false);
  page.show(true);
  page.wait();
  assert.equal(join.mock.callCount(), 0);
  page.online(true);
  page.show(true);
  page.wait();
  page.online(true);
  page.wait();
  assert.equal(join.mock.callCount(), 1);
});

test('hiding during the retry delay defers it and leaving cancels it', (t) => {
  const page = setup(t);
  page.show(true);
  const join = t.mock.fn();
  const leave = scheduleRejoin(join);
  page.show(false);
  page.wait();
  assert.equal(join.mock.callCount(), 0);
  page.show(true);
  leave();
  page.wait();
  page.show(true);
  page.online(true);
  page.wait();
  assert.equal(join.mock.callCount(), 0);
});
