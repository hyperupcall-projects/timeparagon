# TimeTracker

TimeTracker has two implementations that live side by side:

| Crate | UI | Status |
| --- | --- | --- |
| `timetracker` (workspace root) | [iced](https://iced.rs) | The complete application, unchanged |
| `timetracker-core` | none | Shared calendar, OAuth, tasks and time-block data layer |
| `dioxus` (`timetracker-dioxus`) | [Dioxus](https://dioxuslabs.com) | Day view only |

Both UIs use the same Google Calendar and Google Tasks data, the same OAuth
token, the same config file, and the same local task database.

## Run

```sh
# The original iced application
cargo run -p timetracker --release

# The Dioxus day view
cargo run -p timetracker-dioxus
```

The iced package remains the workspace's default member, so a plain `cargo run`
still launches the original app.

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
