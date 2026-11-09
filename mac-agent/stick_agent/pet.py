"""StickyBot's pet progress and the "world" the stick's faces draw (StickyWorld, Aquarium).

The agent keeps the progress, not the stick: it sees every event (merged PRs, CI, Claude
Code finishing), and the progress survives reflashing the stick. It lives in
~/.stick-agent/pet.json; the stick caches the last values it got so it can draw without us.

Rules (the stick has the same ones in m5-stick-s3/src/logic/pet.cpp):
  XP       merged PR +10, CI green +3, a Claude task finished +2, a release +25
  Level    1 at 0 XP; level n -> n+1 needs 10 + 10 * n XP (20, 30, 40...)
  Streak   days in a row with at least one merged PR or finished Claude task
  Unlocks  hat at a 5-day streak (best ever), sunglasses at 10 green builds, crown at 100 PRs
  Games    (stick firmware 2.5.0) a finished game with points +1, a new best +3, at most 10 XP a
           day from games (one merged PR's worth: playing can't outrun real work, and a level
           from level 2 on needs 30+ XP); they don't count for the streak
  City     a building per merged PR (drawn from `seed`), a tower per release (`towers`:
           how many buildings stood when it came)

Sent to the stick as {"src":"agent","world":{...}} (also in every hello), see PROTOCOL.md.
"""
import datetime
import json
import logging
import os
import random
import time
import zlib

from . import config

log = logging.getLogger("pet")

XP_MERGE, XP_GREEN, XP_CLAUDE, XP_RELEASE = 10, 3, 2, 25
XP_GAME, XP_GAME_BEST, GAME_XP_DAY_CAP = 1, 3, 10
GAMES = {"stack": "Stack", "flappy": "Flappy", "dino": "Dino", "reaction": "Reaction",  # firmware 2.5.0
         "maze": "Maze", "balance": "Balance", "catch": "Catch"}
LOWER_IS_BETTER = {"reaction"}  # milliseconds
MAX_SCORE = 10_000_000
HAT, SHADES, CROWN = 1, 2, 4
MAX_TOWERS = 8
MAX_SESSIONS = 6
RUNNING_MAX_S = 2 * 3600  # a prompt with no Stop after this long: the session is gone, not working


def state_path():
    return config.HOME / "pet.json"


def xp_to_next(level):
    return 10 + 10 * level


def level_for(xp):
    """-> (level, xp into it, xp it needs)"""
    xp, level = max(0, int(xp)), 1
    while level < 999 and xp >= xp_to_next(level):
        xp -= xp_to_next(level)
        level += 1
    return level, xp, xp_to_next(level)


def accessories(best_streak, greens, merged):
    return (HAT if best_streak >= 5 else 0) | (SHADES if greens >= 10 else 0) | (CROWN if merged >= 100 else 0)


def today():
    return datetime.date.today().toordinal()


class Pet:
    def __init__(self, initial_merged=0):
        self.s = None
        try:
            self.s = json.loads(state_path().read_text())
        except (OSError, ValueError):
            pass
        if not isinstance(self.s, dict):
            # a new pet: the city starts with the PRs the GitHub watcher already saw merged
            self.s = {"xp": initial_merged * XP_MERGE, "merged": initial_merged, "greens": 0, "claude": 0,
                      "releases": 0, "towers": [], "seed": random.randrange(1, 2**31),
                      "streak": {"days": 0, "best": 0, "last": -1}, "today": {"day": today(), "merged": 0}}
            self.save()

    def save(self):
        try:
            tmp = state_path().with_suffix(".tmp")
            tmp.write_text(json.dumps(self.s))
            tmp.replace(state_path())
        except OSError as e:
            log.warning("pet: can't save (%s)", e)

    # --- what happened ------------------------------------------------------------

    def _active(self, day=None):
        day = today() if day is None else day
        st = self.s["streak"]
        if st["last"] != day:
            st["days"] = st["days"] + 1 if st["last"] >= 0 and day - st["last"] == 1 else 1
            st["last"] = day
            st["best"] = max(st["best"], st["days"])

    def _today(self, day=None):
        day = today() if day is None else day
        if self.s["today"].get("day") != day:
            self.s["today"] = {"day": day, "merged": 0}
        return self.s["today"]

    def merged(self, day=None):
        self.s["merged"] += 1
        self.s["xp"] += XP_MERGE
        self._today(day)["merged"] += 1
        self._active(day)
        self.save()

    def green(self):
        self.s["greens"] += 1
        self.s["xp"] += XP_GREEN
        self.save()

    def claude_done(self, day=None):
        self.s["claude"] += 1
        self.s["xp"] += XP_CLAUDE
        self._active(day)
        self.save()

    def release(self):
        self.s["releases"] += 1
        self.s["xp"] += XP_RELEASE
        self.s["towers"] = (self.s["towers"] + [self.s["merged"]])[-MAX_TOWERS:]
        self.save()

    def game(self, gid, score, best=None, new_best=False, day=None):
        """A game finished on the stick ({"evt":"game"}). Returns the XP it earned (0..3): +1 with
        points, +3 for a new best, at most GAME_XP_DAY_CAP a day. Junk is ignored (0)."""
        if gid not in GAMES or isinstance(score, bool) or not isinstance(score, int) or not 0 <= score <= MAX_SCORE:
            return 0
        day = today() if day is None else day
        g = self.s.setdefault("games", {})
        if g.get("day") != day:
            g["day"], g["xp"] = day, 0
        plays, bests = g.setdefault("plays", {}), g.setdefault("best", {})
        plays[gid] = int(plays.get(gid, 0)) + 1
        if isinstance(best, int) and not isinstance(best, bool) and 0 < best <= MAX_SCORE:
            bests[gid] = best  # the stick keeps the bests (NVS); this is the last it told us
        xp = 0
        if score > 0:
            xp = min(XP_GAME_BEST if new_best is True else XP_GAME, max(0, GAME_XP_DAY_CAP - g["xp"]))
        g["xp"] += xp
        self.s["xp"] += xp
        self.save()
        return xp

    def games(self, day=None):
        """For the dashboard: {"xpToday", "cap", "best": {id: n}, "plays": {id: n}}."""
        day = today() if day is None else day
        g = self.s.get("games") or {}
        return {"xpToday": g.get("xp", 0) if g.get("day") == day else 0, "cap": GAME_XP_DAY_CAP,
                "best": dict(g.get("best") or {}), "plays": dict(g.get("plays") or {})}

    # --- what the stick and the dashboard see -------------------------------------

    def streak(self, day=None):
        day = today() if day is None else day
        st = self.s["streak"]
        return st["days"] if st["last"] >= 0 and day - st["last"] <= 1 else 0

    def summary(self):
        level, into, need = level_for(self.s["xp"])
        return {"level": level, "xp": self.s["xp"], "into": into, "need": need, "streak": self.streak(),
                "best": self.s["streak"]["best"], "merged": self.s["merged"], "mergedToday": self._today()["merged"],
                "greens": self.s["greens"], "claude": self.s["claude"], "releases": self.s["releases"],
                "acc": accessories(self.s["streak"]["best"], self.s["greens"], self.s["merged"]),
                "games": self.games()}

    def pet_json(self):
        return {"xp": self.s["xp"], "streak": self.streak(),
                "acc": accessories(self.s["streak"]["best"], self.s["greens"], self.s["merged"]),
                "seed": self.s["seed"], "greens": self.s["greens"], "towers": self.s["towers"][-MAX_TOWERS:]}


def running_sessions(now=None):
    """Claude Code sessions working right now, from the hooks' file (claude_hooks.py):
    a prompt was sent and its Stop hasn't come yet. -> [{"p": project, "s": secs, "id": n}]"""
    now = now or time.time()
    try:
        d = json.loads((config.HOME / "claude_sessions.json").read_text())
    except (OSError, ValueError):
        return []
    out = []
    for sid, v in d.items() if isinstance(d, dict) else []:
        age = now - float(v.get("t", 0))
        if 0 <= age < RUNNING_MAX_S:
            project = os.path.basename((v.get("cwd") or "").rstrip("/")) or "claude"
            out.append({"p": project[:12], "s": int(age), "id": (zlib.crc32(sid.encode()) & 0xFFFF) or 1})
    out.sort(key=lambda x: -x["s"])
    return out[:MAX_SESSIONS]


def ci_state(gh_state):
    """green | red | none, over your open PRs (red wins)."""
    prs = [p for p in ((gh_state or {}).get("prs") or {}).values() if p.get("state") == "OPEN"]
    if any(p.get("ci") in ("FAILURE", "ERROR") for p in prs):
        return "red"
    if any(p.get("ci") == "SUCCESS" for p in prs):
        return "green"
    return "none"


def world(agent):
    """The compact "world" message for the stick (~150-300 bytes)."""
    from .core import stick_text  # (core imports us)
    gh = getattr(agent.github, "state", None) or {}
    pet = agent.pet
    sessions = [{**s, "p": stick_text(s["p"])} for s in running_sessions()]
    return {"mergedToday": pet._today()["merged"], "merged": pet.s["merged"], "ci": ci_state(gh),
            "asks": len(agent.prompts) + len(getattr(agent, "asks", {}) or {}),
            "reviews": len(gh.get("asks") or {}), "claude": sessions, "pet": pet.pet_json()}
