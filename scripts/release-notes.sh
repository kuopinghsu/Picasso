#!/usr/bin/env bash
# Prints the CHANGELOG.md section of a version, without its heading, for use
# as release notes. Fails when the version has no section or it is empty.
# With --release, also fails while the section is still headed "Unreleased",
# as a tag build must have the release date in place.
#
#   scripts/release-notes.sh [--release] 0.9.4 [CHANGELOG.md]
set -euo pipefail

release=0
if [ "${1:-}" = --release ]; then
  release=1
  shift
fi
version=${1:?usage: $0 [--release] VERSION [CHANGELOG]}
version=${version#v}
changelog=${2:-CHANGELOG.md}

heading=$(grep -E "^## \[$(printf '%s' "$version" | sed 's/[.]/\\./g')\]" "$changelog" | head -n 1 || true)
if [ -z "$heading" ]; then
  echo "error: $changelog has no section for version $version" >&2
  exit 1
fi
if [ "$release" = 1 ] && printf '%s' "$heading" | grep -qi unreleased; then
  echo "error: $changelog still says \"$heading\"; put the release date in its place" >&2
  exit 1
fi

notes=$(awk -v version="$version" '
  /^## / {
    if (found) exit
    heading = $2
    gsub(/\[|\]/, "", heading)
    found = heading == version
    next
  }
  !found || /^\[.+\]: / { next }
  /^[[:space:]]*$/ { if (n) blank++; next }
  { while (blank) { lines[++n] = ""; blank-- }; lines[++n] = $0 }
  END { for (i = 1; i <= n; i++) print lines[i] }
' "$changelog")

if [ -z "$notes" ]; then
  echo "error: $changelog has no notes for version $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
