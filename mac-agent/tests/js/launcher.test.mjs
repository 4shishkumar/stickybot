import test from "node:test";
import assert from "node:assert/strict";
import "./dom.mjs";

const {selectDevice, deviceApi} = await import("../../stick_agent/static/lib.js");
const {launcherEditor} = await import("../../stick_agent/static/pages/launcher.js");
const settle = () => new Promise(resolve => setImmediate(resolve));
const byLabel = (el, label) => el.find(node => node["aria-label"] === label);
const byText = (el, text) => el.find(node => node.tag === "button" && node.textContent === text);
const tiles = el => el.all(node => node.tag === "b").map(node => node.textContent);

function editor(t, state, address, ack) {
  selectDevice("stick A");
  t.after(() => selectDevice(null));
  const writes = [], refreshes = [];
  t.mock.method(globalThis, "fetch", async (path, request) => {
    writes.push([path, JSON.parse(request.body)]);
    return ack ? ack() : Response.json({ok:true, device_address:address, reply:{ok:true}});
  });
  const el = launcherEditor(state, deviceApi(), address, () => refreshes.push("refreshed"));
  el.isConnected = true;
  return {el, writes, refreshes};
}

test("favourites lead in configured order and move within their group; saves pin the BLE peer", async t => {
  const {el, writes, refreshes} = editor(t, {launcher: "grapmfqs", launcherFav: "af"}, "A");
  assert.deepEqual(tiles(el), ["Air mouse", "Faces", "Games", "Remote", "Presenter", "Media", "Quick settings", "Settings"]);
  assert.equal(byLabel(el, "Move Faces down").disabled, true);
  assert.equal(byLabel(el, "Show Settings").disabled, true);
  byLabel(el, "Move Faces up").fire("click");
  byLabel(el, "Favourite Games").fire("click");
  assert.deepEqual(tiles(el).slice(0, 4), ["Games", "Faces", "Air mouse", "Remote"]);
  byText(el, "Save").fire("click");
  await settle();
  assert.deepEqual(writes, [["/api/devices/stick%20A/api/stick/proto", {
    line: "@launcher tiles=games%2Cremote%2Cfaces%2Cpresenter%2Cmedia%2Cair%2Cquick%2Csettings&favorites=games%2Cfaces%2Cair",
    expected_address: "A",
  }]]);
  assert.equal(refreshes.length, 1);
});

test("hiding a favourite clears its star, showing it again keeps the base order", async t => {
  const {el, writes} = editor(t, {launcher:"grapmfqs", launcherFav:"a"}, "A");
  const visible = byLabel(el, "Show Air mouse"); visible.checked = false; visible.fire("change");
  assert.equal(byLabel(el, "Favourite Air mouse").disabled, true);
  assert.equal(tiles(el).at(-1), "Air mouse");
  const hidden = byLabel(el, "Show Air mouse"); hidden.checked = true; hidden.fire("change");
  assert.equal(tiles(el)[2], "Air mouse");
  assert.equal(byLabel(el, "Favourite Air mouse")["aria-pressed"], "false");
  byText(el, "Save").fire("click"); await settle();
  assert.equal(writes[0][1].line, "@launcher tiles=games%2Cremote%2Cair%2Cpresenter%2Cmedia%2Cfaces%2Cquick%2Csettings&favorites=-");
});

test("old firmware offers its original ordering controls and sends tiles only, including over USB", async t => {
  const {el, writes} = editor(t, {launcher:"grqs"}, null);
  assert.equal(byLabel(el, "Favourite Games"), undefined);
  byLabel(el, "Move Games down").fire("click");
  byText(el, "Save").fire("click"); await settle();
  assert.deepEqual(writes[0][1], {line: "@launcher tiles=remote%2Cgames%2Cquick%2Csettings"});
});

test("capable peers require a BLE identity for saving or resetting", async t => {
  const {el, writes} = editor(t, {launcher:"grqs", launcherFav:""}, null);
  assert.equal(byText(el, "Reset").disabled, true);
  assert.equal(byLabel(el, "Favourite Games").disabled, true);
  byText(el, "Reset").fire("click"); await settle();
  assert.deepEqual(writes, []);
});

test("a device switch invalidates pending resets and stale controls", async t => {
  let finish;
  const {el, writes, refreshes} = editor(t, {launcher:"grqs", launcherFav:"g"}, "A", () => new Promise(r => { finish = r; }));
  byText(el, "Reset").fire("click");
  selectDevice("stick B");
  byText(el, "Reset").fire("click");
  finish(Response.json({ok:true, device_address:"A", reply:{ok:true}})); await settle();
  assert.deepEqual(writes, [["/api/devices/stick%20A/api/stick/proto", {line:"@launcher reset=1", expected_address:"A"}]]);
  assert.deepEqual(refreshes, []);
});

test("a mismatched acknowledgement keeps the draft and reports an error", async t => {
  const {el, refreshes} = editor(t, {launcher:"grqs", launcherFav:""}, "A",
    () => Response.json({ok:true, device_address:"B", reply:{ok:true}}));
  byLabel(el, "Favourite Games").fire("click");
  byText(el, "Save").fire("click"); await settle();
  assert.deepEqual(refreshes, []);
  assert.equal(byText(el, "Save").disabled, false);
  assert.match(el.textContent, /did not confirm/);
});


test("old firmware pins tiles-only saves and resets to the peer that supplied the layout", async t => {
  const {el, writes, refreshes} = editor(t, {launcher:"grqs"}, "original", () =>
    Response.json({ok:true, device_address:"replacement", reply:{ok:true}}));
  byLabel(el, "Move Games down").fire("click");
  byText(el, "Save").fire("click"); await settle();
  byText(el, "Reset").fire("click"); await settle();
  assert.deepEqual(writes.map(([, body]) => body), [
    {line:"@launcher tiles=remote%2Cgames%2Cquick%2Csettings", expected_address:"original"},
    {line:"@launcher reset=1", expected_address:"original"},
  ]);
  assert.deepEqual(refreshes, []);
});
