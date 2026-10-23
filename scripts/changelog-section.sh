#!/usr/bin/env bash
# Prints the CHANGELOG.md section of one version (without its heading), for release notes.
#
#   scripts/changelog-section.sh 0.1.0 [CHANGELOG.md]
#
# Matches the Keep a Changelog heading `## [0.1.0] ...` and prints everything up to the next
# `## [` heading, without leading/trailing blank lines. Exits 1 when the section is missing or empty.
set -euo pipefail

version=${1:?usage: changelog-section.sh <version> [changelog]}
changelog=${2:-$(cd "$(dirname "$0")/.." && pwd)/CHANGELOG.md}

section=$(awk -v heading="## [${version}]" '
  /^## \[/ { if (found) exit; if (index($0, heading) == 1) { found = 1; next } }
  found { print }
' "$changelog" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}')

if [ -z "$section" ]; then
  echo "error: no '## [${version}]' section in $changelog" >&2
  exit 1
fi
printf '%s\n' "$section"
