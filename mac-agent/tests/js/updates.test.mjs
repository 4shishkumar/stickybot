// What the update banner and the Settings card show (static/update_ui.js): run with `node --test`.
import test from "node:test";
import assert from "node:assert/strict";
import {fmtBytes, progressPercent, progressText, waitingText, errorHeading, hasInstaller, appFlow, view, updatedNotice}
  from "../../stick_agent/static/update_ui.js";

// what the Rust app with in-app installs answers (an update from 2.5.0 to 2.6.0 that it can install)
const rust = (extra = {}) => ({
  enabled: true, channel: "stable", checking: false, error: "", note: "", available: true, agent_update: true,
  firmware_update: false, title: "Update available: v2.6.0", state: "available", progress: null, waiting: null,
  install_error: null, updated: null, auto: false,
  current: {agent: "2.5.0", firmware: "2.7.0"},
  latest: {tag: "v2.6.0", version: "2.6.0", url: "https://example.invalid/v2.6.0", notes: "What changed", updater_asset: "latest.json"},
  install: {kind: "rust", can_update: true, why: "", download: "https://example.invalid/v2.6.0"},
  ...extra,
});

// what the Python app (and a Rust app from before in-app installs) answers: no `state`
const legacy = () => ({
  enabled: true, channel: "stable", checking: false, error: "", available: true, agent_update: true, firmware_update: false,
  title: "Update available: v2.6.0", current: {agent: "2.5.0", firmware: "2.7.0"},
  latest: {tag: "v2.6.0", version: "2.6.0", url: "https://example.invalid/v2.6.0"},
  install: {kind: "release", can_update: true, why: "", head: null, branch: null},
});

test("bytes read like the Finder's: 12900000 is 12.3 MB", () => {
  assert.equal(fmtBytes(12_900_000), "12.3 MB");
  assert.equal(fmtBytes(39_800_000), "38.0 MB");
  assert.equal(fmtBytes(0), "0 B");
  assert.equal(fmtBytes(900), "900 B");
  assert.equal(fmtBytes(2048), "2.0 KB");
  assert.equal(fmtBytes(undefined), "?");
  assert.equal(fmtBytes(-5), "?");
});

test("the progress bar and its words: 12.3 MB of 38.0 MB is 32 %", () => {
  const p = {bytes: 12_900_000, total: 39_800_000};
  assert.equal(progressPercent(p), 32);
  assert.equal(progressText(p), "Downloading… 12.3 MB of 38.0 MB");
  // the server didn't say how big: an indeterminate bar, the size so far
  assert.equal(progressPercent({bytes: 5_000_000, total: null}), null);
  assert.equal(progressText({bytes: 5_000_000, total: null}), "Downloading… 4.8 MB");
  assert.equal(progressText(null), "Downloading…");
  assert.equal(progressText({bytes: 0, total: 100}), "Downloading…");
  // never past the ends
  assert.equal(progressPercent({bytes: 50, total: 10}), 100);
  assert.equal(progressPercent({bytes: 0, total: 10}), 0);
  assert.equal(progressPercent(undefined), null);
});

test("the old shape (no `state`) keeps the old card and button", () => {
  assert.equal(hasInstaller(legacy()), false);
  assert.deepEqual(view(legacy()), {mode: "legacy"});
  assert.equal(appFlow(legacy()), false);
  assert.equal(hasInstaller({error: "Not in the Rust app yet.", missing: true}), false);
  assert.equal(hasInstaller(null), false);
  assert.equal(hasInstaller(rust()), true);
});

test("an update the app can install offers Install and restart; one it can't gets the download page", () => {
  assert.deepEqual(view(rust()), {mode: "offer"});
  const stuck = rust({install: {kind: "rust", can_update: false, why: "StickyBot is running from a temporary read-only copy.", download: "https://example.invalid/v2.6.0"}});
  assert.deepEqual(view(stuck), {mode: "fallback", why: "StickyBot is running from a temporary read-only copy.", download: "https://example.invalid/v2.6.0"});
  // still checking: the offer stays
  assert.equal(view(rust({state: "checking"})).mode, "offer");
  // nothing new
  assert.deepEqual(view(rust({agent_update: false, available: false, title: "", state: "idle"})), {mode: "none"});
});

test("while it runs: download with its bar, the signature check, the restart", () => {
  const dl = view(rust({state: "downloading", progress: {bytes: 12_900_000, total: 39_800_000}}));
  assert.deepEqual(dl, {mode: "downloading", percent: 32, text: "Downloading… 12.3 MB of 38.0 MB"});
  assert.equal(view(rust({state: "downloading", progress: null})).percent, null);
  assert.deepEqual(view(rust({state: "installing"})), {mode: "installing", text: "Checking the signature and installing…"});
  assert.deepEqual(view(rust({state: "restarting"})), {mode: "restarting", text: "Restarting StickyBot…"});
});

test("a verified download holds back while the stick is busy, and says why", () => {
  const w = view(rust({waiting: "busy — will install when idle: a firmware update is running"}));
  assert.deepEqual(w, {mode: "waiting", text: "Busy — will install when idle: a firmware update is running"});
  assert.equal(waitingText(""), "");
});

test("an error says what happened in words, with Try again and the download page", () => {
  const e = view(rust({state: "error", install_error: {kind: "signature", message: "The download didn't match StickyBot's signing key, so it was not installed.", download: "https://example.invalid/v2.6.0"}}));
  assert.deepEqual(e, {mode: "error", heading: "The update's signature didn't match",
    message: "The download didn't match StickyBot's signing key, so it was not installed.",
    download: "https://example.invalid/v2.6.0", canRetry: true});
  assert.equal(errorHeading("network"), "No connection");
  assert.equal(errorHeading("not-writable"), "Can't write to StickyBot's folder");
  assert.equal(errorHeading("no-update"), "Nothing to install");
  assert.equal(errorHeading("something new"), "The update didn't work");
  // no `install_error` (the pages joined late): still words, and the page from `install`
  const bare = view(rust({state: "error", install_error: null}));
  assert.equal(bare.message, "The update didn't work.");
  assert.equal(bare.download, "https://example.invalid/v2.6.0");
  // can't install here: no Try again
  assert.equal(view(rust({state: "error", install: {can_update: false, download: "x"}})).canRetry, false);
});

test("the app flow covers an app update and its outcome, not a firmware-only update", () => {
  assert.equal(appFlow(rust()), true);
  assert.equal(appFlow(rust({state: "downloading"})), true);
  assert.equal(appFlow(rust({state: "error"})), true);
  assert.equal(appFlow(rust({waiting: "busy — will install when idle: x", state: "available"})), true);
  assert.equal(appFlow(rust({agent_update: false, state: "idle", updated: {from: "2.5.0", to: "2.6.0", at: 5}})), true);
  // the stick's firmware only: the old banner (Update the stick -> the Stick page)
  assert.equal(appFlow(rust({agent_update: false, firmware_update: true, title: "Stick firmware 2.8.0 available", state: "idle"})), false);
  assert.equal(appFlow(rust({agent_update: false, available: false, state: "idle"})), false);
});

test("'Updated to vX' shows once, with the notes link when the check has that release", () => {
  const u = rust({agent_update: false, state: "idle", updated: {from: "2.5.0", to: "2.6.0", at: 1_760_000_000}});
  assert.deepEqual(updatedNotice(u, null), {text: "Updated to v2.6.0", from: "2.5.0", at: 1_760_000_000, notes: "https://example.invalid/v2.6.0"});
  // dismissed in this browser (by the time stamp): gone
  assert.equal(updatedNotice(u, 1_760_000_000), null);
  // a newer release since: no link to the wrong notes
  const newer = rust({agent_update: true, updated: {from: "2.5.0", to: "2.6.0", at: 5}, latest: {version: "2.7.0", url: "https://example.invalid/v2.7.0"}});
  assert.equal(updatedNotice(newer, null).notes, null);
  assert.equal(updatedNotice(rust(), null), null);
  assert.equal(updatedNotice(legacy(), null), null);
});
