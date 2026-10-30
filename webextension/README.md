# TimeSlay browser extension

The toolbar button opens a small form with **Add to inbox**. The title is
prefilled with the page's title, and you can edit it and add a description, a
**due date** and a **deadline** (a day, with an optional time) before sending.
The page's URL is always attached to the task.

The toolbar and extension icons use the same hourglass artwork as the Iced
application, resized from `../assets/noto-hourglass-done.png` to 16, 32, 48 and
128 pixels. Reload the extension after updating its icons.

The popup is painted with the colors of the theme the TimeSlay app is currently
using: it asks the native host for them (the `theme` command) and applies them
as CSS variables, so it matches the app even after you switch themes.

The form sends the task to the `time-slay --webextension` native host. The host
reads a Gmail link for its subject (the same way a pasted link is read in the
app), then asks a running TimeSlay window, panel or dock to write the task. If
no TimeSlay process is running, the host writes to Google Tasks itself.

The popup shows **Success!** with the new task, or the error and the complete
debug trace if anything fails.

## What each field becomes

| Field | Stored as |
| --- | --- |
| Title | The task's title (the page title is prefilled; a Gmail link's subject replaces a blank or bare-link title) |
| Description | The task's notes, with the page's URL appended |
| Due date | Google Tasks' `due` day |
| Deadline | A `DEADLINE:`/`DUE:` line in the notes (Google Tasks has no deadline field); day only, or day and time |

## 1. Build TimeSlay

```sh
cargo build --release
```

## 2. Load the extension

### Chrome, Chromium, Brave, Edge, Vivaldi

1. Open `chrome://extensions` (or the equivalent page).
2. Turn on **Developer mode**.
3. Choose **Load unpacked** and select this `webextension/` directory.

The `key` in `manifest.json` pins the extension ID to
`ebhljjjdkfmgagkochllhjanjncnedlo`, so the native host manifest can name it
without asking Chrome for the ID.

### Firefox

1. Open `about:debugging#/runtime/this-firefox`.
2. Choose **Load Temporary Add-on** and select `manifest.json` in this
   directory.

Firefox identifies the add-on as `timeslay@example.org`.

## 3. Register the native host

```sh
./webextension/install.sh
```

It writes:

- a wrapper at `~/.local/share/timeslay/time-slay-webextension` that runs
  `time-slay --webextension` (browsers cannot pass arguments to a native host);
- `com.timeslay.webextension.json` under the Chrome-family
  `NativeMessagingHosts` directories and under
  `~/.mozilla/native-messaging-hosts`.

If the binary is not on `PATH`, set `TIMESLAY_BIN`:

```sh
TIMESLAY_BIN="$PWD/target/release/time-slay" ./webextension/install.sh
```

Restart the browser after installing the host manifest. Chrome reads native
host manifests at startup.

## How it fits together

```
popup.js
  ├─ chrome.runtime.sendNativeMessage("com.timeslay.webextension", {command:"theme"})
  │    └─ time-slay --webextension          (the app's chosen theme as hex colors)
  └─ chrome.runtime.sendNativeMessage("com.timeslay.webextension", {command:"add-to-inbox", …})
       └─ time-slay --webextension          (length-prefixed JSON on stdin/stdout)
            ├─ integrations::recognize/fetch  (Gmail subject)
            ├─ ipc::WINDOW / SIDEBAR / DOCK   (running app writes the task)
            └─ inbox::add                     (fallback when the app is closed)
```

The native host only talks to the main binary for the write; it never opens a
window or changes visibility. The main binary still owns the desktop panel and
dock.

## Troubleshooting

- **"Specified native messaging host not found"**: run `install.sh`, check that
  the manifest path exists, and restart the browser.
- **"Access to the specified native messaging host is forbidden"**: the
  extension ID in `native-messaging-host.chrome.json` does not match the loaded
  extension. The pinned ID is `ebhljjjdkfmgagkochllhjanjncnedlo`; if you
  replaced the `key`, pass `CHROME_EXTENSION_ID=...` to `install.sh`.
- **The popup shows an error and a trace**: the trace names every stage,
  including which IPC socket was tried and what Google answered. Copy it with
  the **Copy debug info** button.
