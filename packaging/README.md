# Installing TimeTracker as a desktop application

`install.sh` builds the release binary and registers TimeTracker with your
desktop session:

- the binary goes to `~/.local/bin/timetracker` (override with `--prefix`);
- a desktop entry goes to `~/.local/share/applications/timetracker.desktop`,
  so it appears in the application menu;
- the icon goes to `~/.local/share/icons/hicolor/512x512/apps/timetracker.png`;
- a systemd **user** service goes to
  `~/.config/systemd/user/timetracker.service` and is enabled so the timer
  daemon starts with your graphical session;
- a second binary, `timetracker-panel`, and its service
  `timetracker-panel.service` are installed too, so the side panel (below) is
  ready and listening for its shortcut.

Nothing needs root; everything lives under your home directory.

## Usage

```sh
packaging/install.sh              # build, install, enable and start
packaging/install.sh --no-build   # reuse target/release/timetracker
packaging/install.sh --no-enable  # install the unit but do not start it
packaging/install.sh --prefix /usr/local   # install the binary elsewhere
```

To remove it again:

```sh
packaging/uninstall.sh            # remove binary, entry, icon and unit
packaging/uninstall.sh --purge    # also delete config and cache
```

## The systemd unit

`timetracker.service.in` is a template; `@EXEC@` is replaced with the absolute
path of the installed binary. It is a user unit bound to
`graphical-session.target`, so it starts and stops with the desktop session.

Because the daemon draws with iced, it needs a Wayland or X11 display. Most
Wayland sessions export `WAYLAND_DISPLAY`/`DISPLAY` to the systemd user
manager; if yours does not, uncomment the relevant `Environment=` lines in the
unit (or run `systemctl --user import-environment WAYLAND_DISPLAY DISPLAY`).

Useful commands:

```sh
systemctl --user status timetracker.service
systemctl --user restart timetracker.service
journalctl --user -u timetracker.service -f
```

## The side panel

`Meta+U` opens **TimeTracker itself** as a slim panel — 200px wide by default —
down the **left** edge of the screen. It is a separate instance of the real
application, not a second, cut-down view: the same clock, calendar, todos and
settings, just in a narrow window. Pressing `Meta+U` again closes it.

The shortcut is owned by `timetracker-panel`, a small daemon that does nothing
but register the key through KDE's KGlobalAccel and start or stop the panel
instance. It appears in **System Settings → Shortcuts**, where it can be rebound
without editing a file. The daemon must be running for the shortcut to exist:

```sh
systemctl --user status timetracker-panel.service
systemctl --user restart timetracker-panel.service
journalctl --user -u timetracker-panel.service -f
```

Its settings live in `~/.config/timetracker/panel.toml`:

```toml
# Which edge to anchor the panel to: "left" or "right".
side = "left"

# Panel width in logical pixels. Height always fills the screen.
width = 200

# Global shortcut, as Modifier+Key. This is only the default: once registered
# it also appears in System Settings -> Shortcuts, where it can be rebound.
shortcut = "Meta+U"
```

### Why a KWin window rule

On Wayland a client cannot position or raise its own window — the compositor
decides where a window goes and how it stacks, and winit's `set_outer_position`
is a no-op. So the panel asks for the left edge and the full height, but it is
**KWin** that enforces it, through a window rule `install.sh` adds to
`~/.config/kwinrulesrc`:

```ini
[TimeTracker Sidebar]
Description=TimeTracker Sidebar
title=TimeTracker Sidebar
titlematch=2
position=0,0
positionrule=2
above=true
aboverule=2
noborder=true
noborderrule=2
skiptaskbar=true
skiptaskbarrule=2
```

The rule matches the panel by its distinctive title, so it never touches the
main window. `kwriteconfig6` edits the file in place, so any rules you already
have are kept; `uninstall.sh` removes ours again. It can also be edited by hand
in **System Settings → Window Management → Window Rules**.

If you would rather not use a window rule, the alternative is a
`zwlr_layer_shell_v1` surface, which the compositor pins by protocol — but iced
cannot draw to one without the `iced_layershell` crate, which in turn needs
`libxkbcommon-dev` at build time.

To run the panel without installing it, build and launch the daemon directly:

```sh
cargo run --release --bin timetracker-panel
```

## Google Keep browser helper

Build `tools/tasks-bridge` with `deno task compile` before installing to include
Google Keep and scheduled Tasks support. The installer copies the compiled
`tasks-bridge` alongside the application; the uninstaller removes it. The app
reuses the dedicated Google browser profile. See
[the helper README](../tools/tasks-bridge/README.md#google-keep) for sign-in,
runtime permissions, and the read-only Keep integration.
