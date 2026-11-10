// The Stick page's layout, without the DOM (tests/js/stick_layout.test.mjs runs it in Node):
// its sections in priority order, which cards each holds and when a card is there (older
// firmware lacks some), what each card is drawn from (so a change repaints only the cards
// whose data changed), and the one-line summary a section's heading shows.
//
// ctx: {st: the stick's @state reply or null, gamesInfo, snd, petInfo, keys, via, stickName}

// Most-used first; rarely used and risky (renames, forgetting a Mac) last.
export const SECTIONS = [
  {id: "status", title: "Status", short: "Status", cards: ["hero"], fixed: true},
  {id: "screen", title: "Screen & look", short: "Screen", cards: ["screen", "face"]},
  {id: "buttons", title: "Buttons & shortcuts", short: "Buttons", cards: ["combo", "unlock", "launcher"]},
  {id: "sound", title: "Sound", short: "Sound", cards: ["sounds"]},
  {id: "motion", title: "Motion & gestures", short: "Motion", cards: ["gestures"]},
  {id: "remote", title: "Remote modes", short: "Remote", cards: ["air"]},
  {id: "games", title: "Games", short: "Games", cards: ["games"]},
  {id: "faces", title: "More faces", short: "Faces", cards: ["market"]},
  {id: "pairing", title: "Name & Macs", short: "Macs", cards: ["name", "slots"], closed: true},
  {id: "firmware", title: "Firmware & advanced", short: "Firmware", cards: ["firmware", "dev"]},
];

const pick = (o, ...ks) => Object.fromEntries(ks.map(k => [k, o ? o[k] : undefined]));
const always = c => !!c.st;

// has(ctx): the card is shown; deps(ctx): what it's drawn from (compared as JSON).
export const CARDS = {
  hero: {has: () => true, deps: c => [pick(c.st, "name", "battery", "charging", "fw", "bt"), c.via, c.stickName, !!c.st]},
  screen: {has: always, deps: c => pick(c.st, "brightness", "standby", "autoRotate", "landscape", "clock24")},
  face: {has: c => !!(c.st && c.st.faces), deps: c => [pick(c.st, "face", "faces"), c.petInfo]},  // firmware 2.1.0+
  combo: {has: always, deps: c => [pick(c.st, "mods", "key", "combo", "enterDelay"), c.keys]},
  unlock: {has: always, deps: c => pick(c.st, "macPassSet")},
  launcher: {has: c => !!(c.st && c.st.launcher !== undefined), deps: c => pick(c.st, "launcher")},  // 2.5.1+
  sounds: {has: always, deps: c => c.snd},
  gestures: {has: always, deps: c => pick(c.st, "motion")},
  air: {has: c => !!(c.st && c.st.air), deps: c => pick(c.st, "air", "connected")},  // 2.5.0+
  games: {has: c => !!(c.st && c.gamesInfo), deps: c => [c.gamesInfo, c.petInfo && c.petInfo.games]},  // 2.5.0+
  market: {has: c => !!(c.st && c.st.faces), deps: c => pick(c.st, "face")},
  name: {has: always, deps: c => pick(c.st, "name")},
  slots: {has: always, deps: c => pick(c.st, "slots", "slot")},
  firmware: {has: () => true, deps: () => null},  // repaints itself
  dev: {has: always, deps: c => pick(c.st, "devMode", "web")},
};

// [{...section, cards: [the cards shown]}] for the sections with anything in them.
export function layout(ctx) {
  return SECTIONS.map(s => ({...s, cards: s.cards.filter(k => CARDS[k].has(ctx))})).filter(s => s.cards.length);
}

// Changes when cards come or go (the stick answered or stopped answering, other firmware):
// then the page is laid out again; otherwise only changed cards are.
export function structureKey(ctx) {
  return layout(ctx).map(s => s.id + ":" + s.cards.join(",")).join("|");
}

export function slicesOf(ctx) {
  const out = {};
  for (const s of layout(ctx)) for (const k of s.cards) out[k] = JSON.stringify(CARDS[k].deps(ctx) ?? null);
  return out;
}

export function changedCards(prev, next) {
  return Object.keys(next).filter(k => prev[k] !== next[k]);
}

// Which sections are open: the defaults, then what this browser remembered.
export function openSections(saved) {
  const out = {};
  for (const s of SECTIONS) out[s.id] = s.fixed ? true : saved && typeof saved[s.id] === "boolean" ? saved[s.id] : !s.closed;
  return out;
}

const MODE = {mouse: "air mouse on", presenter: "presenter on", media: "media remote on"};

// The short line next to a section's heading (handy when it's folded away).
export function summary(id, ctx) {
  const st = ctx.st;
  if (!st) return "";
  switch (id) {
    case "screen": {
      const face = (st.faces || []).find(f => f.id === st.face);
      return [st.brightness ? `brightness ${st.brightness}%` : null, face ? `${face.title} face` : null,
        st.clock24 ? "24-hour clock" : null].filter(Boolean).join(" · ");
    }
    case "buttons": return st.combo ? `A holds ${st.combo.split("+").join(" + ")}` : "";
    case "sound": return ctx.snd ? (ctx.snd.on ? `on · volume ${Math.round(ctx.snd.volume / 2.55)}%` : "off") : "";
    case "motion": {
      const m = st.motion || {};
      const on = [m.lift ? "lift to wake" : null, m.face ? "face down" : null, m.shake ? "shake" : null].filter(Boolean);
      return on.length ? on.join(", ") : "all off";
    }
    case "remote": return st.air ? MODE[st.air.mode] || "not running" : "";
    case "games": {
      const g = (ctx.petInfo && ctx.petInfo.games) || {};
      return `${g.xpToday || 0} of ${g.cap || 10} XP today`;
    }
    case "pairing": {
      const paired = (st.slots || []).filter(s => s.paired).length;
      return `${st.name || ""}${st.slot ? ` · on Mac ${st.slot}` : ""}${paired ? ` · ${paired} paired` : ""}`;
    }
    case "firmware": return st.fw ? `firmware ${st.fw}` : "";
    default: return "";
  }
}
