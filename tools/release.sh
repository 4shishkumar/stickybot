#!/bin/sh
# Cut a StickyBot release LOCALLY: bump the version, date its CHANGELOG entry,
# commit and tag. It never pushes — it prints the push command, and pushing the
# tag is what starts the release workflow (.github/workflows/release.yml).
#
#   tools/release.sh 1.8.0              a stable release (from main)
#   tools/release.sh 1.8.0-beta.1       a beta (published as a GitHub prerelease)
#   tools/release.sh --dry-run 1.8.0    say what would happen, change nothing
#
# Before: describe the changes under "## [Unreleased]" in CHANGELOG.md (or add a
# "## [X.Y.Z]" entry), and bump FW_VERSION in m5-stick-s3/src/config.h if the
# firmware changed. Details: docs/RELEASING.md.

set -eu

cd "$(dirname "$0")/.."
PY="${PYTHON:-python3}"
TOOL="tools/release_tool.py"

DRY=0
VERSION=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "unknown option $arg" >&2; exit 2 ;;
    *) VERSION="${arg#v}" ;;
  esac
done
[ -n "$VERSION" ] || { echo "usage: tools/release.sh [--dry-run] X.Y.Z[-beta.N]" >&2; exit 2; }
"$PY" "$TOOL" check "$VERSION"
TAG="v$VERSION"
die() { echo "error: $*" >&2; exit 1; }

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists"
if ! git diff --quiet || ! git diff --cached --quiet; then die "uncommitted changes — commit or stash them first"; fi
BRANCH="$(git symbolic-ref --short -q HEAD || echo '(detached)')"
case "$VERSION" in
  *-*) KIND="beta (prerelease)" ;;
  *) KIND="stable"; [ "$BRANCH" = main ] || echo "warning: a stable release from $BRANCH, not main" >&2 ;;
esac

NOW="$("$PY" "$TOOL" current)"
FW="$("$PY" "$TOOL" firmware)"
DATE="$(date +%Y-%m-%d)"
echo "release $TAG ($KIND) on $BRANCH: agent $NOW -> $VERSION, firmware $FW, dated $DATE"

if [ "$DRY" = 1 ]; then
  echo "would: set the version in mac-agent and desktop (Cargo.toml + Cargo.lock), date CHANGELOG.md, commit \"Release $TAG\", tag $TAG"
  echo "release notes:"
  "$PY" "$TOOL" notes "$VERSION" 2>/dev/null || "$PY" "$TOOL" notes Unreleased
  exit 0
fi

"$PY" "$TOOL" bump "$VERSION"
"$PY" "$TOOL" changelog "$VERSION" "$DATE"
git add CHANGELOG.md mac-agent/pyproject.toml mac-agent/stick_agent/__init__.py desktop/Cargo.toml desktop/Cargo.lock
if git diff --cached --quiet; then
  echo "(versions and changelog already up to date — tagging the current commit)"
else
  git commit -q -m "Release $TAG" -m "Firmware $FW."
  echo "committed: $(git log -1 --format='%h %s')"
fi
git tag -a "$TAG" -m "StickyBot $TAG (firmware $FW)"
echo "tagged $TAG — nothing is pushed yet. Check it, then publish with:"
echo
echo "  git push origin $BRANCH $TAG"
echo
echo "(the tag push runs the release workflow: tests, firmware build, GitHub release)"
