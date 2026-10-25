#!/usr/bin/env bash
# Sets the product version everywhere it is declared: the `[workspace.package] version` in
# Cargo.toml (every crate inherits it), Cargo.lock, app/package.json, app/package-lock.json and,
# when it has one, app/src-tauri/tauri.conf.json.
#
# Usage: scripts/set-version.sh <version>      e.g. 0.3.0 or 0.3.0-rc.1
#
# Used to bump a release by hand and by the release workflow for a version override.
set -euo pipefail

version=${1:-}
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: $0 <major.minor.patch[-pre]>" >&2
  exit 2
fi

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Windows runners have `python`, not always `python3`.
python=$(command -v python3 || command -v python)

"$python" - "$version" <<'PY'
import json, re, sys

version = sys.argv[1]

# Cargo.toml: only the `version` key of the [workspace.package] table.
text = open("Cargo.toml", encoding="utf-8").read()
section = re.search(r"(?ms)^\[workspace\.package\]\n(.*?)(?=^\[|\Z)", text)
if not section:
    sys.exit("Cargo.toml has no [workspace.package] table")
body, n = re.subn(r'(?m)^version = "[^"]*"', f'version = "{version}"', section.group(1), count=1)
if n != 1:
    sys.exit("Cargo.toml [workspace.package] has no version")
text = text[: section.start(1)] + body + text[section.end(1) :]
open("Cargo.toml", "w", encoding="utf-8", newline="\n").write(text)

def set_json(path, *keys_list):
    data = json.load(open(path, encoding="utf-8"))
    for keys in keys_list:
        node = data
        for k in keys[:-1]:
            node = node[k]
        node[keys[-1]] = version
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")

set_json("app/package.json", ("version",))
set_json("app/package-lock.json", ("version",), ("packages", "", "version"))
conf = json.load(open("app/src-tauri/tauri.conf.json", encoding="utf-8"))
if "version" in conf:
    set_json("app/src-tauri/tauri.conf.json", ("version",))
PY

# Refresh the workspace crates' entries in Cargo.lock (registry dependencies stay as locked).
cargo update --workspace --quiet

echo "version set to $version"
