#!/bin/zsh
# Save a full copy of the StickS3's 8 MB flash (bootloader, partition table,
# firmware, settings) to backup/, then verify it against the device.
#
# Usage: tools/backup.sh [port]     (default: first /dev/cu.usbmodem*)
set -euo pipefail

cd "${0:A:h}/.."
ESPTOOL=(~/.platformio/penv/bin/python -m esptool)
PORT=${1:-$(ls /dev/cu.usbmodem* 2>/dev/null | head -1)}
[[ -n "$PORT" ]] || { echo "No /dev/cu.usbmodem* port. Plug in the stick (or hold the side button for download mode)."; exit 1; }

mkdir -p backup
OUT="backup/sticks3-factory-$(date +%Y%m%d-%H%M%S).bin"

echo "== Chip info ($PORT)"
"${ESPTOOL[@]}" --chip esp32s3 --port "$PORT" --after no-reset flash-id | tee "${OUT%.bin}.info.txt"

echo "== Reading 8 MB flash -> $OUT (about 1-2 min)"
"${ESPTOOL[@]}" --chip esp32s3 --port "$PORT" --baud 921600 --after no-reset read-flash 0 0x800000 "$OUT"

echo "== Verifying backup against the device"
"${ESPTOOL[@]}" --chip esp32s3 --port "$PORT" --baud 921600 verify-flash 0 "$OUT"

shasum -a 256 "$OUT" | tee -a "${OUT%.bin}.info.txt"
echo "\nBackup OK: $OUT"
echo "The stick stays in download mode (green LED flashing) — press the side power/reset button once to restart it."
echo "Keep the backup/ folder somewhere safe (it's specific to this stick)."
