#!/bin/bash
# Regenerates the STT test WAVs (16 kHz, mono, 16-bit PCM) from texts.tsv with macOS `say`.
# Output: desktop/spikes/stt-testset/wav/NN.wav (git-ignored). The noise is seeded, so reruns match.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/wav"
mkdir -p "$out"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
grep -v '^#' "$here/texts.tsv" | while IFS=$'\t' read -r id voice snr text; do
  say -v "$voice" -o "$tmp/$id.aiff" "$text"
  afconvert -f WAVE -d LEI16@16000 -c 1 "$tmp/$id.aiff" "$out/$id.wav"
  if [ "$snr" != "-" ]; then
    /usr/bin/python3 "$here/add_noise.py" "$out/$id.wav" "$snr" "$id"
  fi
  echo "$id ($voice, noise $snr): $text"
done
