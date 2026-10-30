#!/usr/bin/env bash
# Registers the TimeSlay native-messaging host for the browsers installed for
# this user. Run it after building timeslay and loading the extension.
set -euo pipefail

HOST="com.timeslay.webextension"
# Chrome derives this ID from the "key" in manifest.json. Override it if you
# load a build with a different key.
CHROME_ID="${CHROME_EXTENSION_ID:-ebhljjjdkfmgagkochllhjanjncnedlo}"
FIREFOX_ID="${FIREFOX_EXTENSION_ID:-timeslay@example.org}"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

find_binary() {
  if [[ -n "${TIMESLAY_BIN:-}" ]]; then
    printf '%s\n' "$TIMESLAY_BIN"
    return
  fi
  if command -v timeslay >/dev/null 2>&1; then
    command -v timeslay
    return
  fi
  for candidate in \
    "$repo_root/target/release/timeslay" \
    "$repo_root/target/debug/timeslay"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return
    fi
  done
  return 1
}

if ! BIN="$(find_binary)"; then
  cat >&2 <<'EOF'
Could not find the timeslay binary.
Build it first (cargo build --release) or set TIMESLAY_BIN=/path/to/timeslay.
EOF
  exit 1
fi
BIN="$(readlink -f -- "$BIN")"
if [[ ! -x "$BIN" ]]; then
  echo "Not executable: $BIN" >&2
  exit 1
fi

# Browsers cannot pass arguments to a native host, so the host manifest points
# at this tiny wrapper. It is the only thing that adds --webextension.
# Chrome and Firefox append the caller's origin to the command line; the host
# does not need it, so it is dropped rather than forwarded.
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/timeslay"
mkdir -p -- "$DATA_DIR"
WRAPPER="$DATA_DIR/timeslay-webextension"
cat > "$WRAPPER" <<EOF
#!/bin/sh
exec "$BIN" --webextension
EOF
chmod 0755 -- "$WRAPPER"

write_chrome_manifest() {
  local directory="$1"
  mkdir -p -- "$directory"
  cat > "$directory/$HOST.json" <<EOF
{
  "name": "$HOST",
  "description": "TimeSlay inbox native host",
  "path": "$WRAPPER",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$CHROME_ID/"]
}
EOF
  echo "  $directory/$HOST.json"
}

write_firefox_manifest() {
  local directory="$1"
  mkdir -p -- "$directory"
  cat > "$directory/$HOST.json" <<EOF
{
  "name": "$HOST",
  "description": "TimeSlay inbox native host",
  "path": "$WRAPPER",
  "type": "stdio",
  "allowed_extensions": ["$FIREFOX_ID"]
}
EOF
  echo "  $directory/$HOST.json"
}

CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

echo "Native host: $HOST"
echo "Binary:      $BIN"
echo "Wrapper:     $WRAPPER"
echo
echo "Chrome-family manifests:"
write_chrome_manifest "$CONFIG_HOME/google-chrome/NativeMessagingHosts"
write_chrome_manifest "$CONFIG_HOME/chromium/NativeMessagingHosts"
write_chrome_manifest "$CONFIG_HOME/BraveSoftware/Brave-Browser/NativeMessagingHosts"
write_chrome_manifest "$CONFIG_HOME/microsoft-edge/NativeMessagingHosts"
write_chrome_manifest "$CONFIG_HOME/vivaldi/NativeMessagingHosts"

echo
echo "Firefox manifest:"
write_firefox_manifest "$HOME/.mozilla/native-messaging-hosts"

echo
echo "Done. Load webextension/ as an unpacked extension (Chrome) or a temporary"
echo "add-on (Firefox), then click its toolbar button on a page."
