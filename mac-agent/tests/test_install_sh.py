"""install.sh's hand-over: the setup page in the browser, or `stick-agent setup` in the
terminal. Runs the real script with fake uv / curl / open / stick-agent on PATH and a
throwaway HOME — no network, no login item, nothing real is installed or opened."""
import os
import platform
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "install.sh"

pytestmark = pytest.mark.skipif(platform.system() != "Darwin" or platform.machine() != "arm64",
                                reason="install.sh only runs on Apple Silicon Macs")

STUBS = {
    "uv": """#!/bin/sh
echo "uv $*" >>"$FAKE_LOG"
case "$1" in
  --version) echo "uv 0.0-fake" ;;
  venv) mkdir -p "$4/bin"; cp "$FAKE_BIN/python.stub" "$4/bin/python"; cp "$FAKE_BIN/agent.stub" "$4/bin/stick-agent"
        chmod +x "$4/bin/python" "$4/bin/stick-agent" ;;
esac
""",
    "python.stub": "#!/bin/sh\necho 9.9.9\n",
    "agent.stub": '#!/bin/sh\necho "stick-agent $*" >>"$FAKE_LOG"\nexit "${FAKE_AGENT_RC:-0}"\n',
    "curl": '#!/bin/sh\necho "curl $*" >>"$FAKE_LOG"\nexit "${FAKE_CURL_RC:-0}"\n',
    "open": '#!/bin/sh\necho "open $*" >>"$FAKE_LOG"\nexit "${FAKE_OPEN_RC:-0}"\n',
}


@pytest.fixture
def fake(tmp_path):
    bin_ = tmp_path / "fakebin"
    bin_.mkdir()
    for name, text in STUBS.items():
        (bin_ / name).write_text(text)
        (bin_ / name).chmod(0o755)
    src = tmp_path / "src"
    (src / "mac-agent").mkdir(parents=True)
    (src / "mac-agent/pyproject.toml").write_text("")
    log = tmp_path / "calls.log"
    log.write_text("")
    env = {"PATH": f"{bin_}:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(tmp_path), "FAKE_LOG": str(log),
           "FAKE_BIN": str(bin_), "STICK_AGENT_HOME": str(tmp_path / ".stick-agent"), "STICK_AGENT_PORT": "18998",
           "STICK_AGENT_GUI": "1", "STICK_AGENT_WAIT_S": "2", "TERM": "dumb"}

    def run(*args, **extra):
        r = subprocess.run(["/bin/sh", str(SCRIPT), "--src", str(src), *args], env={**env, **extra},
                           capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL,
                           start_new_session=True)  # no /dev/tty: like a script / an AI agent
        r.calls = [line for line in log.read_text().splitlines() if not line.startswith("uv ")]
        log.write_text("")
        return r
    run.home = tmp_path / ".stick-agent"
    return run


def test_default_opens_the_setup_page(fake):
    r = fake()
    assert r.returncode == 0, r.stdout + r.stderr
    assert "stick-agent setup --yes --only core" in r.calls
    assert "open http://localhost:18998/#install" in r.calls
    assert any(c.startswith("curl ") and "18998/api/status" in c for c in r.calls)
    assert "Continue in your browser → http://localhost:18998/#install" in r.stdout
    assert "stick-agent install-page" in r.stdout
    assert (fake.home / "installer-opens-page").exists()  # the agent won't open a second tab


def test_rerun_restarts_and_opens_again(fake):
    fake()
    r = fake()
    assert r.returncode == 0
    assert r.calls[0] == "stick-agent setup --yes --only core"  # (re)starts the agent: new code runs
    assert "open http://localhost:18998/#install" in r.calls


@pytest.mark.parametrize("args, setup", [
    (["--no-browser"], "stick-agent setup"),
    (["--yes"], "stick-agent setup --yes"),
    (["--yes", "--without", "model,claude"], "stick-agent setup --yes --without model,claude"),
    (["--with=github"], "stick-agent setup --with=github"),
    (["--dry-run"], "stick-agent setup --dry-run"),
])
def test_options_keep_the_terminal_setup(fake, args, setup):
    r = fake(*args)
    assert r.returncode == 0, r.stdout + r.stderr
    assert not any(c.startswith("open ") for c in r.calls)
    if "--dry-run" in args:
        assert r.calls == [] or r.calls == [setup]  # (dry run: nothing is created, so nothing to run)
    else:
        assert r.calls == [setup]


def test_ssh_session_uses_the_terminal(fake):
    r = fake(STICK_AGENT_GUI="", SSH_CONNECTION="10.0.0.2 5555 10.0.0.1 22")
    assert r.calls == ["stick-agent setup"]
    assert "no browser here" in r.stdout


def test_no_gui_uses_the_terminal(fake):
    r = fake(STICK_AGENT_GUI="0")
    assert r.calls == ["stick-agent setup"]


def test_open_failing_falls_back_to_the_terminal(fake):
    r = fake(FAKE_OPEN_RC="1")
    assert r.calls[-1] == "stick-agent setup"
    assert "couldn't open a browser" in r.stdout
    assert not (fake.home / "installer-opens-page").exists()


def test_agent_not_answering_falls_back_to_the_terminal(fake):
    r = fake(FAKE_CURL_RC="7")
    assert r.calls[0] == "stick-agent setup --yes --only core"
    assert r.calls[-1] == "stick-agent setup"
    assert not any(c.startswith("open ") for c in r.calls)
    assert "didn't answer" in r.stdout


def test_help_mentions_the_setup_page(fake):
    r = subprocess.run(["/bin/sh", str(SCRIPT), "--help"], capture_output=True, text=True, timeout=30,
                       env={**os.environ, "STICK_AGENT_HOME": str(fake.home)})
    assert r.returncode == 0 and "--no-browser" in r.stdout and "#install" in r.stdout


def test_posix_syntax():
    assert subprocess.run(["/bin/sh", "-n", str(SCRIPT)]).returncode == 0
