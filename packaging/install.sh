#!/bin/sh
# Install TimeParagon as a desktop application.
#
# This builds the debug binary, drops it on your PATH, registers a desktop
# entry and icon so it shows up in the application menu, and installs systemd
# *user* services so the app and the shortcut daemon start with your graphical
# session and the Slack bridge starts at login. Requires Node 22.18+ and npm.
# There is one `timeparagon` binary: the side panel, the dock and the
# shortcut daemon are its sidebar, dock and daemon subcommands.
#
# Everything lands under your home directory by default; nothing needs root.
# An installation from when the project was called TimeSlay is stopped and
# removed, and its data directories are moved to their TimeParagon names.
#
# Usage:
#   packaging/install.sh [--prefix DIR] [--no-build] [--no-enable] [--help]
#
#   --prefix DIR   Install the binary under DIR/bin (default: ~/.local)
#   --no-build     Use an existing target/debug/timeparagon instead of
#                  building it
#   --no-enable    Install the systemd units but do not enable them or start
#                  the shortcut daemon or Slack bridge
#   --help         Show this message

set -eu

# --- Locate the repository -------------------------------------------------
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

# --- Defaults and argument parsing -----------------------------------------
prefix=${PREFIX:-$HOME/.local}
build=1
enable=1

while [ $# -gt 0 ]; do
    case $1 in
        --prefix)
            [ $# -ge 2 ] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
            prefix=$2
            shift 2
            ;;
        --prefix=*)
            prefix=${1#--prefix=}
            shift
            ;;
        --no-build)
            build=0
            shift
            ;;
        --no-enable)
            enable=0
            shift
            ;;
        --help|-h)
            sed -n '2,/^[^#]/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "install.sh: unknown option: $1" >&2
            exit 2
            ;;
    esac
done

# --- XDG locations ---------------------------------------------------------
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}

bin_dir=$prefix/bin
app_dir=$data_home/applications
icon_dir=$data_home/icons/hicolor/512x512/apps
unit_dir=$config_home/systemd/user

binary=$bin_dir/timeparagon
slack_dir=$data_home/timeparagon/slack
slack_config=$config_home/timeparagon/slack
slack_env=$slack_config/.env
slack_tokens=$slack_config/.slack-app-tokens.json
slack_installations=$slack_config/.slack-installations.json

say() { printf '%s\n' "$*"; }

# Resolve Node now: systemd does not load shell version-manager setup.
node_bin=$(command -v node) || { echo "install.sh: Node 22.18+ is required for the Slack bridge" >&2; exit 1; }
command -v npm >/dev/null 2>&1 || { echo "install.sh: npm is required for the Slack bridge" >&2; exit 1; }
"$node_bin" -e 'const [major, minor] = process.versions.node.split(".").map(Number); process.exit(major > 22 || (major === 22 && minor >= 18) ? 0 : 1)' || {
    echo "install.sh: Node 22.18+ is required for the Slack bridge" >&2
    exit 1
}

# --- Build -----------------------------------------------------------------
if [ "$build" -eq 1 ]; then
    say "Building the debug binary..."
    ( cd "$repo_root" && cargo build -p timeparagon -p timeparagon-core --bin timeparagon --example sync_database )
fi

built=$repo_root/target/debug/timeparagon
if [ ! -x "$built" ]; then
    echo "install.sh: $built not found; run without --no-build" >&2
    exit 1
fi

# Locate the complete Chromium runtime (resources, locales and shared libraries).
cef_runtime=${TIMEPARAGON_CEF_DIR:-${CEF_PATH:-}}
if [ -z "$cef_runtime" ]; then
    for candidate in "$repo_root"/target/debug/build/cef-dll-sys-*/out/cef_linux_*; do
        if [ -f "$candidate/libcef.so" ]; then
            cef_runtime=$candidate
            break
        fi
    done
fi
if [ -z "$cef_runtime" ] || [ ! -f "$cef_runtime/libcef.so" ]; then
    echo "install.sh: Chromium runtime not found; build first or set TIMEPARAGON_CEF_DIR" >&2
    exit 1
fi

# --- Remove the old TimeSlay installation ----------------------------------
# TimeParagon used to be called TimeSlay (and before that, time-slay). Stop
# what that installed, carry its data over and remove the rest. Data is moved
# only when no TimeParagon directory exists yet, so nothing is overwritten.
legacy_units="timeslay.service timeslay-shortcuts.service timeslay-slack.service timeslay-database-sync.service"
if command -v systemctl >/dev/null 2>&1; then
    for unit in $legacy_units; do
        systemctl --user disable --now "$unit" >/dev/null 2>&1 || true
    done
fi
for unit in $legacy_units; do
    if [ -e "$unit_dir/$unit" ]; then
        rm -f "$unit_dir/$unit"
        say "Removed old unit:  $unit_dir/$unit"
    fi
done

# A panel or dock started by hand would keep writing to the old directories.
if command -v pkill >/dev/null 2>&1 && pkill -x 'timeslay|time-slay' 2>/dev/null; then
    say "Stopped running TimeSlay processes."
    tries=0
    while pgrep -x 'timeslay|time-slay' >/dev/null 2>&1 && [ "$tries" -lt 50 ]; do
        sleep 0.1
        tries=$((tries + 1))
    done
fi

# The app moves these itself on first start too; doing it here means the
# Slack bridge finds its credentials when it is restarted below.
for base in "$config_home" "$data_home" "${XDG_STATE_HOME:-$HOME/.local/state}" "${XDG_CACHE_HOME:-$HOME/.cache}"; do
    old=$base/timeslay
    new=$base/timeparagon
    [ -d "$old" ] || continue
    if [ -e "$new" ]; then
        say "Left $old in place: $new already exists. Merge or delete it yourself."
    else
        mv "$old" "$new"
        say "Moved data:        $old -> $new"
    fi
done
# The Slack .env may name the old credential paths and variables.
if [ -f "$slack_env" ] && grep -qi 'timeslay' "$slack_env"; then
    sed -e 's#/timeslay/#/timeparagon/#g' -e 's#/timeslay\(["'"'"']*\)$#/timeparagon\1#' \
        -e 's/TIMESLAY_/TIMEPARAGON_/g' -e 's/`timeslay`/`timeparagon`/g' -e 's/TimeSlay/TimeParagon/g' \
        "$slack_env" > "$slack_env.new"
    chmod 0600 "$slack_env.new"
    mv -f "$slack_env.new" "$slack_env"
    say "Updated old paths: $slack_env"
fi
# Old browser native-host wrappers; the manifests pointing at them go below.
rm -f "$data_home/timeparagon/timeslay-webextension" "$data_home/timeparagon/time-slay-webextension"

for old in \
    "$bin_dir/timeslay" "$bin_dir/timeslay-database-sync" "$bin_dir/timeslay-panel" \
    "$bin_dir/timeslay-dock" "$bin_dir/time-slay" \
    "$prefix/lib/timeslay" \
    "$app_dir/timeslay.desktop" "$app_dir/time-slay.desktop" \
    "$icon_dir/timeslay.png" "$icon_dir/time-slay.png"
do
    if [ -e "$old" ] || [ -L "$old" ]; then
        rm -rf "$old"
        say "Removed old file:  $old"
    fi
done

# The browser extension's native host was renamed, so its manifests are
# replaced once the new binary is installed (see the end of this script).
reregister_webextension=0
for old in \
    "$config_home/google-chrome/NativeMessagingHosts/com.timeslay.webextension.json" \
    "$config_home/chromium/NativeMessagingHosts/com.timeslay.webextension.json" \
    "$config_home/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.timeslay.webextension.json" \
    "$config_home/microsoft-edge/NativeMessagingHosts/com.timeslay.webextension.json" \
    "$config_home/vivaldi/NativeMessagingHosts/com.timeslay.webextension.json" \
    "$HOME/.mozilla/native-messaging-hosts/com.timeslay.webextension.json"
do
    if [ -e "$old" ]; then
        rm -f "$old"
        say "Removed old file:  $old"
        reregister_webextension=1
    fi
done

legacy_rule="TimeSlay Sidebar"
rules_file="$config_home/kwinrulesrc"
if command -v kwriteconfig6 >/dev/null 2>&1 && grep -qxF "[$legacy_rule]" "$rules_file" 2>/dev/null; then
    current=$(sed -n 's/^Order=//p' "$rules_file" | tail -n 1)
    remaining=$(printf '%s' "$current" | tr ',' '\n' | grep -vxF "$legacy_rule" | paste -sd, -)
    kwriteconfig6 --file kwinrulesrc --group General --key Order "$remaining"
    kwriteconfig6 --file kwinrulesrc --group General --key rules "$remaining"
    # KWin drops a rule whose group asks to be deleted on the next load.
    kwriteconfig6 --file kwinrulesrc --group "$legacy_rule" --key deleteRule true
    say "Removed old KWin rule: $legacy_rule"
fi

mkdir -p "$prefix/lib/timeparagon/cef"
cp -aL "$cef_runtime/." "$prefix/lib/timeparagon/cef/"

# --- Install the binaries --------------------------------------------------
mkdir -p "$bin_dir"
install -m 0755 "$built" "$binary"
say "Installed binary:  $binary"
sync_binary=$bin_dir/timeparagon-database-sync
sync_built=$repo_root/target/debug/examples/sync_database
sync_installed=0
if [ -x "$sync_built" ]; then
    install -m 0755 "$sync_built" "$sync_binary.new"
    mv -f "$sync_binary.new" "$sync_binary"
    sync_installed=1
fi

# --- Install the desktop entry and icon ------------------------------------
mkdir -p "$app_dir" "$icon_dir"
sed "s|@EXEC@|$binary|g" "$script_dir/timeparagon.desktop.in" \
    > "$app_dir/timeparagon.desktop"
chmod 0644 "$app_dir/timeparagon.desktop"
say "Installed desktop: $app_dir/timeparagon.desktop"

install -m 0644 "$repo_root/assets/noto-hourglass-done.png" "$icon_dir/timeparagon.png"
say "Installed icon:    $icon_dir/timeparagon.png"

# --- Install the systemd user services -------------------------------------
mkdir -p "$unit_dir"
sed "s|@EXEC@|$binary|g" "$script_dir/timeparagon.service.in" \
    > "$unit_dir/timeparagon.service"
chmod 0644 "$unit_dir/timeparagon.service"
say "Installed unit:    $unit_dir/timeparagon.service"

sed "s|@EXEC@|$binary|g" "$script_dir/timeparagon-shortcuts.service.in" \
    > "$unit_dir/timeparagon-shortcuts.service"
chmod 0644 "$unit_dir/timeparagon-shortcuts.service"
say "Installed unit:    $unit_dir/timeparagon-shortcuts.service"

# --- Install the Slack runtime and preserve credentials across upgrades ----
# Stop an installed bridge before replacing its code/dependencies or tokens.
if command -v systemctl >/dev/null 2>&1; then
    if systemctl --user is-active --quiet timeparagon-slack.service; then
        systemctl --user stop timeparagon-slack.service
    fi
fi
mkdir -p "$slack_dir/src" "$slack_config"
chmod 0700 "$slack_config"
install -m 0644 "$repo_root/slack/package.json" "$slack_dir/package.json"
install -m 0644 "$repo_root/slack/src/"*.ts "$slack_dir/src/"
if [ -f "$repo_root/slack/package-lock.json" ]; then
    install -m 0644 "$repo_root/slack/package-lock.json" "$slack_dir/package-lock.json"
    ( cd "$slack_dir" && npm ci --omit=dev --ignore-scripts --no-audit --no-fund )
else
    ( cd "$slack_dir" && npm install --omit=dev --ignore-scripts --no-audit --no-fund )
fi
install -m 0600 "$repo_root/slack/.env.example" "$slack_config/.env.example"
if [ ! -e "$slack_env" ] && [ -f "$repo_root/slack/.env" ]; then
    install -m 0600 "$repo_root/slack/.env" "$slack_env"
fi
# The original refresh token may already be consumed. Migrate the latest pair,
# and never overwrite the installed cache on subsequent installs.
if [ ! -e "$slack_tokens" ] && [ -f "$repo_root/slack/.slack-app-tokens.json" ]; then
    install -m 0600 "$repo_root/slack/.slack-app-tokens.json" "$slack_tokens"
fi

# Preserve connected workspaces across upgrades and copy a development store once.
if [ ! -e "$slack_installations" ] && [ -f "$repo_root/slack/.slack-installations.json" ]; then
    install -m 0600 "$repo_root/slack/.slack-installations.json" "$slack_installations"
fi

# Escape paths for quoted systemd values and sed replacement strings.
unit_value() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g' | sed 's/[\\&|]/\\&/g'
}
# Single-path directives take the complete value literally, without quotes.
unit_path() { printf '%s' "$1" | sed 's/%/%%/g; s/[\\&|]/\\&/g'; }
sed -e "s|@NODE@|$(unit_value "$node_bin")|g" \
    -e "s|@EXEC@|$(unit_value "$binary")|g" \
    -e "s|@SLACK_DIR@|$(unit_value "$slack_dir")|g" \
    -e "s|@ENV_FILE@|$(unit_value "$slack_env")|g" \
    -e "s|@TOKEN_FILE@|$(unit_value "$slack_tokens")|g" \
    -e "s|@INSTALLATIONS_FILE@|$(unit_value "$slack_installations")|g" \
    -e "s|@ENV_PATH@|$(unit_path "$slack_env")|g" \
    -e "s|@WORKDIR@|$(unit_path "$slack_dir")|g" \
    "$script_dir/timeparagon-slack.service.in" > "$unit_dir/timeparagon-slack.service"
chmod 0644 "$unit_dir/timeparagon-slack.service"
if [ "$sync_installed" -eq 1 ]; then
    sed "s|@SYNC_BINARY@|$(unit_value "$sync_binary")|g" \
        "$script_dir/timeparagon-database-sync.service.in" > "$unit_dir/timeparagon-database-sync.service"
    chmod 0644 "$unit_dir/timeparagon-database-sync.service"
fi
say "Installed unit:    $unit_dir/timeparagon-slack.service"
if [ ! -f "$slack_env" ]; then
    say "Slack setup: copy $slack_config/.env.example to $slack_env and fill in the credentials."
    say "Then run: systemctl --user start timeparagon-slack.service"
fi

# --- Pin the panel with a KWin window rule --------------------------------
# A Wayland client cannot position or raise its own window, so the panel is
# pinned to the left edge by KWin instead. The rule matches the panel's
# distinctive title ("TimeParagon Sidebar") and is added to KWin's ordered rule
# list so it is loaded. kwriteconfig6 edits the file in place, so any rules you
# already have are left alone.
rule="TimeParagon Sidebar"
if command -v kwriteconfig6 >/dev/null 2>&1; then
    for pair in \
        "Description=$rule" \
        "title=$rule" \
        "titlematch=2" \
        "position=0,0" \
        "positionrule=2" \
        "above=true" \
        "aboverule=2" \
        "noborder=true" \
        "noborderrule=2" \
        "skiptaskbar=true" \
        "skiptaskbarrule=2"
    do
        kwriteconfig6 --file kwinrulesrc --group "$rule" --key "${pair%%=*}" "${pair#*=}"
    done

    rules_file="$config_home/kwinrulesrc"
    current=$(sed -n 's/^Order=//p' "$rules_file" 2>/dev/null | tail -n 1)
    case ",$current," in
        *",$rule,"*) ;;
        *)
            if [ -n "$current" ]; then new="$current,$rule"; else new="$rule"; fi
            kwriteconfig6 --file kwinrulesrc --group General --key Order "$new"
            kwriteconfig6 --file kwinrulesrc --group General --key rules "$new"
            ;;
    esac
    say "Installed KWin rule: $rule"

    if command -v qdbus6 >/dev/null 2>&1; then
        qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure >/dev/null 2>&1 || true
    fi
else
    say "kwriteconfig6 not found; add a KWin window rule for '$rule' yourself"
    say "  (System Settings -> Window Management -> Window Rules)"
fi

# --- Refresh caches and (optionally) enable --------------------------------
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$app_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

if [ "$enable" -eq 1 ]; then
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload
        if [ "$sync_installed" -eq 1 ]; then
            systemctl --user enable timeparagon-database-sync.service
            systemctl --user restart timeparagon-database-sync.service
        fi
        systemctl --user enable timeparagon.service
        systemctl --user enable timeparagon-shortcuts.service
        systemctl --user restart timeparagon-shortcuts.service
        systemctl --user enable timeparagon-slack.service
        systemctl --user restart timeparagon-slack.service
        say "Enabled for the next graphical session: timeparagon.service"
        say "Enabled and restarted: timeparagon-shortcuts.service"
        say "Enabled for login and restarted (when configured): timeparagon-slack.service"
    else
        say "systemctl not found; enable the units yourself with:"
        say "  systemctl --user enable timeparagon.service timeparagon-shortcuts.service timeparagon-slack.service"
        say "  systemctl --user start timeparagon-shortcuts.service timeparagon-slack.service"
    fi
fi

# --- PATH hint -------------------------------------------------------------
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *)
        say ""
        say "Note: $bin_dir is not on your PATH."
        say "Add it so the desktop entry and the 'timeparagon' command resolve:"
        say "  export PATH=\"$bin_dir:\$PATH\""
        ;;
esac

if [ "$reregister_webextension" -eq 1 ]; then
    say ""
    say "Registering the renamed browser native host (it replaces the old TimeSlay one):"
    TIMEPARAGON_BIN=$binary "$repo_root/webextension/install.sh"
    say "Reload the browser extension; its Firefox ID is now timeparagon@example.org."
fi

say ""
say "Done. TimeParagon is installed as a desktop application."
