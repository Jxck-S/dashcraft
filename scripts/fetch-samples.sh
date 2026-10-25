#!/bin/sh
# Fetch the Microsoft Power BI Desktop sample files used as test fixtures
# (MIT; see reference/samples/microsoft/README.md and LICENSE).
#
#   scripts/fetch-samples.sh          download the large samples into reference/samples/downloads/
#   scripts/fetch-samples.sh --all    also re-fetch the committed small samples and verify them
#
# Every file is pinned to one upstream commit and checked against its SHA-256.
# Files that are already present with the right hash are left alone, so the
# script is idempotent. Fixture root: $DASHCRAFT_REFERENCE, else <repo>/reference.
# Needs curl and one of sha256sum, shasum or openssl. Exit status 1 on any error.

set -eu

REPO="microsoft/powerbi-desktop-samples"
COMMIT="d5ec562d24b1890c2fd2976da625dd0920d35871"
BASE_URL="https://raw.githubusercontent.com/$REPO/$COMMIT"

# set: <set>|<upstream path>|<local dir under samples/>|<sha256>|<bytes>
# "download" files are gitignored; "committed" files are in the repository.
MANIFEST='download|2026 Power BI Samples Revamp/AdventureWorks Sales.pbix|downloads|76c89b64cd1c5a9514b43cc5ea356efc82f8e92f9a96b49f6679d9ccd57fc4f5|8390782
download|2026 Power BI Samples Revamp/Store Sales.pbix|downloads|160b5b48b74ee1b5ea7259b6c8f0f1e2ec011e94c7b743742a14e6cad1153e6f|9701642
committed|LICENSE|microsoft|dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6|1183
committed|2026 Power BI Samples Revamp/Revenue Opportunities.pbix|microsoft|3c95c4fb5a261e9d13a7152a2e14b18ea9e9efac0105dbd9ed46ba0137489c27|245930
committed|2026 Power BI Samples Revamp/Corporate Spend.pbix|microsoft|2e56785790d20bc4596d2aec8ede7128093b4affaaf2204198b382c9192f6ca6|716704
committed|2026 Power BI Samples Revamp/Regional Sales Sample.pbix|microsoft|050e6b8549c222489ef51ec6c7647609b9401d42da0daf793f91603dd2d8f3da|1018743
committed|powerbi-service-samples/COVID-19 US Tracking Sample.pbit|microsoft|714d3b00e5b02d4c0d87c73363b00829d14c77f609c49ea2c23b4f3392ea626d|1276277'

usage() {
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
}

die() {
    printf 'fetch-samples: error: %s\n' "$*" >&2
    exit 1
}

say() {
    printf 'fetch-samples: %s\n' "$*"
}

ALL=0
for arg in "$@"; do
    case "$arg" in
        --all) ALL=1 ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown argument: $arg (see --help)" ;;
    esac
done

command -v curl >/dev/null 2>&1 || die "curl is required but was not found in PATH"

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v openssl >/dev/null 2>&1; then
    sha256() { openssl dgst -sha256 -r "$1" | cut -d' ' -f1; }
else
    die "need sha256sum, shasum or openssl to verify downloads"
fi

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
REF=${DASHCRAFT_REFERENCE:-$ROOT/reference}
SAMPLES="$REF/samples"

PART=""
cleanup() {
    if [ -n "$PART" ] && [ -f "$PART" ]; then
        rm -f -- "$PART"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# download <upstream path> <destination file>: fetch into a hidden .part file
# next to the destination (its path is left in $PART).
download() {
    url="$BASE_URL/$(printf '%s' "$1" | sed 's/ /%20/g')"
    PART="$(dirname -- "$2")/.$(basename -- "$2").part"
    rm -f -- "$PART"
    if ! curl --fail --location --silent --show-error --proto '=https' \
        --retry 3 --connect-timeout 30 --output "$PART" "$url" </dev/null; then
        printf 'fetch-samples: error: download failed: %s\n' "$url" >&2
        rm -f -- "$PART"
        return 1
    fi
}

failed=0
fetched=0
ok=0
while IFS='|' read -r set upstream dir sum size; do
    [ -n "$set" ] || continue
    if [ "$set" = committed ] && [ "$ALL" -eq 0 ]; then
        continue
    fi
    name=$(basename -- "$upstream")
    dest="$SAMPLES/$dir/$name"
    mkdir -p -- "$SAMPLES/$dir" || die "cannot create $SAMPLES/$dir"

    if [ "$set" = download ] && [ -f "$dest" ] && [ "$(sha256 "$dest")" = "$sum" ]; then
        say "ok        samples/$dir/$name (already present, SHA-256 verified)"
        ok=$((ok + 1))
        continue
    fi

    if ! download "$upstream" "$dest"; then
        failed=$((failed + 1))
        continue
    fi
    got=$(sha256 "$PART")
    got_size=$(wc -c <"$PART" | tr -d ' ')
    if [ "$got" != "$sum" ]; then
        printf 'fetch-samples: error: SHA-256 mismatch for %s (corrupt download or changed upstream file)\n  expected %s (%s bytes)\n  got      %s (%s bytes)\n' \
            "$upstream" "$sum" "$size" "$got" "$got_size" >&2
        rm -f -- "$PART"
        failed=$((failed + 1))
        continue
    fi

    if [ -f "$dest" ] && [ "$(sha256 "$dest")" = "$sum" ]; then
        rm -f -- "$PART"
        say "verified  samples/$dir/$name (upstream and local copy match the pinned SHA-256)"
        ok=$((ok + 1))
    else
        if [ -f "$dest" ]; then
            say "warning: samples/$dir/$name did not match the pinned SHA-256; replaced with the upstream file" >&2
        fi
        mv -f -- "$PART" "$dest"
        say "fetched   samples/$dir/$name ($got_size bytes, SHA-256 verified)"
        fetched=$((fetched + 1))
    fi
    PART=""
done <<EOF
$MANIFEST
EOF

say "$ok up to date, $fetched fetched, $failed failed (commit ${COMMIT%"${COMMIT#???????}"}; root $REF)"
[ "$failed" -eq 0 ] || exit 1
