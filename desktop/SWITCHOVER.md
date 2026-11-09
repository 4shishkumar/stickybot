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

## Steps

1. **Build and install** the signed app: `STICKYBOT_SIGNING_IDENTITY=<SHA-1> desktop/scripts/build-app.sh`,
   then copy `desktop/target/release/bundle/macos/StickyBot (Rust).app` to `/Applications`.
2. **Import the settings** (if the Rust app ran before, its settings are its own): Rust dashboard →
   Settings → *Import from StickyBot* (`POST /api/import`). It takes the stick, GitHub (primary
   repo), brain / model, wireless mic, sounds, events and every other shared setting, the voice
   commands, sound packs, history, the pet, the GitHub watcher's state and Claude's `CLAUDE.md`;
   this app's replaced files are kept as `*.before-import`. On a first start with an empty
   `~/.stickybot-rs/` this happens on its own.
3. **Quit the Python app** (its menu bar icon → Quit). If it's set to start at login, turn that off
   in its dashboard (Settings → Components) or System Settings → General → Login Items, so it
   doesn't take port 8660 back at the next login. (Don't uninstall it yet.)
4. **Switch over**: Rust dashboard → Settings → *Switch over* (or
   `curl -X POST localhost:8670/api/switch-over -H "X-Stick-Token: $(cat ~/.stickybot-rs/token)" -d '{"confirm":true}'`).
   It refuses while anything still answers on 8660. It copies the Python app's token into
   `~/.stickybot-rs/token` (the old one is kept as `token.before-switch`) and writes
   `~/.stickybot-rs/switched-over`.
5. **Restart the Rust app** (menu bar → Quit, open it again). It now serves
   <http://localhost:8660>, and the "Python agent has the stick" check is off (it would ask itself).
   macOS may ask for Bluetooth once more if this is the first signed build.
6. **Start at login**: Settings → *Start at login* on. If it says "needs approval", allow
   StickyBot (Rust) in System Settings → General → Login Items.
7. **Check**: the stick connects (*agent ready*); hold B and ask something; run a Claude Code tool
   that needs approval in a terminal (A / B on the stick); a long Claude Code task ends with
   "Claude finished"; `claude mcp get stick` still points at `127.0.0.1:8660/mcp`; Settings →
   Wireless mic (the Rust app's mic has the same name, so Wispr's ranking needs no change).

## Going back

Rust dashboard → `POST /api/switch-over {"undo": true}` (or delete `~/.stickybot-rs/switched-over`),
quit the Rust app, start the Python app. The Rust app goes back to port 8670 and waits for the
Python app to pause again before it takes the stick. The token stays the Python one (harmless; the
old Rust token is in `token.before-switch`).

## Not covered yet

- The Python installer's venv, `stickybot` command, LaunchAgent and BlackHole download aren't ported
  (the Rust app has none of them; BlackHole only if you installed it with Homebrew). The Claude Code
  hooks the Python installer registered keep working through port 8660; to use the Rust app's own
  (`<app> --hook …`, no Python), run Setup → Claude Code hooks after step 5 (it keeps a backup of
  `~/.claude/settings.json`).
- Self-update: the Rust app is built from the repo (`POST /api/update/agent` says so); firmware
  updates (Bluetooth, USB) are ported.
- OpenRouter: the key is the same Keychain item the Python app saved (macOS asks once to let this app read it).
