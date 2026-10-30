# Installing TimeSlay as a desktop application

`install.sh` builds the debug binaries and registers TimeSlay with your
desktop session:

- the binary goes to `~/.local/bin/timeslay` (override with `--prefix`);
- a desktop entry goes to `~/.local/share/applications/timeslay.desktop`,
  so it appears in the application menu;
- the icon goes to `~/.local/share/icons/hicolor/512x512/apps/timeslay.png`;
- a systemd **user** service goes to
  `~/.config/systemd/user/timeslay.service` and is enabled so the timer
  daemon starts with your graphical session;
- a second service, `timeslay-shortcuts.service`, runs
  `timeslay --daemon` so the global shortcuts (below) are ready and
  listening;
- `timeslay-slack.service` runs the Slack inbox bridge at login and restarts
  it on failure. The installer requires Node **22.18+** and npm and installs
  the bridge's production dependencies automatically.

There is a single `timeslay` binary. The side panel, the dock and the
shortcut daemon are modes of it, chosen by a flag (`timeslay --help` lists
them):

| Command | What it runs |
| --- | --- |
| `timeslay` | The side panel (same as `--sidebar`) |
| `timeslay --sidebar` | The side panel: the full app in a wide strip down a screen edge |
| `timeslay --dock` | The dock: a very thin strip of today along a screen edge |
| `timeslay --daemon` | The background process that owns the global shortcuts |

The installer removes the separate `timeslay-panel` and `timeslay-dock`
binaries and the `timeslay-panel.service` that earlier versions installed.

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
`timeslay-shortcuts.service`, and the Slack bridge, so both are ready immediately,
unless `--no-enable` is passed.

Google Tasks and Calendar sync use the public Google APIs and require no
browser helper. Every event TimeSlay writes lives on a single **TimeSlay**
calendar, created automatically on the first sync that needs it; in Settings,
choose the Google Tasks list scheduled tasks go to, or let the first scheduling
operation create/reuse. Reconnect Google once to authorize
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

`timeslay.service.in` is a template; `@EXEC@` is replaced with the absolute
path of the installed binary. It is a user unit bound to
`graphical-session.target`, so it starts and stops with the desktop session.

TimeSlay is Wayland-only, so the daemon needs a Wayland display. Most
Wayland sessions export `WAYLAND_DISPLAY` to the systemd user manager; if
yours does not, uncomment the `Environment=` line in the unit (or run
`systemctl --user import-environment WAYLAND_DISPLAY`).

Useful commands:

```sh
systemctl --user status timeslay.service
systemctl --user restart timeslay.service
journalctl --user -u timeslay.service -f
```

### Slack service

The installer copies the Slack runtime to
`~/.local/share/timeslay/slack` (or `$XDG_DATA_HOME/timeslay/slack`).
The service uses an absolute Node path and the installed `timeslay` binary;
it does not need a shell, an open terminal, or this checkout after installation.
It starts with the systemd user manager at login, independently of the GUI.

Credentials and the rotating token cache live under
`~/.config/timeslay/slack/` (or `$XDG_CONFIG_HOME/timeslay/slack/`):

- `.env`: Slack tokens, client ID, and client secret.
- `.slack-app-tokens.json`: the current access/refresh pair; retain it across
  restarts because the original refresh token is consumed after use.

On first installation, the installer copies `slack/.env` and the latest
`slack/.slack-app-tokens.json` from the checkout if present. Later installs
preserve the installed credentials and cache. Stop any manually started
checkout bridge before installing; use the service afterward so only one
process owns the rotating tokens. For a fresh setup, fill in the installed
`.env.example` and save it as `.env`; the service waits until `.env` exists.
The service sets `TIMESLAY_BIN` and `SLACK_APP_TOKEN_FILE` itself.

```sh
systemctl --user status timeslay-slack.service
systemctl --user restart timeslay-slack.service
journalctl --user -u timeslay-slack.service -f
```

`--no-enable` installs the service without enabling or starting it.
Uninstalling removes the service and runtime; credentials and token state
remain unless `packaging/uninstall.sh --purge` is used.

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

Widths are in logical pixels. If `calendar_width` is omitted, the side panel
uses the `[panel]` width. The margin defaults
to 8 and leaves transparent space along the panel's screen edge, top and bottom
(mirrored for a right-side panel). It also applies while the time block pane is
open.

In Weekly, the panel temporarily widens to fit the configured day columns
and restores its usual width when you leave the view. If the columns exceed
the screen, scroll horizontally. On Wayland, the compositor decides the
available window size and placement.

In Daily, clicking a calendar event or a Google Task opens its details in a pane
on the right rather than over the view: the window widens by `detail_width`
beside the calendar or backlog. When the time block editor is open too, the
details pane sits to the right of it. `detail_width` is the pane's default width.

`timeslay --sidebar` opens **TimeSlay itself** as a slim panel — 200px
wide by default — down the **left** edge of the screen. It is a separate
instance of the real application, not a second, cut-down view: the same clock,
calendar, todos and settings, just in a narrow window.

### Shortcuts

The shortcuts are owned by `timeslay --daemon`, a small process that does
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
systemctl --user status timeslay-shortcuts.service
systemctl --user restart timeslay-shortcuts.service
journalctl --user -u timeslay-shortcuts.service -f
```

Its settings live in the `[panel]` table of `~/.config/timeslay/config.toml`
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

### Panel placement and space reservation

On Wayland the side panel is an Iced layer-shell surface, anchored directly
by the compositor. It needs no KWin position rule. The ordinary application
window continues to use the standard Iced window backend. On X11 the panel
uses that backend too, and the installer supplies its legacy KWin rule.

In Settings, **Dock → Reserve screen space** and **Side panel → Reserve
screen space** independently choose whether each surface reduces the work
area available to maximized/tiled windows. Both default to off, preserving
the overlay behavior. These reservations require Wayland layer-shell support.

Changes apply to a visible panel immediately and to a visible dock after the
settings file is saved. Hiding a surface releases its reservation; showing it
again uses its own saved setting. The dock reserves only its visible strip
and the margin from its anchored edge, never its transparent hover/task-card
area. The panel reserves its body, including inline panes and screen gutter;
widget tooltips and popup overlays do not add reserved space. This is desktop
work-area reservation, not a native shared tiling divider or a guarantee that
floating/fullscreen windows cannot overlap it.

To run the shortcut daemon without installing it, launch it directly:

```sh
cargo run --release -- --daemon
```

## Dock strip

`timeslay --dock` is a much thinner companion than the panel: a thin line
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
- The **+** at the end of the strip opens a task form beside it. Add a title
  and optional deadline or due date, then click **Add task** (or press Enter)
  to save to Inbox. Escape or Cancel dismisses the form.
- Hover cards are click-through. The task form accepts pointer and keyboard
  input while open, including Tab to move between fields and Ctrl+V to paste.

Switching modes: in the side panel, open the calendar **Menu** and press
**Dock to screen edge**. That starts the dock and hides the panel. The small
square at the very top of the dock brings back the panel and closes the dock.

```sh
timeslay --dock
cargo run --release -- --dock   # from a checkout
```

The **Toggle Shape** shortcut switches between the panel and the dock without
going through the menu.

It reads `$XDG_CONFIG_HOME/timeslay/dock.toml` at start-up; every key is
optional:

```toml
popup_width = 300    # hover card width
popup_gap = 6        # space between strip and card
```

The strip's edge, width, left/right margin, and its two borders (each 0–5px
with its own colour) are set from **Settings → Dock** and apply the next time
the dock opens.

It shares the main app's config, theme, sign-in and day cache, and syncs today
itself every `sync_minutes`.

## Google Keep browser helper

Build `tools/keep-bridge` with `deno task compile` before installing to include
Google Keep support. The installer copies the compiled
`keep-bridge` alongside the application; the uninstaller removes it. The app
reuses the dedicated Google browser profile. See
[the helper README](../tools/keep-bridge/README.md#google-keep) for sign-in,
runtime permissions, and the read-only Keep integration.

### Multiple Slack workspaces

The bridge supports one unlisted Slack app connected to multiple workspaces.
Follow the [multi-workspace setup](../slack/README.md#connect-multiple-workspaces-with-one-app)
to configure its HTTPS OAuth endpoint and authorize each workspace. The service
keeps workspace bot tokens and permitted user IDs in
`~/.config/timeslay/slack/.slack-installations.json`, separately from its
app-level token cache. Installation copies a development installation file only
when the destination does not exist; upgrades preserve it. Ordinary uninstall
preserves credentials; `--purge` removes them.

## Embedded location map

Every location search offers **Choose on map**. It opens Mapbox GL JS in
`iced_webview_v2` using Chromium (CEF). Search suggestions fly to a place;
clicking the map resolves a nearby address or named feature. **Confirm** becomes
available after a valid place is selected. **Back to search** or Escape cancels
without changing the field. Home is the initial map center and search bias;
panning updates search proximity to the area being explored.

Set a Mapbox public access token in Settings. Maps and search require internet
access and a graphics driver supporting WebGL2. The first build downloads CEF;
the installed runtime is considerably larger than the TimeSlay binary.
`install.sh` copies the complete CEF runtime to `PREFIX/lib/timeslay/cef`.
For a custom build/runtime location, set `CEF_PATH` at build time and
`TIMESLAY_CEF_DIR` at install/runtime. Mapbox attribution remains visible.
