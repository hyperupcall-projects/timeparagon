#!/bin/sh
# Install TimeTracker as a desktop application.
#
# This builds the debug binary, drops it on your PATH, registers a desktop
# entry and icon so it shows up in the application menu, and installs a
# systemd *user* service so the timer daemon starts with your graphical
# session.
#
# Everything lands under your home directory by default; nothing needs root.
#
# Usage:
#   packaging/install.sh [--prefix DIR] [--no-build] [--no-enable] [--help]
#
#   --prefix DIR   Install the binary under DIR/bin (default: ~/.local)
#   --no-build     Use an existing target/debug/timetracker instead of
#                  building it
#   --no-enable    Install the systemd units but do not enable them or start
#                  the shortcut daemon
#   --help         Show this message

set -eu

# --- Locate the repository -------------------------------------------------
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

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

binary=$bin_dir/timetracker
panel_binary=$bin_dir/timetracker-panel
dock_binary=$bin_dir/timetracker-dock

say() { printf '%s\n' "$*"; }

# --- Build -----------------------------------------------------------------
if [ "$build" -eq 1 ]; then
    say "Building debug binaries..."
    ( cd "$repo_root" && cargo build )
fi

built=$repo_root/target/debug/timetracker
if [ ! -x "$built" ]; then
    echo "install.sh: $built not found; run without --no-build" >&2
    exit 1
fi

built_panel=$repo_root/target/debug/timetracker-panel
if [ ! -x "$built_panel" ]; then
    echo "install.sh: $built_panel not found; run without --no-build" >&2
    exit 1
fi

built_dock=$repo_root/target/debug/timetracker-dock
if [ ! -x "$built_dock" ]; then
    echo "install.sh: $built_dock not found; run without --no-build" >&2
    exit 1
fi

# --- Install the binaries --------------------------------------------------
mkdir -p "$bin_dir"
install -m 0755 "$built" "$binary"
say "Installed binary:  $binary"
install -m 0755 "$built_panel" "$panel_binary"
say "Installed binary:  $panel_binary"
install -m 0755 "$built_dock" "$dock_binary"
say "Installed binary:  $dock_binary"

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
sed "s|@EXEC@|$binary|g" "$script_dir/timetracker.desktop.in" \
    > "$app_dir/timetracker.desktop"
chmod 0644 "$app_dir/timetracker.desktop"
say "Installed desktop: $app_dir/timetracker.desktop"

install -m 0644 "$repo_root/assets/noto-hourglass-done.png" "$icon_dir/timetracker.png"
say "Installed icon:    $icon_dir/timetracker.png"

# --- Install the systemd user services -------------------------------------
mkdir -p "$unit_dir"
sed "s|@EXEC@|$binary|g" "$script_dir/timetracker.service.in" \
    > "$unit_dir/timetracker.service"
chmod 0644 "$unit_dir/timetracker.service"
say "Installed unit:    $unit_dir/timetracker.service"

sed "s|@EXEC@|$panel_binary|g" "$script_dir/timetracker-panel.service.in" \
    > "$unit_dir/timetracker-panel.service"
chmod 0644 "$unit_dir/timetracker-panel.service"
say "Installed unit:    $unit_dir/timetracker-panel.service"

# --- Pin the panel with a KWin window rule --------------------------------
# A Wayland client cannot position or raise its own window, so the panel is
# pinned to the left edge by KWin instead. The rule matches the panel's
# distinctive title ("TimeTracker Sidebar") and is added to KWin's ordered rule
# list so it is loaded. kwriteconfig6 edits the file in place, so any rules you
# already have are left alone.
rule="TimeTracker Sidebar"
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
        systemctl --user enable timetracker.service
        systemctl --user enable timetracker-panel.service
        systemctl --user restart timetracker-panel.service
        say "Enabled for the next graphical session: timetracker.service"
        say "Enabled and restarted: timetracker-panel.service"
    else
        say "systemctl not found; enable the units yourself with:"
        say "  systemctl --user enable timetracker.service timetracker-panel.service"
        say "  systemctl --user start timetracker-panel.service"
    fi
fi

# --- PATH hint -------------------------------------------------------------
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *)
        say ""
        say "Note: $bin_dir is not on your PATH."
        say "Add it so the desktop entry and the 'timetracker' command resolve:"
        say "  export PATH=\"$bin_dir:\$PATH\""
        ;;
esac

say ""
say "Done. TimeTracker is installed as a desktop application."
