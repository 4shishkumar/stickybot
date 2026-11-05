"""The setup page (dashboard #install): the install walked through one step per screen.

install.sh only puts the app in place and starts it; then it opens this page, which does
the rest with buttons: the speech model, Claude Code (+ login), its hooks, the stick tools
(MCP), GitHub alerts (gh login with a one-time code), the wireless Wispr mic (macOS 14.2+:
the Apple audio tap, one permission prompt; older macOS or by choice: BlackHole through the
Mac's own password dialog), and the stick (the existing setup wizard).

  GET  /api/install                      {"steps": [...], "next", "pending", "skipped"}
  GET  /api/install/{step}               one step: {"id","title","kind","why","needs","state","detail","data","job"}
  POST /api/install/{step}/skip|unskip   -> the listing (skips are kept in config "install_skipped")
  POST /api/install/{step}/run           -> {"job"}  (model, claude, hooks, mcp, github, wispr)
                                         wispr: {"carrier": "tap" | "blackhole"} (default: the Mac's best)
  POST /api/install/github/repo          {"repo": "owner/name", "alerts": true|false|null} -> {"ok", "github"}
  GET  /api/install/jobs/{job}           SSE: {"line"} / {"progress","stage",...} / {"code","url"} / {"done",...}
  POST /api/install/jobs/{job}/cancel    stops a running job (e.g. the gh login)

A step's state: done | todo | skipped | unavailable. Nothing changes the Mac until its button
is clicked (the guard in server.py wants the token for every POST).
"""
import asyncio
import logging
import os
import platform
import shutil
import subprocess
import threading
from pathlib import Path

from aiohttp import web

from . import config, installer
from .firmware_api import Job, onboarding_state, stream_job

log = logging.getLogger("install")

STEPS = [
    {"id": "welcome", "title": "Welcome", "kind": "required",
     "why": "A quick check that this Mac can run Stick Agent."},
    {"id": "model", "title": "Speech model", "kind": "recommended",
     "why": "Turns what you say into text, on this Mac (nothing is sent anywhere). Needed for hold B.",
     "needs": {"download": "2.5 GB"}},
    {"id": "claude", "title": "Claude Code", "kind": "optional",
     "why": "Answers your voice requests and does the work. Uses your own Claude login.",
     "needs": {"terminal": "log in once in Terminal"}},
    {"id": "hooks", "title": "Approvals on the stick", "kind": "optional",
     "why": "Approve Claude Code's requests with A / B on the stick, and get a \"Claude finished\" alert."},
    {"id": "mcp", "title": "Stick tools for Claude Code", "kind": "optional",
     "why": "Any Claude Code session can notify you, ask A/B questions and show progress on the stick."},
    {"id": "github", "title": "GitHub alerts", "kind": "optional",
     "why": "Celebrate merged PRs and get a heads-up on review requests and failed CI.",
     "needs": {"browser": "log in to GitHub"}},
    {"id": "wispr", "title": "Wireless Wispr mic", "kind": "optional",
     "why": "Dictate with Wispr Flow through the stick's mic without a cable.",
     "needs": {"admin": True, "download": "0.1 MB"}},
    {"id": "stick", "title": "Your stick", "kind": "recommended",
     "why": "Pair the stick, pick it, name it, update its firmware and try it."},
    {"id": "done", "title": "Done", "kind": "required", "why": "What's set up, and where to change it later."},
]
BY_ID = {s["id"]: s for s in STEPS}
SKIPPABLE = {s["id"] for s in STEPS} - {"welcome", "done"}
RUNNABLE = {"model", "claude", "hooks", "mcp", "github", "wispr"}
MIN_FREE_BYTES = 4_000_000_000
MIN_MACOS = (13, 5)  # MLX (the speech model)


def skipped(cfg):
    return [s for s in (cfg.get("install_skipped") or []) if s in SKIPPABLE]


def quick_pending(cfg):
    """Cheap check for the menu bar: is a required / recommended step still to do (and not skipped)?"""
    skip = set(skipped(cfg))
    if "stick" not in skip and not cfg.get("stick"):
        return True
    return "model" not in skip and not installer.Model()._cached()


# --- the checks (blocking: run in a worker thread) ------------------------------------------

def _vtuple(v):
    out = []
    for p in (v or "").split("."):
        out.append(int(p) if p.isdigit() else 0)
    return tuple(out + [0, 0])[:2]


def check_welcome(_agent):
    mach, mac = platform.machine(), platform.mac_ver()[0]
    try:
        free = shutil.disk_usage(Path.home()).free
    except OSError:
        free = None
    need_model = not installer.Model()._cached()
    arm = mach == "arm64"
    mac_ok = bool(mac) and _vtuple(mac) >= MIN_MACOS
    checks = [
        {"label": "Apple Silicon", "level": "ok" if arm else "err",
         "detail": "the speech model runs on its GPU" if arm else f"this Mac is {mach}; Apple Silicon is needed"},
        {"label": "macOS", "level": "ok" if mac_ok else "err",
         "detail": f"macOS {mac or '?'}" + ("" if mac_ok else " — 13.5 or newer is needed")},
    ]
    if free is not None:
        low = need_model and free < MIN_FREE_BYTES
        note = " — the speech model needs 2.5 GB; free up some space first" if low else ""
        checks.append({"label": "Free disk space", "level": "warn" if low else "ok",
                       "detail": f"{installer.fmt_bytes(free)} free{note}"})
    bad = any(c["level"] == "err" for c in checks)
    return ("unavailable" if bad else "done"), ("this Mac can't run it" if bad else "this Mac is ready"), \
        {"checks": checks}


def check_model(agent):
    m = installer.Model()
    repo = m._repo()
    have = installer.model_cached_bytes(repo)
    ready = bool(getattr(agent, "stt_ready", False))
    data = {"repo": repo, "bytes": have, "stt_ready": ready,
            "agent_downloading": not ready and have > 0}
    if m._cached():
        return "done", "downloaded" + ("" if ready else " — loading it…"), data
    return "todo", "not downloaded yet (2.5 GB, once)", data


def _claude_info():
    claude = config.claude_path(config.load())
    if not claude:
        return None, None, False
    r = installer.run([claude, "--version"], timeout=15)
    version = r.stdout.strip() if r.returncode == 0 else None
    return claude, version, installer.Claude()._logged_in(claude)


def check_claude(_agent):
    claude, version, logged_in = _claude_info()
    data = {"installed": bool(claude), "version": version, "logged_in": logged_in,
            "install_cmd": installer.CLAUDE_INSTALL, "path": claude}
    if not claude:
        return "todo", "not installed", data
    if not logged_in:
        return "todo", f"installed ({version or claude}), not logged in", data
    return "done", f"{version or 'installed'} · logged in", data


def hooks_preview():
    """Exactly what gets added to Claude Code's settings.json (next to anything already there)."""
    return {"hooks": {event: [{"hooks": [entry]}] for event, entry in installer.our_hooks().items()}}


def check_hooks(_agent):
    path = installer.claude_settings_path()
    data = {"path": str(path), "exists": path.exists(), "preview": hooks_preview(),
            "claude_installed": bool(config.claude_path(config.load()))}
    try:
        current, present = installer.hooks_state(installer.read_settings(path))
    except ValueError:
        return "unavailable", f"{path} isn't valid JSON — fix it first (nothing will be changed)", data
    data.update(current=current, present=present)
    if current == 3:
        return "done", f"all 3 hooks are in {path}", data
    return "todo", (f"{current} of 3 hooks in place" if present else "not added"), data


def check_mcp(_agent):
    from . import mcp_setup
    if not config.claude_path(config.load()):
        return "unavailable", "needs Claude Code (the step before)", {}
    st = mcp_setup.status()
    data = {"command": st.get("command"), "conflict": bool(st.get("conflict"))}
    if st["installed"]:
        return "done", st["detail"], data
    return ("unavailable" if st.get("conflict") else "todo"), st["detail"], data


def check_github(_agent, login=None):
    g = installer.GitHub()
    gh = g._gh()
    cfg = (config.load().get("github") or {})
    data = {"gh": bool(gh), "brew": bool(installer.brew_path()), "brew_cmd": "brew install gh",
            "brew_install": installer.BREW_INSTALL, "logged_in": False, "repo": cfg.get("primary_repo") or "",
            "on": cfg.get("on", True) is not False, "login": login}
    if not gh:
        return "todo", "the GitHub CLI (gh) isn't installed", data
    data["logged_in"] = g._logged_in(gh)
    if not data["logged_in"]:
        return "todo", "gh is installed, not logged in", data
    if not (data["on"] and installer.valid_repo(data["repo"])):
        return "todo", "logged in — pick your main repo", data
    return "done", f"watching {data['repo']}", data


def _tap_info(agent):
    """The agent's tap check as the page shows it: {"state", "detail", "carrier", "running"}."""
    from . import stick_mic_device
    dev = getattr(agent, "mic_dev", None)
    check = dict(getattr(dev, "check", None) or {})
    holds = getattr(agent, "holds", None)
    wispr = getattr(agent, "wispr", None)
    return {"state": "checking" if getattr(holds, "checking", False) else check.get("state"),
            "detail": check.get("detail") or "", "carrier": getattr(wispr, "carrier", None),
            "hint": stick_mic_device.PERMISSION_HINT}


def check_wispr(agent):
    from . import stick_mic_device
    bh = installer.blackhole_installed()
    version = installer.blackhole_version() if bh else None
    cfg = config.load()
    on = bool(cfg.get("wireless_mic"))
    mic = getattr(getattr(agent, "mic_dev", None), "name", None) or "<stick>-mic-bluetooth"
    supported = stick_mic_device.macos_supports_tap()
    pref = cfg.get("wireless_mic_carrier") or "auto"
    data = {"blackhole": bh, "version": version, "pinned": installer.BLACKHOLE_PIN["version"],
            "on": on, "brew": bool(installer.brew_path()), "brew_cmd": installer.BLACKHOLE_CASK, "mic": mic,
            "tap_supported": supported, "pref": pref, "tap_mode": supported and pref != "blackhole",
            "tap": _tap_info(agent), "tap_note": installer.TAP_NOTE}
    if data["tap_mode"]:
        data["needs"] = {"permission": "System Audio Recording (macOS asks once)"}
        tap = data["tap"]
        if not on:
            return "todo", "no driver needed on this Mac — turn it on", data
        if tap["carrier"] == "tap":
            return "done", "on · carried by the Apple audio tap", data
        if tap["state"] in (None, "checking"):
            return "todo", "on · checking the Apple audio tap…", data
        if tap["carrier"] == "blackhole":  # auto, and the tap isn't allowed: BlackHole carries it meanwhile
            return "done", "on · carried by BlackHole 2ch (the Apple audio tap isn't allowed)", data
        if tap["state"] == "failed":
            return "todo", "on · allow System Audio Recording for the helper app", data
        return "todo", f"on · the Apple audio tap doesn't work here ({tap['detail']}) — use BlackHole instead", data
    pin = installer.BLACKHOLE_PIN["version"]
    found = f"BlackHole {version} found" + ("" if version in (pin, None) else f" (tested with {pin})") if bh else ""
    if bh and on:
        return "done", f"on · {found}", data
    if bh:
        return "todo", f"{found} · the wireless mic is off", data
    return "todo", "needs BlackHole 2ch (a free audio driver)", data


CHECKS = {"welcome": check_welcome, "model": check_model, "claude": check_claude, "hooks": check_hooks,
          "mcp": check_mcp, "github": check_github, "wispr": check_wispr}


async def check_stick(agent):
    stick_api = getattr(agent, "stick_api", None)
    stick = agent.cfg.get("stick")
    if stick_api is None:
        steps = {"picked": bool(stick)}
    else:
        steps = (await onboarding_state(agent, stick_api))["steps"]
    data = {"steps": steps, "stick": stick, "connected": bool(getattr(agent.link, "connected", False))}
    if steps.get("picked") and steps.get("firmware", True):
        return "done", f"using {stick['name'] if stick else 'your stick'}", data
    return "todo", ("pick your stick" if not stick else "update its firmware"), data


# --- running things (in a worker thread) ------------------------------------------------------

def stream_process(argv, emit, timeout=600, on_output=None, env=None, cancel=None):
    """Runs argv, sending its output to emit({"line"}) as it comes. on_output(text so far, proc) sees
    unfinished lines too (gh's "Press Enter…" has no newline). -> exit code (None if it timed out)."""
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            env=env, start_new_session=True)
    if cancel is not None:
        cancel.append(proc.terminate)
    expired = threading.Event()
    timer = threading.Timer(timeout, lambda: (expired.set(), proc.kill()))
    timer.start()
    text, pending = "", ""
    try:
        fd = proc.stdout.fileno()
        while True:
            chunk = os.read(fd, 4096)
            if not chunk:
                break
            s = chunk.decode(errors="replace")
            text += s
            pending += s
            if on_output:  # first: it may change what the lines show (the gh code is redacted)
                on_output(text, proc)
            *lines, pending = pending.replace("\r", "\n").split("\n")
            for line in lines:
                if line.strip():
                    emit({"line": line.rstrip()})
        if pending.strip():
            emit({"line": pending.rstrip()})
        code = proc.wait()
    finally:
        timer.cancel()
        try:
            proc.stdin.close()
        except OSError:
            pass
    return None if expired.is_set() else code


def agent_env():
    """PATH with the places Claude Code, gh and Homebrew install to (launchd's PATH is short)."""
    env = dict(os.environ)
    extra = [str(Path.home() / ".local/bin"), "/opt/homebrew/bin", "/usr/local/bin"]
    env["PATH"] = ":".join(extra + [p for p in env.get("PATH", "/usr/bin:/bin").split(":") if p not in extra])
    return env


def run_claude_install(ui, emit, cancel):
    if config.claude_path(config.load()):
        ui.say("Claude Code is already installed")
        return
    ui.say(f"running Anthropic's installer: {installer.CLAUDE_INSTALL}")
    code = stream_process(["/bin/sh", "-c", installer.CLAUDE_INSTALL], emit, timeout=900, env=agent_env(),
                          cancel=cancel)
    if code != 0:
        raise installer.NotAvailable(f"the installer stopped (exit {code}) — see the log above")
    if not config.claude_path(config.load()):
        raise installer.NotAvailable("the installer finished but `claude` isn't in ~/.local/bin")
    ui.say("Claude Code installed. Next: log in once in Terminal (run `claude`).")


def run_gh_login(ui, emit, cancel, flow):
    g = installer.GitHub()
    gh = g._gh()
    if not gh:
        raise installer.NotAvailable("install the GitHub CLI first: brew install gh")
    if g._logged_in(gh):
        ui.say("already logged in to GitHub")
        return
    shown = {"code": None, "entered": False}

    def on_output(text, proc):
        if "Press Enter" in text and not shown["entered"]:  # (a terminal-style gh): don't block on it
            shown["entered"] = True
            try:
                proc.stdin.write(b"\n")
                proc.stdin.flush()
            except OSError:
                pass
        code, url = installer.parse_device_code(text)
        if code and code != shown["code"]:
            shown["code"] = code
            flow.gh_login = {"code": code, "url": url, "job": flow.current_job_id("github")}
            emit({"code": code, "url": url})

    env = {**agent_env(), "GH_PROMPT_DISABLED": "1", "NO_COLOR": "1", "GH_NO_UPDATE_NOTIFIER": "1",
           "GH_BROWSER": "/usr/bin/true"}  # the page shows the code first, then opens the page itself
    try:
        code = stream_process(installer.gh_login_args(gh), lambda e: emit(redact_gh(e, shown["code"])),
                              timeout=900, on_output=on_output, env=env, cancel=cancel)
    finally:
        flow.gh_login = None
    if code != 0 or not g._logged_in(gh):
        raise installer.NotAvailable("GitHub login didn't finish — try again, or run `gh auth login` in Terminal")
    ui.say("logged in to GitHub ✓")


def redact_gh(e, code):
    """The one-time code is shown big on the page; it isn't repeated in the log lines."""
    line = e.get("line")
    if line and code and code in line:
        return {"line": line.replace(code, "••••-••••")}
    return e


def run_wispr(ui, _emit, _cancel, carrier=None):
    """carrier "tap": just on (the agent checks the tap afterwards); "blackhole": install BlackHole and
    use it; None: what this Mac does best."""
    from . import stick_mic_device
    cfg = config.load()
    if carrier == "blackhole" or (carrier is None and not installer.tap_wanted(cfg)):
        installer.install_blackhole(ui)
        if stick_mic_device.macos_supports_tap():
            cfg = config.load()
            cfg["wireless_mic_carrier"] = "blackhole"  # chose it over the tap
            config.save(cfg)
    elif carrier == "tap" and cfg.get("wireless_mic_carrier") == "blackhole":
        cfg["wireless_mic_carrier"] = "auto"
        config.save(cfg)
    installer.WirelessMic().install(ui)


# --- routes -----------------------------------------------------------------------------------

class Flow:
    def __init__(self):
        self.jobs = {}
        self.running = {}     # step -> job
        self.gh_login = None  # {"code", "url", "job"} while a gh login waits for the browser

    def current_job_id(self, step):
        job = self.running.get(step)
        return job.id if job and not job.done else None


def routes(agent):
    flow = Flow()

    async def one(sid, loop=None):
        loop = loop or asyncio.get_running_loop()
        s = BY_ID[sid]
        if sid == "stick":
            state, detail, data = await check_stick(agent)
        elif sid == "done":
            state, detail, data = "todo", "", {}
        elif sid == "github":
            state, detail, data = await loop.run_in_executor(None, check_github, agent, flow.gh_login)
        else:
            state, detail, data = await loop.run_in_executor(None, CHECKS[sid], agent)
        if state == "todo" and sid in skipped(agent.cfg):
            state = "skipped"
        needs = (data.pop("needs", None) if isinstance(data, dict) else None) or s.get("needs", {})
        return {"id": sid, "title": s["title"], "kind": s["kind"], "why": s["why"], "needs": needs,
                "skippable": sid in SKIPPABLE, "runnable": sid in RUNNABLE, "state": state, "detail": detail,
                "data": data, "job": flow.current_job_id(sid)}

    async def listing_data():
        steps = list(await asyncio.gather(*(one(s["id"]) for s in STEPS)))
        rest = [s for s in steps if s["id"] != "done"]
        done = steps[-1]
        done["state"] = "done" if all(s["state"] != "todo" for s in rest) else "todo"
        done["data"] = {"summary": {s["id"]: s["state"] for s in rest}}
        nxt = next((s["id"] for s in rest if s["state"] == "todo"), "done")
        pending = any(s["state"] == "todo" and s["kind"] in ("required", "recommended") for s in rest)
        return {"steps": steps, "next": nxt, "pending": pending, "skipped": skipped(agent.cfg)}

    def step_or_404(request):
        sid = request.match_info["step"]
        return sid if sid in BY_ID else None

    async def listing(_):
        return web.json_response(await listing_data())

    async def get_step(request):
        sid = step_or_404(request)
        if not sid:
            return web.json_response({"error": "no such step"}, status=404)
        if sid == "done":
            return web.json_response(next(s for s in (await listing_data())["steps"] if s["id"] == "done"))
        return web.json_response(await one(sid))

    async def skip(request):
        sid = step_or_404(request)
        if not sid:
            return web.json_response({"error": "no such step"}, status=404)
        if sid not in SKIPPABLE:
            return web.json_response({"error": f"{BY_ID[sid]['title']} can't be skipped"}, status=400)
        cur = skipped(agent.cfg)
        if request.match_info["action"] == "skip":
            new = cur if sid in cur else [*cur, sid]
        else:
            new = [s for s in cur if s != sid]
        agent.update_config({"install_skipped": new})
        return web.json_response(await listing_data())

    async def run(request):
        sid = step_or_404(request)
        if not sid:
            return web.json_response({"error": "no such step"}, status=404)
        if sid not in RUNNABLE:
            return web.json_response({"error": f"{BY_ID[sid]['title']} has nothing to run"}, status=400)
        if flow.current_job_id(sid):
            return web.json_response({"error": "already running", "job": flow.current_job_id(sid)}, status=409)
        try:
            body = await request.json() if request.can_read_body else {}
        except ValueError:
            body = {}
        carrier = (body or {}).get("carrier") if isinstance(body, dict) else None
        if carrier not in (None, "tap", "blackhole"):
            return web.json_response({"error": "carrier must be tap or blackhole"}, status=400)
        job = Job()
        flow.jobs[job.id] = job
        flow.running[sid] = job
        loop = asyncio.get_running_loop()
        stoppers = []
        job.cancel = lambda: [f() for f in stoppers]

        def emit(e):
            loop.call_soon_threadsafe(job.emit, e)

        def work():
            ui = installer.UI(interactive=False, emit=emit)
            try:
                if sid == "model":
                    installer.Model().install(ui)
                elif sid == "claude":
                    run_claude_install(ui, emit, stoppers)
                elif sid == "hooks":
                    installer.Claude().install(ui)
                elif sid == "mcp":
                    installer.Mcp().install(ui)
                elif sid == "github":
                    run_gh_login(ui, emit, stoppers, flow)
                elif sid == "wispr":
                    run_wispr(ui, emit, stoppers, carrier)
                return True, None
            except installer.NotAvailable as e:
                return False, str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("setup step %s failed", sid)
                return False, str(e)

        async def go():
            ok, err = await loop.run_in_executor(None, work)
            log.info("setup step %s: %s", sid, "ok" if ok else err)
            try:
                if sid == "wispr":  # on even when BlackHole is still missing (like the Components card)
                    saved = config.load()
                    agent.update_config({"wireless_mic": saved.get("wireless_mic"),
                                         "wireless_mic_carrier": saved.get("wireless_mic_carrier", "auto")})
                    if ok and getattr(agent, "_tap_wanted", lambda: False)():
                        job.emit({"line": "checking the Apple audio tap (inaudible) — "
                                          "if macOS asks about System Audio Recording, press Allow"})
                        res = await agent.check_tap(force=True)
                        if res and res["state"] == "ok":
                            job.emit({"line": "the Apple audio tap works ✓"})
                        elif res:
                            ok, err = False, res["detail"]
                if sid == "model" and ok:
                    await reload_speech(agent)
            except Exception as e:  # noqa: BLE001
                log.exception("applying %s failed", sid)
                ok, err = False, err or str(e)
            finally:
                job.emit({"done": True, "ok": ok, "error": err})
                job.done = True

        asyncio.ensure_future(go())
        return web.json_response({"job": job.id})

    async def set_repo(request):
        try:
            body = await request.json()
        except ValueError:
            body = {}
        repo = str(body.get("repo") or "").strip()
        if not installer.valid_repo(repo):
            return web.json_response({"error": "use the form owner/name, e.g. acme/web"}, status=400)
        alerts = body.get("alerts")
        if alerts is not None and not isinstance(alerts, bool):
            return web.json_response({"error": "alerts must be true, false or null"}, status=400)
        g = await asyncio.get_running_loop().run_in_executor(None, installer.set_primary_repo, repo, alerts)
        agent.update_config({"github": g})
        return web.json_response({"ok": True, "github": g})

    async def job_events(request):
        job = flow.jobs.get(request.match_info["job"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
        return await stream_job(request, job)

    async def job_cancel(request):
        job = flow.jobs.get(request.match_info["job"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
        if job.done:
            return web.json_response({"error": "it already finished"}, status=409)
        job.cancel()
        return web.json_response({"ok": True})

    return [
        web.get("/api/install", listing),
        web.post("/api/install/github/repo", set_repo),
        web.get("/api/install/jobs/{job}", job_events),
        web.post("/api/install/jobs/{job}/cancel", job_cancel),
        web.get("/api/install/{step}", get_step),
        web.post("/api/install/{step}/{action:skip|unskip}", skip),
        web.post("/api/install/{step}/run", run),
    ]


async def reload_speech(agent):
    """After the setup page downloaded the model: load it if the agent's own start-up attempt failed
    (e.g. it started offline). A start-up load still in progress finishes by itself."""
    if getattr(agent, "stt_ready", True) or getattr(agent, "state", "") != "error":
        return
    from . import stt
    loop = asyncio.get_running_loop()
    agent.set_state("starting", "loading speech model")
    try:
        await loop.run_in_executor(agent.stt.thread, agent.stt.load, agent.cfg.get("stt_model") or stt.MODEL)
        agent.stt_ready = True
        agent.set_state("ready" if agent.link_state == "connected" else agent.link_state)
    except Exception as e:  # noqa: BLE001
        log.exception("speech model failed")
        agent.set_state("error", f"speech model failed: {e}")


def page_url(step=""):
    return f"http://localhost:{config.PORT}/#install" + (f"/{step}" if step else "")


def open_page():
    """`stick-agent install-page`: opens the setup page (the agent must be running). -> exit code"""
    import json
    import urllib.request
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{config.PORT}/api/status", timeout=3) as r:
            json.load(r)
    except (OSError, ValueError):
        print("the agent isn't running — start it with: stick-agent start   (then run this again)")
        return 1
    url = page_url()
    r = subprocess.run(["open", url], capture_output=True, text=True)
    if r.returncode != 0:
        print(f"couldn't open a browser here — open {url} on this Mac")
        return 1
    print(f"opened {url}")
    return 0
