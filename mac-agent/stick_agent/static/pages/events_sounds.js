// The Events page's level + sound per event kind, without the DOM (tests/js/events_sounds.test.mjs
// runs it in Node): the sound list a row offers, what its ▶ sends, and saving one change at a time.

export const SOUND_LABELS = {
  applause: "Applause", clap: "Clap", cheer: "Cheer", party_horn: "Party horn", tada: "Ta-da", drumroll: "Drum roll",
  level_up: "Level up", coins: "Coins", jingle_win: "Victory jingle", jingle_sax: "Sax jingle",
  confirmation_002: "Chime", question_001: "Question",
};

// The options for a kind's sound: "" (the level's own sound, named), "off", then the stick's
// tunes (the celebrations first). A saved name the stick doesn't have stays listed, marked, so
// nothing changes behind the user's back.
export function soundOptions(tunes, current, level, levelSounds) {
  const lvl = (levelSounds || {})[level];
  const opts = [{value: "", label: lvl ? `Level default (${SOUND_LABELS[lvl] || lvl})` : "Level default"},
    {value: "off", label: "Off (silent)"}];
  const list = [...new Set((tunes || []).filter(t => t && t !== "off"))];
  const fav = Object.keys(SOUND_LABELS);
  list.sort((a, b) => (fav.includes(a) ? fav.indexOf(a) : 1e3) - (fav.includes(b) ? fav.indexOf(b) : 1e3));
  for (const t of list) opts.push({value: t, label: SOUND_LABELS[t] ? `${SOUND_LABELS[t]} (${t})` : t});
  if (current && current !== "off" && !list.includes(current)) opts.push({value: current, label: `${current} (not on this stick)`});
  return opts;
}

// What a row's ▶ sends: the level and sound shown, saved or not.
export function testBody(kind, level, sound) {
  return {kind, level, sound: sound ?? ""};
}

// The name to play when only the sound is auditioned: the level's own for "".
export function audition(sound, level, levelSounds) {
  if (sound === "off") return null;
  return sound || (levelSounds || {})[level] || null;
}

// Saves one change at a time, in order (a quick second change can't overtake the first).
export function serial(fn) {
  let last = Promise.resolve();
  return (...args) => {
    const run = last.then(() => fn(...args));
    last = run.catch(() => {});
    return run;
  };
}
