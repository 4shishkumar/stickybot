#!/bin/sh
# Creates (or updates) the repo's pull request labels (docs/CONTRIBUTING.md). Needs `gh` logged in
# with write access: tools/setup_labels.sh [owner/repo]
set -eu
REPO="${1:-4shishkumar/stickybot}"
label() { gh label create "$1" --repo "$REPO" --color "$2" --description "$3" --force; }
label stick          "0e8a16" "The stick's firmware or screens"
label mac-app        "1d76db" "The Mac app (desktop/)"
label dashboard      "5319e7" "The dashboard pages"
label firmware       "006b75" "Firmware build, flashing, OTA"
label release        "fbca04" "A release PR"
label bug            "d73a4a" "Something doesn't work"
label feature        "a2eeef" "New behaviour"
label no-changelog   "cfd3d7" "Docs, CI or tests only: no CHANGELOG.md line needed"
label needs-hardware "e99695" "Test on a real stick before merging"
