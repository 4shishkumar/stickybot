#!/bin/bash
# Builds "StickyBot (Rust).app", signed with a stable identity so macOS keeps its permissions
# (Bluetooth, System Audio Recording, the login item) across rebuilds.
#
#   STICKYBOT_SIGNING_IDENTITY=<SHA-1 or name> desktop/scripts/build-app.sh
#
# The identity is a code-signing certificate in your login keychain (a Developer ID, or a
# self-signed "Code Signing" certificate made in Keychain Access → Certificate Assistant). List
# yours with `security find-identity -v -p codesigning`. Without one the app is signed ad hoc:
# it works, but every rebuild is a new identity and macOS asks for its permissions again.
# Tauri reads APPLE_SIGNING_IDENTITY (it overrides bundle.macOS.signingIdentity).
set -euo pipefail
cd "$(dirname "$0")/../crates/stickybot-app"

if [ -n "${STICKYBOT_SIGNING_IDENTITY:-}" ]; then
  export APPLE_SIGNING_IDENTITY="$STICKYBOT_SIGNING_IDENTITY"
  echo "signing with $APPLE_SIGNING_IDENTITY"
else
  echo "STICKYBOT_SIGNING_IDENTITY not set: ad-hoc signature (macOS will ask for permissions again)" >&2
fi

cargo tauri build --bundles app
APP="$(cd ../.. && pwd)/target/release/bundle/macos/StickyBot (Rust).app"
codesign --verify --strict --verbose=1 "$APP"
codesign -dv "$APP" 2>&1 | grep -E "^(Identifier|Authority|TeamIdentifier|Signature)" || true
echo "built: $APP"
