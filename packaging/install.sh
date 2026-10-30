#!/bin/sh
# Install TimeSlay as a desktop application.
#
# This builds the debug binary, drops it on your PATH, registers a desktop
# entry and icon so it shows up in the application menu, and installs systemd
# *user* services so the app and the shortcut daemon start with your graphical
# session and the Slack bridge starts at login. Requires Node 22.18+ and npm.
# There is one `timeslay` binary: the side panel, the dock and the
# shortcut daemon are its --sidebar, --dock and --daemon modes.
#
# Everything lands under your home directory by default; nothing needs root.
#
# Usage:
#   packaging/install.sh [--prefix DIR] [--no-build] [--no-enable] [--help]
#
#   --prefix DIR   Install the binary under DIR/bin (default: ~/.local)
#   --no-build     Use an existing target/debug/timeslay instead of
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

binary=$bin_dir/timeslay
slack_dir=$data_home/timeslay/slack
slack_config=$config_home/timeslay/slack
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
    ( cd "$repo_root" && cargo build )
fi

built=$repo_root/target/debug/timeslay
if [ ! -x "$built" ]; then
    echo "install.sh: $built not found; run without --no-build" >&2
    exit 1
fi

# --- Install the binaries --------------------------------------------------
mkdir -p "$bin_dir"
install -m 0755 "$built" "$binary"
say "Installed binary:  $binary"

# Earlier versions installed separate panel and dock binaries. They are now
# modes of the one binary (--sidebar, --dock, --daemon).
rm -f "$bin_dir/timeslay-panel" "$bin_dir/timeslay-dock"

# Google Keep uses this optional browser helper. Task sync uses Google APIs. Desktop
# launchers do not inherit a shell's PATH or the checkout's working directory.
bridge=$repo_root/tools/keep-bridge/keep-bridge
if [ -x "$bridge" ]; then
    install -m 0755 "$bridge" "$bin_dir/keep-bridge"
    # Retire the helper name used by earlier installations.
    rm -f "$bin_dir/tasks-bridge"
    say "Installed helper:  $bin_dir/keep-bridge"
else
    say "Browser helper is not built. Google Keep requires it; task sync does not."
    say "Run 'deno task compile' in tools/keep-bridge, then rerun this installer."
fi

# --- Install the desktop entry and icon ------------------------------------
mkdir -p "$app_dir" "$icon_dir"
sed "s|@EXEC@|$binary|g" "$script_dir/timeslay.desktop.in" \
    > "$app_dir/timeslay.desktop"
chmod 0644 "$app_dir/timeslay.desktop"
say "Installed desktop: $app_dir/timeslay.desktop"

install -m 0644 "$repo_root/assets/noto-hourglass-done.png" "$icon_dir/timeslay.png"
say "Installed icon:    $icon_dir/timeslay.png"

# --- Install the systemd user services -------------------------------------
mkdir -p "$unit_dir"
sed "s|@EXEC@|$binary|g" "$script_dir/timeslay.service.in" \
    > "$unit_dir/timeslay.service"
chmod 0644 "$unit_dir/timeslay.service"
say "Installed unit:    $unit_dir/timeslay.service"

sed "s|@EXEC@|$binary|g" "$script_dir/timeslay-shortcuts.service.in" \
    > "$unit_dir/timeslay-shortcuts.service"
chmod 0644 "$unit_dir/timeslay-shortcuts.service"
say "Installed unit:    $unit_dir/timeslay-shortcuts.service"

# --- Install the Slack runtime and preserve credentials across upgrades ----
# Stop an installed bridge before replacing its code/dependencies or tokens.
if command -v systemctl >/dev/null 2>&1; then
    if systemctl --user is-active --quiet timeslay-slack.service; then
        systemctl --user stop timeslay-slack.service
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
    "$script_dir/timeslay-slack.service.in" > "$unit_dir/timeslay-slack.service"
chmod 0644 "$unit_dir/timeslay-slack.service"
say "Installed unit:    $unit_dir/timeslay-slack.service"
if [ ! -f "$slack_env" ]; then
    say "Slack setup: copy $slack_config/.env.example to $slack_env and fill in the credentials."
    say "Then run: systemctl --user start timeslay-slack.service"
fi

# The shortcut daemon used to be the separate timeslay-panel.service.
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now timeslay-panel.service >/dev/null 2>&1 || true
fi
rm -f "$unit_dir/timeslay-panel.service"

# --- Pin the panel with a KWin window rule --------------------------------
# A Wayland client cannot position or raise its own window, so the panel is
# pinned to the left edge by KWin instead. The rule matches the panel's
# distinctive title ("TimeSlay Sidebar") and is added to KWin's ordered rule
# list so it is loaded. kwriteconfig6 edits the file in place, so any rules you
# already have are left alone.
rule="TimeSlay Sidebar"
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
        systemctl --user enable timeslay.service
        systemctl --user enable timeslay-shortcuts.service
        systemctl --user restart timeslay-shortcuts.service
        systemctl --user enable timeslay-slack.service
        systemctl --user restart timeslay-slack.service
        say "Enabled for the next graphical session: timeslay.service"
        say "Enabled and restarted: timeslay-shortcuts.service"
        say "Enabled for login and restarted (when configured): timeslay-slack.service"
    else
        say "systemctl not found; enable the units yourself with:"
        say "  systemctl --user enable timeslay.service timeslay-shortcuts.service timeslay-slack.service"
        say "  systemctl --user start timeslay-shortcuts.service timeslay-slack.service"
    fi
fi

# --- PATH hint -------------------------------------------------------------
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *)
        say ""
        say "Note: $bin_dir is not on your PATH."
        say "Add it so the desktop entry and the 'timeslay' command resolve:"
        say "  export PATH=\"$bin_dir:\$PATH\""
        ;;
esac

say ""
say "Done. TimeSlay is installed as a desktop application."
