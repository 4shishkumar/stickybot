#!/bin/zsh
# Sets up the Python environment for the Claude Code stick hook and prints
# the snippet to add to ~/.claude/settings.json. Doesn't edit your settings.
set -euo pipefail

HERE="${0:A:h}"
ENV="$HOME/.claude-stick/venv"
PY=$(command -v python3.14 || command -v python3.13 || command -v python3.12 || command -v python3)

echo "Creating $ENV with $PY..."
"$PY" -m venv "$ENV"
"$ENV/bin/pip" install -q --upgrade pip bleak

cat <<EOF

Done. Add this to ~/.claude/settings.json (merge into "hooks" if you have one):

{
  "hooks": {
    "PermissionRequest": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "$ENV/bin/python $HERE/stick_hook.py",
            "timeout": 45,
            "statusMessage": "Waiting for the stick…"
          }
        ]
      }
    ]
  }
}

Test without Claude Code:
  echo '{"tool_name":"Bash","tool_input":{"command":"ls"},"cwd":"$PWD"}' | $ENV/bin/python $HERE/stick_hook.py
EOF
