#!/bin/zsh
# Write a full-flash backup (from tools/backup.sh) back to the StickS3,
# returning it exactly to the state it was in when backed up.
#
# Usage: tools/restore.sh backup/sticks3-factory-XXXX.bin [port]
set -euo pipefail

cd "${0:A:h}/.."
ESPTOOL=(~/.platformio/penv/bin/python -m esptool)
FILE=${1:-}
[[ -f "$FILE" ]] || { echo "Usage: tools/restore.sh backup/<file>.bin [port]"; ls backup/*.bin 2>/dev/null; exit 1; }
PORT=${2:-$(ls /dev/cu.usbmodem* 2>/dev/null | head -1)}
[[ -n "$PORT" ]] || { echo "No /dev/cu.usbmodem* port. Hold the side button until the green LED flashes, then retry."; exit 1; }

SIZE=$(stat -f %z "$FILE")
[[ "$SIZE" -eq 8388608 ]] || { echo "$FILE is $SIZE bytes, expected 8388608 (8 MB). Not a full backup?"; exit 1; }

echo "This overwrites everything on the stick at $PORT with $FILE."
read -q "?Continue? [y/N] " || { echo; exit 1; }
echo

"${ESPTOOL[@]}" --chip esp32s3 --port "$PORT" --baud 921600 --after no-reset write-flash 0 "$FILE"
"${ESPTOOL[@]}" --chip esp32s3 --port "$PORT" --baud 921600 verify-flash 0 "$FILE"
echo "\nRestored. Press the side power/reset button once to start it."
