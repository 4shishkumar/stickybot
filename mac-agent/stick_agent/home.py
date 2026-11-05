"""The bottom card on the stick's home screen.

Pinned when something needs you (urgent, orange on the stick):
  - approvals waiting ("Claude wants Bash", "A yes · B no")
Otherwise it rotates every ROTATE_S between:
  - your last answer (for 3 min after it arrived)
  - Claude Code sessions open
  - fun facts / news / on this day (experimental, see feed.py)
  - a greeting when there's nothing else
(The stick itself covers Bluetooth, battery and "agent offline".)

Sent to the stick as {"k": kind, "a": first line, "b": second line, "u": urgent}.
"""
import os
import subprocess
import time

FRESH_REPLY_S = 180
ROTATE_S = 16


def claude_sessions(exclude_pid=None):
    """Claude Code processes running on this Mac (terminal, IDE or desktop app)."""
    try:
        out = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True, timeout=3).stdout
    except (OSError, subprocess.TimeoutExpired):
        return 0
    n = 0
    for line in out.splitlines():
        pid, _, comm = line.strip().partition(" ")
        if os.path.basename(comm.strip()) == "claude" and pid != str(exclude_pid or ""):
            n += 1
    return n


def greeting():
    h = time.localtime().tm_hour
    part = "morning" if 5 <= h < 12 else "afternoon" if 12 <= h < 17 else "evening"
    return {"k": "hello", "a": f"Good {part}", "b": "Hold B to ask me anything."}


class Home:
    def __init__(self):
        self._slot = 0
        self._last_turn = 0.0
        self._current = None

    def card(self, agent):
        if agent.prompts:
            p = next(iter(agent.prompts.values()))
            more = f" +{len(agent.prompts) - 1}" if len(agent.prompts) > 1 else ""
            return {"k": "approval", "a": f"Claude wants {p['tool']}{more}", "b": "A yes  ·  B no", "u": True}

        # Rotation candidates, rebuilt each time
        cands = []
        last = agent.last_reply
        if last and time.time() - last["t"] < FRESH_REPLY_S:
            cands.append({"k": "reply", "a": "Last answer", "b": last["text"]})
        own = getattr(agent.brain, "pid", None)  # our own Claude Code process isn't one of yours
        n = claude_sessions(own)
        if n:
            cands.append({"k": "claude", "a": "Claude Code", "b": f"{n} session{'s' if n != 1 else ''} open"})
        if agent.feed.items and agent.cfg.get("ticker", True):
            cands.append("feed")
        # (the eyes peeking from behind the card now and then are drawn by the stick itself)
        if not cands:
            return greeting()

        if self._current is None or time.time() - self._last_turn >= ROTATE_S:
            self._slot = (self._slot + 1) % len(cands)
            pick = cands[self._slot]
            self._current = agent.feed.next() if pick == "feed" else pick
            self._last_turn = time.time()
        card = dict(self._current or greeting())
        card["i"], card["n"] = self._slot, len(cands)  # for the source dots on the stick
        return card
