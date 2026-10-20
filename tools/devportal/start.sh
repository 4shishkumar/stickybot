#!/bin/zsh
# Starts the dev portal on http://localhost:8650 (Ctrl+C to stop).
# First run creates its Python environment (pyserial + zeroconf).
set -euo pipefail
HERE="${0:A:h}"
ENV="$HOME/.macro-devportal/venv"
if [[ ! -x "$ENV/bin/python" ]]; then
  PY=$(command -v python3.14 || command -v python3.13 || command -v python3)
  "$PY" -m venv "$ENV"
  "$ENV/bin/pip" install -q --upgrade pip pyserial zeroconf
fi
( sleep 1; open "http://localhost:${PORTAL_PORT:-8650}" ) &
exec "$ENV/bin/python" "$HERE/server.py"
