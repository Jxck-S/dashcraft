#!/usr/bin/env bash
# Runs one cargo-fuzz target the way the nightly `fuzz.yml` workflow does:
# new inputs go to fuzz/corpus/<target> (gitignored; cached between CI runs),
# the committed seeds in fuzz/seeds/<target> are replayed first, and the
# target's libFuzzer dictionary from fuzz/dictionaries is used.
#
# Usage: fuzz/run.sh <target> [seconds=60] [extra libFuzzer flags...]
# Needs a nightly toolchain and cargo-fuzz (`cargo install cargo-fuzz --locked`).
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "usage: $0 <target> [seconds] [libFuzzer flags...]" >&2
  exit 2
fi
cd "$(dirname "$0")/.."
root="$(pwd)"
target="$1"
seconds="${2:-60}"
shift $(($# < 2 ? $# : 2))

case "$target" in
  m_parse | m_eval) dict=m ;;
  dax_parse) dict=dax ;;
  tmdl) dict=tmdl ;;
  tmsl | pbir_json | pbir_layout) dict=json ;;
  *) dict= ;;
esac

args=(-max_total_time="$seconds" -timeout=60 -rss_limit_mb=2048)
if [ -n "$dict" ]; then
  args+=(-dict="$root/fuzz/dictionaries/$dict.dict")
fi
corpora=("$root/fuzz/corpus/$target")
if [ -d "$root/fuzz/seeds/$target" ]; then
  corpora+=("$root/fuzz/seeds/$target")
fi
mkdir -p "$root/fuzz/corpus/$target"

exec cargo +nightly fuzz run "$target" "${corpora[@]}" -- "${args[@]}" "$@"
