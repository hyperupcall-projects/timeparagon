# TimeSlay

Slay the day with TimeSlay.

TimeSlay has two implementations that live side by side:

| Crate | UI | Status |
| --- | --- | --- |
| `time-slay` (workspace root) | [iced](https://iced.rs) | The complete application, unchanged |
| `time-slay-core` | none | Shared calendar, OAuth, tasks and time-block data layer |
| `dioxus` (`time-slay-dioxus`) | [Dioxus](https://dioxuslabs.com) | Day view only |

Both UIs use the same Google Calendar and Google Tasks data, the same OAuth
token, the same config file, and the same local task database.

## Run

```sh
# The original iced application
cargo run -p time-slay --release

# The Dioxus day view
cargo run -p time-slay-dioxus
```

The iced package remains the workspace's default member, so a plain `cargo run`
still launches the original app.

## Daily and weekly calendar

The iced app has **Daily** and **Weekly** tabs. Weekly starts with the day
before the selected date, followed by seven or eight days beginning with that
date. Use the month picker to select a date; hovering a date previews the
displayed range. The arrows move to the previous or next range. The window
widens to fit the columns where the screen allows, then returns to its normal
width when you leave Weekly. Scroll horizontally if the columns exceed the
available screen width, and vertically to move through time.

Calendar settings control the number of weekly days and each column's width.
Event, task, and time-block details open over neighboring columns, leaving the
selected day visible.

## Android

The Dioxus day view also builds as an Android app:

```sh
./scripts/build-android.sh
```

The script produces a debug-signed APK in `dist/` (arm64-v8a by default). It
embeds no config or credentials: the app asks for the Google client ID and
secret on first launch. `--release` needs your own signing keystore. Use
`./scripts/build-android.sh --help` for ABI, release and install options.

The Android build needs the Android SDK, an NDK, Java 17+, the Rust Android
targets, and `dx` (Dioxus CLI 0.7). The script checks for these and prints
install commands when something is missing.

See [`dioxus/README.md`](dioxus/README.md) for what the day view includes and
how the mobile build is wired up.
