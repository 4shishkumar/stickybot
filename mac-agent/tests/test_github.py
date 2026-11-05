"""GitHub watcher: state changes -> events, using recorded-style GraphQL data."""
import copy
import json
from pathlib import Path

import pytest

from stick_agent import github

BASE = json.loads((Path(__file__).parent / "fixtures" / "github_base.json").read_text())


@pytest.fixture
def gh(agent, monkeypatch):
    """A GitHub watcher whose `gh` query returns whatever the test puts in .data."""
    monkeypatch.setattr(github.shutil, "which", lambda name: "/usr/bin/gh")
    agent.cfg["github"] = {"on": True, "interval_s": 120, "primary_repo": "acme/web"}
    agent.cfg["event_rules"] = {"release": "nice"}  # so release events are visible here
    w = github.GitHub(agent)
    w.data = copy.deepcopy(BASE)

    async def query(primary):
        return w.data
    w._query = query
    agent.github = w
    return w


def fired(agent):
    return [(m["event"]["title"], m["event"]["sub"]) for m in agent.link.sent if "event" in m]


def pr(data, number):
    return next(n for n in data["mine"]["nodes"] if n and n["number"] == number)


async def test_first_look_only_seeds(gh, agent):
    await gh.poll()
    assert fired(agent) == []
    assert gh.state["prs"]["acme/web#5507"]["ci"] == "FAILURE"
    assert json.loads(github.STATE.read_text())["primary"] == "acme/web"


async def test_no_change_no_events(gh, agent):
    await gh.poll()
    await gh.poll()
    assert fired(agent) == []


@pytest.mark.parametrize("change, title", [
    (lambda d: pr(d, 5507).update(merged=True, state="MERGED"), "PR #5507 merged!"),
    (lambda d: pr(d, 5507).update(reviewDecision="APPROVED"), "PR #5507 approved"),
    (lambda d: pr(d, 5507).update(reviewDecision="CHANGES_REQUESTED"), "Changes requested"),
    (lambda d: pr(d, 5507)["commits"]["nodes"][0]["commit"]["statusCheckRollup"].update(state="SUCCESS"),
     "CI is green again"),
    (lambda d: pr(d, 88)["commits"]["nodes"][0]["commit"]["statusCheckRollup"].update(state="FAILURE"),
     "CI failed"),
    (lambda d: pr(d, 5507)["reviews"]["nodes"].append(
        {"id": "R1", "author": {"login": "devin-ai-integration"}, "state": "COMMENTED"}), "Devin reviewed #5507"),
    (lambda d: d["asks"]["nodes"].append({"number": 5510, "title": "Update deps",
                                          "repository": {"nameWithOwner": "acme/web"}}), "Review requested"),
    (lambda d: d["primary"]["releases"]["nodes"][0].update(tagName="v0.1.498"), "Release v0.1.498"),
])
async def test_transitions(gh, agent, change, title):
    await gh.poll()
    change(gh.data)
    await gh.poll()
    assert [t for t, _ in fired(agent)] == [title]


async def test_primary_repo_short_name_in_sub(gh, agent):
    await gh.poll()
    pr(gh.data, 5507).update(merged=True, state="MERGED")
    pr(gh.data, 88)["commits"]["nodes"][0]["commit"]["statusCheckRollup"].update(state="FAILURE")
    await gh.poll()
    subs = dict(fired(agent))
    assert subs["PR #5507 merged!"] == "web #5507 - Add stick agent"
    assert subs["CI failed"] == "acme/other #88 - Docs tweak"
    assert [t for t, _ in fired(agent)][0] == "PR #5507 merged!"   # primary repo first


async def test_new_pr_opened_after_last_look(gh, agent, monkeypatch):
    await gh.poll()
    new = copy.deepcopy(pr(gh.data, 88))
    new.update(number=89, title="Brand new", createdAt="2099-01-01T00:00:00Z")
    gh.data["mine"]["nodes"].append(new)
    await gh.poll()
    assert fired(agent) == [("PR #89 opened", "acme/other #89 - Brand new")]


async def test_old_pr_that_was_touched_is_not_new(gh, agent):
    await gh.poll()
    old = copy.deepcopy(pr(gh.data, 88))
    old.update(number=12, title="Ancient", createdAt="2020-01-01T00:00:00Z")
    gh.data["mine"]["nodes"].append(old)
    await gh.poll()
    assert fired(agent) == []


async def test_pr_dropping_out_of_the_window_is_remembered(gh, agent):
    await gh.poll()
    gone = pr(gh.data, 88)
    gh.data["mine"]["nodes"].remove(gone)
    await gh.poll()
    gh.data["mine"]["nodes"].append(gone)  # comes back unchanged
    await gh.poll()
    assert fired(agent) == []


async def test_changing_primary_repo_reseeds(gh, agent):
    await gh.poll()
    agent.cfg["github"]["primary_repo"] = "acme/other"
    pr(gh.data, 5507).update(merged=True, state="MERGED")
    await gh.poll()
    assert fired(agent) == []


async def test_gh_missing(gh, agent, monkeypatch):
    monkeypatch.setattr(github.shutil, "which", lambda name: None)
    await gh.poll()
    assert "gh CLI not found" in gh.last_error


def test_helpers():
    assert github._epoch("2026-10-20T10:12:00Z") == 1792491120
    assert github._epoch(None) == 0
    assert github._short("acme/web", "acme/web") == "web"
    assert github._short("acme/x", "acme/web") == "acme/x"
    assert github.settings({"github": {"interval_s": 60}})["primary_repo"] == ""   # no repo until the user sets one
