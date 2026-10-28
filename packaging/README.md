# Installing TimeSlay as a desktop application

`install.sh` builds the debug binaries and registers TimeSlay with your
desktop session:

- the binary goes to `~/.local/bin/time-slay` (override with `--prefix`);
- a desktop entry goes to `~/.local/share/applications/time-slay.desktop`,
  so it appears in the application menu;
- the icon goes to `~/.local/share/icons/hicolor/512x512/apps/time-slay.png`;
- a systemd **user** service goes to
  `~/.config/systemd/user/time-slay.service` and is enabled so the timer
  daemon starts with your graphical session;
- a second service, `time-slay-shortcuts.service`, runs
  `time-slay --daemon` so the global shortcuts (below) are ready and
  listening.

There is a single `time-slay` binary. The side panel, the dock and the
shortcut daemon are modes of it, chosen by a flag (`time-slay --help` lists
them):

| Command | What it runs |
| --- | --- |
| `time-slay` | The regular window |
| `time-slay --sidebar` | The side panel: the full app in a wide strip down a screen edge |
| `time-slay --dock` | The dock: a very thin strip of today along a screen edge |
| `time-slay --daemon` | The background process that owns the global shortcuts |

The installer removes the separate `time-slay-panel` and `time-slay-dock`
binaries and the `time-slay-panel.service` that earlier versions installed.

Nothing needs root; everything lives under your home directory.

## Usage

```sh
packaging/install.sh              # build, install and enable for future sessions
packaging/install.sh --no-build   # reuse binaries in target/debug
packaging/install.sh --no-enable  # install the units without enabling or starting
packaging/install.sh --prefix /usr/local   # install the binary elsewhere
```

The installer does not launch the TimeSlay application. Open it from the
application menu when ready; its service will start with your next graphical
session. The installer restarts the background shortcut daemon,
`time-slay-shortcuts.service`, so the shortcuts are ready immediately,
unless `--no-enable` is passed.

Google Tasks and Calendar sync use the public Google APIs and require no
browser helper. Every event TimeSlay writes lives on a single **TimeSlay**
calendar, created automatically on the first sync that needs it; in Settings,
choose the Google Tasks list scheduled tasks go to, or let the first scheduling
operation create/reuse **Has Time**. Reconnect Google once to authorize
calendar creation. Google Tasks only stores a day for a task, so times and
deadlines live in reserved lines anywhere in the task's notes:

```text
DEADLINE: 2026-10-30 17:00   deadline, day and time
DUE: 2026-10-30              deadline, day only (DEADLINE wins if both exist)
DUE_TIME: 09:30              show the task at 09:30 on its due day
```

`DUE_TIME` uses the task's Google due day and lasts as long as its estimate
(30 minutes if none). A task scheduled through the app takes its time from
Calendar instead. Moving a time block preserves the deadline. Completing a
task retains its Calendar event and work history.

Google Keep still uses the optional helper described below.
To remove it again:

```sh
packaging/uninstall.sh            # remove binary, entry, icon and unit
packaging/uninstall.sh --purge    # also delete config and cache
```

## The systemd unit

`time-slay.service.in` is a template; `@EXEC@` is replaced with the absolute
path of the installed binary. It is a user unit bound to
`graphical-session.target`, so it starts and stops with the desktop session.

Because the daemon draws with iced, it needs a Wayland or X11 display. Most
Wayland sessions export `WAYLAND_DISPLAY`/`DISPLAY` to the systemd user
manager; if yours does not, uncomment the relevant `Environment=` lines in the
unit (or run `systemctl --user import-environment WAYLAND_DISPLAY DISPLAY`).

Useful commands:

```sh
systemctl --user status time-slay.service
systemctl --user restart time-slay.service
journalctl --user -u time-slay.service -f
```

## The side panel

**Settings → View layout** controls the calendar width, the width of the time
block pane beside it, the width of the event details pane, and the screen
margin. Changes apply immediately and save automatically in `config.toml`.
Width fields have −10/+10 buttons; the margin field has −1/+1 buttons and
accepts 0–30 pixels. Values can also be typed directly:

```toml
[views]
calendar_width = 280.0
time_block_width = 400.0
detail_width = 360.0
margin = 8.0
```

Widths are in logical pixels. If `calendar_width` is omitted, the main window
keeps its saved width and the side panel uses the `[panel]` width. The margin defaults
to 8 and leaves transparent space along the panel's screen edge, top and bottom
(mirrored for a right-side panel). It also applies while the time block pane is
open. The regular desktop window keeps its normal window placement.

In Weekly, the panel temporarily widens to fit the configured day columns
and restores its usual width when you leave the view. If the columns exceed
the screen, scroll horizontally. On Wayland, the compositor decides the
available window size and placement.

In Daily, clicking a calendar event or a Google Task opens its details in a pane
on the right rather than over the view: the window widens by `detail_width`
beside the calendar or backlog. When the time block editor is open too, the
details pane sits to the right of it. `detail_width` is the pane's default width.

`time-slay --sidebar` opens **TimeSlay itself** as a slim panel — 200px
wide by default — down the **left** edge of the screen. It is a separate
instance of the real application, not a second, cut-down view: the same clock,
calendar, todos and settings, just in a narrow window.

### Shortcuts

The shortcuts are owned by `time-slay --daemon`, a small process that does
nothing but register the keys through KDE's KGlobalAccel and show, hide or
reshape the panel and the dock. There are two actions:

- **Toggle Visibility** (`Meta+U` by default) hides TimeSlay completely, as
  the panel or as the dock. Pressed again, it brings back the same shape as it
  was: the panel keeps running while hidden, so its tab, open panes and width
  are all as you left them.
- **Toggle Shape** (`Meta+Shift+U` by default) switches between the panel and
  the dock. While TimeSlay is hidden it comes back in the other shape.

Each action can have **several** shortcuts, and any of them triggers it. Open
**Settings → Keyboard shortcuts**, type a combination such as `Meta+J` or
`Ctrl+Alt+T` under the action, and select **Add shortcut**; **Remove** drops
one. Changes take effect immediately and KDE remembers them across restarts.
The actions also appear in KDE **System Settings → Shortcuts**. The daemon must
be running for the shortcuts to exist:

```sh
systemctl --user status time-slay-shortcuts.service
systemctl --user restart time-slay-shortcuts.service
journalctl --user -u time-slay-shortcuts.service -f
```

Its settings live in the `[panel]` table of `~/.config/time-slay/config.toml`
(an older standalone `panel.toml` is still read until `config.toml` has a
`[panel]` table, and the next save carries its values over):

```toml
[panel]
# Which edge to anchor the panel to: "left" or "right".
side = "left"

# Panel width in logical pixels. Height always fills the screen.
width = 200

# Global shortcuts, as Modifier+Key. Any number per action. These are only the
# defaults: once registered they also appear in System Settings -> Shortcuts,
# where they can be rebound. The old single `shortcut = "Meta+U"` key is still
# read as toggle_visibility.
toggle_visibility = ["Meta+U"]
toggle_shape = ["Meta+Shift+U"]
```

### Why a KWin window rule

On Wayland a client cannot position or raise its own window — the compositor
decides where a window goes and how it stacks, and winit's `set_outer_position`
is a no-op. So the panel asks for the left edge and the full height, but it is
**KWin** that enforces it, through a window rule `install.sh` adds to
`~/.config/kwinrulesrc`:

```ini
[TimeSlay Sidebar]
Description=TimeSlay Sidebar
title=TimeSlay Sidebar
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

To run the shortcut daemon without installing it, launch it directly:

```sh
cargo run --release -- --daemon
```

## Dock strip

`time-slay --dock` is a much thinner companion than the panel: a thin line
(the **Dock width** setting, 10px by default) down the left edge of the screen
showing today. It is a real `zwlr_layer_shell_v1` surface, so it needs no window rule. KDE Plasma, Sway,
Hyprland, niri and COSMIC provide that protocol; GNOME does not.

- It draws only the leftmost column of the day view, so where events overlap
  you see the one from the source you ranked highest.
- Calendar events and Google Tasks are coloured spans. Gaps are the day view's
  background colour (from the configured theme), not transparent.
- A red line marks the current time.
- Hovering a span opens a card beside the strip, level with that span.
- Clicking a span pins its card open with everything about it (the whole
  description, and for a time block every task assigned to it). It stays when
  the pointer leaves; click the span again to close it.
- A faint gray line marks each hour across the strip. Turn this off in
  Settings → Windowing.
- Settings → Windowing sets how the dock appears: the desktop default, a fade
  in (with a duration in ms), or instantly. The dock does this itself; the side
  panel loads a temporary KWin script effect (so it needs KDE) and carries the
  mode in its application ID, which means it loses its desktop-file icon.
- Only the strip takes the pointer; the card area is transparent and click-through.

Switching modes: in the regular window or the side panel, open the calendar
**Menu** and press **Dock to screen edge**. That starts the dock and puts the
window away (the window quits; the side panel just hides). The small square at
the very top of the dock, as wide as the strip, brings back whatever you came
from and closes the dock. The dock is told where it came from with
`--from window|sidebar`, which defaults to `window`.

```sh
time-slay --dock
cargo run --release -- --dock   # from a checkout
```

The **Toggle Shape** shortcut switches between the panel and the dock without
going through the menu.

It reads `$XDG_CONFIG_HOME/time-slay/dock.toml` at start-up; every key is
optional:

```toml
side = "left"        # or "right"
popup_width = 300    # hover card width
popup_gap = 6        # space between strip and card
```

It shares the main app's config, theme, sign-in and day cache, and syncs today
itself every `sync_minutes`.

## Google Keep browser helper

Build `tools/keep-bridge` with `deno task compile` before installing to include
Google Keep support. The installer copies the compiled
`keep-bridge` alongside the application; the uninstaller removes it. The app
reuses the dedicated Google browser profile. See
[the helper README](../tools/keep-bridge/README.md#google-keep) for sign-in,
runtime permissions, and the read-only Keep integration.
