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
    "agent.stub": ('#!/bin/sh\necho "stick-agent $*" >>"$FAKE_LOG"\n'
                   '[ "$1" = install-page ] && exit "${FAKE_PAGE_RC:-0}"\nexit "${FAKE_AGENT_RC:-0}"\n'),
    # real git, except what would reach the network
    "git": ('#!/bin/sh\ncase "$3" in fetch|pull) echo "git $*" >>"$FAKE_LOG"; exit 0 ;; esac\n'
            'exec /usr/bin/git "$@"\n'),
    "curl": '#!/bin/sh\necho "curl $*" >>"$FAKE_LOG"\nexit "${FAKE_CURL_RC:-0}"\n',
    "open": '#!/bin/sh\necho "open $*" >>"$FAKE_LOG"\nexit "${FAKE_OPEN_RC:-0}"\n',
    # the Command Line Tools: there unless FAKE_NO_CLT
    "xcode-select": ('#!/bin/sh\necho "xcode-select $*" >>"$FAKE_LOG"\n'
                     '[ -n "$FAKE_NO_CLT" ] && exit 2\necho /Library/Developer/CommandLineTools\n'),
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

    alone = tmp_path / "dl" / "install.sh"  # as if downloaded (curl | sh): not next to a clone
    alone.parent.mkdir()
    alone.write_text(SCRIPT.read_text())

    def run(*args, src_arg=True, **extra):
        script = SCRIPT if src_arg else alone
        r = subprocess.run(["/bin/sh", str(script), *(["--src", str(src)] if src_arg else []), *args],
                           env={**env, **extra},
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
    assert "stick-agent install-page" in r.calls  # the StickyBot window, else the browser
    assert any(c.startswith("curl ") and "18998/api/status" in c for c in r.calls)
    assert "Continue on the setup page → http://localhost:18998/#install" in r.stdout
    assert "stickybot install-page" in r.stdout
    assert (fake.home / "installer-opens-page").exists()  # the agent won't open a second tab


def test_rerun_restarts_and_opens_again(fake):
    fake()
    r = fake()
    assert r.returncode == 0
    assert r.calls[0] == "stick-agent setup --yes --only core"  # (re)starts the agent: new code runs
    assert "stick-agent install-page" in r.calls


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
    r = fake(FAKE_PAGE_RC="1")
    assert r.calls[-1] == "stick-agent setup"
    assert "couldn't open the setup page" in r.stdout
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


def test_an_install_clone_under_the_old_repo_name_follows_the_rename(fake):
    src = fake.home / "src"
    subprocess.run(["git", "init", "-q", str(src)], check=True)
    subprocess.run(["git", "-C", str(src), "remote", "add", "origin",
                    "https://github.com/ashish-kumar-noon/stick-key.git"], check=True)
    (src / "mac-agent").mkdir()
    (src / "mac-agent/pyproject.toml").write_text("")
    r = fake(src_arg=False)
    assert r.returncode == 0, r.stdout + r.stderr
    url = subprocess.run(["git", "-C", str(src), "remote", "get-url", "origin"], capture_output=True, text=True)
    assert url.stdout.strip() == "https://github.com/4shishkumar/stickybot.git"
    assert "the repo was renamed" in r.stdout
    assert any(c.startswith("git -C") and "fetch" in c for c in r.calls)  # then fetched as before
    r = fake(src_arg=False)  # once
    assert "the repo was renamed" not in r.stdout


def test_the_one_line_install_uses_the_new_repo_name():
    text = SCRIPT.read_text()
    assert 'REPO="4shishkumar/stickybot"' in text
    assert "raw.githubusercontent.com/4shishkumar/stickybot/main/install.sh" in text


def test_no_command_line_tools_asks_for_them_and_stops(fake, tmp_path):
    (tmp_path / "fakebin/git").unlink()  # git is macOS's /usr/bin/git stub again
    r = fake(src_arg=False, FAKE_NO_CLT="1")
    assert r.returncode == 1 and "Command Line Tools" in r.stdout + r.stderr
    assert "xcode-select --install" in r.calls and not any(c.startswith("git") for c in r.calls)


def test_with_command_line_tools_git_is_fine(fake, tmp_path):
    (tmp_path / "fakebin/git").unlink()
    r = fake("--dry-run", src_arg=False)
    assert r.returncode == 0 and "git ✓" in r.stdout and "xcode-select --install" not in r.calls
    r = fake("--dry-run", src_arg=False, FAKE_NO_CLT="1")  # a dry run only says it
    assert r.returncode == 0 and "would ask macOS to install the Command Line Tools" in r.stdout
    assert "xcode-select --install" not in r.calls
