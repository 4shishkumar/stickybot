"""Events for the stick: celebrations, nice-to-know toasts and heads-ups.

Something happens (a PR merged, Claude Code finished a long task...) -> a
"kind". Each kind has a level, set in the dashboard (Events tab):
  celebrate  full-screen confetti + fanfare      (PR merged, release, CI fixed)
  nice       toast + chime                       (PR created / approved, Claude done)
  headsup    orange toast, stays until a button  (review requested, CI failed)
  off        nothing
During quiet hours events still show, but silently.

Sent to the stick as:
  {"src":"agent","event":{"id","level","title","sub","icon","sound"}}
and the stick answers {"src":"agent","evt":"event_shown","id"}.
"""
import json
import logging
import time

from . import config

log = logging.getLogger("events")
LEVELS = ("celebrate", "nice", "headsup", "off")

# kind -> (default level, icon, sample title, sample sub) — samples are for the Test buttons
KINDS = {
    "pr_merged":         ("celebrate", "merge",  "PR #5507 merged!",          "loom - Add stick agent"),
    "release":           ("celebrate", "star",   "Release v2.4.0",            "loom"),
    "ci_fixed":          ("celebrate", "check",  "CI is green again",         "loom #5507"),
    "pr_created":        ("nice",      "pr",     "PR #5508 opened",           "loom - Fix weather chip"),
    "pr_approved":       ("nice",      "check",  "PR #5507 approved",         "loom - by a teammate"),
    "claude_done":       ("nice",      "claude", "Claude finished in loom",   "took 4 min"),
    "devin_review":      ("nice",      "review", "Devin reviewed #5507",      "loom - 3 comments"),
    "review_requested":  ("headsup",   "review", "Review requested",          "loom #5510 - Update deps"),
    "changes_requested": ("headsup",   "x",      "Changes requested",         "loom #5507"),
    "ci_failed":         ("headsup",   "x",      "CI failed",                 "loom #5507 - build"),
    # notify() / celebrate() from a Claude session (the stick tools, mcp_server.py): the session picks
    # the level; "off" here silences them all
    "claude_tool":       ("nice",      "claude", "Tests passed",              "from loom"),
    "test":              ("nice",      "bell",   "Test event",                "from the dashboard"),
}
DEFAULT_RULES = {k: v[0] for k, v in KINDS.items()}
EVENTS_LOG = config.HOME / "events.jsonl"


def rules(cfg):
    r = dict(DEFAULT_RULES)
    r.update({k: v for k, v in (cfg.get("event_rules") or {}).items() if v in LEVELS})
    return r


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

    async def notify(self, kind, title, sub="", icon=None, level=None, sound=""):
        """Fires an event of `kind` (its level comes from the rules unless given).
        Returns the event dict, or None if this kind is turned off."""
        from .core import stick_text  # (core imports us)
        cfg = self.agent.cfg
        level = level or rules(cfg).get(kind, "nice")
        if level == "off":
            log.info("event %s (off): %s", kind, title)
            return None
        self._n += 1
        quiet = quiet_now(cfg)
        ev = {"id": f"e{self._n}", "level": level, "title": stick_text(title)[:40], "sub": stick_text(sub)[:90],
              "icon": icon or KINDS.get(kind, (None, "bell"))[1], "sound": "off" if quiet else sound}
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

    async def test(self, kind):
        lvl, icon, title, sub = KINDS.get(kind, KINDS["test"])
        return await self.notify(kind, title, sub, icon)

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
