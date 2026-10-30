### Add links from a browser or Slack

The `timeslay` binary doubles as a native-messaging host (`timeslay
--webextension`), which the browser extension and the Slack app both use to add
the current page or a Slack message to the inbox.

- Browser extension: [`webextension/`](./webextension) — load it unpacked and
  run `./webextension/install.sh` to register the native host.
- Slack app: [`slack/`](./slack) — a socket-mode Bolt app run without a build
  step (`npm start`).

When a TimeSlay window, panel or dock is running it writes the task and shows it
immediately; otherwise the host writes straight to Google Tasks.

## Inbox timestamps and sorting

New tasks created through TimeSlay (including browser and Slack additions) store
an immutable UTC creation timestamp as `tt-created_at` in their Google Tasks
metadata. Google supplies the last-modified timestamp on sync. The Inbox shows
both in local time and offers oldest/newest-first sorting for each, plus manual
order. The selected sort is saved in settings. Unknown timestamps sort last in
both directions; tasks created outside TimeSlay and older tasks have no recorded
creation time, because Google Tasks does not expose one.

## Remote metadata and migration

SQLite remains local (`tasks.sqlite3`), caching task metadata and allocating IDs.
Google is the source of truth after migration: Calendar uses
`extendedProperties.private`; Tasks uses a versioned footer at the end of notes:

```text
normal thing
---
tt-version: 1
tt-id: 100
tt-legacy_key: null
tt-deadline: null
tt-block: null
tt-ignored: false
tt-minutes: 45
tt-order: 0
```

Values are JSON so lists, strings, booleans, and scheduling links round-trip.
Projects, labels, priority, location, and schedule are included when set.
Human notes stay above the separator. Existing work-history keys are preserved.
Task metadata is uploaded on edit and at scheduling checkpoints; fresh syncs
rebuild the SQLite cache from remote metadata instead of overwriting it.
Oversized, malformed, and unsupported-version footers fail without replacement.

**Temporary backwards compatibility:** search for `LEGACY_METADATA_COMPAT`.
On sync, tasks without a footer export their existing SQLite values (or older
`tt-timeblocking:`/`ttid:` notes). Old `DUE:`/`DEADLINE:` lines move into the footer.
Calendar `Type:` footers move into private properties. Failed exports are retried
on the next sync; legacy sources are retained until the remote write succeeds.
Once migration support is no longer needed, remove the marked legacy readers,
SQLite fallback branch in `calendar::task_metadata::load`, and Calendar footer
migration helper. Keep the cache, allocator, aliases, and versioned footer codec.
The shared core crate also supplies these rules to the Dioxus app.

## Cached views and automatic sync

Day, week, and month navigation reads the local cache immediately and never
starts a Google refresh. A request already in progress can fill the cache for
the view it fetched without navigating back or starting another request.
Dates without cached data stay empty until a scheduled or manual refresh.

Automatic refresh defaults to **one hour**. In **Settings → Google Calendar →
Automatic sync**, choose an interval from one minute to 24 hours. Existing saved
intervals are preserved. The last successful refresh time and calendar source
names persist with the event cache, so restarting with fresh data avoids an
immediate refresh. Manual sync and writes you explicitly make still run immediately.
The Dioxus day view follows the same cache and interval policy.

SQLite remains the task metadata cache, ID allocator, and legacy migration
source. Its small schema is retained while migration is supported; event
snapshots continue to use the existing month files. No additional database or
migration is needed for the new refresh policy.

