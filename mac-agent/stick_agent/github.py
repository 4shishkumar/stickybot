"""GitHub watcher: turns changes on your pull requests into stick events.

Uses the `gh` CLI (your own login, read-only) every `interval_s` seconds:
  - your PRs (updated in the last 14 days): opened, merged, approved / changes
    requested, CI failed / fixed, Devin reviews
  - PRs waiting for your review
  - releases of the primary repo
The first poll only records the current state (no events), so starting the
agent doesn't replay history. State lives in ~/.stick-agent/github_state.json.
"""
import asyncio
import json
import logging
import shutil
import time

from . import config

log = logging.getLogger("github")
STATE = config.HOME / "github_state.json"
MAX_BACKOFF_S = 30 * 60
DEFAULT = {"on": True, "interval_s": 120, "primary_repo": ""}

QUERY = """
query($mine: String!, $asks: String!, $owner: String!, $name: String!, $withRepo: Boolean!) {
  mine: search(query: $mine, type: ISSUE, first: 40) {
    nodes { ... on PullRequest { number title state merged createdAt repository { nameWithOwner } reviewDecision
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      reviews(last: 20) { nodes { id author { login } state } } } }
  }
  asks: search(query: $asks, type: ISSUE, first: 40) {
    nodes { ... on PullRequest { number title repository { nameWithOwner } } }
  }
  primary: repository(owner: $owner, name: $name) @include(if: $withRepo) {
    releases(first: 1, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { tagName name } }
  }
}
"""


def settings(cfg):
    g = dict(DEFAULT)
    g.update(cfg.get("github") or {})
    return g


def _epoch(iso):
    """'2026-10-20T10:12:00Z' -> seconds since 1970 (0 if missing)."""
    import calendar
    try:
        return calendar.timegm(time.strptime(iso, "%Y-%m-%dT%H:%M:%SZ"))
    except (TypeError, ValueError):
        return 0


def _short(repo, primary):
    """'acme/web' -> 'web' (primary repo by its name; others in full)."""
    return repo.split("/")[-1] if repo == primary else repo


class GitHub:
    def __init__(self, agent):
        self.agent = agent
        self.state = None
        self.backoff = 0
        self.last_ok = 0.0
        self.last_error = ""
        try:
            self.state = json.loads(STATE.read_text())
        except (OSError, ValueError):
            self.state = None

    def status(self):
        return {"ok": self.last_ok, "error": self.last_error, "tracked": len((self.state or {}).get("prs", {}))}

    async def _query(self, primary):
        since = time.strftime("%Y-%m-%d", time.localtime(time.time() - 14 * 86400))
        owner, _, name = primary.partition("/")
        args = ["gh", "api", "graphql", "-f", f"query={QUERY}",
                "-F", f"mine=is:pr author:@me updated:>={since}",
                # review requests: the primary repo only (there are too many across all repos)
                "-F", f"asks=is:pr is:open review-requested:@me{' repo:' + primary if owner and name else ''}",
                "-F", f"owner={owner or 'x'}", "-F", f"name={name or 'x'}",
                "-F", f"withRepo={'true' if owner and name else 'false'}"]
        p = await asyncio.create_subprocess_exec(*args, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        try:
            out, err = await asyncio.wait_for(p.communicate(), 40)
        except TimeoutError:
            p.kill()
            raise RuntimeError("gh timed out") from None
        if p.returncode != 0:
            raise RuntimeError(err.decode(errors="replace").strip()[-200:] or f"gh exit {p.returncode}")
        d = json.loads(out)
        if d.get("errors"):
            raise RuntimeError(str(d["errors"])[:200])
        return d["data"]

    @staticmethod
    def _snapshot(data):
        prs = {}
        for n in (data.get("mine") or {}).get("nodes") or []:
            if not n:
                continue
            key = f"{n['repository']['nameWithOwner']}#{n['number']}"
            commit = ((n.get("commits") or {}).get("nodes") or [{}])[0].get("commit") or {}
            devin = [r["id"] for r in (n.get("reviews") or {}).get("nodes") or []
                     if r.get("author") and "devin" in r["author"]["login"].lower()]
            prs[key] = {"repo": n["repository"]["nameWithOwner"], "number": n["number"], "title": n["title"],
                        "state": n["state"], "merged": n["merged"], "decision": n.get("reviewDecision"),
                        "created": _epoch(n.get("createdAt")),
                        "ci": (commit.get("statusCheckRollup") or {}).get("state"), "devin": devin}
        asks = {f"{n['repository']['nameWithOwner']}#{n['number']}": {
                    "repo": n["repository"]["nameWithOwner"], "number": n["number"], "title": n["title"]}
                for n in (data.get("asks") or {}).get("nodes") or [] if n}
        rel = ((data.get("primary") or {}).get("releases") or {}).get("nodes") or []
        return {"prs": prs, "asks": asks, "release": rel[0]["tagName"] if rel else None,
                "release_name": rel[0].get("name") if rel else None}

    async def _diff(self, old, new, primary):
        ev = self.agent.notifier.notify
        # primary repo first
        order = sorted(new["prs"].items(), key=lambda kv: (kv[1]["repo"] != primary, kv[0]))
        for key, pr in order:
            was = old["prs"].get(key)
            where = f"{_short(pr['repo'], primary)} #{pr['number']}"
            title = pr["title"][:60]
            if was is None:
                # only PRs created since the last look (an old PR that got updated isn't "new")
                if pr["state"] == "OPEN" and pr["created"] >= old.get("seen_at", 0) - 300:
                    await ev("pr_created", f"PR #{pr['number']} opened", f"{where} - {title}")
                continue
            if pr["merged"] and not was["merged"]:
                self.agent.pet.merged()
                await ev("pr_merged", f"PR #{pr['number']} merged!", f"{where} - {title}")
            if pr["decision"] != was["decision"]:
                if pr["decision"] == "APPROVED":
                    await ev("pr_approved", f"PR #{pr['number']} approved", f"{where} - {title}")
                elif pr["decision"] == "CHANGES_REQUESTED":
                    await ev("changes_requested", "Changes requested", f"{where} - {title}")
            if pr["ci"] != was["ci"]:
                if pr["ci"] == "SUCCESS":  # a green build (the pet's sunglasses come at 10)
                    self.agent.pet.green()
                if pr["ci"] in ("FAILURE", "ERROR"):
                    await ev("ci_failed", "CI failed", f"{where} - {title}")
                elif pr["ci"] == "SUCCESS" and was["ci"] in ("FAILURE", "ERROR"):
                    await ev("ci_fixed", "CI is green again", f"{where} - {title}")
            fresh = [r for r in pr["devin"] if r not in set(was.get("devin") or [])]
            if fresh:
                await ev("devin_review", f"Devin reviewed #{pr['number']}", f"{where} - {title}")
        for key, a in sorted(new["asks"].items(), key=lambda kv: (kv[1]["repo"] != primary, kv[0])):
            if key not in old["asks"]:
                await ev("review_requested", "Review requested",
                         f"{_short(a['repo'], primary)} #{a['number']} - {a['title'][:60]}")
        if new["release"] and old.get("release") and new["release"] != old["release"]:
            self.agent.pet.release()
            await ev("release", f"Release {new['release']}"[:40], _short(primary, primary))

    async def poll(self):
        cfg = settings(self.agent.cfg)
        if not shutil.which("gh"):
            self.last_error = "gh CLI not found (brew install gh, then gh auth login)"
            return
        primary = cfg.get("primary_repo") or ""
        data = await self._query(primary)
        new = self._snapshot(data)
        old = self.state
        if old is None or old.get("primary") != primary:
            log.info("github: tracking %d PRs, %d review requests (first look, no events)",
                     len(new["prs"]), len(new["asks"]))
        else:
            # PRs that dropped out of the 14-day window: keep their last state so they don't
            # look "new" if they come back
            await self._diff(old, new, primary)
            for k, v in old["prs"].items():
                new["prs"].setdefault(k, v)
            if len(new["prs"]) > 500:  # keep the newest-numbered ones
                new["prs"] = dict(sorted(new["prs"].items(), key=lambda kv: -kv[1]["number"])[:500])
        new["primary"] = primary
        new["seen_at"] = time.time()
        self.state = new
        STATE.write_text(json.dumps(new))
        if old is not None:
            await self.agent.push_world(only_changed=True)  # CI, reviews and merges for the faces
        self.last_ok, self.last_error = time.time(), ""

    async def run(self):
        while True:
            cfg = settings(self.agent.cfg)
            wait = max(30, int(cfg.get("interval_s") or 120))
            if cfg.get("on"):
                try:
                    await self.poll()
                    self.backoff = 0
                except Exception as e:  # noqa: BLE001 — offline, gh logged out, rate-limited...
                    self.last_error = str(e)
                    self.backoff = min(MAX_BACKOFF_S, max(wait, self.backoff * 2 or wait))
                    log.warning("github: %s (next try in %d s)", e, self.backoff)
                    wait = self.backoff
            await asyncio.sleep(wait)
