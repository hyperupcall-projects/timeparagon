# TimeTracker — Dioxus day view

This is the Dioxus implementation of TimeTracker. It deliberately implements
one screen: **the day**. Everything on it comes from the same Google Calendar
and Google Tasks data the iced application uses.

The iced application is still the root package of this repository and is
untouched. Both apps share data through the renderer-agnostic
`timetracker-core` crate:

- the config at `~/.config/timetracker/config.toml`,
- the OAuth token at `~/.config/timetracker/google-token.toml`,
- the local task database and time-block metadata,
- the calendar sync and Google Tasks code in `core/` / `src/calendar/`.

`timetracker-core` contains no iced UI, which is what lets it cross-compile for
Android.

## Run it

```sh
cargo run -p timetracker-dioxus
```

Dioxus desktop needs the usual WebKitGTK and GTK 3 development packages on
Linux (`webkit2gtk-4.1`, `gtk+-3.0`), plus `libxdo-dev` for Dioxus's native menu
dependencies.

The iced app still builds and runs with the normal command:

```sh
cargo run -p timetracker --release
```

## Android APK

```sh
./scripts/build-android.sh
```

Useful options:

```sh
# Emulator build
./scripts/build-android.sh --abi x86_64

# Release profile, signed with your own keystore (see below)
TIMETRACKER_KEYSTORE=~/keys/timetracker.jks \
TIMETRACKER_KEYSTORE_PASSWORD=... \
TIMETRACKER_KEY_ALIAS=timetracker \
./scripts/build-android.sh --release

# Build, install and launch on a connected device
./scripts/build-android.sh --install
```

The APK is copied to `dist/timetracker-<abi>-<profile>.apk`.

The default (debug) APK is debuggable and debug-signed: use it for development
only. `--release` builds Gradle's release variant, signs it with the keystore
named by `TIMETRACKER_KEYSTORE`, `TIMETRACKER_KEYSTORE_PASSWORD`,
`TIMETRACKER_KEY_ALIAS` and (optionally) `TIMETRACKER_KEY_PASSWORD`, and fails
unless the result is non-debuggable and not signed with the Android debug key.
Create a keystore once with `keytool -genkeypair -v -keystore
~/keys/timetracker.jks -alias timetracker -keyalg RSA -keysize 4096 -validity
10000` and keep it outside the repository. Passwords must avoid the characters
`" ' \` $ \ & < > =`. The signing block is added to `dioxus/Dioxus.toml` only
while the build runs.

The manifest is `dioxus/AndroidManifest.xml`. It disables Auto Backup. Android
tokens are encrypted with AES-256-GCM using a non-exportable Android Keystore
key. Only ciphertext is stored in `getNoBackupFilesDir()`, which also excludes
tokens from device-to-device transfer independently of manufacturer backup
behavior. Plaintext tokens from older installations are migrated after the
encrypted replacement has been saved, then removed along with old temporary
files. Encryption/decryption failures never fall back to plaintext storage;
if the key becomes unavailable, connect Google again. Desktop token storage
continues to use the existing private token file.

Release verification checks that the APK's signer matches the configured
keystore certificate, and fails if APK inspection or signature verification
fails. Keystore settings must be single-line values. Signing configuration
and generated files are restricted to the build user while building, and
concurrent builds are rejected while signing settings are in use.

### Build requirements

- Android SDK with `platform-tools`, `platforms;android-34` and
  `build-tools;34.0.0`
- An Android NDK (`ANDROID_NDK_HOME`, `NDK_HOME`, or `$ANDROID_HOME/ndk/...`)
- Java 17 to 21 for `--release` (the Android Gradle plugin's lint fails on JDK 24+); any
  Java 17+ for debug builds
- Rust Android targets (installed by the script with `rustup` when needed)
- `dx` from Dioxus CLI 0.7:

  ```sh
  cargo binstall dioxus-cli@0.7.10
  ```

The Android build does **not** compile iced or any desktop-only crates; it uses
`timetracker-core` only.

### Config on the device

Nothing is embedded in the APK: no config, client ID or secret. On first
launch the app asks for the Google OAuth client ID and secret and saves them to
its private storage (`config.toml`, readable only by the app):

```text
/data/user/0/dev.timetracker.day/files/config/timetracker/config.toml
```

After **Save and connect**, sign in with Google in the browser. The rest of the
config (calendars, task lists) arrives from Google Drive after sign-in, if you
have configured it in the iced app.

The app sets `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` to
its Android internal storage before starting, so the shared data layer uses
the same relative layout as it does on desktop.

### Config sync through Google Drive

Both apps keep `config.toml` in step through Google Drive's hidden **app data
folder** (the `drive.appdata` scope). The folder is private to the app: it does
not appear in Drive and the app cannot see any of your other files. The sync is
`src/config_sync.rs`, shared by the iced app and this one.

- `config.toml` is watched; an edit is uploaded a couple of seconds after it
  settles. Drive is polled every 30 seconds, and a newer copy replaces the
  local file (the iced app and the day view both reload it).
- If only one side changed since the last sync, that side wins. If both did,
  or on the first sync after launch, the **newest timestamp wins**: the file's
  modification time against Drive's `modifiedTime`. An upload stamps the Drive
  copy with the local time and a download stamps the local file with Drive's,
  so syncing does not make either side look newer.
- An **empty** config (missing, blank, or only comments) never replaces
  another one, and a config that does not parse is neither uploaded nor
  overwritten.
- A config file stamped with the epoch is treated as a placeholder, so a config
  already in Drive replaces it.

Google Drive must be enabled for the OAuth client's Cloud project, and the
token needs the Drive permission. Existing sign-ins predate it: disconnect and
**Connect Google** once on each device. The day view's header shows the sync
state. Every device must use the same OAuth client ID, since an app data
folder belongs to one client.

### Mobile layout

At widths of 820px or less, **Calendar** and **Tasks** tabs replace the side-by-side
layout, with Calendar selected by default. The header and tabs stay visible while
the selected pane scrolls independently. All-day events appear in Calendar, and
switching tabs preserves each pane's scroll position. The calendar opens at the
current time (or 8am on another day), and the time-block popup becomes a bottom sheet.

## What the day view shows

- **All-day events** from every visible calendar, in the left rail.
- **Every Google Task for the day**, grouped by task list. Completed and
  ignored tasks stay visible with their status, estimate, and a link to the
  time block they belong to.
- **The timed grid** with calendar events and time blocks, using the same
  overlap packing and end-of-day boundary as the iced app.
- A live **current-time needle**, automatic scroll to now, day navigation,
  manual refresh, and a refresh on the configured sync interval.
- Clicking a **time block** opens a popup listing **every task assigned to
  that block**, in planned order, with each task's list, status, estimate,
  and the block's total planned time against its capacity.

Time blocks are read from `calendar.time_block_calendar`. A block referenced by
a task's metadata is still recognized if that setting is later cleared.

## Notes

- Only the Dioxus crate enables the desktop/mobile renderers; the root
  workspace defaults to the iced package (`default-members = ["."]`).
- The day view has no settings screen. On Android it only asks for the Google
  client ID and secret. Configure the time-block calendar, task lists, and
  visible calendars in the iced application; this crate reloads the same config
  at startup.
