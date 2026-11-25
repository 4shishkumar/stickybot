import test from "node:test";
import assert from "node:assert/strict";

class Element extends EventTarget {
  constructor(tag) { super(); this.tag = tag; this.children = []; this.value = ""; }
  appendChild(child) { this.children.push(child); return child; }
  replaceChildren(...children) { this.children = children; }
  setAttribute(key, value) { this[key] = key === "hidden" ? true : value; }
  removeAttribute(key) { delete this[key]; }
  get firstChild() { return this.children[0]; }
  set textContent(text) { this.children = [text]; }
  get textContent() { return this.children.map(child => child.textContent ?? child).join(""); }
  find(pred) {
    if (pred(this)) return this;
    for (const child of this.children) { const match = child.find?.(pred); if (match) return match; }
  }
  fire(type, value) { if (value !== undefined) this.value = value; this.dispatchEvent(new Event(type)); }
}

globalThis.window = {};
globalThis.Node = globalThis.HTMLElement = Element;
globalThis.document = {createElement: tag => new Element(tag), createTextNode: text => text};
const {render, leave, buttonSettingsLink} = await import("../../stick_agent/static/pages/buttons.js");
const {selectDevice, store} = await import("../../stick_agent/static/lib.js");
const defaults = {b2:"unlock", b3:"launcher", ab:"reply", bh:"talk"};
const settle = () => new Promise(resolve => setImmediate(resolve));
const state = buttons => Response.json({ok:true, via:"ble", reply:{buttons}});
const id = (root, name) => root.find(el => el.id === name);
const saveButton = root => root.find(el => el.tag === "button");
const notice = root => root.find(el => el.role === "status");

function device(t, name = "stick A") {
  selectDevice(name);
  store.online = true;
  store.status = {link:"connected", stick:{name}};
  t.after(() => { leave(); selectDevice(null); store.status = {}; store.online = false; });
}

test("Settings hides the link for older firmware and drops a pending read after disconnection", async t => {
  device(t);
  let answer;
  t.mock.method(globalThis, "fetch", () => new Promise(resolve => {answer = resolve;}));
  const link = buttonSettingsLink(); t.after(link.leave);
  await settle();
  answer(state(undefined)); await settle();
  assert.equal(link.el.hidden, true);
  store.emit("event", {kind:"link", state:"searching"}); await settle();
  store.emit("event", {kind:"link", state:"connected"}); await settle();
  answer(state(defaults)); await settle();
  assert.equal(link.el.hidden, false);
  store.emit("event", {kind:"link", state:"searching"}); await settle();
  assert.equal(link.el.hidden, true);
  store.emit("event", {kind:"link", state:"connected"}); await settle();
  const stale = answer;
  store.emit("event", {kind:"link", state:"searching"}); await settle();
  stale(state(defaults)); await settle();
  assert.equal(link.el.hidden, true);
  store.emit("event", {kind:"link", state:"connected"}); await settle();
  answer(Response.json({ok:true, via:"usb", reply:{buttons:defaults}})); await settle();
  assert.equal(link.el.hidden, true);
});

test("unscoped writes are unavailable during Rust device discovery; legacy single-device reads work", async t => {
  device(t, null);
  let legacy = false, reads = 0;
  t.mock.method(globalThis, "fetch", async path => {
    if (path === "/api/devices") return legacy ? new Response(null, {status:404}) : Response.json({devices:[]});
    assert.equal(path, "/api/stick/proto"); ++reads; return state(defaults);
  });
  const multi = buttonSettingsLink();
  try { await settle(); assert.equal(multi.el.hidden, true); assert.equal(reads, 0); } finally { multi.leave(); }
  legacy = true;
  const single = buttonSettingsLink();
  try { await settle(); assert.equal(single.el.hidden, false); assert.equal(reads, 1); } finally { single.leave(); }
});

test("editor offers the four mappings, validates names, and sends one device-bound update", async t => {
  device(t);
  const lines = [];
  let ack = defaults;
  t.mock.method(globalThis, "fetch", async (path, opts) => {
    if (path === "/api/commands") return Response.json({commands:[{name:"Music"}]});
    assert.equal(path, "/api/devices/stick%20A/api/stick/proto");
    const {line} = JSON.parse(opts.body);
    if (line === "@state") return state(defaults);
    lines.push(line); return state(ack);
  });
  const root = new Element("div"); render(root); await settle();
  const b2 = id(root, "gesture-b2"), b3 = id(root, "gesture-b3"), ab = id(root, "gesture-ab"), bh = id(root, "gesture-bh");
  const appName = id(root, "gesture-name-ab");
  assert.deepEqual([b2.value,b3.value,ab.value,bh.value], Object.values(defaults));
  assert.deepEqual(bh.children.map(o => o.value), ["talk", "dictation"]);
  assert.deepEqual(ab.children.filter(o => o.value.startsWith("tile:")).map(o => o.value), [
    "tile:games", "tile:remote", "tile:air", "tile:presenter", "tile:media", "tile:faces", "tile:quick", "tile:settings"]);
  b2.fire("change", "launcher"); b3.fire("change", "unlock"); ab.fire("change", "app"); bh.fire("change", "dictation");
  for (const invalid of ["", "a/b", "a\\b", "abc\n", "🙃".repeat(9)]) {
    appName.fire("input", invalid); saveButton(root).fire("click");
    assert.equal(saveButton(root).disabled, true); assert.equal(lines.length, 0);
  }
  appName.fire("input"," Café & Tea ");
  ack = {b2:"launcher", b3:"unlock", ab:"app:Café & Tea", bh:"dictation"};
  saveButton(root).fire("click"); await settle();
  assert.deepEqual(lines, ["@buttons b2=launcher&b3=unlock&ab=app%3ACaf%C3%A9%20%26%20Tea&bh=dictation"]);
  assert.equal(appName.value, "Café & Tea"); assert.equal(saveButton(root).disabled, true);
  assert.equal(notice(root).textContent, "Saved to stick.");
  ab.fire("change", "quick"); appName.fire("input", "Music"); ack = undefined;
  saveButton(root).fire("click"); await settle();
  assert.match(notice(root).textContent, /did not confirm/); assert.equal(saveButton(root).disabled, false);
});

test("a previous device's controls and delayed save never update the new stick", async t => {
  device(t);
  let finish; const writes = [];
  t.mock.method(globalThis, "fetch", async (path, opts) => {
    if (path === "/api/commands") return Response.json({commands:[]});
    const {line} = JSON.parse(opts.body);
    if (line === "@state") return state(defaults);
    writes.push(path);
    return new Promise(resolve => {finish = resolve;});
  });
  const first = new Element("div"); render(first); await settle();
  id(first, "gesture-b2").fire("change", "none");
  const oldSave = saveButton(first);
  oldSave.fire("click"); await settle(); leave();
  selectDevice("stick B");
  const second = new Element("div"); render(second);
  store.status = {link:"connected", stick:{name:"stick B"}};
  store.emit("status", store.status); await settle();
  oldSave.fire("click"); finish(state({...defaults, b2:"none"})); await settle();
  assert.deepEqual(writes, ["/api/devices/stick%20A/api/stick/proto"]);
  assert.equal(id(second, "gesture-b2").value, "unlock"); assert.equal(notice(second).textContent, "");
});
