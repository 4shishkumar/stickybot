"""The faces' pet progress (pet.py) and the "world" message for the stick."""
import json
import time

from stick_agent import config, pet


def test_levels_match_the_stick():
    assert pet.level_for(0) == (1, 0, 20)
    assert pet.level_for(19) == (1, 19, 20)
    assert pet.level_for(20) == (2, 0, 30)
    assert pet.level_for(50)[0] == 3
    assert pet.level_for(90)[0] == 4
    assert pet.level_for(-3)[0] == 1


def test_accessories():
    assert pet.accessories(4, 9, 99) == 0
    assert pet.accessories(5, 0, 0) == pet.HAT
    assert pet.accessories(0, 10, 0) == pet.SHADES
    assert pet.accessories(0, 0, 100) == pet.CROWN


def test_new_pet_starts_with_the_merged_prs_and_persists(home):
    p = pet.Pet(initial_merged=3)
    assert p.s["merged"] == 3 and p.s["xp"] == 30
    seed = p.s["seed"]
    p.merged()
    again = pet.Pet(initial_merged=99)  # the file wins
    assert again.s["merged"] == 4
    assert again.s["seed"] == seed
    assert json.loads((config.HOME / "pet.json").read_text())["xp"] == 40


def test_xp_streak_and_towers(home):
    p = pet.Pet()
    day = pet.today()
    p.merged(day=day - 2)
    p.claude_done(day=day - 1)
    p.merged(day=day)
    assert p.s["streak"]["days"] == 3
    assert p.streak(day) == 3
    assert p.streak(day + 1) == 3   # yesterday still counts
    assert p.streak(day + 2) == 0   # a day missed
    p.green()
    p.release()
    assert p.s["xp"] == 10 + 2 + 10 + 3 + 25
    assert p.s["towers"] == [2]     # after the second building
    assert p.summary()["mergedToday"] == 1
    for _ in range(12):
        p.release()
    assert len(p.s["towers"]) == pet.MAX_TOWERS


def test_running_sessions(home):
    now = time.time()
    (config.HOME / "claude_sessions.json").write_text(json.dumps({
        "s1": {"t": now - 300, "cwd": "/Users/me/src/web"},
        "s2": {"t": now - 30, "cwd": "/Users/me/src/a-very-long-project-name/"},
        "old": {"t": now - 3 * 3600, "cwd": "/x/stale"},
    }))
    s = pet.running_sessions(now)
    assert [x["p"] for x in s] == ["web", "a-very-long-"]
    assert s[0]["s"] == 300
    assert s[0]["id"] == pet.running_sessions(now)[0]["id"] != s[1]["id"]


def test_ci_state():
    assert pet.ci_state(None) == "none"
    prs = {"a": {"state": "OPEN", "ci": "SUCCESS"}, "b": {"state": "MERGED", "ci": "FAILURE"}}
    assert pet.ci_state({"prs": prs}) == "green"
    prs["c"] = {"state": "OPEN", "ci": "ERROR"}
    assert pet.ci_state({"prs": prs}) == "red"


def test_world_in_the_hello_and_small(agent):
    agent.prompts["p1"] = {"tool": "Bash"}
    extra = agent._hello_extra()
    w = extra["world"]
    assert w["asks"] == 1
    assert w["ci"] == "none"
    assert set(w["pet"]) == {"xp", "streak", "acc", "seed", "greens", "towers"}
    assert len(json.dumps(w)) < 400


async def test_claude_finishing_feeds_the_pet(agent):
    xp = agent.pet.s["xp"]
    await agent.claude_finished(5, "/Users/me/web", "fix it")  # short: no event, still XP
    assert agent.pet.s["xp"] == xp + pet.XP_CLAUDE
    assert any("world" in m for m in agent.link.sent)


async def test_events_carry_their_kind(agent):
    ev = await agent.notifier.notify("pr_merged", "PR #1 merged!", "web")
    assert ev["kind"] == "pr_merged"


async def test_push_world_only_when_changed(agent):
    assert await agent.push_world(only_changed=True)
    assert not await agent.push_world(only_changed=True)
    agent.pet.merged()
    assert await agent.push_world(only_changed=True)
