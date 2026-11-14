// What to show for an app update (updates.js draws it): pure functions over the /api/update the Rust
// app serves, so they run under `node --test` (tests/js/updates.test.mjs).
//
// Only the Rust app installs updates itself. Its /api/update has `state` ("idle" | "checking" |
// "available" | "downloading" | "installing" | "restarting" | "error"), `progress {bytes, total}`,
// `waiting` (why a verified download holds back), `install_error {kind, message, download}`,
// `updated {from, to}` and `auto`. The Python app's has none of them (nor does an older Rust
// app's), and the page then runs its old code path unchanged: `view` answers "legacy".

const MB = 1048576;

// 12900000 -> "12.3 MB" (the size the Finder shows is the same unit: 1 MB = 1024 x 1024 bytes).
export function fmtBytes(n) {
  if (typeof n !== "number" || !(n >= 0)) return "?";
  if (n < 1024) return `${Math.round(n)} B`;
  if (n < MB) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / MB).toFixed(1)} MB`;
}

// 0..100, or null when the size isn't known (an indeterminate bar).
export function progressPercent(p) {
  if (!p || !(p.total > 0) || typeof p.bytes !== "number") return null;
  return Math.max(0, Math.min(100, Math.round(p.bytes * 100 / p.total)));
}

export function progressText(p) {
  if (!p || typeof p.bytes !== "number" || p.bytes <= 0) return "Downloading…";
  return p.total > 0 ? `Downloading… ${fmtBytes(p.bytes)} of ${fmtBytes(p.total)}` : `Downloading… ${fmtBytes(p.bytes)}`;
}

// "busy — will install when idle: a firmware update is running" -> sentence case.
export function waitingText(w) {
  const s = String(w || "");
  return s ? s[0].toUpperCase() + s.slice(1) : "";
}

const HEADINGS = {
  "signature": "The update's signature didn't match",
  "network": "No connection",
  "not-writable": "Can't write to StickyBot's folder",
  "no-update": "Nothing to install",
};

export function errorHeading(kind) {
  return HEADINGS[kind] || "The update didn't work";
}

// The Rust app with in-app installs (the page switches to the new flow).
export function hasInstaller(u) {
  return !!u && !u.error && u.state !== undefined;
}

const RUNNING = ["downloading", "installing", "restarting"];

// Is there an app update (or an install, or its outcome) for the banner / card to talk about? A
// firmware-only update isn't: the old banner (Update the stick -> Stick page) keeps that.
export function appFlow(u) {
  return hasInstaller(u) && !!(u.agent_update || RUNNING.includes(u.state) || u.state === "error" || u.waiting || u.updated);
}

// -> {mode, ...what that mode needs}
//   legacy       no `state`: the old code path
//   none         nothing to say about the app
//   offer        an update this app can install: Install and restart
//   fallback     one it can't (`why`): the release page
//   downloading  {percent | null, text}
//   waiting      {text}: verified, holds back while the stick is busy (Cancel)
//   installing / restarting  {text}
//   error        {heading, message, download}: Try again, the release page
export function view(u) {
  if (!hasInstaller(u)) return {mode: "legacy"};
  const download = (u.install && u.install.download) || (u.latest && u.latest.url) || null;
  if (u.waiting) return {mode: "waiting", text: waitingText(u.waiting)};
  switch (u.state) {
    case "downloading":
      return {mode: "downloading", percent: progressPercent(u.progress), text: progressText(u.progress)};
    case "installing":
      return {mode: "installing", text: "Checking the signature and installing…"};
    case "restarting":
      return {mode: "restarting", text: "Restarting StickyBot…"};
    case "error": {
      const e = u.install_error || {};
      return {mode: "error", heading: errorHeading(e.kind), message: e.message || "The update didn't work.",
              download: e.download || download, canRetry: !!(u.install && u.install.can_update)};
    }
    default:
  }
  if (!u.agent_update) return {mode: "none"};
  if (u.install && u.install.can_update) return {mode: "offer"};
  return {mode: "fallback", why: (u.install && u.install.why) || "", download};
}

// "Updated to vX": from `updated` {from, to, at}, once. `seenAt` is the `at` the person already
// dismissed (kept in the browser); `latest` is the last release the check saw (for its page).
export function updatedNotice(u, seenAt) {
  const up = u && u.updated;
  if (!up || !up.to || up.at === seenAt) return null;
  const lat = u.latest;
  return {text: `Updated to v${up.to}`, from: up.from, at: up.at, notes: lat && lat.version === up.to && lat.url ? lat.url : null};
}
