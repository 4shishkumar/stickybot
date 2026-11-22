import test from "node:test";
import assert from "node:assert/strict";

class Element extends EventTarget {
  constructor() { super(); this.children = []; }
  appendChild(node) { this.children.push(node); }
  setAttribute(key, value) { this[key] = key === "hidden" ? true : value; }
  find(predicate) {
    if (predicate(this)) return this;
    for (const child of this.children) {
      const found = child.find?.(predicate);
      if (found) return found;
    }
  }
}
globalThis.window = new EventTarget();
globalThis.HTMLElement = globalThis.Node = Element;
globalThis.document = {createElement: () => new Element(), createTextNode: s => s};
const {macVoiceFields, shortcutFromEvent} = await import("../../stick_agent/static/mac_voice.js");
const settle = () => new Promise(resolve => setImmediate(resolve));

test("shortcut capture uses physical key codes and canonical modifier order", () => {
  assert.equal(shortcutFromEvent({code:"Space", ctrlKey:true, shiftKey:true}), "Control+Shift+Space");
  assert.equal(shortcutFromEvent({code:"KeyA", key:"q", ctrlKey:true, altKey:true, metaKey:true}), "Control+Option+Command+KeyA");
  assert.equal(shortcutFromEvent({code:"Digit4", metaKey:true}), "Command+Digit4");
  assert.equal(shortcutFromEvent({code:"KeyF", shiftKey:true}), null);
  assert.equal(shortcutFromEvent({code:"Escape", ctrlKey:true}), null);
});

test("unavailable hosts hide keyboard settings; conflict keeps the previous binding", async () => {
  const state = {supported:true, enabled:true, shortcut:"Control+Shift+Space", permission:"authorized", input_name:"Mac microphone"};
  let supported = false;
  const writes = [];
  globalThis.fetch = async (path, opts) => {
    assert.equal(path, "/api/mac-voice");
    if (opts.method === "PUT") {
      writes.push(JSON.parse(opts.body));
      return Response.json({error:"Shortcut already in use"}, {status:409});
    }
    return Response.json(supported ? state : {supported:false});
  };
  const absent = macVoiceFields();
  await settle();
  assert.ok(absent.el.hidden);
  absent.leave();
  supported = true;
  const fields = macVoiceFields();
  await settle();
  assert.equal(fields.el.hidden, false);
  const record = fields.el.find(e => e.id === "macVoiceShortcut");
  assert.equal(record.textContent, "Control + Shift + Space");
  record.dispatchEvent(new Event("click"));
  const press = Object.assign(new Event("keydown", {cancelable:true}), {key:"c", code:"KeyC", metaKey:true});
  record.dispatchEvent(press);
  await settle();
  assert.deepEqual(writes, [{enabled:true, shortcut:"Command+KeyC"}]);
  assert.equal(record.textContent, "Control + Shift + Space");
  assert.equal(fields.el.find(e => e.role === "status").textContent, "Shortcut already in use");
  fields.leave();
});
