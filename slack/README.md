# TimeSlay for Slack

A small Slack app that adds a message to your TimeSlay inbox. It talks to the
same `time-slay --webextension` native host the browser extension uses, so a
Slack task gets the Gmail-aware link handling and the running-app IPC for free.

The task's description holds the **whole message** (text, attachments and file
links) plus a link that opens the message in Slack. Slack redirects that link
to the desktop app when it is installed. The link form also takes an optional
**due date** and **deadline**, stored the way the app stores them (the due day
on the task, the deadline as a line in its notes).

## What you get

Slack only lets a third-party app add entries to the **shortcuts menu** and the
**message actions menu**. It cannot add an item to Slack's built-in "Connect
apps" area, which belongs to Slack itself. Within that limit, this app appears
in every place Slack offers an app:

| Where | How |
| --- | --- |
| Message actions menu | Hover a message → **⋯** → **Add to inbox** |
| Shortcuts menu / global | **Add to inbox** opens a small form (link, optional title, note, due date and deadline) |
| Slash command | `/add-to-inbox <link> [note]` |
| App Home | A **Home** tab with the same instructions |

## Requirements

- Node **22.18+** (this project runs TypeScript directly, without a build
  step, using Node's type stripping).
- The `time-slay` binary built from this repository and reachable on `PATH`,
  or `TIMESLAY_BIN` pointing at it.

## 1. Build TimeSlay

```sh
cargo build --release
export TIMESLAY_BIN="$PWD/target/release/time-slay"
```

## 2. Create the Slack app

1. Go to <https://api.slack.com/apps> → **Create New App** → **From a
   manifest**.
2. Pick your workspace and paste `manifest.yaml` from this directory.
3. **Install App** → install to the workspace. Copy the **Bot User OAuth
   Token** (`xoxb-…`).
4. **Basic Information** → **App-Level Tokens** → generate a token with the
   `connections:write` scope. Copy it (`xapp-…`).

## 3. Run it

```sh
cd slack
npm install
cp .env.example .env   # then fill in the two tokens
npm start
```

`npm start` runs `src/app.ts` with `node --experimental-strip-types`. There is
no compile step: `.ts` files run as-is.

Leave it running while you use Slack. Socket mode means no public URL or
tunnel is needed.

## How it fits together

```
Slack (shortcut / command / home)
  └─ app.ts                       (builds the task from the Slack message)
       └─ timeslay.ts             (spawns the native host, frames one message)
            └─ time-slay --webextension
                 ├─ integrations::recognize/fetch   (Gmail subject, if any)
                 ├─ ipc::WINDOW / SIDEBAR / DOCK    (running app writes it)
                 └─ inbox::add                      (fallback if the app is closed)
```

The reply is reported back to Slack as an ephemeral message (or a DM, for the
form). On success you get the task title and a link; on failure you get the
error and the full debug trace, exactly as the browser popup does.

## Scripts

```sh
npm start        # run the app
npm run dev      # run and restart on change
npm test         # unit tests (format helpers and the native-host bridge)
npm run typecheck
```

## Troubleshooting

- **`Could not run time-slay`**: set `TIMESLAY_BIN` in `.env`, or put the
  binary on `PATH`. The app prints the path it tried in the error trace.
- **Nothing appears on a message**: the message shortcut must be enabled in the
  app manifest and the app installed after adding it.
- **A Slack link adds only the link, not the text**: the bot cannot read that
  channel. Invite it with `/invite @TimeSlay`, or add the `channels:history` /
  `groups:history` scope and reinstall.
