# TimeSlay for Slack

A small Slack app that adds a message to your TimeSlay inbox. It talks to the
same `timeslay --webextension` native host the browser extension uses, so a
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
- The `timeslay` binary built from this repository and reachable on `PATH`,
  or `TIMESLAY_BIN` pointing at it.

## 1. Build TimeSlay

```sh
cargo build --release
export TIMESLAY_BIN="$PWD/target/release/timeslay"
```

## 2. Create the Slack app

1. Go to <https://api.slack.com/apps> → **Create New App** → **From a
   manifest**.
2. Pick your workspace and paste `manifest.yaml` from this directory.
3. **Install App** → install to the workspace. Copy the **Bot User OAuth
   Token** (`xoxb-…`).
4. **Basic Information** → **App-Level Tokens** → generate a token with the
   `connections:write` scope. For a rotating token, copy **both** the access
   token (`xoxe.xapp-…`) and refresh token when shown; Slack shows them only once.
5. **Basic Information** → **App Credentials** → copy the client ID and client
   secret. These are required to refresh the app-level token.

## 3. Run it

```sh
cd slack
npm install
cp .env.example .env   # fill in tokens, client ID, and client secret
npm start
```

`npm start` runs `src/app.ts` with `node --experimental-strip-types`. There is
no compile step: `.ts` files run as-is.

Leave it running while you use Slack. Socket Mode needs no public URL or
tunnel for normal operation. Connecting multiple workspaces uses an HTTPS
installation endpoint, described below.

For automatic startup, run `packaging/install.sh` from the repository root.
It installs and enables the **`timeslay-slack.service`** systemd user service,
copies the current credentials and token cache on first installation, and
starts the bridge. Stop a manually running `npm start` before switching to
the service. After installation, edit `~/.config/timeslay/slack/.env` and
use `systemctl --user restart timeslay-slack.service` to apply changes.
See [the packaging guide](../packaging/README.md#slack-service) for paths,
logs, and uninstall behavior.

### App-level token rotation

Set `SLACK_APP_TOKEN`, `SLACK_APP_REFRESH_TOKEN`, `SLACK_CLIENT_ID`, and
`SLACK_CLIENT_SECRET` in `.env` alongside `SLACK_BOT_TOKEN`. The app calls
`oauth.v2.access` with `grant_type=refresh_token` on its first startup and
then refreshes five minutes before expiration, using Slack's `expires_in`.
It reconnects Socket Mode with each replacement access token.

Every new access/refresh pair is saved atomically to `.slack-app-tokens.json`
in this directory, with owner-only permissions. This ignored file takes
precedence over the original tokens in `.env` on later starts: refresh tokens
are single-use. Keep this file across restarts and run only one instance per
token cache. Use `SLACK_APP_TOKEN_FILE` to choose a different persistent path
(its parent directory must already exist). When replacing the app credentials
with a newly generated pair, stop the app and remove the old cache first.

The bot token and the Socket Mode app token have separate lifecycles. This
refresh token rotates only the app-level token; the manifest's bot OAuth
rotation setting is independent. A long-lived `xapp-…` app token is still
supported without refresh/client credentials: omit `SLACK_APP_REFRESH_TOKEN`.

See [Slack's token rotation guide](https://docs.slack.dev/authentication/using-token-rotation/).

## Connect multiple workspaces with one app

Use **unlisted distribution** for your personal workspaces. You do not need a
Slack Marketplace listing. Each workspace still needs its own authorization;
use the same connection link once per workspace.

1. Keep your existing Slack app and its Socket Mode app-level credentials.
   In **OAuth & Permissions**, keep **bot token rotation disabled** (the
   manifest default). App-level token rotation remains supported separately.
2. Choose an HTTPS hostname and forward its `/slack/*` paths to the bridge at
   `http://127.0.0.1:3000`, using a reverse proxy or HTTPS tunnel on this machine.
   Register `https://YOUR-HOST/slack/oauth/callback` under **OAuth & Permissions
   → Redirect URLs**. The URL must match exactly. The listener binds only to
   loopback; expose it through your HTTPS proxy.
3. In **Manage Distribution → Share Your App with Other Workspaces**, complete
   Slack's checklist and click **Activate Public Distribution**. Keep Socket
   Mode enabled. Do not submit a Marketplace listing for this personal setup.
4. Add to the bridge's `.env` (for the installed service, edit
   `~/.config/timeslay/slack/.env`):

   ```dotenv
   SLACK_CLIENT_ID=your-client-id
   SLACK_CLIENT_SECRET=your-client-secret
   SLACK_REDIRECT_URI=https://YOUR-HOST/slack/oauth/callback
   # SLACK_OAUTH_PORT=3000
   ```

   Keep `SLACK_APP_TOKEN` and any app-level refresh settings. `SLACK_BOT_TOKEN`
   is ignored in this mode. Restart `npm start` or the installed
   `timeslay-slack.service` after upgrading it with `packaging/install.sh`.
5. Open the **private connection link** printed at startup. For the service:

   ```sh
   journalctl --user -u timeslay-slack.service -n 50 --no-pager
   ```

   Choose a workspace and authorize using **your own Slack account**. The
   installer account becomes the only account allowed to add to your inbox
   from that workspace. Repeat with the same link for each workspace,
   **including the original workspace**, to register all their tokens locally.
   New connections work immediately without restarting the bridge.

The link contains a private setup key: anyone holding it can connect their own
workspace to your local inbox. Keep it and proxy access logs private. The key
changes on every process restart; get the latest link from startup output.
Authorization also uses a browser cookie and expiring, single-use OAuth state.
Complete one workspace connection at a time in the same browser. Admin approval
policies still apply. If approval requires another person, have them approve the
app, then finish the connection as your own account.

Tokens and the allowed user for each workspace are saved atomically with
owner-only permissions in `.slack-installations.json`. For the installed service,
the file lives in `~/.config/timeslay/slack/` and survives upgrades and ordinary
uninstall. For manual runs it defaults to the `slack/` directory; override it
with `SLACK_INSTALLATIONS_FILE` (parent directory must exist). Run only one
bridge per installation file, and preserve it alongside the app-token cache.
A reinstall by the same user updates tokens; a different user cannot replace
an existing workspace's owner. To disconnect a workspace, stop the bridge,
uninstall the app from that workspace in Slack, remove its entry from the
installation file, then restart. Enterprise organization-wide installation and
rotating **bot** tokens are not supported by this personal bridge.

Without `SLACK_REDIRECT_URI`, the original single-workspace configuration
continues to work. Set `SLACK_ALLOWED_USER_ID` to your Slack member ID to restrict
that mode to your account too (Slack profile → **⋯ → Copy member ID**).

See [Slack's distribution guide](https://docs.slack.dev/app-management/distribution/)
and [OAuth guide](https://docs.slack.dev/authentication/installing-with-oauth/).

## How it fits together

```
Slack (shortcut / command / home)
  └─ app.ts                       (builds the task from the Slack message)
       └─ timeslay.ts             (spawns the native host, frames one message)
            └─ timeslay --webextension
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

- **Menu entry appears, but Slack says “Sorry, that didn’t work. Try again?”**:
  the menu is registered with Slack even when the local bridge is offline.
  Run `cd slack && npm start` from the repository root and check that it prints
  `TimeSlay Slack app is running (socket mode).` If startup reports missing
  `SLACK_CLIENT_ID` or `SLACK_CLIENT_SECRET`, add both from the app's
  **Basic Information → App Credentials** to `slack/.env` and start again.
  Leave the bridge running while using the shortcut.
- **`Could not run timeslay`**: set `TIMESLAY_BIN` in `.env`, or put the
  binary on `PATH`. The app prints the path it tried in the error trace.
- **Nothing appears on a message**: the message shortcut must be enabled in the
  app manifest and the app installed after adding it.
- **A Slack link adds only the link, not the text**: the bot cannot read that
  channel. Invite it with `/invite @TimeSlay`, or add the `channels:history` /
  `groups:history` scope and reinstall.
