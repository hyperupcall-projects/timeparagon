#!/usr/bin/env bash
# Runs the Rust tests with the clock on the days daylight saving changes, in a
# zone that has them. A date-dependent test otherwise only fails twice a year.
#
# Needs libfaketime. Set TIMEPARAGON_FAKETIME_LIB to its libfaketimeMT.so.1 if it
# is not in the system's library directory.
set -euo pipefail

cd "$(dirname "$0")/.."

library=${TIMEPARAGON_FAKETIME_LIB:-$(find /usr/lib -name 'libfaketimeMT.so.1' -print -quit)}
if [[ ! -e $library ]]; then
    echo "libfaketime not found; install the faketime package" >&2
    exit 1
fi

# Built outside the fake clock, which would confuse cargo's change detection.
binaries=$(
    cargo test --workspace --locked --no-run --message-format=json 2>/dev/null |
        python3 -c '
import json, os, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "compiler-artifact" and message.get("executable") and message["profile"]["test"]:
        print(os.path.dirname(message["manifest_path"]), message["executable"])
'
)

status=0
# Just before each change, then with it behind us.
for moment in "2026-11-01 00:30:00" "2026-11-01 01:30:00" "2027-03-14 01:30:00" "2027-03-14 03:30:00"; do
    echo "=== $moment America/Los_Angeles"
    while read -r directory binary; do
        # A separate data directory: tests must not see each other's files.
        scratch=$(mktemp -d)
        (
            cd "$directory"
            env TZ=America/Los_Angeles \
                XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/data" \
                XDG_CACHE_HOME="$scratch/cache" XDG_STATE_HOME="$scratch/state" \
                LD_PRELOAD="$library" FAKETIME="@$moment" \
                NO_FAKE_STAT=1 FAKETIME_DONT_FAKE_MONOTONIC=1 \
                "$binary"
        ) || status=1
        rm -rf "$scratch"
    done <<<"$binaries"
done
exit "$status"
