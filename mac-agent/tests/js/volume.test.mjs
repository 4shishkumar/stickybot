import test from 'node:test';
import assert from 'node:assert/strict';

class Element extends EventTarget {
  constructor() { super(); this.children = []; }
  appendChild(node) { this.children.push(node); }
  setAttribute(name, value) { this[name] = value; }
  fire(type) { this.dispatchEvent(new Event(type)); }
}
globalThis.Node = Element;
globalThis.window = {};
globalThis.document = {createElement: () => new Element(), createTextNode: () => new Element()};
const {volumeControl} = await import('../../stick_agent/static/volume.js');
const settle = () => new Promise(resolve => setImmediate(resolve));

test('slider and mute round-trip exact levels, including legacy zero, across reports', async () => {
  const writes = [];
  const target = {async put(path, payload) { assert.equal(path, '/api/sounds'); writes.push(payload); return payload; }};
  const control = volumeControl({on:true, volume:93}, target, 'volume');
  const [mute, slider, out] = control.el.children;
  assert.equal(out.textContent, '36%');
  mute.fire('click'); await settle();
  mute.fire('click'); await settle();
  assert.deepEqual(writes, [{on:false, volume:93}, {on:true, volume:93}]);
  slider.value = 37; slider.fire('input'); slider.fire('change'); await settle();
  assert.deepEqual(writes.at(-1), {on:true, volume:94});
  slider.value = 0; slider.fire('input'); slider.fire('change'); await settle();
  assert.deepEqual(writes.at(-1), {on:false, volume:94});
  control.sync({on:true, volume:0});
  mute.fire('click'); await settle();
  assert.deepEqual(writes.at(-1), {on:true, volume:140});
  control.link(false);
  assert.ok(mute.disabled && slider.disabled);
});

test('a slow stick receives the final drag position and stale reports do not move the thumb', async () => {
  const writes = [];
  let reply;
  const target = {put(_path, payload) { writes.push(payload); return new Promise(resolve => {reply = () => resolve(payload);}); }};
  const control = volumeControl({on:true, volume:140}, target, 'vol');
  const [, slider, out] = control.el.children;
  slider.fire('pointerdown');
  slider.value = 31; slider.fire('input');
  await new Promise(resolve => setTimeout(resolve, 220));
  slider.value = 63; slider.fire('input');
  control.sync({on:true, volume:79});
  assert.equal(out.textContent, '63%');
  slider.fire('pointerup');
  assert.equal(writes.length, 1);
  reply(); await new Promise(resolve => setTimeout(resolve, 220));
  assert.deepEqual(writes, [{on:true, volume:79}, {on:true, volume:161}]);
  reply(); await settle();
  assert.equal(out.textContent, '63%');
});

test('a late HTTP acknowledgement preserves a newer hardware report', async () => {
  let resolve;
  const target = {put() { return new Promise(r => {resolve = r;}); }};
  const control = volumeControl({on:true, volume:94}, target, 'vol');
  control.el.children[0].fire('click');
  control.sync({on:true, volume:161});
  resolve({on:false, volume:94});
  await settle();
  assert.equal(control.el.children[2].textContent, '63%');
});
