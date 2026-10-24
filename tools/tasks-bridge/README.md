# tasks-bridge

Reads the **time of day** of a Google Task out of the browser, because the
Google Tasks API only ever returns a date: `due` is midnight UTC, and the app
currently turns that into an all-day chip.

The Rust app keeps the **date** and the **task id** from the API. This helper
supplies only the **time**. Neither side guesses at the other's timezone.

## Why a browser at all

The Tasks API has no usable time field. The time exists only in the UI, so the
only way to read it is the UI. We use a real Chrome profile the user signs into
by hand. We do not script a login, do not export a portable credential file, and
do not open a debugging port.

## Requirements

- Deno 2.x
- Google Chrome installed. `status`/`dump`/`fetch` launch it via Playwright's
  `chrome` channel; `signin` spawns it directly. Either way nothing is
  downloaded and the browser is a real one.

## Commands

| Command | What it does | Privilege |
| --- | --- | --- |
| `deno task signin` | Opens a **plain** Chrome (not Playwright-driven) on the dedicated profile. Sign in, wait for Tasks to load, then close the window. Playwright then verifies the session. | headed, user present |
| `deno task status` | Headless. Prints `{ signedIn, state, url, cookies, googleCookies }`. Exit 3 if not signed in. | headless |
| `deno task dump` | Headless. Writes discovery artifacts for a signed-in session (see below). | headless |
| `deno task fetch` | Headless. Prints per-task time-of-day as JSON. | headless |
| `deno task selftest` | Launches Chrome and prints its version. Proves the driver works. | headless |

Build a single binary:

```
deno task compile   # -> ./tasks-bridge
```

## Wiring into TimeKeeper (the Rust app)

The app calls the helper through `src/calendar/bridge.rs`. On each calendar sync
it runs `tasks-bridge fetch`, parses the JSON, and promotes matching all-day
task chips to timed events.

**Build it where the app can find it:**

```
cd tools/tasks-bridge
deno task compile          # -> tools/tasks-bridge/tasks-bridge
```

The app looks for the helper in this order and uses the first that runs:

1. `$TIMETRACKER_TASKS_BRIDGE` (explicit path)
2. next to the application binary (`tasks-bridge`) — the install layout
3. `tools/tasks-bridge/tasks-bridge`, found by **walking up from the binary**
   (`target/debug`, `target/release` and `target/debug/deps` all resolve) — the
   checkout layout. This deliberately does not depend on the working directory:
   a launcher or an IDE does not run with the crate root as the CWD.
4. `tools/tasks-bridge/tasks-bridge` relative to the working directory
5. `tasks-bridge` on `PATH`

The helper binary is **not** committed (see `.gitignore`).

**No window, ever, during a sync.** `fetch` and `status` are headless, and the
app removes `TASKS_BRIDGE_HEADED` from the child's environment so an exported
value cannot make a browser appear. Only `signin` shows a window, on purpose.

Rust API in `src/calendar/bridge.rs`:

| Function | Purpose |
| --- | --- |
| `promote_task_times(&mut Snapshot)` | Called by `sync_day`; best-effort, never fails a sync. |
| `signin() -> Result<(), String>` | Blocking. For the "Sign in to Google Tasks" button. |
| `signed_in() -> bool` | Blocking. Whether the profile has a session. |

Behaviour when things are missing — all of it non-fatal, tasks stay all-day:

- the helper is not installed → skipped, logged at debug;
- the profile is not signed in (exit 3) → skipped, logged at info;
- the helper errors → logged at warn;
- no task lists, or no all-day tasks on the day → the browser is not started at all.

**Matching.** The DOM only exposes the web app's internal id, so a row is matched
to a task by `(list name, title)`, lower-cased. If two rows disagree on the time
for the same pair, no time is trusted and the task stays all-day.

**Block length.** Google Tasks stores a point in time, not a duration, so a
scheduled task is drawn as a 30-minute block: `TASK_BLOCK_MINUTES` in
`src/calendar/bridge.rs`.

## Runtime permissions

The helper uses `--allow-all` for running and compiling. Deno 2.9.7 rejects
Playwright 1.63's WSL detection probe of `/proc/sys/fs/binfmt_misc/WSLInterop`
under the previous individual grants, even with an explicit read grant. The
older minimal-permission recipe was verified on Deno 2.7 and no longer works
with this runtime. This helper therefore runs as trusted local code with the
same access as its Chrome subprocess; it must only run from a trusted checkout.

## Security

- The profile lives at `$XDG_DATA_HOME/timetracker/tasks-bridge/chrome-profile`
  (`~/.local/share/...` by default). It holds live Google session cookies and is
  a credential in its own right: keep it out of the repo and out of any backup
  you do not control. `chmod 700` it.
- No password is ever handled. No debugging port is opened.

## Verified findings (this machine, Chrome 154, Deno 2.7.14)

- **A signed-out `tasks.google.com` serves a document titled "Google Tasks"
  whose body is the Google sign-in form.** So URL *and* title checks give false
  positives — an early version reported `signedIn: true` on a brand-new profile.
  Auth is therefore decided by DOM evidence: the `#identifierId` email field, or
  the `accounts.google.com` host. Confirmed against a real signed-out page.
- Playwright runs **in-process** under Deno, and from a `deno compile`d binary —
  no driver re-exec problem. Verified by `selftest` from the compiled artifact.
- Chrome resolves at `/opt/google/chrome/chrome` via the `chrome` channel.

## The sandbox, and the `--no-sandbox` flag

`--no-sandbox` is **not** a valid flag here. It is not a Deno permission, not a
`deno compile` option, and passing it to Playwright's `args` fights the library
over a flag Playwright manages itself. Playwright's own error text names the
supported switch: `chromiumSandbox: false`, after which Playwright adds
`--no-sandbox` internally.

So the supported way to turn the sandbox off is:

```
TASKS_BRIDGE_NO_SANDBOX=1 deno task dump
```

You should not need it: Chrome's sandbox works on a normal desktop. It exists
for containers and CI where the SUID sandbox helper is unavailable.

## "Couldn't sign you in. This browser or app may not be secure."

This is Google's anti-automation refusal, and it is expected when the sign-in
page is loaded by a Playwright-driven browser. `selftest` prints the two signals
Google reads:

```
{"chrome":"154.0.8037.57","webdriver":true,"ua":"… HeadlessChrome/154.0.0.0 …"}
```

`navigator.webdriver` is `true` and the user agent says `HeadlessChrome`.

Two things were worth verifying rather than assuming:

- Stripping the automation arg (`ignoreDefaultArgs: ["--enable-automation"]`)
  does **not** clear `navigator.webdriver` in Playwright 1.63 — it still reports
  `true`. So flag-stripping is not a fix and the tool does not pretend it is.
- The block is on **sign-in**, so the fix is to keep sign-in out of an automated
  browser entirely. `signin` therefore spawns a plain `google-chrome` with
  `--user-data-dir=<profile>` and no automation at all. The human signs in a
  normal browser; Playwright only takes over afterwards, to read a session that
  is already authenticated.

Override the Chrome binary with `TASKS_BRIDGE_CHROME` if it is not on `PATH`.

## "Still not signed in (signed-out)" right after signing in

This was a **cookie-encryption mismatch**, and it is the bug that survives the
fix above.

Playwright launches Chromium with `--password-store=basic` — it is in its
default args, next to `--no-first-run`. That encrypts cookies with a hardcoded
key. A normal desktop Chrome instead derives the key from the OS keyring
(gnome-keyring / kwallet). **The two keys are mutually unreadable**, so the
session a plain Chrome writes is invisible to Playwright: the sign-in works, the
profile has the cookies, and Playwright still sees an empty jar.

Measured by loading `google.com` in a plain Chrome and then reading the same
profile back with Playwright:

```
{"label":"keyring","total":0,"google":0}
{"label":"basic","total":3,"google":3}
```

Same profile, same page — only the password store differs.

**Fix:** `signin` now launches Chrome with `--password-store=basic
--use-mock-keychain`, matching Playwright exactly.

**You must re-run `deno task signin`.** Cookies already in the profile were
written with the keyring key and cannot be read under `basic`, so the old
session has to be recreated.

**Trade-off:** with `basic`, cookies are encrypted with a well-known key rather
than one protected by the OS keyring. The profile is already a credential and is
dedicated to this tool — keep it `chmod 700` and out of backups you do not
control. This is the same trade-off Playwright makes for every Chromium launch.

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `TASKS_BRIDGE_CHROME` | `google-chrome` | Chrome binary used by `signin`. |
| `TASKS_BRIDGE_NO_SANDBOX` | off | `1` sets `chromiumSandbox: false`. |
| `TASKS_BRIDGE_PROFILE` | data dir | Persistent profile directory. |
| `TASKS_BRIDGE_DUMPS` | data dir | Where `dump` writes. |
| `TASKS_BRIDGE_ROW_SELECTOR` | `[role="listitem"]` | Task-row selector for `fetch`. |
| `TASKS_BRIDGE_TIMEZONE` | machine zone | Pins the browser timezone. |
| `TASKS_BRIDGE_URL` | `https://tasks.google.com/` | Override the target URL. |
| `TASKS_BRIDGE_HEADED` | off | `1` runs `fetch` headed. |

## Output contract

`fetch` writes JSON to stdout, diagnostics to stderr:

```json
{
  "selector": "[role=\"listitem\"]",
  "count": 75,
  "withTime": 7,
  "tasks": [
    {
      "list": "Daily Agenda",
      "section": "Active tasks",
      "taskId": "YUp0pXj0a64pIu2X",
      "title": "Get small pen from bag",
      "schedule": "Scheduled for Today, 7:00 PM",
      "dateText": "Today",
      "time": "19:00",
      "ambiguous": false
    }
  ],
  "warnings": []
}
```

- `time` is **time-of-day only**, deliberately. The app already has the date
  from the API, so the two combine into a local datetime with no timezone
  guesswork.
- `time: null` means the UI showed no time (date-only or unscheduled) — **leave
  the task all-day**. `ambiguous: true` (a time with no AM/PM) is also treated as
  no time by the caller.
- `section` is `Active tasks` or `Completed tasks`. `dateText` is the raw phrase
  (`Today`, `2 days ago`, `Thursday, October 8, 2026`).
- Exit **3** = not signed in (show "Reconnect"). Exit **2** = error or timeout.

## The browser DOM, as measured

Established from a real signed-in `dump`, not guessed:

| Thing | Where it lives |
| --- | --- |
| Task row | `[role="listitem"]` inside `[role="list"]` |
| List name | nearest `<h1..h3>` preceding the containing list |
| Active vs completed | the list's `aria-label` (`Active tasks` / `Completed tasks`) |
| **Schedule** | a child with `aria-label="Scheduled for <date>[, <time>]"` |
| Title | a child with `data-aria-label="Task title"`, text in `data-rendered-value` |
| Task id | `data-task-id` — 16 chars, the web app's internal id |

There is **no `<time>` element and no `datetime` attribute**; the time exists
only in the schedule label's accessible name.

Label shapes seen on the day of writing (today = 2026-10-02):

```
Scheduled for Today, 7:00 PM                     -> 19:00
Scheduled for Today, 9:30 AM                     -> 09:30
Scheduled for Thursday, October 8, 2026, 6:00 PM -> 18:00
Scheduled for 2 days ago                         -> no time
Scheduled for Monday, October 5, 2026            -> no time
```

On that session `fetch` returned 75 rows, 7 with times, and zero parse warnings.

## The join with the API (important)

The DOM's `data-task-id` is a **16-character internal id** (e.g.
`YUp0pXj0a64pIu2X`), not the long base64 id the Tasks API returns. The API id
cannot be recovered from the DOM, so the join is:

`(list name, task title)`, with `dateText` as a tiebreak when one list has
duplicate titles.

That is a real limitation: two tasks with the same title in the same list on the
same day are indistinguishable. `taskId` is still emitted in case a mapping turns
up later.

The web page also embeds Google's internal data blob, which does carry the
scheduled timestamp and timezone — but it is undocumented and churns silently, so
this tool reads the accessibility labels instead.

## `dump` is kept for re-verification

If Google restyles Tasks, re-run `deno task dump`. It writes
`~/.local/share/timetracker/tasks-bridge/dumps/` (`aria.txt`, `page.html`,
`candidates.json`) so the shapes above can be re-checked against reality rather
than assumed.

## Google Keep

The same binary, launcher, dedicated profile and manual sign-in support Keep:

- `deno task keep-fetch` emits `{ notes, warnings }`, reading active and
  archived notes (excluding Trash).
- `deno task keep-signin` opens ordinary Chrome on Keep, then verifies the
  session after the user closes it.
- Rebuild with `deno task compile` after upgrading; an older helper does not
  recognize Keep commands. Copy the rebuilt binary alongside `timetracker`
  for an installed app, or set `TIMETRACKER_TASKS_BRIDGE`.

All commands acquire an OS file lock next to the profile, so Tasks and Keep
cannot launch competing browser processes, including across app instances.
Only sign-in shows a browser window. A complete Keep read opens each note's
viewer without typing or changing note properties, captures its full rendered
content and direct `#NOTE/...` URL, then closes the viewer. Notes are identified
by that URL, so duplicate titles remain separate. Large collections take longer
because Keep exposes note URLs through the viewer. Previously shared cards
that Keep itself will not open are retained with a warning and a link to the
containing Keep page.

`keep.ts` uses selectors measured against the signed-in Keep UI. Lazy-loaded
cards are scrolled to a stable end, and broken selectors or timeouts fail the
refresh rather than silently replacing the last snapshot with partial results.
Google can change this interface; failures should be reported and reverified.

The native screen searches title/body text locally with substring matches before
word-level fuzzy matches (one edit for 4–7 characters, two for longer words,
including transpositions). Labels and Active/All/Archived filters intersect with
the query. An empty query preserves pinned-first browser order. It refreshes on
entry and via Refresh, with no periodic browser launches.

Each note's original preview preserves the actual rendered colors, background
art, rich text, checklist state, media, reminder/sharing chips and timestamps.
Readable text offers wrapping text and metadata in narrow windows. Preview
images and extracted text live in application memory, never in the settings
file or a persistent notes cache. The previews are read-only; Edit opens that
specific note in the default browser using the app's existing URL launcher.
Sign into the matching account in that browser if necessary. Media playback,
editing and note actions remain on Google's website.
