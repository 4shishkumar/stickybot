#!/bin/zsh
# Opens the settings page for use over USB (no Wi-Fi). Chrome/Edge only —
# Web Serial needs a localhost page, so this serves web/ locally.
cd "${0:A:h}/../web"
PORT=8642
URL="http://localhost:$PORT/settings.html"
echo "USB settings page: $URL  (Ctrl+C to stop)"
# Web Serial works in Chromium browsers (Chrome, Edge, Brave, Arc), not Safari/Firefox.
( sleep 1
  for app in "Google Chrome" "Microsoft Edge" "Brave Browser" "Arc"; do
    open -a "$app" "$URL" 2>/dev/null && exit
  done
  echo "Open $URL in Chrome, Edge or Brave." ) &
exec python3 -m http.server $PORT --bind 127.0.0.1
