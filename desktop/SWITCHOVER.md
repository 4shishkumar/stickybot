# Switching from StickyBot (Python) to StickyBot (Rust)

Until now the two apps ran side by side: the Rust app on port **8670** with its own folder
(`~/.stickybot-rs/`), token and bundle id (`com.stickybot.rs`), taking the stick only while the
Python app has paused it. The switch-over makes the Rust app *the* StickyBot: it takes the Python
app's port (**8660**) and token, so everything that talks to StickyBot keeps working without
changes:

- the Claude Code hooks the Python installer registered in `~/.claude/settings.json` (approvals
  through `hook.py`, "Claude finished" and the session list through `claude_hooks.py`): they post to
  `127.0.0.1:8660` with `~/.stick-agent/token`;
- terminal sessions' stick tools (`claude mcp add … http://127.0.0.1:8660/mcp` with that token);
- dashboard bookmarks (`http://localhost:8660`).

Nothing in `~/.stick-agent/` is changed, moved or deleted by the Rust app at any step: it only
reads. Going back is always possible (see the end).

**A Mac without the Python app needs none of this.** On its first start (no `config.json` in
`~/.stickybot-rs/` yet) the Rust app looks for the Python app: its folder `~/.stick-agent/`
(`$STICKYBOT_RS_IMPORT_FROM`) or anything answering on port 8660 (`$STICKYBOT_RS_PYTHON_PORT`).
When there's neither, it is StickyBot right away: it writes `~/.stickybot-rs/switched-over` with
the note "fresh install: no StickyBot (Python) app on this Mac", serves port 8660 as "StickyBot"
with its own token, and has no "the Python agent has the stick" check; Settings shows no
*Switch over* card (`GET /api/switch-over` says `"fresh": true`). This is decided once, on the
first start (never with the fake stick): when the Python app is there, everything below applies
unchanged, and deleting the marker later (or *Undo*) puts the app on 8670 for good.
(`switch::adopt_if_fresh`, `Options::start_port`.)

## Decisions

- **Bundle id stays `com.stickybot.rs`.** Taking over `com.stickybot.app` would not carry the Python
  app's macOS permissions over (TCC checks the code signature as well, and the signatures differ), it
  would make two different apps look like one to macOS while both are installed, and there's no
  LaunchAgent label to inherit (the Rust app's login item is `SMAppService`). Port + token are what
  give continuity.
- **Signing**: one stable code-signing identity from the login keychain for every build, so the
  Bluetooth and System Audio Recording permissions survive rebuilds:
  `STICKYBOT_SIGNING_IDENTITY=<SHA-1> desktop/scripts/build-app.sh` (it sets Tauri's
  `APPLE_SIGNING_IDENTITY`; `security find-identity -v -p codesigning` lists the identities). The
  repo's `tauri.conf.json` stays ad hoc, so anyone can build without that certificate.
- **Start at login**: `SMAppService.mainAppService` (Settings → *Start at login*), no LaunchAgent and
  no `launchctl`.

## What keeps the Python app alive (and how to stop it without uninstalling)

The Python app is a **LaunchAgent**, `com.stickkey.agent`
(`~/Library/LaunchAgents/com.stickkey.agent.plist`, or `~/.stick-agent/` when that folder wasn't
writable): `RunAtLoad` true and `KeepAlive {SuccessfulExit: false}`. So:

- its menu bar **Quit** exits 0: launchd doesn't restart it, **but it starts again at the next
  login** and takes port 8660 first;
- `stickybot stop` is `launchctl bootout` only: same, back at the next login;
- `stickybot start` / `restart` / `app rebuild`, its installer, and **opening the Python
  `StickyBot.app` from Finder / Spotlight** write the plist again and `bootstrap` / `kickstart` it;
- its dashboard has **no** start-at-login switch (Settings → Components only shows "this is what's
  running").

If both apps start at login, they race for 8660: the Rust app waits 10 s for the port and then
quits (`app.log`: "can't listen on 127.0.0.1:8660 … is StickyBot (Python) running?"); if the Rust
app wins, the Python app's dashboard dies in its thread and leaves a dead icon in the menu bar.

**`launchctl disable`** is the one that sticks: it's kept by launchd across logins, `bootstrap`
refuses a disabled service (so `stickybot start` and opening the Python app can't bring it back by
accident), nothing is deleted, and `launchctl enable` undoes it. Moving the plist away also works
but is undone by anything that writes it again. The GUI equivalent is System Settings → General →
Login Items → *Allow in the Background* → **StickyBot** off.

## Steps (in this order)

Nothing here changes `~/.stick-agent/`, the Keychain, Wispr Flow or `~/.claude/settings.json`
(except the optional step 9, which asks first and keeps a backup).

1. **Install** the app in `/Applications` (the hooks of step 9 point at this path; don't move it
   afterwards). Either download it: `StickyBot-<version>-macos-arm64.zip` from the
   [latest release](https://github.com/4shishkumar/stickybot/releases/latest), as in
   [Install the Mac app](../README.md#install-the-mac-app) (signed ad hoc: the first open needs
   right-click → Open, and each new download asks for the permissions again). Or build it signed
   with your own identity, so the permissions survive updates:
   `STICKYBOT_SIGNING_IDENTITY=<SHA-1> desktop/scripts/build-app.sh`, then copy
   `desktop/target/release/bundle/macos/StickyBot.app` to `/Applications`. (The Python app's bundle
   is `~/Applications/StickyBot.app`, another folder: both are called StickyBot in Spotlight; the
   Rust one is the one in `/Applications`.)
2. **Import the settings** with both apps running (the Rust app on 8670, holding off the stick):
   Rust menu bar → *Open StickyBot (Rust)* → Settings → *Import from StickyBot* (`POST /api/import`).
   It takes the stick, GitHub (primary repo), brain / model, wireless mic, sounds, events and every
   other shared setting, the voice commands, sound packs, the GitHub watcher's state and Claude's
   `CLAUDE.md` (this app's replaced files are kept as `*.before-import`), and **merges** the request
   history, the events log and the pet with this app's own: the histories line up by time with no
   request twice, and the pet's XP, PRs, builds, Claude tasks, releases and game plays add up (see
   *Importing* in [README.md](README.md)). On a first start with an empty `~/.stickybot-rs/` this
   happens on its own. Importing again is safe: the pet only gets what the Python app earned since
   the last import, and the history only the new requests. Do it once more right after step 3, so
   what the Python app did meanwhile comes over too (settings and commands: the Python app's win
   again, so do it before you change them here).
3. **Stop the Python app and keep it from coming back** (it stays installed):
   1. Python menu bar icon → **Quit StickyBot** (exit 0: launchd leaves it stopped).
   2. In a terminal:
      ```sh
      launchctl disable gui/$(id -u)/com.stickkey.agent     # not at the next login, not by `stickybot start`
      launchctl bootout gui/$(id -u)/com.stickkey.agent     # unload it now ("No such process" is fine)
      ```
   3. Check: `launchctl print-disabled gui/$(id -u) | grep stickkey` says `=> disabled` (or `true`);
      `curl -s -m 2 localhost:8660/api/status` gets no answer.
   From here on don't run `stickybot start|restart|app rebuild|setup|uninstall` (the last one also
   removes the `stick` MCP registration and the Claude Code hooks both apps use).
4. **Switch over**: Rust dashboard → Settings → *Switch over* (or
   `curl -X POST localhost:8670/api/switch-over -H "X-Stick-Token: $(cat ~/.stickybot-rs/token)" -d '{"confirm":true}'`).
   It refuses while anything still answers on 8660, and the card warns while the Python
   LaunchAgent's plist is installed (it can't see whether it's disabled: step 3.3 is the check). It
   copies the Python app's token into `~/.stickybot-rs/token` (the old one is kept as
   `token.before-switch`) and writes `~/.stickybot-rs/switched-over`.
5. **Restart the Rust app**: its menu → **Restart**. It now serves <http://localhost:8660>, the menu
   and the window say **StickyBot** (no "(Rust)"), and the "Python agent has the stick" check is
   off. macOS may ask for Bluetooth once more if this is the first signed build.
6. **Start at login**: Settings → *Start at login* on. If it says "needs approval", allow
   StickyBot in System Settings → General → Login Items.
7. **Check**: the stick connects (*agent ready*; the menu says "Ready — hold B on the stick" with
   the eyes icon); hold B and ask something (the icon fills while listening); run a Claude Code tool
   that needs approval in a terminal (the icon gets its dot, the menu says "Waiting for approval
   (1)"; A / B on the stick); a long Claude Code task ends with "Claude finished";
   `claude mcp get stick` still points at `127.0.0.1:8660/mcp`; Stick volume / Sounds on the stick
   in the menu; Settings → Wireless mic (the Rust app's mic has the same name, so Wispr's ranking
   needs no change).
8. **Log out and in** once: only the Rust app comes up (one icon), on 8660.
9. *(Optional, asks before it changes `~/.claude/settings.json`; keeps a dated backup)* Setup →
   **Claude Code hooks**: replaces the Python hooks (`<venv>/python …/hook.py`,
   `…/claude_hooks.py`) with this app's (`/Applications/StickyBot.app/Contents/MacOS/stickybot-rs --hook …`).
   Until then the Python hooks keep working (they post to 8660 with the same token), but they
   need the Python venv: don't delete `mac-agent/.venv` (or wherever the installer put it) before
   this step.

## Going back

1. Rust dashboard → Settings → Switch over → **Undo** (`POST /api/switch-over {"undo": true}`, or
   delete `~/.stickybot-rs/switched-over`), and Settings → *Start at login* off.
2. Rust menu → **Quit StickyBot**.
3. ```sh
   launchctl enable gui/$(id -u)/com.stickkey.agent
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.stickkey.agent.plist   # or: stickybot start
   ```
4. If you did step 9, Setup → Claude Code hooks in the Python app puts its hooks back (or restore
   the backup next to `~/.claude/settings.json`).

The Rust app goes back to port 8670 and waits for the Python app to pause before it takes the
stick. The token stays the Python one (harmless; the old Rust token is in `token.before-switch`).

## Parity beyond the routes (after the switch-over)

| Python | Rust | Matters for the switch-over? |
|---|---|---|
| menu bar menu and state icon (menubar.py) | the same items, icon states and refresh (`tray.rs`, `icon.rs`) | done |
| `confirm()` / `prompt()` in its window (NSAlert; `prompt` never worked) | WKWebView has no panels in Tauri: the pages use `lib.js` `ask()` (in-page) | done (IR deletes / renames, Import, Switch over) |
| links / `target=_blank` open in the browser | the same (`window.rs`) | done |
| reopening the app shows the window | the same (`RunEvent::Reopen`) | done |
| LaunchAgent with `KeepAlive` (restarted after a crash) | `SMAppService` login item: started at login, **not** restarted after a crash | no; reopen it from /Applications |
| `stickybot` CLI (run/start/stop/restart/status/logs/open/install-page/app/flash/setup/mcp/uninstall) | none: the dashboard does status / logs / setup / flashing / MCP; `--hook` only | no |
| self-update (git + venv + rebuild) | **Install update vX and restart…** (menu) / *Install and restart* (dashboard): downloads the signed `StickyBot.app.tar.gz`, checks it against the key built into the app, swaps the bundle and reopens (`selfupdate.rs`, `updater.rs`); waits while the stick is busy; where it can't replace itself, or for a release without `latest.json`, it opens the release page. Releases up to 2.5.0 can't: install the first one with it by hand once | done |
| BlackHole: downloads and installs the pkg | shows `brew install --cask blackhole-2ch` (the Apple audio tap needs no driver) | no |
| window remembers its size and place; Dock icon while it's open | neither (menu bar only) | no |
| first start opens the setup page by itself | the menu's *Finish setup…* | no |
| notifications | toasts on the stick only, in both | — |
| hooks, MCP tools (12), stick USB state, Wispr warm-up, `sound get` / `@state` name on connect | at parity | — |

## Not covered yet

- See the table: crash restart, a CLI, self-update and the BlackHole download aren't ported.
- OpenRouter: the key is the same Keychain item the Python app saved (macOS asks once to let this
  app read it).
