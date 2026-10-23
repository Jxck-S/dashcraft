#!/usr/bin/env bash
# Regenerates the app icons in app/src-tauri/icons/ from app-icon.svg (original artwork).
#
#   scripts/make-app-icon.sh        # needs `npm ci` in app/ (uses the pinned @tauri-apps/cli)
#
# 1. Rasterizes the SVG to a 1024x1024 master PNG (target/app-icon/1024x1024.png).
# 2. Runs `tauri icon` on the master: PNG sizes, Windows Square*Logo/StoreLogo, icon.ico, icon.icns.
# 3. Drops the Android/iOS sets (desktop-only app).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
icons="$root/app/src-tauri/icons"
work="$root/target/app-icon"

rm -rf "$work"
mkdir -p "$work"
cd "$root/app"
npx --no-install tauri icon "$icons/app-icon.svg" --png 1024 -o "$work"
npx --no-install tauri icon "$work/1024x1024.png" -o "$icons"
rm -rf "$icons/android" "$icons/ios"
echo "Icons written to $icons (master: $work/1024x1024.png)"
