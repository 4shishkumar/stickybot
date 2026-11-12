"""Events for the stick: celebrations, nice-to-know toasts and heads-ups.

Something happens (a PR merged, Claude Code finished a long task...) -> a
"kind". Each kind has a level, set in the dashboard (Events tab):
  celebrate  full-screen confetti + fanfare      (PR merged, release, CI fixed)
  nice       toast + chime                       (PR created / approved, Claude done)
  headsup    orange toast, stays until a button  (review requested, CI failed)
  off        nothing
During quiet hours events still show, but silently.

Each kind can also have its own sound (Events tab, config "event_sounds" {kind: name | "off"}):
a clip or tune the stick has ("applause"), "off", or "" for its level's default sound (the stick
picks it: celebrate jingle_win, nice confirmation_002, headsup question_001). PR merged, release
and CI fixed have their own by default (DEFAULT_SOUNDS, firmware 2.7.0's celebration clips). A
name the stick doesn't list goes out as "" (firmware 2.7.0 falls back by itself; older ones would
play nothing).

Sent to the stick as:
  {"src":"agent","event":{"id","level","title","sub","icon","sound"}}
and the stick answers {"src":"agent","evt":"event_shown","id"}.
"""
import json
import logging
import re
import time

from . import config

log = logging.getLogger("events")
LEVELS = ("celebrate", "nice", "headsup", "off")

# kind -> (default level, icon, sample title, sample sub) — samples are for the Test buttons
KINDS = {
    "pr_merged":         ("celebrate", "merge",  "PR #5507 merged!",          "web - Add stick agent"),
    "release":           ("celebrate", "star",   "Release v2.4.0",            "web"),
    "ci_fixed":          ("celebrate", "check",  "CI is green again",         "web #5507"),
    "pr_created":        ("nice",      "pr",     "PR #5508 opened",           "web - Fix weather chip"),
    "pr_approved":       ("nice",      "check",  "PR #5507 approved",         "web - by a teammate"),
    "claude_done":       ("nice",      "claude", "Claude finished in web",   "took 4 min"),
    "devin_review":      ("nice",      "review", "Devin reviewed #5507",      "web - 3 comments"),
    "review_requested":  ("headsup",   "review", "Review requested",          "web #5510 - Update deps"),
    "changes_requested": ("headsup",   "x",      "Changes requested",         "web #5507"),
    "ci_failed":         ("headsup",   "x",      "CI failed",                 "web #5507 - build"),
    # notify() / celebrate() from a Claude session (the stick tools, mcp_server.py): the session picks
    # the level; "off" here silences them all
    "claude_tool":       ("nice",      "claude", "Tests passed",              "from web"),
    "update":            ("nice",      "star",   "Update v1.8.0 ready",       "see the dashboard"),  # updates.py
    "test":              ("nice",      "bell",   "Test event",                "from the dashboard"),
}
DEFAULT_RULES = {k: v[0] for k, v in KINDS.items()}
# kind -> its own sound (the rest: "" = the level's default, chosen on the stick)
DEFAULT_SOUNDS = {"pr_merged": "applause", "release": "party_horn", "ci_fixed": "cheer"}
LEVEL_SOUNDS = {"celebrate": "jingle_win", "nice": "confirmation_002", "headsup": "question_001"}  # events.cpp
# firmware 2.7.0's celebration clips: offered while the stick hasn't told us its list
CELEBRATION_SOUNDS = ("applause", "clap", "cheer", "party_horn", "tada", "drumroll", "level_up", "coins")
_NAME = re.compile(r"[A-Za-z0-9_-]{1,32}")
EVENTS_LOG = config.HOME / "events.jsonl"


def rules(cfg):
    r = dict(DEFAULT_RULES)
    r.update({k: v for k, v in (cfg.get("event_rules") or {}).items() if v in LEVELS})
    return r


def _sound_ok(v):
    return isinstance(v, str) and (v in ("", "off") or bool(_NAME.fullmatch(v)))


def sounds(cfg):
    """kind -> its sound: a name, "off", or "" (the level's default)."""
    s = {k: DEFAULT_SOUNDS.get(k, "") for k in KINDS}
    s.update({k: v for k, v in (cfg.get("event_sounds") or {}).items() if k in KINDS and _sound_ok(v)})
    return s


def tune_list(stick_sounds):
    """The stick's tunes from its last sound report, or None if it hasn't sent one."""
    t = (stick_sounds or {}).get("tunes")
    return [x for x in t if isinstance(x, str)] if isinstance(t, list) else None


def for_stick(name, tunes):
    """The sound to send: a name the stick doesn't have becomes "" (its level's default)."""
    return name if name in ("", "off") or tunes is None or name in tunes else ""


def valid_config(body, tunes=None, current=None):
    """'' if event_sounds in a PUT /api/config body is fine, else what's wrong. New names must be
    ones the stick has (when it has told us); a kind's current one (saved or default) may stay, as
    the page sends them all (another stick, older firmware: they go out as "" then)."""
    es = body.get("event_sounds")
    if es is None:
        return ""
    if not isinstance(es, dict):
        return "event_sounds must map event kinds to sound names"
    now = sounds(current or {})
    for k, v in es.items():
        if k not in KINDS:
            return f"event_sounds: no event kind {k!r}"
        if not _sound_ok(v):
            return f"event_sounds.{k}: not a sound name"
        if tunes is not None and v not in ("", "off") and v not in tunes and now.get(k) != v:
            return f"event_sounds.{k}: the stick has no sound {v!r}"
    return ""


def _minutes(hhmm):
    h, _, m = str(hhmm).partition(":")
    return int(h) * 60 + int(m or 0)


def quiet_now(cfg):
    q = cfg.get("quiet_hours") or {}
    if not q.get("on"):
        return False
    try:
        start, end = _minutes(q.get("from", "19:00")), _minutes(q.get("to", "09:00"))
    except ValueError:
        return False
    t = time.localtime()
    now = t.tm_hour * 60 + t.tm_min
    return start <= now < end if start <= end else now >= start or now < end  # may wrap midnight


class Events:
    def __init__(self, agent):
        self.agent = agent
        self._n = 0
        self.shown = set()  # ids the stick acknowledged

    async def notify(self, kind, title, sub="", icon=None, level=None, sound=None):
        """Fires an event of `kind` (its level and sound come from the settings unless given;
        sound "" = the level's default, "off" = silent). Returns the event dict, or None if this
        kind is turned off."""
        from .core import stick_text  # (core imports us)
        cfg = self.agent.cfg
        level = level or rules(cfg).get(kind, "nice")
        if level == "off":
            log.info("event %s (off): %s", kind, title)
            return None
        self._n += 1
        quiet = quiet_now(cfg)
        if sound is None:
            sound = sounds(cfg).get(kind, "")
        sound = for_stick(sound, tune_list(self.agent.sounds))
        ev = {"id": f"e{self._n}", "level": level, "title": stick_text(title)[:40], "sub": stick_text(sub)[:90],
              "icon": icon or KINDS.get(kind, (None, "bell"))[1], "sound": "off" if quiet else sound,
              "kind": kind}  # the stick's faces react to it (firmware 2.1.0+; older ones ignore it)
        sent = await self.agent.link.send({"src": "agent", "event": ev})
        entry = {"t": time.time(), "kind": kind, **ev, "sent": sent, "quiet": quiet}
        try:
            with EVENTS_LOG.open("a") as f:
                f.write(json.dumps(entry) + "\n")
        except OSError:
            pass
        self.agent.emit("event", ev_kind=kind, **{k: v for k, v in entry.items() if k not in ("t", "kind")})
        log.info("event %s -> %s: %s / %s%s", kind, level, ev["title"], ev["sub"],
                 "" if sent else " (stick not connected)")
        return ev

    async def test(self, kind, level=None, sound=None):
        """The kind's sample, with the level / sound picked on the page if given (saved or not)."""
        lvl, icon, title, sub = KINDS.get(kind, KINDS["test"])
        level = level if level in LEVELS else None
        sound = sound if _sound_ok(sound) else None
        return await self.notify(kind, title, sub, icon, level=level, sound=sound)

    def acked(self, eid):
        self.shown.add(eid)


def history(n=50):
    try:
        lines = EVENTS_LOG.read_text().splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in reversed(lines):
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out
