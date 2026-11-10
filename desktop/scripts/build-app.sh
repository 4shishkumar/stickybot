#!/bin/bash
# Builds "StickyBot.app" (desktop/target/release/bundle/macos/StickyBot.app).
#
#   desktop/scripts/build-app.sh                                              ad-hoc signature
#   STICKYBOT_SIGNING_IDENTITY=<SHA-1 or name> desktop/scripts/build-app.sh    a stable identity
#   STICKYBOT_VERSION=2.4.0 desktop/scripts/build-app.sh      the bundle's version (default: tauri.conf.json)
#
# A stable identity makes macOS keep the app's permissions (Bluetooth, System Audio Recording,
# the login item) across rebuilds. It is a code-signing certificate in your login keychain (a
# Developer ID, or a self-signed "Code Signing" certificate made in Keychain Access → Certificate
# Assistant); list yours with `security find-identity -v -p codesigning`. Without one the app is
# signed ad hoc (`codesign -s -`): it works, but every rebuild is a new identity and macOS asks
# for its permissions again. The release workflow builds it ad hoc (.github/workflows/release.yml).
# Tauri reads APPLE_SIGNING_IDENTITY (it overrides bundle.macOS.signingIdentity); "-" is ad hoc.
set -euo pipefail
cd "$(dirname "$0")/../crates/stickybot-app"

if [ -n "${STICKYBOT_SIGNING_IDENTITY:-}" ]; then
  export APPLE_SIGNING_IDENTITY="$STICKYBOT_SIGNING_IDENTITY"
fi
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && [ "$APPLE_SIGNING_IDENTITY" != "-" ]; then
  echo "signing with $APPLE_SIGNING_IDENTITY"
else
  export APPLE_SIGNING_IDENTITY="-"
  echo "no signing identity: ad-hoc signature (macOS will ask for permissions again)" >&2
fi

args=(--bundles app)
if [ -n "${STICKYBOT_VERSION:-}" ]; then
  args+=(--config "{\"version\": \"$STICKYBOT_VERSION\"}")
fi
cargo tauri build "${args[@]}"
APP="$(cd ../.. && pwd)/target/release/bundle/macos/StickyBot.app"
codesign --verify --deep --strict --verbose=1 "$APP"
codesign -dv "$APP" 2>&1 | grep -E "^(Identifier|Authority|TeamIdentifier|Signature|CodeDirectory)" || true
echo "built: $APP"
