"""`stick-agent setup`: install / check / remove the companion app's parts.

Each part is a Component with a status(), install(ui) and uninstall(ui). The
command-line (interactive menu or flags), the dashboard's Components card and
install.sh all go through this module, so they always agree.

  stick-agent setup                    interactive: pick components, confirm, install
  stick-agent setup --yes              defaults, no questions
  stick-agent setup --with github --without model
  stick-agent setup --status [--json]  what's installed
  stick-agent setup --only core        just these components (not the defaults)
  stick-agent setup --dry-run          say what would happen, change nothing
  stick-agent setup --uninstall all    (or a list) remove them again
  stick-agent uninstall [--purge]      = setup --uninstall all (+ delete ~/.stick-agent)

Test-friendly: STICK_AGENT_HOME, STICK_AGENT_PORT, STICK_AGENT_LABEL,
STICK_AGENT_BIN, STICK_AGENT_CLAUDE_SETTINGS move everything it touches, and
--no-login-item (or STICK_AGENT_NO_LOGIN_ITEM=1) skips launchd entirely.
"""
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

from . import config

PKG = Path(__file__).resolve().parent
HOOK = PKG / "hook.py"
CLAUDE_HOOKS = PKG / "claude_hooks.py"
CLAUDE_INSTALL = "curl -fsSL https://claude.ai/install.sh | bash"  # Anthropic's official installer


def bin_dir():
    return Path(os.environ.get("STICK_AGENT_BIN", Path.home() / ".local/bin"))


def claude_settings_path():
    return Path(os.environ.get("STICK_AGENT_CLAUDE_SETTINGS", Path.home() / ".claude/settings.json"))


def no_login_item():
    return os.environ.get("STICK_AGENT_NO_LOGIN_ITEM") == "1"


def venv_python():
    return Path(sys.executable)


def run(cmd, timeout=30, **kw):
    """subprocess.run with text output; never raises for a missing program."""
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, **kw)
    except (OSError, subprocess.TimeoutExpired) as e:
        return subprocess.CompletedProcess(cmd, 127, "", str(e))


# --- output / questions ------------------------------------------------------------------

class UI:
    """How components talk to the person installing. The CLI prints and asks; the
    dashboard (interactive=False) collects lines for a progress view."""

    def __init__(self, interactive=False, assume_yes=False, emit=None):
        self.interactive = interactive
        self.assume_yes = assume_yes
        self._emit = emit

    def say(self, line):
        if self._emit:
            self._emit({"line": line})
        else:
            print(f"  {line}", flush=True)

    def progress(self, pct, stage="", **extra):
        if self._emit:
            self._emit({"progress": int(pct), "stage": stage, **extra})

    def ask(self, question, default=""):
        if not self.interactive or self.assume_yes:
            return default
        try:
            answer = input(f"  {question}" + (f" [{default}]" if default else "") + ": ").strip()
        except EOFError:
            return default
        return answer or default

    def confirm(self, question, default=True):
        if not self.interactive or self.assume_yes:
            return default
        hint = "Y/n" if default else "y/N"
        try:
            answer = input(f"  {question} ({hint}) ").strip().lower()
        except EOFError:
            return default
        return default if not answer else answer.startswith("y")


class NotAvailable(Exception):
    """This component can't be installed (yet / from here)."""


class Component:
    id = ""
    title = ""
    description = ""
    default = False
    needs = {}            # {"admin", "network", "usb", "download": "2.5 GB", "terminal": "why"}
    from_dashboard = True  # can the dashboard's Components card install / remove it?
    removable = True

    def status(self):
        """-> (state, detail): state is installed | partial | missing | n/a."""
        return "missing", ""

    def plan(self):
        """One line for --dry-run / the summary."""
        return self.description

    def install(self, ui):
        raise NotImplementedError

    def uninstall(self, ui):
        raise NotImplementedError

    def info(self):
        state, detail = self.status()
        return {"id": self.id, "title": self.title, "description": self.description, "default": self.default,
                "needs": self.needs, "from_dashboard": self.from_dashboard, "removable": self.removable,
                "status": state, "detail": detail}


# --- core: the agent itself -------------------------------------------------------------

class Core(Component):
    id = "core"
    title = "Companion app"
    description = "the agent, the `stick-agent` command, its login item and the dashboard"
    default = True
    from_dashboard = False  # it's what runs the dashboard

    def _link(self):
        return bin_dir() / "stick-agent"

    def _target(self):
        return venv_python().parent / "stick-agent"

    def _login_loaded(self):
        from . import __main__ as cli
        return cli.launchctl("print", f"{cli.domain()}/{cli.LABEL}").returncode == 0

    def _dashboard_up(self):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{config.PORT}/api/status", timeout=1.5):
                return True
        except OSError:
            return False

    def status(self):
        bits = []
        link = self._link()
        cmd_ok = link.exists() or shutil.which("stick-agent") is not None
        bits.append("command ✓" if cmd_ok else "no `stick-agent` command")
        if no_login_item():
            login_ok = True
            bits.append("login item skipped")
        else:
            login_ok = self._login_loaded()
            bits.append("login item ✓" if login_ok else "login item missing")
        up = self._dashboard_up()
        bits.append(f"dashboard on :{config.PORT}" if up else "not running")
        state = "installed" if cmd_ok and login_ok and (up or no_login_item()) else "partial"
        return state, ", ".join(bits)

    def plan(self):
        extra = "" if no_login_item() else ", start it now and at every login"
        return f"link {self._link()} → {self._target()}{extra}"

    def install(self, ui):
        from . import __main__ as cli
        config.ensure_home()
        link, target = self._link(), self._target()
        link.parent.mkdir(parents=True, exist_ok=True)
        if link.is_symlink() and link.resolve() != target.resolve():
            link.unlink()  # ours from an older install (or broken): point it here
        if not link.is_symlink() and not link.exists():
            link.symlink_to(target)
        if link.is_symlink():
            ui.say(f"`stick-agent` command: {link}")
        else:
            ui.say(f"left {link} alone (it's not a link we made)")
        if str(link.parent) not in os.environ.get("PATH", "").split(":"):
            ui.say(f"note: add {link.parent} to your PATH to use `stick-agent` from any terminal")
        if no_login_item():
            ui.say("login item skipped (--no-login-item)")
            return
        path = cli.write_plist()
        cli.launchctl("bootout", f"{cli.domain()}/{cli.LABEL}")
        r = cli.launchctl("bootstrap", cli.domain(), str(path))
        if r.returncode != 0:
            raise RuntimeError(f"couldn't start the login item: {r.stderr.strip()}")
        if path == cli.LOCAL_PLIST:
            ui.say(f"note: {cli.LOGIN_PLIST.parent} isn't writable, so it won't start by itself at the next "
                   f"login. Fix once with: sudo chown $(whoami) {cli.LOGIN_PLIST.parent}")
        for _ in range(40):  # up to ~20 s
            if self._dashboard_up():
                ui.say(f"running — dashboard http://localhost:{config.PORT}")
                return
            time.sleep(0.5)
        ui.say("started; the dashboard isn't answering yet (first start can take a while)")

    def uninstall(self, ui):
        from . import __main__ as cli
        if not no_login_item():
            cli.launchctl("bootout", f"{cli.domain()}/{cli.LABEL}")
            for p in (cli.LOGIN_PLIST, cli.LOCAL_PLIST):
                if p.exists():
                    p.unlink()
                    ui.say(f"removed {p}")
        link = self._link()
        if link.is_symlink():
            link.unlink()
            ui.say(f"removed {link}")


# --- model: speech to text ----------------------------------------------------------------

MODEL_SIZE_GUESS = 2_500_000_000  # when the Hub can't say (offline / old huggingface_hub)


def model_cache_dir(repo):
    """The Hugging Face cache folder of a model repo (whatever downloads it: us or the agent's start-up)."""
    from huggingface_hub import constants
    return Path(constants.HF_HUB_CACHE) / ("models--" + repo.replace("/", "--"))


def model_cached_bytes(repo):
    """Bytes in the repo's blob store so far, half-downloaded files included."""
    total = 0
    try:
        for e in os.scandir(model_cache_dir(repo) / "blobs"):
            try:
                total += e.stat(follow_symlinks=False).st_size
            except OSError:
                pass
    except OSError:
        pass
    return total


def model_total_bytes(repo):
    """The repo's download size from the Hub (one metadata call), or the usual size."""
    try:
        from huggingface_hub import HfApi
        info = HfApi().model_info(repo, files_metadata=True, timeout=10)
        size = sum((f.size or 0) for f in info.siblings or [])
        return size or MODEL_SIZE_GUESS
    except Exception:  # noqa: BLE001
        return MODEL_SIZE_GUESS


def fmt_bytes(n):
    return f"{n / 1e9:.1f} GB" if n >= 1e9 else f"{n / 1e6:.0f} MB"


def progress_text(done, total, rate):
    """'1.2 GB of 2.5 GB · 14 MB/s · about 2 min left'"""
    bits = [f"{fmt_bytes(done)} of {fmt_bytes(total)}"]
    if rate > 0:
        bits.append(f"{rate / 1e6:.0f} MB/s")
        left = max(0, total - done) / rate
        bits.append(f"about {max(1, round(left / 60))} min left" if left >= 60 else f"{int(left)} s left")
    return " · ".join(bits)


def watch_download(repo, total, ui, stop, every=1.0):
    """Reports the download's bytes / percent / rate / ETA once a second until `stop` is set,
    by measuring the cache folder (works with any download backend, and for a download that
    another process — the agent's start-up — is doing)."""
    start = last = model_cached_bytes(repo)
    t0 = last_t = time.monotonic()
    rate = 0.0
    while not stop.wait(every):
        done = model_cached_bytes(repo)
        now = time.monotonic()
        if now > last_t:
            inst = (done - last) / (now - last_t)
            rate = inst if not rate else 0.7 * rate + 0.3 * inst  # smoothed
        last, last_t = done, now
        if done - start <= 0 and now - t0 < 5:
            continue
        pct = min(99, int(done * 100 / total)) if total else 0
        ui.progress(pct, progress_text(done, total, rate), bytes=done, total=total,
                    eta_s=int(max(0, total - done) / rate) if rate > 0 else None)


class Model(Component):
    id = "model"
    title = "Speech model"
    description = "Parakeet speech-to-text for hold B (runs on this Mac)"
    default = True
    needs = {"network": True, "download": "2.5 GB"}

    def _repo(self):
        return config.load().get("stt_model") or "mlx-community/parakeet-tdt-0.6b-v2"

    def _cached(self):
        try:
            from huggingface_hub import snapshot_download
            snapshot_download(self._repo(), local_files_only=True)
            return True
        except Exception:  # noqa: BLE001 — not downloaded (or hub not installed)
            return False

    def status(self):
        return ("installed", self._repo()) if self._cached() else ("missing", f"{self._repo()} not downloaded")

    def plan(self):
        return f"download {self._repo()} (~2.5 GB, once)"

    def install(self, ui):
        if self._cached():
            ui.say(f"{self._repo()} is already downloaded")
            return
        from huggingface_hub import snapshot_download
        ui.say(f"downloading {self._repo()} (~2.5 GB) — this takes a few minutes")
        ui.progress(0, "download")
        total = model_total_bytes(self._repo())
        stop = threading.Event()
        watcher = threading.Thread(target=watch_download, args=(self._repo(), total, ui, stop), daemon=True)
        watcher.start()
        try:
            snapshot_download(self._repo())
        finally:
            stop.set()
            watcher.join(timeout=3)
        ui.progress(100, "download", bytes=total, total=total)
        ui.say("speech model ready")

    def uninstall(self, ui):
        try:
            from huggingface_hub import scan_cache_dir
            cache = scan_cache_dir()
        except Exception:  # noqa: BLE001
            ui.say("no model cache found")
            return
        for repo in cache.repos:
            if repo.repo_id == self._repo():
                cache.delete_revisions(*[r.commit_hash for r in repo.revisions]).execute()
                ui.say(f"removed {self._repo()} from the model cache")
                return
        ui.say("the speech model wasn't downloaded")


# --- claude: Claude Code + our hooks ---------------------------------------------------------

def our_hooks():
    """event -> our hook entry (the same command for every install of this checkout)."""
    py = str(venv_python())
    return {
        "PermissionRequest": {"type": "command", "command": f"{py} {HOOK}", "timeout": 45,
                              "statusMessage": "Waiting for the stick…"},
        "UserPromptSubmit": {"type": "command", "command": f"{py} {CLAUDE_HOOKS}", "timeout": 5},
        "Stop": {"type": "command", "command": f"{py} {CLAUDE_HOOKS}", "timeout": 5},
    }


def is_ours(entry, event):
    """Our hook, from this or any other install (paths differ, the script names don't)."""
    cmd = entry.get("command", "") if isinstance(entry, dict) else ""
    script = "stick_agent/hook.py" if event == "PermissionRequest" else "stick_agent/claude_hooks.py"
    return script in cmd


def merge_hooks(settings):
    """Adds / updates our hooks in a Claude Code settings dict, keeping everything else.
    Returns (new settings, changed)."""
    s = json.loads(json.dumps(settings or {}))
    hooks = s.setdefault("hooks", {})
    changed = False
    for event, ours in our_hooks().items():
        groups = hooks.setdefault(event, [])
        found = False
        for g in groups:
            for i, h in enumerate(g.get("hooks", [])):
                if is_ours(h, event):
                    found = True
                    if h != ours:
                        g["hooks"][i] = ours
                        changed = True
        if not found:
            plain = next((g for g in groups if not g.get("matcher")), None)  # a group for every tool
            if plain is None:
                groups.append({"hooks": [ours]})
            else:
                plain.setdefault("hooks", []).append(ours)
            changed = True
    return s, changed


def remove_hooks(settings):
    """Removes only our hooks; drops groups / events that become empty. -> (settings, changed)."""
    s = json.loads(json.dumps(settings or {}))
    hooks = s.get("hooks") or {}
    changed = False
    for event in list(hooks):
        groups = []
        for g in hooks[event]:
            kept = [h for h in g.get("hooks", []) if not is_ours(h, event)]
            if len(kept) != len(g.get("hooks", [])):
                changed = True
            if kept:
                groups.append({**g, "hooks": kept})
        if groups:
            hooks[event] = groups
        else:
            del hooks[event]
            changed = True
    if "hooks" in s and not s["hooks"]:
        del s["hooks"]
    return s, changed


def hooks_state(settings):
    """-> (n of our 3 hooks present and current, n present at all)."""
    current = present = 0
    for event, ours in our_hooks().items():
        for g in (settings.get("hooks") or {}).get(event, []):
            for h in g.get("hooks", []):
                if is_ours(h, event):
                    present += 1
                    current += h == ours
                    break
    return current, present


def write_settings(path, new, ui):
    """Writes Claude Code's settings.json, keeping a timestamped backup of the old one."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        backup = path.with_name(f"{path.name}.bak-{time.strftime('%Y%m%d-%H%M%S')}")
        shutil.copy2(path, backup)
        ui.say(f"backup: {backup}")
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(new, indent=2) + "\n")
    tmp.replace(path)


def read_settings(path):
    if not path.exists():
        return {}
    text = path.read_text()
    if not text.strip():
        return {}
    return json.loads(text)  # a broken file is an error: we never overwrite what we can't read


class Claude(Component):
    id = "claude"
    title = "Claude Code integration"
    description = "approvals on the stick and \"Claude finished\" alerts (adds 3 hooks to Claude Code's settings)"
    default = True

    def _claude(self):
        return config.claude_path(config.load())

    def _logged_in(self, claude):
        r = run([claude, "auth", "status"], timeout=15)
        if r.returncode != 0:
            return False
        try:
            return bool(json.loads(r.stdout).get("loggedIn"))
        except ValueError:
            return "logged in" in r.stdout.lower()

    def status(self):
        claude = self._claude()
        try:
            current, present = hooks_state(read_settings(claude_settings_path()))
        except ValueError:
            return "partial", f"{claude_settings_path()} isn't valid JSON"
        hooks = f"hooks {current}/3" + (" (some point to another install)" if present > current else "")
        if not claude:
            return ("partial" if present else "missing"), f"Claude Code not installed · {hooks}"
        state = "installed" if current == 3 else "partial" if present else "missing"
        return state, f"Claude Code found · {hooks}"

    def plan(self):
        return f"add / update 3 hooks in {claude_settings_path()} (backup first)"

    def install(self, ui):
        claude = self._claude()
        if not claude:
            ui.say("Claude Code isn't installed. Anthropic's installer:")
            ui.say(f"  {CLAUDE_INSTALL}")
            if ui.interactive and not ui.assume_yes and ui.confirm("Run it now?", default=True):
                subprocess.run(["/bin/sh", "-c", CLAUDE_INSTALL])
                claude = self._claude()
            if not claude:
                ui.say("continuing without it — the hooks are added now and work once Claude Code is installed")
        if claude:
            r = run([claude, "--version"], timeout=15)
            ui.say(f"Claude Code {r.stdout.strip() or 'found'}")
            if not self._logged_in(claude):
                ui.say("not logged in — run `claude` once and log in (your own account; no API key needed)")
        path = claude_settings_path()
        new, changed = merge_hooks(read_settings(path))
        if changed:
            write_settings(path, new, ui)
            ui.say(f"hooks added to {path}")
        else:
            ui.say("hooks already in place")

    def uninstall(self, ui):
        path = claude_settings_path()
        if not path.exists():
            ui.say("no Claude Code settings file")
            return
        new, changed = remove_hooks(read_settings(path))
        if changed:
            write_settings(path, new, ui)
            ui.say(f"removed our hooks from {path} (others kept)")
        else:
            ui.say("our hooks weren't there")


# --- github: PR alerts --------------------------------------------------------------------

REPO_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9-]{0,38}/[A-Za-z0-9._-]{1,100}$")
GH_DEVICE_URL = "https://github.com/login/device"
GH_CODE_RE = re.compile(r"one-time code:\s*([A-Z0-9]{4}-[A-Z0-9]{4})")
GH_URL_RE = re.compile(r"(https://github\.com/login/device)\b")
BREW_INSTALL = '/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"'


def brew_path():
    return shutil.which("brew") or next((p for p in ("/opt/homebrew/bin/brew", "/usr/local/bin/brew")
                                         if Path(p).exists()), None)


def valid_repo(repo):
    """owner/name as GitHub allows it (and nothing a URL or a shell could trip over)."""
    return bool(REPO_RE.match(repo or "")) and repo.split("/")[1] not in (".", "..")


def set_primary_repo(repo, alerts=None):
    """The main repo: GitHub alerts watch it first, and the "PR 1234" / Devin review quick
    commands open its pull requests (the setup wizard and the setup page call this through
    POST /api/install/github/repo). alerts: True / False turns the watcher on / off, None keeps
    it. -> the new github config."""
    if repo:
        try:
            doc = json.loads(config.COMMANDS.read_text())
        except (OSError, ValueError):
            doc = None
        if isinstance(doc, dict) and isinstance(doc.get("commands"), list):
            changed = False
            for c in doc["commands"]:
                url = c.get("open_url") if isinstance(c, dict) else None
                if not isinstance(url, str):
                    continue
                new = re.sub(r"(github\.com/)[^/]+/[^/]+(/pull/)", lambda m: f"{m[1]}{repo}{m[2]}", url)
                new = re.sub(r"(app\.devin\.ai/review/)[^/]+/[^/]+(/pull/)", lambda m: f"{m[1]}{repo}{m[2]}", new)
                if new != url:
                    c["open_url"], changed = new, True
            if changed:
                config.COMMANDS.write_text(json.dumps(doc, indent=2))
    cfg = config.load()
    g = dict(cfg.get("github") or {})
    g["primary_repo"] = repo
    g["on"] = bool(alerts) if alerts is not None else g.get("on", True) is not False
    g["interval_s"] = g.get("interval_s", 120)
    cfg["github"] = g
    config.save(cfg)
    return g


def gh_login_args(gh):
    """`gh auth login` in its browser (one-time code) flow, asking nothing."""
    return [gh, "auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web"]


def parse_device_code(text):
    """gh's one-time code and the page to enter it on, from its output so far -> (code, url) or
    (None, None). gh prints "! First copy your one-time code: ABCD-1234", then either "Open this URL
    to continue in your web browser: https://github.com/login/device" (not a terminal) or "Press
    Enter to open https://github.com/login/device in your browser..." (a terminal)."""
    text = re.sub(r"\x1b\[[0-9;]*m", "", text or "")
    m = GH_CODE_RE.search(text)
    if not m:
        return None, None
    u = GH_URL_RE.search(text)
    return m.group(1), (u.group(1) if u else GH_DEVICE_URL)


class GitHub(Component):
    id = "github"
    title = "GitHub alerts"
    description = "celebrate merged PRs, heads-up on review requests / failed CI (uses your `gh` login)"
    default = False
    needs = {"terminal": "`gh auth login` asks you to log in"}

    def _gh(self):
        return shutil.which("gh") or next((p for p in ("/opt/homebrew/bin/gh", "/usr/local/bin/gh")
                                           if Path(p).exists()), None)

    def _logged_in(self, gh):
        return run([gh, "auth", "status"], timeout=15).returncode == 0

    def status(self):
        gh = self._gh()
        g = config.load().get("github") or {}
        if not gh:
            return "missing", "the GitHub CLI (gh) isn't installed"
        if not self._logged_in(gh):
            return "partial", "gh found, not logged in"
        on = g.get("on", False)
        return ("installed" if on else "missing"), f"watching {g.get('primary_repo') or 'your PRs'}" if on else "off"

    def plan(self):
        return "check `gh`, log in if needed, set your main repo, turn the watcher on"

    def install(self, ui):
        gh = self._gh()
        if not gh:
            raise NotAvailable("install the GitHub CLI first: brew install gh (or https://cli.github.com)")
        if not self._logged_in(gh):
            if ui.interactive and not ui.assume_yes:
                ui.say("logging in to GitHub (gh auth login)…")
                subprocess.run([gh, "auth", "login"])
            if not self._logged_in(gh):
                raise NotAvailable("not logged in to GitHub — run `gh auth login`, then try again")
        g = config.load().get("github") or {}
        repo = ui.ask("Your main repo (owner/name)", g.get("primary_repo") or "")
        if repo and not valid_repo(repo):
            raise NotAvailable(f"{repo!r} isn't a repo name like owner/name")
        set_primary_repo(repo, alerts=True)
        ui.say(f"GitHub alerts on{f' — main repo {repo}' if repo else ''} (restart the agent to apply)")

    def uninstall(self, ui):
        cfg = config.load()
        g = dict(cfg.get("github") or {})
        g["on"] = False
        cfg["github"] = g
        config.save(cfg)
        ui.say("GitHub alerts off")


# --- firmware: flash a stick on USB ---------------------------------------------------------

def fetch_firmware(ui):
    """Downloads the latest release's firmware (update channel from the config) into
    ~/.stick-agent/firmware/<version>/, sha256-checked. Never fails: without a release (or
    the network) whatever firmware is already here is used."""
    from . import releases
    channel = config.load().get("update_channel") or "stable"
    try:
        rel, _folder, manifest = releases.ensure_latest_firmware(channel, say=ui.say)
        ui.say(f"firmware {manifest['version']} from release {rel['tag']} (sha256 checked)")
    except releases.NoRelease as e:
        ui.say(str(e))
    except releases.ReleaseError as e:
        ui.say(f"couldn't download the latest release ({e}) — using the firmware already here, if any")


class Firmware(Component):
    id = "firmware"
    title = "Flash a stick"
    description = "download the latest release's firmware and put it on a stick plugged in by USB (keeps its settings)"
    default = False
    needs = {"usb": True, "network": True}
    from_dashboard = False  # the dashboard has its own flow: Stick → Firmware
    removable = False

    def status(self):
        from . import flasher
        fw = flasher.bundled()
        if not fw:
            return "n/a", "no firmware here yet (downloads the latest release)"
        ports = flasher.espressif_ports()
        return "n/a", f"firmware {fw['version']} ({flasher.describe(fw)}) · {len(ports)} stick(s) on USB"

    def plan(self):
        return "download the latest release's firmware, flash it to the stick on USB (asks about a backup first)"

    def install(self, ui):
        from . import flasher
        fetch_firmware(ui)
        if not flasher.bundled():
            raise NotAvailable(flasher.NO_FIRMWARE)
        ports = flasher.espressif_ports()
        if not ports:
            raise NotAvailable("no stick on USB — plug it in (or hold its side button for download mode)")
        port = ports[0]["port"]
        if len(ports) > 1:
            names = ", ".join(p["port"] for p in ports)
            port = ui.ask(f"Several sticks on USB ({names}) — which port", port)
        backup = ui.confirm("Back up the stick's current firmware first? (adds ~1 min)", default=False)
        code = flasher.cli(port, backup)
        if code:
            raise RuntimeError("flashing failed (see above)")

    def uninstall(self, ui):
        raise NotAvailable("nothing to remove")


# --- wireless mic for Wispr ------------------------------------------------------------------

BLACKHOLE_CASK = "brew install --cask blackhole-2ch"
BLACKHOLE_DRIVERS = [Path("/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver"),
                     Path.home() / "Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver"]


def blackhole_installed():
    """BlackHole 2ch's Core Audio driver is installed (what the Homebrew cask puts in place).
    Checked on disk, so it needs no audio library and doesn't disturb a running agent's stream."""
    return any(p.exists() for p in BLACKHOLE_DRIVERS)


def blackhole_version():
    """The installed BlackHole 2ch's version (its driver's Info.plist), or None."""
    import plistlib
    for d in BLACKHOLE_DRIVERS:
        try:
            with (d / "Contents/Info.plist").open("rb") as f:
                return str(plistlib.load(f).get("CFBundleShortVersionString") or "?")
        except (OSError, ValueError, plistlib.InvalidFileException):
            continue
    return None


# The one-click install (the setup page) uses this exact, tested package — not whatever is newest.
# Bump it only after testing the new version with Wispr: take version / url / sha256 from
# https://formulae.brew.sh/api/cask/blackhole-2ch.json (see docs/RELEASING.md).
BLACKHOLE_PIN = {
    "version": "0.7.1",
    "url": "https://existential.audio/downloads/BlackHole2ch-0.7.1.pkg",
    "sha256": "57b540f27a3e29c37e310e01bee0fdfab76733087e47f997ef9dccf851400dcf",
}
BLACKHOLE_SIGNER = "Developer ID Installer: Existential Audio Inc. (Q5C99V536K)"
BLACKHOLE_MAX_BYTES = 30_000_000  # the pkg is ~100 kB; anything huge isn't it


def download_verified(url, sha256, dest, ui=None, opener=None):
    """Downloads url (HTTPS only) to dest, checking its sha256 on the way; a mismatch deletes
    the file and raises NotAvailable. -> dest"""
    import hashlib
    if not url.startswith("https://"):
        raise NotAvailable(f"refusing a download that isn't HTTPS: {url}")
    opener = opener or (lambda u: urllib.request.urlopen(u, timeout=30))
    h = hashlib.sha256()
    n = 0
    try:
        with opener(url) as r, open(dest, "wb") as f:
            while chunk := r.read(65536):
                n += len(chunk)
                if n > BLACKHOLE_MAX_BYTES:
                    raise NotAvailable("the download is far bigger than expected — stopped")
                h.update(chunk)
                f.write(chunk)
    except OSError as e:
        Path(dest).unlink(missing_ok=True)
        raise NotAvailable(f"couldn't download {url}: {e}") from e
    except NotAvailable:
        Path(dest).unlink(missing_ok=True)
        raise
    if h.hexdigest() != sha256:
        Path(dest).unlink(missing_ok=True)
        raise NotAvailable(f"the download's checksum is wrong (got {h.hexdigest()[:12]}…, expected "
                           f"{sha256[:12]}…) — not installing it")
    if ui:
        ui.say(f"downloaded {n // 1000} kB, sha256 checked")
    return dest


def signature_ok(output):
    """`pkgutil --check-signature` output -> (ok, why). It must be a notarized Developer ID
    Installer package signed by Existential Audio (BlackHole's developer)."""
    out = output or ""
    if "Status: signed by a developer certificate issued by Apple" not in out:
        return False, "the package isn't signed with an Apple-issued developer certificate"
    if "Notarization: trusted by the Apple notary service" not in out:
        return False, "the package isn't notarized by Apple"
    first = re.search(r"^\s*1\.\s*(.+?)\s*$", out, re.M)
    if not first or first.group(1) != BLACKHOLE_SIGNER:
        return False, f"the package is signed by {first.group(1) if first else 'nobody we know'}, not Existential Audio"
    return True, ""


def applescript_quote(s):
    """A string literal for AppleScript."""
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def admin_install_args(pkg):
    """osascript argv that installs pkg with the Mac's own admin password dialog (we never see the
    password) and restarts Core Audio in the same call, so BlackHole shows up without a reboot and
    with a single prompt (its installer only says "restart" — its scripts don't reload Core Audio)."""
    sh = f"/usr/sbin/installer -pkg {shlex.quote(str(pkg))} -target / && (/usr/bin/killall coreaudiod || true)"
    prompt = "Stick Agent wants to install BlackHole 2ch, the audio driver for the wireless Wispr mic."
    script = (f"do shell script {applescript_quote(sh)} with administrator privileges "
              f"with prompt {applescript_quote(prompt)}")
    return ["/usr/bin/osascript", "-e", script]


def install_blackhole(ui, runner=None, opener=None, workdir=None):
    """BlackHole 2ch, the pinned package: download, sha256, signature, then the macOS admin dialog.
    runner(argv, timeout=) -> CompletedProcess (tests inject one). Already installed: nothing."""
    import tempfile
    runner = runner or run
    if blackhole_installed():
        v = blackhole_version()
        pin = BLACKHOLE_PIN["version"]
        ui.say(f"BlackHole {v} found" + ("" if v == pin else f" (tested with {pin})"))
        return
    base = Path(workdir or config.HOME)
    base.mkdir(parents=True, exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix="blackhole-", dir=base))  # 0700: only we can swap the file
    pkg = folder / f"BlackHole2ch-{BLACKHOLE_PIN['version']}.pkg"
    try:
        ui.say(f"downloading BlackHole 2ch {BLACKHOLE_PIN['version']} from existential.audio…")
        ui.progress(10, "Downloading BlackHole 2ch…")
        download_verified(BLACKHOLE_PIN["url"], BLACKHOLE_PIN["sha256"], pkg, ui, opener)
        ui.progress(40, "Checking its signature…")
        r = runner(["/usr/sbin/pkgutil", "--check-signature", str(pkg)], timeout=60)
        ok, why = signature_ok(r.stdout)
        if r.returncode != 0 or not ok:
            raise NotAvailable(f"not installing BlackHole: {why or 'pkgutil could not check it'}")
        ui.say("signed by Existential Audio and notarized by Apple ✓")
        ui.progress(60, "Waiting for your Mac password (a macOS dialog)…")
        ui.say("macOS asks for your password now (its own dialog — Stick Agent never sees it)")
        r = runner(admin_install_args(pkg), timeout=600)
        if r.returncode != 0:
            err = (r.stderr or r.stdout or "").strip()
            if "-128" in err or "canceled" in err.lower() or "cancelled" in err.lower():
                raise NotAvailable("cancelled — nothing was installed")
            raise NotAvailable(f"the installer failed: {err[:300]}")
        ui.progress(90, "Restarting Core Audio…")
        for _ in range(10):
            if blackhole_installed():
                break
            time.sleep(0.5)
        if not blackhole_installed():
            raise NotAvailable("the installer finished but BlackHole 2ch isn't there — restart the Mac and check again")
        ui.say(f"BlackHole 2ch {blackhole_version() or BLACKHOLE_PIN['version']} installed")
    finally:
        shutil.rmtree(folder, ignore_errors=True)


RANK_HINT = ("once: hold A, then in Wispr Flow → Settings → Microphone drag \"<stick>-mic-bluetooth\" "
             "(e.g. stick-key-mic-bluetooth) to the top "
             "(don't pick BlackHole 2ch itself)")


TAP_NOTE = ("macOS asks once to allow \"System Audio Recording\" for Python — that's the helper app; "
            "it only taps its own sound")


def tap_wanted(cfg=None):
    """The Apple audio tap carries the wireless mic (macOS 14.2+, not set to BlackHole): no driver needed."""
    from . import stick_mic_device
    cfg = cfg if cfg is not None else config.load()
    return stick_mic_device.macos_supports_tap() and cfg.get("wireless_mic_carrier", "auto") != "blackhole"


class WirelessMic(Component):
    id = "wireless_mic"
    title = "Wireless Wispr mic"
    description = ("the stick's mic for Wispr without a cable: while A is held a \"<stick>-mic-bluetooth\" mic appears "
                   "(an Apple audio tap of the agent's own sound carries it on macOS 14.2+, else BlackHole 2ch; "
                   "Wispr is never set to either)")
    default = False

    @property
    def needs(self):
        if tap_wanted():
            return {"permission": "System Audio Recording (macOS asks once)"}
        return {"admin": True, "terminal": f"BlackHole 2ch: `{BLACKHOLE_CASK}` in a terminal"}

    def status(self):
        cfg = config.load()
        on = bool(cfg.get("wireless_mic"))
        if tap_wanted(cfg):
            if on:
                return "installed", f"on · the Apple audio tap carries it ({TAP_NOTE}) — {RANK_HINT}"
            return "missing", "off · no driver needed on this Mac (the Apple audio tap)"
        bh = blackhole_installed()
        if bh and on:
            return "installed", f"on · BlackHole 2ch found — {RANK_HINT}"
        if on:
            return "partial", f"on, but BlackHole 2ch isn't installed — run: {BLACKHOLE_CASK}"
        return "missing", "off · BlackHole 2ch found" if bh else f"off · needs BlackHole 2ch ({BLACKHOLE_CASK})"

    def plan(self):
        if tap_wanted():
            return f"turn the wireless mic on (the Apple audio tap: {TAP_NOTE})"
        return "check for BlackHole 2ch (or say how to install it), turn the wireless mic on"

    def install(self, ui):
        cfg = config.load()
        cfg["wireless_mic"] = True
        config.save(cfg)
        ui.say("wireless Wispr mic on (from a terminal: restart the agent to apply)")
        if tap_wanted(cfg):
            ui.say(f"the Apple audio tap carries it, no driver needed: {TAP_NOTE}")
            ui.say(RANK_HINT)
            return
        if not blackhole_installed():
            # Never run it ourselves: the cask installs a system audio driver and asks for the admin password
            raise NotAvailable(f"BlackHole 2ch isn't installed yet — install it from the setup page "
                               f"(stick-agent install-page), or in a terminal run: {BLACKHOLE_CASK} "
                               "(it asks for your Mac password)")
        ui.say(f"BlackHole 2ch found — {RANK_HINT}")

    def uninstall(self, ui):
        cfg = config.load()
        cfg["wireless_mic"] = False
        config.save(cfg)
        ui.say("wireless Wispr mic off (BlackHole stays installed: brew uninstall --cask blackhole-2ch removes it)")


# --- mcp: the stick tools for terminal Claude Code sessions ---------------------------------

class Mcp(Component):
    id = "mcp"
    title = "Stick tools for Claude Code"
    description = ("any Claude Code session can notify you, ask A/B questions and show progress on the stick "
                   "(runs `claude mcp add --scope user stick …`; voice requests have them already)")
    default = False

    def status(self):
        from . import mcp_setup
        st = mcp_setup.status()
        if st["installed"]:
            return "installed", st["detail"]
        if st["installed"] is None:
            return "missing", st["detail"]
        return ("partial" if st.get("conflict") else "missing"), f"{st['detail']} — runs: {st['command']}"

    def plan(self):
        from . import mcp_setup
        return f"run: {mcp_setup.shown(mcp_setup.add_args('claude', config.token()))}"

    def install(self, ui):
        from . import mcp_setup
        ok, msg = mcp_setup.install(confirm=None if ui.assume_yes or not ui.interactive else ui.confirm,
                                    say=ui.say)
        ui.say(msg)
        if not ok:
            raise NotAvailable(msg)

    def uninstall(self, ui):
        from . import mcp_setup
        ok, msg = mcp_setup.uninstall(confirm=None, say=ui.say)
        ui.say(msg)
        if not ok:
            raise NotAvailable(msg)


COMPONENTS = [Core(), Model(), Claude(), GitHub(), Firmware(), WirelessMic(), Mcp()]
BY_ID = {c.id: c for c in COMPONENTS}


def selection(with_=(), without=(), only=None):
    """The components to install: defaults (or just `only`), plus --with, minus --without (core always)."""
    unknown = [x for x in (*with_, *without, *(only or ())) if x not in BY_ID]
    if unknown:
        raise SystemExit(f"unknown component(s): {', '.join(unknown)} — choose from {', '.join(BY_ID)}")
    picked = set(only) if only is not None else {c.id for c in COMPONENTS if c.default}
    picked |= set(with_)
    picked -= set(without)
    picked.add("core")
    return [c for c in COMPONENTS if c.id in picked]


# --- command line ---------------------------------------------------------------------------

def parse(argv):
    opts = {"yes": False, "with": [], "without": [], "dry_run": False, "status": False, "uninstall": None,
            "json": False, "purge": False, "only": None}
    i = 0

    def value():
        nonlocal i
        i += 1
        if i >= len(argv):
            raise SystemExit(f"{argv[i - 1]} needs a value")
        return [x.strip() for x in argv[i].split(",") if x.strip()]

    while i < len(argv):
        a = argv[i]
        if a in ("--yes", "-y"):
            opts["yes"] = True
        elif a == "--with":
            opts["with"] += value()
        elif a.startswith("--with="):
            opts["with"] += [x for x in a.split("=", 1)[1].split(",") if x]
        elif a == "--without":
            opts["without"] += value()
        elif a == "--only":  # just these (install.sh: --only core; the setup page does the rest)
            opts["only"] = (opts["only"] or []) + value()
        elif a.startswith("--without="):
            opts["without"] += [x for x in a.split("=", 1)[1].split(",") if x]
        elif a == "--dry-run":
            opts["dry_run"] = True
        elif a == "--status":
            opts["status"] = True
        elif a == "--json":
            opts["json"] = True
        elif a == "--purge":
            opts["purge"] = True
        elif a == "--no-login-item":
            os.environ["STICK_AGENT_NO_LOGIN_ITEM"] = "1"
        elif a == "--uninstall":  # --uninstall [all | a,b]
            what = "all"
            if i + 1 < len(argv) and not argv[i + 1].startswith("-"):
                i += 1
                what = argv[i]
            opts["uninstall"] = list(BY_ID) if what == "all" else [x for x in what.split(",") if x]
        elif a in ("-h", "--help"):
            print(__doc__)
            raise SystemExit(0)
        else:
            raise SystemExit(f"unknown option {a} (see stick-agent setup --help)")
        i += 1
    return opts


ICON = {"installed": "✓", "partial": "◐", "missing": "·", "n/a": " "}


def print_status():
    for c in COMPONENTS:
        state, detail = c.status()
        print(f"  {ICON.get(state, '?')} {c.title:<26} {state:<10} {detail}")


def menu(picked):
    """Interactive picker: toggle by number, Enter to go on."""
    chosen = {c.id for c in picked}
    while True:
        print("\nWhat to install:")
        for n, c in enumerate(COMPONENTS, 1):
            state, _ = c.status()
            mark = "x" if c.id in chosen else " "
            note = []
            if c.needs.get("download"):
                note.append(f"downloads {c.needs['download']}")
            if c.needs.get("admin"):
                note.append("needs your admin password")
            if c.needs.get("usb"):
                note.append("stick on USB")
            if c.needs.get("terminal"):
                note.append(c.needs["terminal"])
            if c.needs.get("permission"):
                note.append(f"macOS permission: {c.needs['permission']}")
            locked = " (always)" if c.id == "core" else ""
            print(f"  {n}. [{mark}] {c.title}{locked} — {c.description}"
                  f"{'  · ' + '; '.join(note) if note else ''}  [{state}]")
        try:
            answer = input("\nNumbers to toggle (e.g. 2 4), Enter to continue: ").strip()
        except EOFError:
            answer = ""
        if not answer:
            return [c for c in COMPONENTS if c.id in chosen]
        for tok in answer.replace(",", " ").split():
            if tok.isdigit() and 1 <= int(tok) <= len(COMPONENTS):
                cid = COMPONENTS[int(tok) - 1].id
                if cid == "core":
                    continue
                chosen.symmetric_difference_update({cid})


def run_components(components, action, ui, dry_run=False):
    """Installs / uninstalls in order. -> {id: "ok" | "skipped: …" | "failed: …"}."""
    results = {}
    for c in components:
        if ui._emit is None:
            print(f"\n→ {c.title}")
        else:
            ui.say(f"— {c.title}")
        if dry_run:
            ui.say(f"would {action}: {c.plan()}")
            results[c.id] = "dry-run"
            continue
        try:
            (c.install if action == "install" else c.uninstall)(ui)
            results[c.id] = "ok"
        except NotAvailable as e:
            ui.say(str(e))
            results[c.id] = f"skipped: {e}"
        except Exception as e:  # noqa: BLE001 — keep going with the rest
            ui.say(f"failed: {e}")
            results[c.id] = f"failed: {e}"
    return results


def purge(ui):
    home = config.HOME
    if home.exists():
        shutil.rmtree(home)
        ui.say(f"deleted {home} (settings, history, logs, the Python env)")


def main(argv):
    opts = parse(argv)
    interactive = sys.stdin.isatty() and not opts["yes"] and not opts["json"]
    ui = UI(interactive=interactive, assume_yes=opts["yes"])

    if opts["status"]:
        if opts["json"]:
            print(json.dumps([c.info() for c in COMPONENTS], indent=2))
        else:
            print_status()
        return 0

    if opts["uninstall"] is not None:
        ids = opts["uninstall"]
        unknown = [x for x in ids if x not in BY_ID]
        if unknown:
            raise SystemExit(f"unknown component(s): {', '.join(unknown)}")
        # remove in reverse order (core last: it's what's running), skipping non-removable ones
        targets = [c for c in reversed(COMPONENTS) if c.id in ids and c.removable]
        print("Will remove: " + ", ".join(c.title for c in targets) + (" and delete ~/.stick-agent" if opts["purge"]
                                                                          else ""))
        if not opts["dry_run"] and not ui.confirm("Go ahead?", default=not interactive):
            print("cancelled")
            return 1
        results = run_components(targets, "uninstall", ui, opts["dry_run"])
        if opts["purge"] and not opts["dry_run"]:
            purge(ui)
        print_results(results, opts)
        return 0 if all(not r.startswith("failed") for r in results.values()) else 1

    picked = selection(opts["with"], opts["without"], opts["only"])
    if interactive:
        picked = menu(picked)
    print("\nPlan:")
    for c in picked:
        print(f"  • {c.title}: {c.plan()}")
    if opts["dry_run"]:
        run_components(picked, "install", ui, dry_run=True)
        print("\n(dry run — nothing changed)")
        return 0
    if interactive and not ui.confirm("\nInstall these?", default=True):
        print("cancelled")
        return 1
    results = run_components(picked, "install", ui)
    print_results(results, opts)
    return 0 if all(not r.startswith("failed") for r in results.values()) else 1


def print_results(results, opts):
    if opts["json"]:
        print(json.dumps(results, indent=2))
        return
    print("\nDone:")
    for cid, r in results.items():
        print(f"  {'✓' if r == 'ok' else '·' if r.startswith('skipped') or r == 'dry-run' else '✗'} "
              f"{BY_ID[cid].title}: {r}")
    print(f"\nDashboard: http://localhost:{config.PORT}   ·   status: stick-agent setup --status")
