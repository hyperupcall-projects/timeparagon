#!/usr/bin/env bash
# Migrate a Linux user installation AFTER renaming/building the project.
# Usage: scripts/migrate-timeslay.sh [--apply] [--prefix DIR]
# Default: read-only preview. Requires bash, Python 3, GNU cp, and the
# renamed packaging/install.sh with target/debug/timeslay already built.
# This script does not rename source, migrate Android, or start the app.
set -euo pipefail
umask 077

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
prefix=${PREFIX:-$HOME/.local}
apply=0
while (($#)); do
    case "$1" in
        --apply) apply=1; shift ;;
        --prefix) [[ $# -ge 2 ]] || exit 2; prefix=$2; shift 2 ;;
        --help|-h) sed -n '2,7p' "$0"; exit 0 ;;
        *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ $prefix = /* ]] || { echo '--prefix must be absolute' >&2; exit 1; }
command -v python3 >/dev/null
command -v rg >/dev/null

config=${XDG_CONFIG_HOME:-$HOME/.config}
data=${XDG_DATA_HOME:-$HOME/.local/share}
state=${XDG_STATE_HOME:-$HOME/.local/state}
cache=${XDG_CACHE_HOME:-$HOME/.cache}
roots=("$config" "$data" "$state" "$cache")
units=(time-slay.service time-slay-shortcuts.service time-slay-slack.service time-slay-panel.service)

# Check every destination before stopping services or moving anything. The
# extension already uses share/timeslay; allow disjoint children there.
python3 - "$data" "${roots[@]}" <<'PY'
import os, pathlib, sys
data = pathlib.Path(sys.argv[1])
for base in dict.fromkeys(sys.argv[2:]):
    old, new = pathlib.Path(base)/'time-slay', pathlib.Path(base)/'timeslay'
    if not os.path.lexists(old):
        continue
    if old.is_symlink() or not old.is_dir():
        sys.exit(f'Refusing non-directory or symlink: {old}')
    if os.path.lexists(new):
        if new.is_symlink() or not new.is_dir() or pathlib.Path(base) != data:
            sys.exit(f'Destination already exists; resolve manually: {new}')
        for child in old.iterdir():
            if os.path.lexists(new/child.name):
                sys.exit(f'Destination conflict: {new/child.name}')
    print(f'Data: {old} -> {new}')
PY
printf 'Install from: %s\nPrefix: %s\n' "$repo" "$prefix"
if (( ! apply )); then
    echo 'Preview only. After renaming and building, rerun with --apply.'
    exit 0
fi

# Fail before touching the installation if source/build are not ready.
[[ -x $repo/target/debug/timeslay ]] || {
    echo 'First rename the source and run: cargo build -p timeslay' >&2; exit 1;
}
[[ $("$repo/target/debug/timeslay" --version) == timeslay\ * ]] || {
    echo 'The new binary must identify itself as timeslay.' >&2; exit 1;
}
[[ -f $repo/packaging/timeslay.desktop.in ]] || {
    echo 'Rename the packaging templates and installer references first.' >&2; exit 1;
}
if rg -q 'target/debug/time-slay|binary=\$bin_dir/time-slay' "$repo/packaging/install.sh"; then
    echo 'packaging/install.sh still installs the old binary.' >&2; exit 1
fi

# Serialize apply runs; dry runs create no files.
command -v flock >/dev/null
mkdir -p -- "$config"
exec 9>"$config/.timeslay-migration.lock"
flock -n 9 || { echo 'Another migration is running.' >&2; exit 1; }

backup=$(mktemp -d "$HOME/timeslay-migration-backup.XXXXXXXX")
mkdir -m 0700 -- "$backup/installed"
printf 'Backup: %s\n' "$backup"
trap 'echo "Migration interrupted. Backup and service state: $backup. Do not start both installations." >&2' ERR
has_systemd=0
if command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; then
    has_systemd=1
    for unit in "${units[@]}" timeslay.service timeslay-shortcuts.service timeslay-slack.service; do
        enabled=$(systemctl --user is-enabled "$unit" 2>/dev/null || true)
        active=$(systemctl --user is-active "$unit" 2>/dev/null || true)
        printf '%s enabled=%s active=%s\n' "$unit" "$enabled" "$active" >> "$backup/services.txt"
        if [[ $active == active || $active == activating ]]; then
            systemctl --user stop "$unit"
        fi
    done
fi

# Refuse manual GUI/helper/Slack processes rather than killing unrelated Node
# or Chromium processes. Print PIDs only, never command lines with secrets.
python3 - "$repo" "$data" <<'PY'
import os, pathlib, sys
repo, data = sys.argv[1:]
busy = []
for proc in pathlib.Path('/proc').glob('[0-9]*'):
    try:
        if proc.stat().st_uid != os.getuid(): continue
        exe = pathlib.Path(os.readlink(proc/'exe')).name.removesuffix(' (deleted)')
        args = (proc/'cmdline').read_bytes().decode(errors='replace').split('\0')
        cwd = os.readlink(proc/'cwd')
        profiles = [f'{data}/{name}/{bridge}/chrome-profile'
                    for name in ('time-slay', 'timeslay')
                    for bridge in ('keep-bridge', 'tasks-bridge')]
        if (exe in ('time-slay', 'timeslay', 'keep-bridge', 'tasks-bridge')
            or any(profile in arg for profile in profiles for arg in args)
            or (exe == 'node' and any('app.ts' in arg for arg in args)
                and cwd in (f'{repo}/slack', f'{data}/time-slay/slack', f'{data}/timeslay/slack'))):
            busy.append(proc.name)
    except (OSError, PermissionError): pass
if busy:
    sys.exit('Close the app, Keep browser, and manual Slack bridge, then rerun. PIDs: ' + ', '.join(busy))
PY

# Back up data and installation metadata, preserving mode and timestamps.
i=0
for root in "${roots[@]}"; do
    for name in time-slay timeslay; do
        if [[ -e $root/$name ]]; then
            cp -a -- "$root/$name" "$backup/data-$i-$name"
            printf '%s\t%s\n' "$backup/data-$i-$name" "$root/$name" >> "$backup/paths.tsv"
        fi
    done
    i=$((i+1))
done
for path in "$config/systemd/user" "$config/kglobalshortcutsrc" \
    "$config/kwinrulesrc" "$config/plasma-org.kde.plasma.desktop-appletsrc" \
    "$data/applications/time-slay.desktop" \
    "$data/icons/hicolor/512x512/apps/time-slay.png" "$prefix/bin/time-slay"; do
    if [[ -e $path ]]; then
        cp -a --parents -- "$path" "$backup/installed"
    fi
done

# Keep the old KDE component ID to retain saved shortcut bindings. Changing
# this ID needs a separate KDE-aware migration, outside this script's scope.
if ! rg -q 'pub const COMPONENT: &str = "time-slay";' "$repo/src/panel/hotkey.rs"; then
    echo 'Note: KDE shortcut component changed; rebind shortcuts in Settings after migration.'
fi

python3 - "${roots[@]}" <<'PY'
import pathlib, sys
for base in dict.fromkeys(sys.argv[1:]):
    old, new = pathlib.Path(base)/'time-slay', pathlib.Path(base)/'timeslay'
    if not old.exists(): continue
    if new.exists():
        for child in old.iterdir(): child.rename(new/child.name)
        old.rmdir()
    else:
        old.rename(new)
PY

# Install without starting anything: verify the migration before enabling sync.
bash "$repo/packaging/install.sh" --prefix "$prefix" --no-build --no-enable
[[ -x $prefix/bin/timeslay ]] || { echo 'Installer did not install timeslay' >&2; exit 1; }
if (( has_systemd )); then
    for unit in "${units[@]}"; do
        if [[ -f $config/systemd/user/$unit ]]; then
            systemctl --user disable "$unit"
            rm -- "$config/systemd/user/$unit"
        fi
    done
    systemctl --user daemon-reload
fi
rm -f -- "$data/applications/time-slay.desktop" \
    "$data/icons/hicolor/512x512/apps/time-slay.png"

# Retain the old command as an alias, forwarding to the NEW binary/data paths.
ln -sfn -- timeslay "$prefix/bin/time-slay"
if [[ -d $data/applications ]] && command -v update-desktop-database >/dev/null; then
    update-desktop-database "$data/applications" || echo 'Refresh the desktop cache after migration.' >&2
fi
trap - ERR
printf '\nMigration complete; services remain stopped. Backup: %s\n' "$backup"
printf 'Inspect settings/data, then launch: %q\n' "$prefix/bin/timeslay"
echo 'Enable services when ready:'
echo '  systemctl --user enable timeslay.service timeslay-shortcuts.service timeslay-slack.service'
echo '  systemctl --user start timeslay-shortcuts.service timeslay-slack.service'
echo 'Rebuild/reinstall the Keep helper to use its new profile path.'
echo 'Rerun webextension/install.sh to refresh browser native-host paths.'
echo 'Update pinned launchers and any TIME_SLAY_* environment variable names you renamed.'
