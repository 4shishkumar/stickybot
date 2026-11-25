// @state.launcher is the configured order. Capable sticks group its visible
// favourites first on screen, preserving the order within each group.
import {card, clear, deviceId, h, proto} from "../lib.js";
import {TILES} from "./stick_layout.js";

export function launcherEditor(state, target, address, refresh) {
  const capable = typeof state.launcherFav === "string";
  const peer = typeof address === "string" && address ? address : undefined;
  const writable = !capable || !!peer;
  const shown = [...String(state.launcher || "")].map(letter => TILES.find(t => t[0] === letter)).filter(Boolean);
  const rows = [...shown, ...TILES.filter(t => !shown.includes(t))].map(tile => ({
    tile, shown: shown.includes(tile), favourite: capable && shown.includes(tile) && state.launcherFav.includes(tile[0]),
  }));
  function group(x) {
    if (!capable || x.favourite) return 0;
    return x.shown ? 1 : 2;
  }
  const args = () => {
    const names = rows.filter(x => x.shown).map(x => x.tile[1]);
    const favourites = rows.filter(x => x.favourite).map(x => x.tile[1]);
    return capable ? {tiles: names.join(","), favorites: favourites.join(",") || "-"} : {tiles: names.join(",")};
  };
  const baseline = JSON.stringify(args());
  let busy = false;
  const box = h("div", {class: "list"});
  const message = h("span", {class: "small dim", role: "status", "aria-live": "polite"});
  const save = h("button", {class: "btn primary", type: "button", onclick: () => submit(false)}, "Save");
  const reset = h("button", {class: "btn ghost", type: "button", onclick: () => submit(true)}, "Reset");
  const el = card("Launcher",
    h("p", {class: "lead"}, "The Launcher shows six tiles per page. Choose which tiles appear and reorder them.",
      capable ? " Star a visible tile to move it to the front." : ""),
    box, h("div", {class: "row", style: "margin-top:10px"}, save, reset, message));
  const current = () => el.isConnected && target.id === deviceId();
  async function submit(resetting) {
    if (!current() || (resetting ? reset : save).disabled) return;
    const change = args();
    busy = true;
    paint();
    message.textContent = "Saving…";
    const reply = await proto("launcher", resetting ? {reset: 1} : change, target, peer);
    if (!current()) return;
    if (!reply.ok || reply.reply?.ok === false || (peer && reply.device_address !== peer)) {
      message.textContent = reply.error || reply.reply?.error || "The stick did not confirm the layout. Try again.";
      busy = false;
      paint();
      return;
    }
    await refresh(resetting);
  }
  function paint() {
    clear(box);
    const visibleOrder = [...rows].sort((a, b) => group(a) - group(b));
    for (const [i, x] of visibleOrder.entries()) {
      const [, , name, about, fixed] = x.tile;
      const disabled = busy || !writable;
      function arrow(offset) {
        const other = visibleOrder[i + offset];
        const canMove = other && group(other) === group(x) && (!capable || x.shown);
        return h("button", {class: "btn sm ghost", type: "button", disabled: disabled || !canMove,
          "aria-label": `Move ${name} ${offset < 0 ? "up" : "down"}`, onclick: () => {
            const a = rows.indexOf(x), b = rows.indexOf(other);
            [rows[a], rows[b]] = [other, x];
            paint();
          }}, offset < 0 ? "↑" : "↓");
      }
      box.appendChild(h("div", {class: "item" + (x.shown ? "" : " faint")},
        h("label", {class: "switch", title: fixed ? "Always on the stick" : "Show this tile"},
          h("input", {type: "checkbox", checked: x.shown, disabled: disabled || !!fixed, "aria-label": `Show ${name}`,
            onchange: e => { x.shown = e.target.checked; if (!x.shown) x.favourite = false; paint(); }}), h("span", null, "")),
        h("div", {class: "what"}, h("b", null, name), h("span", {class: "sub"}, fixed ? `${about} · always shown` : about)),
        h("div", {class: "row"}, capable ? h("button", {class: "btn sm ghost", type: "button",
          "aria-label": `Favourite ${name}`, "aria-pressed": String(x.favourite), disabled: disabled || !x.shown,
          title: x.favourite ? "Remove from favourites" : "Add to favourites",
          onclick: () => { x.favourite = !x.favourite; paint(); }}, x.favourite ? "★" : "☆") : null,
          arrow(-1), arrow(1))));
    }
    save.disabled = busy || !writable || JSON.stringify(args()) === baseline;
    reset.disabled = busy || !writable;
    if (!writable) message.textContent = "Connect this stick over Bluetooth to edit the Launcher.";
  }
  paint();
  return el;
}
