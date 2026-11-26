import test from "node:test";
import assert from "node:assert/strict";

class Element extends EventTarget {
  constructor() { super(); this.children = []; this.attrs = {}; }
  appendChild(node) { this.children.push(node); }
  removeChild(node) { this.children = this.children.filter(c => c !== node); }
  get firstChild() { return this.children[0]; }
  setAttribute(key, value) { this.attrs[key] = value; this[key] = key === "hidden" ? true : value; }
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
globalThis.document = Object.assign(new EventTarget(), {createElement: () => new Element(), createTextNode: s => s});
const {memoryPanel, exportHref} = await import("../../stick_agent/static/memory.js");
const settle = () => new Promise(resolve => setImmediate(resolve));

test("export is the facts as JSON", () => {
  const href = exportHref([{id: "m1", text: "standup 10:30"}]);
  assert.ok(href.startsWith("data:application/json"));
  assert.deepEqual(JSON.parse(decodeURIComponent(href.split(",")[1])), {items: [{id: "m1", text: "standup 10:30"}]});
});

test("lists facts, adds one, and turns cloud sharing off", async () => {
  let state = {on: true, cloud: true, max: 200, items: [{id: "m1", text: "I take the train", source: "agent"}]};
  const calls = [];
  globalThis.fetch = async (path, opts = {}) => {
    calls.push([opts.method || "GET", path, opts.body ? JSON.parse(opts.body) : null]);
    if (opts.method === "POST") { state.items.unshift({id: "m2", text: JSON.parse(opts.body).text, source: "dashboard"}); return Response.json(state.items[0]); }
    if (opts.method === "PUT") { state = {...state, ...JSON.parse(opts.body)}; return Response.json(state); }
    return Response.json(state);
  };
  const panel = memoryPanel();
  await settle();
  assert.equal(panel.el.hidden, false);
  const rows = panel.el.find(e => e.className === "memory-list").children;
  assert.equal(rows.length, 1);
  const input = panel.el.find(e => e.id === "memoryNew");
  input.value = "Standup is at 10:30";
  input.dispatchEvent(Object.assign(new Event("keydown"), {key: "Enter"}));
  await settle(); await settle();
  assert.deepEqual(calls.find(c => c[0] === "POST"), ["POST", "/api/memory", {text: "Standup is at 10:30"}]);
  const cloud = panel.el.find(e => e.id === "memoryCloud");
  cloud.checked = false;
  cloud.dispatchEvent(new Event("change"));
  await settle();
  assert.deepEqual(calls.find(c => c[0] === "PUT"), ["PUT", "/api/memory", {cloud: false}]);
  assert.equal(state.cloud, false);
  panel.leave();
});
