"""The card at the bottom of the stick's home screen."""
import time

import pytest

from stick_agent import home


class FakeFeed:
    def __init__(self, items):
        self.items = items
        self._i = -1

    def next(self):
        self._i = (self._i + 1) % len(self.items)
        return self.items[self._i]


class FakeBrain:
    busy = False
    proc = None


class FakeAgent:
    def __init__(self, items=(), cfg=None):
        self.prompts = {}
        self.last_reply = None
        self.feed = FakeFeed(list(items))
        self.brain = FakeBrain()
        self.cfg = cfg or {}


@pytest.fixture
def no_sessions(monkeypatch):
    monkeypatch.setattr(home, "claude_sessions", lambda exclude_pid=None: 0)


@pytest.fixture
def clock(monkeypatch):
    now = [1000.0]
    monkeypatch.setattr(home.time, "time", lambda: now[0])
    return now


def test_greeting_when_nothing_to_show(no_sessions):
    card = home.Home().card(FakeAgent())
    assert card["k"] == "hello" and card["a"].startswith("Good ")


def test_approval_is_pinned_and_urgent(no_sessions):
    a = FakeAgent()
    a.prompts = {"p1": {"tool": "Bash"}, "p2": {"tool": "Edit"}}
    card = home.Home().card(a)
    assert card == {"k": "approval", "a": "Claude wants Bash +1", "b": "A yes  ·  B no", "u": True}


def test_rotation_and_position(monkeypatch, clock):
    monkeypatch.setattr(home, "claude_sessions", lambda exclude_pid=None: 2)
    a = FakeAgent(items=[{"k": "fact", "a": "Fun fact", "b": "x"}])
    h = home.Home()
    seen = []
    for _ in range(4):
        c = h.card(a)
        seen.append((c["k"], c["i"], c["n"]))
        clock[0] += home.ROTATE_S
    assert seen == [("fact", 1, 2), ("claude", 0, 2), ("fact", 1, 2), ("claude", 0, 2)]


def test_card_holds_until_rotation_time(monkeypatch, clock):
    monkeypatch.setattr(home, "claude_sessions", lambda exclude_pid=None: 1)
    a = FakeAgent(items=[{"k": "news", "a": "Hacker News", "b": "y"}])
    h = home.Home()
    first = h.card(a)
    clock[0] += home.ROTATE_S - 1
    assert h.card(a)["k"] == first["k"]


def test_fresh_reply_then_it_expires(no_sessions, clock):
    a = FakeAgent()
    a.last_reply = {"t": clock[0], "text": "It's 28°C."}
    h = home.Home()
    assert h.card(a)["k"] == "reply"
    clock[0] += home.FRESH_REPLY_S + home.ROTATE_S
    assert h.card(a)["k"] == "hello"


def test_ticker_off_hides_the_feed(no_sessions):
    a = FakeAgent(items=[{"k": "fact", "a": "Fun fact", "b": "x"}], cfg={"ticker": False})
    assert home.Home().card(a)["k"] == "hello"


def test_claude_sessions_counts_claude_processes(monkeypatch):
    out = ("  101 /usr/bin/zsh\n  102 claude\n"
           "  103 /Users/x/Library/Claude/claude-code/2.1/claude.app/Contents/MacOS/claude\n"
           "  104 /opt/claude-helper\n  105 claude\n")

    class R:
        stdout = out
    monkeypatch.setattr(home.subprocess, "run", lambda *a, **k: R())
    assert home.claude_sessions() == 3
    assert home.claude_sessions(exclude_pid=105) == 2


def test_claude_sessions_survives_ps_failing(monkeypatch):
    def boom(*a, **k):
        raise OSError("no ps")
    monkeypatch.setattr(home.subprocess, "run", boom)
    assert home.claude_sessions() == 0


def test_greeting_by_time_of_day(monkeypatch):
    for hour, word in [(8, "morning"), (14, "afternoon"), (21, "evening")]:
        monkeypatch.setattr(home.time, "localtime", lambda h=hour: time.struct_time((2026, 10, 20, h, 0, 0, 1, 293, 0)))
        assert word in home.greeting()["a"]
