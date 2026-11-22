// Both dashboard pages use the stick's saved on/off flag as mute, so the
// hardware remembers the exact level (0–255) across apps and restarts.
import {h, report} from "./lib.js";

// Reuse the dashboard's Sounds icon in both volume controls and the sidebar.
export function speakerPaths(muted) {
  const speaker = '<path d="M4 9.5h3l5-4v13l-5-4H4z"/>';
  const mark = muted ? '<path d="M3 3l18 18"/>' : '<path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"/>';
  return speaker + mark;
}

export function volumeControl(initial, target, id) {
  let confirmed = initial, desired = initial, queued = null, reportVersion = 0;
  let writing = false, timer = null, dragging = false, connected = true;
  const slider = h("input", {type: "range", id, min: 0, max: 100, step: 1, "aria-label": "Stick volume"});
  const level = h("output", {for: id, class: "num small", style: "min-width:4ch"});
  const mute = h("button", {type: "button", class: "btn icon"});
  const el = h("div", {class: "row"}, mute, slider, level);

  function paint() {
    const pct = desired.on && desired.volume ? Math.max(1, Math.round(desired.volume * 100 / 255)) : 0;
    slider.value = pct;
    slider.setAttribute("aria-valuetext", pct ? pct + "%" : "Muted");
    level.textContent = connected ? pct + "%" : "—";
    const action = pct ? "Mute stick speaker" : "Unmute stick speaker";
    if (mute.title !== action) {
      mute.title = action;
      mute.setAttribute("aria-label", action);
      mute.innerHTML = `<svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${speakerPaths(!pct)}</svg>`;
    }
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
  const end = () => {
    dragging = false;
    if (queued) flush();
    else if (!writing) { desired = confirmed; paint(); }
  };
  slider.addEventListener("change", end);
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
