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
#
# In-app updates: with the update key in TAURI_SIGNING_PRIVATE_KEY (the key itself or a path to
# it; its password in TAURI_SIGNING_PRIVATE_KEY_PASSWORD) the build also makes the signed update,
# StickyBot.app.tar.gz and StickyBot.app.tar.gz.sig next to the .app, and fails if either is
# missing. Without the key (a local or pull request build) it builds as before and says it
# skipped them. The key is the release owner's (docs/RELEASING.md, "Update signing key"): never
# put it in the repo.
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
# asking Tauri for updater artifacts without a key fails the build, so only ask when there is one
# (tauri.conf.json does not set bundle.createUpdaterArtifacts for that reason)
updater=no
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  args+=(--config '{"bundle": {"createUpdaterArtifacts": true}}')
  updater=yes
else
  echo "no update key: updater artifacts skipped (this build can't be installed from inside the app)" >&2
fi
cargo tauri build "${args[@]}"
BUNDLE="$(cd ../.. && pwd)/target/release/bundle/macos"
APP="$BUNDLE/StickyBot.app"
if [ "$updater" = yes ]; then
  for f in "$BUNDLE/StickyBot.app.tar.gz" "$BUNDLE/StickyBot.app.tar.gz.sig"; do
    if [ ! -s "$f" ]; then
      echo "error: $f was not made (is TAURI_SIGNING_PRIVATE_KEY a valid key, and its password in TAURI_SIGNING_PRIVATE_KEY_PASSWORD?)" >&2
      exit 1
    fi
  done
fi
codesign --verify --deep --strict --verbose=1 "$APP"
codesign -dv "$APP" 2>&1 | grep -E "^(Identifier|Authority|TeamIdentifier|Signature|CodeDirectory)" || true

# The app is signed with the hardened runtime, under which macOS refuses EventKit (the Calendar
# connection) unless the calendars entitlement is signed in (entitlements.plist, applied through
# bundle.macOS.entitlements in tauri.conf.json). Without it the permission question never appears
# and Calendar quietly reads nothing, so a build that lost it fails here, loudly.
ENTITLEMENTS="$(codesign -d --entitlements :- "$APP" 2>/dev/null || true)"
if ! grep -q "com.apple.security.personal-information.calendars" <<<"$ENTITLEMENTS"; then
  echo "error: $APP is not signed with the calendars entitlement" >&2
  echo "  (bundle.macOS.entitlements in crates/stickybot-app/tauri.conf.json -> entitlements.plist)" >&2
  exit 1
fi
echo "signed with the calendars entitlement"
echo "built: $APP"
if [ "$updater" = yes ]; then
  echo "update files: $BUNDLE/StickyBot.app.tar.gz and .sig"
fi
