# Claude Code hook — approve on the stick

Sends Claude Code permission prompts (terminal and IDE sessions) to the
M5StickS3 over Bluetooth. The stick shows **"Terminal: <project> — <tool> —
<command/file>"**; **A** approves, **B** denies.

- Stick not found within ~4 s, or no button within 25 s → the hook prints
  nothing and Claude Code shows its normal terminal prompt. It never blocks you.
- Needs the stick's **Claude buddy** setting on (it uses the buddy Bluetooth
  service) and the stick paired with this Mac.
- The Claude **desktop app** doesn't need this hook — it talks to the stick
  directly (Developer → Open Hardware Buddy).

## Install

```sh
tools/claude-hook/install.sh
```

Creates `~/.claude-stick/venv` (Python + `bleak`) and prints the snippet for
`~/.claude/settings.json`:

```json
"hooks": {
  "PermissionRequest": [
    { "hooks": [ { "type": "command",
        "command": "~/.claude-stick/venv/bin/python <repo>/m5-stick-s3/tools/claude-hook/stick_hook.py",
        "timeout": 45, "statusMessage": "Waiting for the stick…" } ] }
  ]
}
```

Merge it into any existing `hooks`. Open `/hooks` in Claude Code (or restart
it) to load the change. macOS asks once whether your terminal may use
Bluetooth — allow it.

## Test without Claude Code

```sh
echo '{"tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"'$PWD'"}' \
  | ~/.claude-stick/venv/bin/python tools/claude-hook/stick_hook.py
```

Prints the decision JSON after A/B, or nothing after the 25 s timeout.

## Tuning

Environment variables (set them in the hook's `command`):

| Variable | Default | Meaning |
|---|---|---|
| `STICK_FIND_SECONDS` | 4 | how long to look for the stick |
| `STICK_WAIT_SECONDS` | 25 | how long to wait for a button |

Log: `~/.claude-stick/hook.log`. Remove the `PermissionRequest` entry from
`~/.claude/settings.json` (or use `/hooks`) to turn it off.
