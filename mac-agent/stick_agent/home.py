"""The bottom card on the stick's home screen.

Pinned when something needs you (urgent, orange on the stick):
  - approvals waiting ("Claude wants Bash", "A yes · B no")
  - a question from a Claude session (the stick tools' ask(), see mcp_server.py)
Then a line a Claude session put there with the stick tools (progress() or pin_home()),
until it expires or is replaced; the newer of the two wins.
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
PROGRESS_S = 600       # a progress line stays this long after its last update
PROGRESS_DONE_S = 120  # ... or this long once done == total


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
        self.slots = {}  # "progress" | "pin" -> {"k", "a", "b", "until", "at"} (set by the stick tools)

    def put(self, slot, a, b, secs):
        """Shows a line on the card for `secs` (a Claude session's progress or pinned line)."""
        now = time.time()
        self.slots[slot] = {"k": "claude", "a": a, "b": b, "until": now + secs, "at": now}

    def clear(self, slot):
        return self.slots.pop(slot, None) is not None

    def pinned(self):
        """The newest progress / pinned line that hasn't expired, or None."""
        now = time.time()
        for slot in [s for s, v in self.slots.items() if v["until"] <= now]:
            del self.slots[slot]
        if not self.slots:
            return None
        top = max(self.slots.values(), key=lambda v: v["at"])
        return {"k": top["k"], "a": top["a"], "b": top["b"]}

    def card(self, agent):
        if agent.prompts:
            p = next(iter(agent.prompts.values()))
            more = f" +{len(agent.prompts) - 1}" if len(agent.prompts) > 1 else ""
            return {"k": "approval", "a": f"Claude wants {p['tool']}{more}", "b": "A yes  ·  B no", "u": True}
        asks = getattr(agent, "asks", None)
        if asks:
            q = next(iter(asks.values()))
            return {"k": "approval", "a": f"{q['from']} asks", "b": q["question"], "u": True}
        pin = self.pinned()
        if pin:
            return pin

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
