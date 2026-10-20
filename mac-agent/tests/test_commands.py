"""Quick commands: what you say -> which command, and what it runs."""
import json

import pytest

from stick_agent import commands, config


@pytest.mark.parametrize("said, normal", [
    ("Open PR 1234.", "open pr 1234"),
    ("PR number 1,234", "pr 1234"),
    ("pr twelve thirty four", "pr 1234"),
    ("pr one two three four", "pr 1234"),
    ("PR four thousand five hundred twelve", "pr 4512"),
    ("pr double five", "pr 55"),
    ("Can you open pull request #42?", "can you open pull request 42"),
    ("Please open the PR 1234", "please open pr 1234"),
    ("P R 1234", "pr 1234"),
    ("П Р 1234", "pr 1234"),  # Cyrillic look-alikes from a multilingual speech model
    ("what's the weather", "what s weather"),
])
def test_normalize(said, normal):
    assert commands.normalize(said) == normal


def test_words_to_number_rejects_non_numbers():
    assert commands.words_to_number(["four", "banana"]) is None


DEFAULTS = json.loads((config.DEFAULTS / "commands.json").read_text())["commands"]


def first_match(text, cmds=DEFAULTS):
    norm = commands.normalize(text)
    for cmd in cmds:
        for phrase in cmd["phrases"]:
            m = commands.phrase_regex(phrase).match(norm)
            if m:
                return cmd["name"], {k: v for k, v in m.groupdict().items() if v}
    return None


@pytest.mark.parametrize("said, name, slots", [
    ("Open PR 1234.", "GitHub PR", {"n": "1234"}),
    ("pull request 99", "GitHub PR", {"n": "99"}),
    ("hey can you open the pr 7", "GitHub PR", {"n": "7"}),
    ("Devin review 5678", "Devin review", {"n": "5678"}),
    ("Open Slack.", "Open app", {"app": "slack"}),
    ("launch visual studio code", "Open app", {"app": "visual studio code"}),
])
def test_default_phrases_match(said, name, slots):
    assert first_match(said) == (name, slots)


@pytest.mark.parametrize("said", ["what's the weather", "how much disk space do I have", "pr", ""])
def test_no_match_goes_to_claude(said):
    assert first_match(said) is None


def test_match_reads_the_users_commands(home):
    config.COMMANDS.write_text(json.dumps({"commands": [
        {"name": "Docs", "phrases": ["docs {q}"], "open_url": "https://example.com/search?q={q}"}]}))
    cmd, slots = commands.match("Docs bluetooth pairing")
    assert cmd["name"] == "Docs" and slots == {"q": "bluetooth pairing"}


def test_match_survives_a_broken_commands_file(home):
    config.COMMANDS.write_text("{not json")
    assert commands.match("pr 1") is None


@pytest.fixture
def apps(monkeypatch):
    fake = {"slack": "Slack", "visual studio code": "Visual Studio Code", "zoom.us": "zoom.us",
            "system settings": "System Settings", "brave browser": "Brave Browser", "linear": "Linear"}
    monkeypatch.setattr(commands, "list_apps", lambda: fake)
    return fake


@pytest.mark.parametrize("spoken, app", [
    ("slack", "Slack"), ("Slack app", "Slack"), ("vs code", "Visual Studio Code"), ("zoom", "zoom.us"),
    ("settings", "System Settings"), ("brave", "Brave Browser"), ("lineer", "Linear"), ("photoshop", None),
])
def test_find_app(apps, spoken, app):
    assert commands.find_app(spoken) == app


class Recorder:
    def __init__(self):
        self.calls = []

    def __call__(self, args, **kw):
        self.calls.append((args, kw))

        class R:
            stdout, stderr = "out\n", ""
        return R()


@pytest.fixture
def ran(monkeypatch):
    r = Recorder()
    monkeypatch.setattr(commands.subprocess, "run", r)
    return r


def test_run_open_url_in_browser(ran):
    cmd = {"name": "GitHub PR", "open_url": "https://github.com/acme/app/pull/{n}", "browser": "Brave Browser"}
    assert commands.run(cmd, {"n": "1234"}) == "Opened GitHub PR #1234"
    assert ran.calls[0][0] == ["open", "-a", "Brave Browser", "https://github.com/acme/app/pull/1234"]


def test_run_open_url_default_browser(ran):
    commands.run({"open_url": "https://x.test/{n}"}, {"n": "1"})
    assert ran.calls[0][0] == ["open", "https://x.test/1"]


def test_run_placeholder_repo_is_not_opened(ran):
    out = commands.run({"open_url": "https://github.com/<org>/<repo>/pull/{n}"}, {"n": "1"})
    assert "Set your repo" in out and ran.calls == []


def test_run_open_app(ran, apps):
    assert commands.run({"open_app": "{app}"}, {"app": "slack"}) == "Opened Slack."
    assert ran.calls[0][0] == ["open", "-a", "Slack"]


def test_run_unknown_app_falls_through_to_claude(ran, apps):
    assert commands.run({"open_app": "{app}"}, {"app": "photoshop"}) is None
    assert ran.calls == []


def test_run_shell(ran):
    assert commands.run({"shell": "echo {q}"}, {"q": "hi"}) == "out"
    args, kw = ran.calls[0]
    assert args == "echo hi" and kw["shell"] is True
