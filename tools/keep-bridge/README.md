# keep-bridge

Google Keep's optional browser helper. Google Tasks and Calendar sync use the
public APIs and do not launch this executable.

## Google Keep

Requires Deno 2.x and an installed Google Chrome.

```sh
cd tools/keep-bridge
deno task compile
deno task keep-signin
deno task keep-fetch
```

The compiled executable is `keep-bridge`. The desktop installer places it
beside TimeSlay and removes the previous helper executable name.

`keep-signin` opens a normal Chrome window. Sign in to Google Keep and close
the window; the helper then verifies that Keep loads. `keep-fetch` runs
headlessly. Task browser commands are not supported.

New profiles use
`$XDG_DATA_HOME/timeslay/keep-bridge/chrome-profile` (or the equivalent
`~/.local/share` path). If that profile does not exist, an existing
`timeslay/tasks-bridge/chrome-profile` is reused in place, preserving
sign-in and the shared profile lock. No profile or cookies are moved or deleted.

| Variable | Meaning |
| --- | --- |
| `TIMESLAY_KEEP_BRIDGE` | Explicit helper executable path. |
| `KEEP_BRIDGE_CHROME` | Chrome executable for Keep sign-in. |
| `KEEP_BRIDGE_PROFILE` | Override the persistent browser profile. |
| `KEEP_BRIDGE_TIMEZONE` | Override the browser time zone. |
| `KEEP_BRIDGE_NO_SANDBOX` | Set to `1` when the host requires Chrome's sandbox to be disabled. |

The older `TIMESLAY_TASKS_BRIDGE` and `TASKS_BRIDGE_*` names remain
fallback aliases. New names take precedence. An explicit executable path must
point to an existing executable; update it if it points into the old source
directory.

Chrome sign-in and Playwright use the same password-store flags so the helper
can read the dedicated profile's session. Live profiles and cookies must not
be committed. See `keep.ts` for note extraction and metadata handling.
Failed refreshes preserve the app's last successful Keep snapshot.
