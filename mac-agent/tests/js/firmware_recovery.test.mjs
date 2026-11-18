import test from 'node:test';
import assert from 'node:assert/strict';

// Exercise the actual panel with a small DOM and fake HTTP; no browser or device needed.
class Element {
  constructor(tag, text = '') { this.tag = tag; this.text = text; this.children = []; this.attrs = {}; this.events = {}; this.style = {}; this.value = ''; this.disabled = false; this.checked = false; }
  appendChild(child) { this.children.push(child); return child; }
  removeChild(child) { this.children.splice(this.children.indexOf(child), 1); }
  setAttribute(key, value) { this.attrs[key] = value; if (key === 'disabled') this.disabled = true; }
  addEventListener(key, fn) { this.events[key] = fn; }
  get firstChild() { return this.children[0]; }
  get textContent() { return this.text + this.children.map(c => c.textContent).join(''); }
}
function all(el) { return [el, ...el.children.flatMap(all)]; }
const settled = () => new Promise(resolve => setImmediate(resolve));

test('USB recovery requires explicit port/model and keeps its original device route; legacy panel stays compatible', async () => {
  globalThis.Node = Element;
  globalThis.window = {STICK_TOKEN: ''};
  globalThis.localStorage = {setItem() {}, getItem() {return null;}};
  globalThis.document = {createElement: tag => new Element(tag), createTextNode: text => new Element('#text', text)};
  const streams = [];
  globalThis.EventSource = class { constructor(url) { streams.push(url); } };
  const calls = [];
  let legacy = false;
  globalThis.fetch = async (path, opts = {}) => {
    calls.push({path, ...opts});
    const data = opts.method === 'POST' ? {job: 'test-job'} : path.endsWith('/api/firmware')
      ? legacy ? {bundled: {version: '2.0.0'}, usb: [{port: '/dev/cu.legacy', board: 'm5-sticks3'}]}
        : {bundled: null, usb: [], usb_recovery: {ports: [{port: '/dev/cu.rom', product: 'ROM'}], boards: ['m5-sticks3', 'm5-stickc-plus2']}}
      : {};
    return new Response(JSON.stringify(data), {headers: {'content-type': 'application/json'}});
  };
  const {selectDevice} = await import('../../stick_agent/static/lib.js');
  const {firmwarePanel} = await import('../../stick_agent/static/firmware.js');
  selectDevice('one');
  const panel = firmwarePanel();
  await settled();
  const nodes = all(panel);
  const port = nodes.find(n => n.attrs['aria-label'] === 'USB recovery port');
  const model = nodes.find(n => n.attrs['aria-label'] === 'USB recovery model');
  const button = nodes.find(n => n.tag === 'button' && n.textContent === 'Recover selected USB device');
  assert.ok(button.disabled);
  port.value = '/dev/cu.rom'; port.events.change();
  assert.ok(button.disabled);
  model.value = 'm5-sticks3'; model.events.change();
  assert.equal(button.disabled, false, 'recovery works even without a connected board or normal bundled selection');
  selectDevice('two');
  await button.events.click();
  const sent = calls.find(c => c.method === 'POST');
  assert.equal(sent.path, '/api/devices/one/api/firmware/flash');
  assert.deepEqual(JSON.parse(sent.body), {port: '/dev/cu.rom', backup: false, recovery_board: 'm5-sticks3'});
  assert.equal(streams[0], '/api/devices/one/api/firmware/jobs/test-job');

  legacy = true; selectDevice(null); calls.length = 0;
  const old = firmwarePanel(); await settled();
  assert.ok(!all(old).some(n => n.attrs['aria-label'] === 'USB recovery model'));
  await all(old).find(n => n.tag === 'button' && n.textContent === 'Update via USB').events.click();
  const plain = calls.find(c => c.method === 'POST');
  assert.equal(plain.path, '/api/firmware/flash');
  assert.deepEqual(JSON.parse(plain.body), {port: '/dev/cu.legacy', backup: false});
});
