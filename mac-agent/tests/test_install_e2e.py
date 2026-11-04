"""End to end: run the real install.sh into a throwaway HOME (downloads uv,
Python 3.14 and the libraries), check `stick-agent setup --status`, uninstall.

Slow and needs the network, so it's opt-in: `make install-e2e` (or
pytest -m slow tests/test_install_e2e.py). The login item, the Claude Code
settings and the port are all namespaced, so it can't touch the real setup.
"""
import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.slow
def test_install_status_uninstall(tmp_path):
    env = {**os.environ, "HOME": str(tmp_path), "STICK_AGENT_HOME": str(tmp_path / ".stick-agent"),
           "STICK_AGENT_LABEL": "com.stickkey.agent.e2etest", "STICK_AGENT_PORT": "18997",
           "STICK_AGENT_NO_LOGIN_ITEM": "1", "STICK_AGENT_CLAUDE_SETTINGS": str(tmp_path / ".claude/settings.json")}
    install = [str(ROOT / "install.sh"), "--yes", "--without", "model,claude,github,firmware", "--no-login-item"]
    r = subprocess.run(install, env=env, capture_output=True, text=True, timeout=900, stdin=subprocess.DEVNULL)
    assert r.returncode == 0, r.stdout[-3000:] + r.stderr[-2000:]
    cli = tmp_path / ".local/bin/stick-agent"
    assert cli.is_symlink()

    r = subprocess.run([str(cli), "setup", "--status", "--json"], env=env, capture_output=True, text=True,
                       timeout=120)
    status = {c["id"]: c["status"] for c in json.loads(r.stdout)}
    assert status["core"] == "installed" and status["claude"] == "missing"

    r = subprocess.run(install, env=env, capture_output=True, text=True, timeout=600, stdin=subprocess.DEVNULL)
    assert r.returncode == 0, "re-running should be fine"

    r = subprocess.run([str(cli), "uninstall", "--purge", "--yes"], env=env, capture_output=True, text=True,
                       timeout=300)
    assert r.returncode == 0, r.stdout
    assert not cli.exists() and not (tmp_path / ".stick-agent").exists()
