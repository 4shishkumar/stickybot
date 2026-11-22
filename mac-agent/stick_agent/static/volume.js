// Both dashboard pages use the stick's saved on/off flag as mute, so the
// hardware remembers the exact level (0–255) across apps and restarts.
import {h, report} from "./lib.js";

export function volumeControl(initial, target, id) {
  let confirmed = initial, desired = initial, queued = null, reportVersion = 0;
  let writing = false, timer = null, dragging = false, connected = true;
  const slider = h("input", {type: "range", id, min: 0, max: 100, step: 1, "aria-label": "Stick volume"});
  const level = h("output", {for: id, class: "num small", style: "min-width:4ch"});
  const mute = h("button", {type: "button", class: "btn sm"});
  const el = h("div", {class: "row"}, mute, slider, level);

  function paint() {
    const pct = desired.on && desired.volume ? Math.max(1, Math.round(desired.volume * 100 / 255)) : 0;
    slider.value = pct;
    slider.setAttribute("aria-valuetext", pct ? pct + "%" : "Muted");
    level.textContent = connected ? pct + "%" : "—";
    mute.textContent = pct ? "Mute" : "Unmute";
    mute.setAttribute("aria-label", pct ? "Mute stick speaker" : "Unmute stick speaker");
    slider.disabled = mute.disabled = !connected;
  }

  // At most one request in flight. During a drag send the latest level at most
  // five times per second; always flush the final position on release.
  async function flush() {
    clearTimeout(timer);
    timer = null;
    if (writing || !queued || !connected) return;
    const payload = queued, version = reportVersion;
    queued = null;
    writing = true;
    const answer = await target.put("/api/sounds", payload);
    writing = false;
    // An event can already have delivered a newer hardware level while HTTP was returning.
    if (report(answer) && version === reportVersion) confirmed = answer;
    if (queued) schedule();
    else if (!dragging) { desired = confirmed; paint(); }
  }
  function schedule() {
    if (!writing && timer === null) timer = setTimeout(flush, 200);
  }
  function select(pct) {
    desired = {on: pct > 0, volume: pct ? Math.round(pct * 255 / 100) : desired.volume};
    queued = desired;
    paint();
  }
  slider.addEventListener("pointerdown", () => { dragging = true; });
  slider.addEventListener("input", () => { select(+slider.value); schedule(); });
  slider.addEventListener("change", () => {
    dragging = false;
    flush();
  });
  const end = () => { dragging = false; flush(); };
  slider.addEventListener("pointerup", end);
  slider.addEventListener("pointercancel", end);
  mute.addEventListener("click", () => {
    desired = {on: !desired.on || !desired.volume, volume: desired.volume || 140};
    queued = desired;
    paint();
    flush();
  });
  paint();
  return {
    el,
    sync(s) {
      reportVersion++;
      confirmed = s;
      if (!writing && !queued && !dragging) { desired = s; paint(); }
    },
    link(up) {
      connected = up;
      if (!up) { clearTimeout(timer); timer = null; queued = null; dragging = false; }
      paint();
    },
  };
}
