"""Cutting a release: versions (SemVer order), the CHANGELOG helpers the workflow uses,
and tools/release.sh end to end in a throwaway git repo (it commits and tags there,
never pushes)."""
import importlib.util
import os
import shutil
import subprocess
import types
from pathlib import Path

import pytest

from stick_agent import __version__, releases

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release_tool", ROOT / "tools" / "release_tool.py")
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)

LOG = """# Changelog

Intro.

## [Unreleased]

### Added
- a new thing

## [1.7.2] - 2026-11-05

### Added
- the first release

[Unreleased]: https://github.com/4shishkumar/stickybot/compare/v1.7.2...HEAD
[1.7.2]: https://github.com/4shishkumar/stickybot/releases/tag/v1.7.2
"""


# --- versions --------------------------------------------------------------------------

def test_parse_version():
    assert releases.parse_version("v1.8.0-beta.2") == (1, 8, 0, ("beta", 2))
    assert releases.parse_version("1.7.2") == (1, 7, 2, ())
    assert releases.parse_version("1.7.2+build.5") == (1, 7, 2, ())
    for bad in ("1.7", "latest", "", None, "v1.7.x"):
        assert releases.parse_version(bad) is None


def test_semver_order_including_prereleases():
    # https://semver.org/#spec-item-11
    order = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11",
             "1.0.0-rc.1", "1.0.0", "1.0.1", "1.2.0", "1.10.0", "2.0.0-beta.1", "2.0.0"]
    assert sorted(reversed(order), key=releases.version_key) == order
    assert releases.newer("1.8.0", "1.8.0-beta.3") and not releases.newer("1.8.0-beta.3", "1.8.0")
    assert releases.newer("1.7.3", None) and not releases.newer("junk", "1.0.0")
    assert releases.is_prerelease("v2.0.0-rc.1") and not releases.is_prerelease("2.0.0")


def test_the_repo_versions_agree():
    """The agent's __version__, pyproject and the changelog: what the release workflow checks."""
    assert tool.current() == __version__
    assert f'version = "{__version__}"' in (ROOT / "mac-agent" / "pyproject.toml").read_text()
    assert tool.check_tag(f"v{__version__}") == __version__
    assert tool.notes(__version__)


# --- changelog -------------------------------------------------------------------------

def test_notes_are_one_section_without_the_links():
    assert tool.notes("1.7.2", LOG) == "### Added\n- the first release"
    assert tool.notes("Unreleased", LOG) == "### Added\n- a new thing"
    with pytest.raises(tool.Fail):
        tool.notes("9.9.9", LOG)


def test_changelog_turns_unreleased_into_the_release():
    new = tool.changelog("1.8.0", "2026-12-01", LOG)
    assert "## [Unreleased]\n\n## [1.8.0] - 2026-12-01\n\n### Added\n- a new thing" in new
    assert tool.notes("1.8.0", new) == "### Added\n- a new thing"
    assert tool.notes("Unreleased", new) == ""
    assert "[Unreleased]: https://github.com/4shishkumar/stickybot/compare/v1.8.0...HEAD" in new
    assert "[1.8.0]: https://github.com/4shishkumar/stickybot/releases/tag/v1.8.0" in new
    assert "[1.7.2]: " in new


def test_changelog_redates_an_existing_entry_and_refuses_an_empty_one():
    again = tool.changelog("1.7.2", "2026-11-09", LOG)
    assert "## [1.7.2] - 2026-11-09" in again and "- a new thing" in again
    empty = LOG.replace("### Added\n- a new thing\n\n", "")
    with pytest.raises(tool.Fail, match="nothing under"):
        tool.changelog("1.8.0", "2026-12-01", empty)


def test_check_tag_wants_the_mac_agent_version():
    with pytest.raises(tool.Fail, match="mac-agent says"):
        tool.check_tag("v99.0.0")
    with pytest.raises(tool.Fail):
        tool.check_tag("latest")


# --- tools/release.sh -------------------------------------------------------------------

@pytest.fixture
def repo(tmp_path):
    """A minimal copy of the repo (just what release.sh touches), committed on main."""
    r = tmp_path / "repo"
    for rel in ("tools/release.sh", "tools/release_tool.py", "mac-agent/stick_agent/__init__.py",
                "mac-agent/stick_agent/releases.py", "mac-agent/pyproject.toml", "m5-stick-s3/src/config.h"):
        (r / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / rel, r / rel)
    (r / "CHANGELOG.md").write_text(LOG)
    env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com", "GIT_CONFIG_GLOBAL": os.devnull}
    git = lambda *a: subprocess.run(["git", "-C", str(r), *a], check=True, capture_output=True, text=True, env=env)
    git("init", "-q", "-b", "main")
    git("add", "-A")
    git("commit", "-q", "-m", "init")
    return types.SimpleNamespace(path=r, git=git, env=env)


def release_sh(repo, *args):
    return subprocess.run(["sh", str(repo.path / "tools/release.sh"), *args], cwd=repo.path, capture_output=True,
                          text=True, env=repo.env)


def test_release_sh_bumps_commits_and_tags_without_pushing(repo):
    r = release_sh(repo, "1.8.0")
    assert r.returncode == 0, r.stderr
    assert "git push origin main v1.8.0" in r.stdout
    assert '__version__ = "1.8.0"' in (repo.path / "mac-agent/stick_agent/__init__.py").read_text()
    assert 'version = "1.8.0"' in (repo.path / "mac-agent/pyproject.toml").read_text()
    assert "## [1.8.0] - " in (repo.path / "CHANGELOG.md").read_text()
    assert repo.git("log", "-1", "--format=%s").stdout.strip() == "Release v1.8.0"
    assert repo.git("tag", "--list").stdout.split() == ["v1.8.0"]
    assert repo.git("remote").stdout.strip() == ""  # nowhere to push to, and nothing tried
    again = release_sh(repo, "1.8.0")
    assert again.returncode != 0 and "already exists" in again.stderr


def test_release_sh_dry_run_and_refusals(repo):
    r = release_sh(repo, "--dry-run", "v1.8.0-beta.1")
    assert r.returncode == 0 and "beta (prerelease)" in r.stdout and "- a new thing" in r.stdout
    assert repo.git("status", "--porcelain", "--untracked-files=no").stdout == "" and repo.git("tag").stdout == ""
    assert release_sh(repo, "1.8").returncode != 0
    (repo.path / "CHANGELOG.md").write_text(LOG + "\nedited\n")
    dirty = release_sh(repo, "1.8.0")
    assert dirty.returncode != 0 and "uncommitted" in dirty.stderr
