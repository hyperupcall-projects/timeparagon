#!/bin/sh
# Remove the TimeTracker desktop application installed by install.sh.
#
# Usage:
#   packaging/uninstall.sh [--prefix DIR] [--purge] [--help]
#
#   --prefix DIR   The prefix the binary was installed under (default: ~/.local)
#   --purge        Also delete the user's config and cache
#   --help         Show this message

set -eu

prefix=${PREFIX:-$HOME/.local}
purge=0

while [ $# -gt 0 ]; do
    case $1 in
        --prefix)
            [ $# -ge 2 ] || { echo "uninstall.sh: --prefix needs a directory" >&2; exit 2; }
            prefix=$2
            shift 2
            ;;
        --prefix=*)
            prefix=${1#--prefix=}
            shift
            ;;
        --purge)
            purge=1
            shift
            ;;
        --help|-h)
            sed -n '2,/^[^#]/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "uninstall.sh: unknown option: $1" >&2
            exit 2
            ;;
    esac
done

data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}

say() { printf '%s\n' "$*"; }

# --- Stop and disable the service ------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now timetracker.service >/dev/null 2>&1 || true
    systemctl --user disable --now timetracker-panel.service >/dev/null 2>&1 || true
fi

# --- Remove installed files ------------------------------------------------
rm -f "$config_home/systemd/user/timetracker.service"
rm -f "$config_home/systemd/user/timetracker-panel.service"
rm -f "$data_home/applications/timetracker.desktop"
rm -f "$data_home/icons/hicolor/512x512/apps/timetracker.png"
rm -f "$prefix/bin/timetracker"
rm -f "$prefix/bin/timetracker-panel"
rm -f "$prefix/bin/tasks-bridge"
say "Removed binaries, desktop entry, icon and systemd units."

# --- Remove the KWin window rule -------------------------------------------
rule="TimeTracker Sidebar"
if command -v kwriteconfig6 >/dev/null 2>&1; then
    rules_file="$config_home/kwinrulesrc"
    current=$(sed -n 's/^Order=//p' "$rules_file" 2>/dev/null | tail -n 1)
    remaining=$(printf '%s' "$current" | tr ',' '\n' | grep -vxF "$rule" | paste -sd, -)
    kwriteconfig6 --file kwinrulesrc --group General --key Order "$remaining"
    kwriteconfig6 --file kwinrulesrc --group General --key rules "$remaining"
    # KWin drops a rule whose group asks to be deleted on the next load.
    kwriteconfig6 --file kwinrulesrc --group "$rule" --key deleteRule true
    if command -v qdbus6 >/dev/null 2>&1; then
        qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure >/dev/null 2>&1 || true
    fi
    say "Removed KWin rule: $rule"
fi

if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

# --- Optionally remove user data -------------------------------------------
if [ "$purge" -eq 1 ]; then
    rm -rf "$config_home/timetracker"
    rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/timetracker"
    say "Purged config and cache."
fi

say "Done."
