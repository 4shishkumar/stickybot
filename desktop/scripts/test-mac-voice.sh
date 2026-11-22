#!/bin/sh
# Run from a logged-in macOS GUI session. No shortcut or microphone permission is required.
set -eu
binary=$(mktemp -t stickybot-voice-test)
trap 'rm -f "$binary"' EXIT
source_dir=$(dirname "$0")/../crates/stickybot-app/tests
xcrun clang -mmacosx-version-min=13.0 -fobjc-arc -fblocks -Wall -Wextra -Werror -Wno-unused-parameter \
    "$source_dir/mac_voice_test.m" -framework AppKit -framework QuartzCore \
    -framework Carbon -framework AVFoundation -framework CoreAudio -o "$binary"
"$binary"
