# Setup

TimeParagon talks to a few services you set up yourself. This guide explains each one: what it is for, how to get it running, and what to enter in the app.

| Service | What it is for | Required |
| --- | --- | --- |
| [Database sync](#database-sync) | Sharing data and settings between devices | Yes |
| [Google Cloud OAuth client](#google-cloud-oauth-client) | Calendar, Tasks, Gmail and contacts | Yes |
| [Google sign-in](#google-sign-in) | Using that client with your account | Yes |
| [Notes folder](#notes-folder) | Markdown notes | Yes |
| [Mapbox](#mapbox-optional) | Place search and travel time | No |
| [OpenRouter](#openrouter-optional) | AI for voice, images and extraction | No |
| [Notifications](#notifications-optional) | Reminders and alarms on Android | No |

Set up your first device (usually the desktop) completely before adding others. Settings that are shared between devices, such as the Google client and the Mapbox token, then reach every later device through database sync, so only device-specific things need entering again.

## Where to enter things

- **Desktop app**: everything is in **Settings**.
- **Android app**: the first start walks through a setup flow with one page per section of this guide, in the same order, and each page links to its section here. Afterwards, everything is in **Settings**, and **Settings → Application → Run setup** goes through the pages again.

The Android setup flow keeps its place: the page you are on and everything you have typed is saved as you type, so leaving the app reopens setup on the same page with the same text. The app opens only once the last page is done. Required pages cannot be left with **Next** until they are done; optional ones have **Skip**. Phones that were already configured before the setup flow existed skip it.

## Database sync

Each device keeps its own local SQLite database. Habits, goals, settings, routines, alarms, cached calendar data and the rest are swapped between devices through a [Turso](https://turso.tech) sync server that **you** run, usually on your desktop or a home server.

Every device needs the server's URL. On Android, setup does not continue until **Connect and sync** has succeeded, which does two things:

1. **Testing the connection.** It connects the same way sync does, with a scratch copy that is thrown away afterwards, and gives up after 30 seconds.
2. **Syncing the database.** It saves the URL and waits, with a progress bar, until the first sync with the server has finished, so everything the server holds is on the device before you go on. This gives up after three minutes; the first sync of a large history can take a minute.

Database sync comes first because, once it works, a new device receives your shared settings, including the Google client from the next section.

### Running the sync server

On the computer that will host it:

```sh
turso --sync-server localhost:<INTERNAL_PORT>
```

Pick any free port for `<INTERNAL_PORT>`, such as `8080`.

> [!WARNING]
> The sync server has **no authentication**. Bind it to `localhost` only, as above, and never expose that port to your network or the internet directly. Tailscale (below) is how other devices reach it.

### Reaching it with Tailscale (recommended)

[Tailscale](https://tailscale.com) gives your devices a private network that works from anywhere, so a phone can sync on mobile data without opening any port to the internet. Its free tier is plenty.

1. Install Tailscale on the server and sign in.
2. Install Tailscale on every other device and sign in with the **same** account. Leave it connected.
3. In the [Tailscale admin console](https://login.tailscale.com/admin/dns), under **DNS**, enable **MagicDNS** and **HTTPS certificates**.
4. On the server, put the sync server on your tailnet with HTTPS:

   ```sh
   sudo tailscale serve --bg --https <TAILNET_PORT> <INTERNAL_PORT>
   ```

   such as `sudo tailscale serve --bg --https 8443 8080`. `tailscale serve status` shows what is being served.
5. The URL is `https://<machine-name>.<tailnet-name>.ts.net:<TAILNET_PORT>`, such as `https://desktop.tail1234.ts.net:8443`. Find the full name in the Tailscale app, or with `tailscale status --json | jq -r .Self.DNSName` on the server.

Enter that same URL on every device.

### Accepted URLs

`https://`, `http://`, `libsql://` and `turso://` URLs are accepted. Plain `http://` works but sends your data unencrypted; with Tailscale use the `https://` address, since Tailscale already provides the certificate.

### If it does not connect

- *Timed out*: the device can't reach the server. Check that Tailscale is connected on it, that the server is awake, and that `tailscale serve status` lists the port.
- *Connection refused*: the server is reachable but nothing is listening. Check that `turso --sync-server` is running and that the ports in the `tailscale serve` command match.
- *Certificate errors*: use the full `*.ts.net` name, not the machine's short name or its `100.x.y.z` address, and check that HTTPS certificates are enabled for the tailnet.

## Google Cloud OAuth client

Calendar, Tasks, Gmail and contacts are read from Google with an OAuth client that lives in **your own** Google Cloud project. Nothing passes through a TimeParagon server; there isn't one. Google's free tier is plenty.

You need one client for all your devices. It is a shared setting: once one device has it and database sync works, other devices receive it, and the Android setup fills it in for you, since the database has finished syncing by the time you reach this page.

To create one:

1. Open the [Google Cloud console](https://console.cloud.google.com) and sign in with the Google account whose calendar you want to use.
2. Create a project: project picker at the top → **New project**. Any name works, such as `TimeParagon`.
3. Enable the APIs the app uses. Go to **APIs & Services → Library**, search for each of these and press **Enable**:
   - **Google Calendar API**
   - **Google Tasks API**
   - **Gmail API** (to read the subject of Gmail links added as tasks)
   - **People API** (for contacts and the directory on the People screen)
4. Configure the consent screen. Go to **APIs & Services → OAuth consent screen** (called **Google Auth Platform** in newer consoles):
   - **App name**: anything, such as `TimeParagon`. Use your own email for the support and developer contact addresses.
   - **Audience / User type**: **External** (or **Internal** if your account is in a Google Workspace organisation and you only use it there).
   - **Test users**: add your own Google address, and any other account you will sign in with. While the app is in *Testing*, only test users can sign in.
   - You do not need to add scopes here, or submit the app for verification.
5. Create the client. Go to **APIs & Services → Credentials** (or **Google Auth Platform → Clients**) → **Create credentials → OAuth client ID**:
   - **Application type**: **Desktop app**, on every platform including Android. The app signs in with a loopback (`http://127.0.0.1`) redirect, which only Desktop clients allow; an *Android* client type does **not** work.
   - **Name**: anything.
6. Copy the **Client ID** (ends in `.apps.googleusercontent.com`) and **Client secret** (usually starts with `GOCSPX-`) into the app. You can reopen the client in the console later to copy them again.

Both the ID and the secret are required.

> [!NOTE]
> Google calls the secret of a Desktop client a secret, but it does not treat it as confidential: it identifies the client rather than protecting it. Sign-in is protected with PKCE.

### Common problems

- *"Access blocked: app has not completed the Google verification process"*: your account is not in the test users list (step 4).
- *"redirect_uri_mismatch"* or *"invalid_client"*: the client is not of type **Desktop app**, or the ID or secret was pasted with a missing character.

## Google sign-in

Each device signs in on its own. Press **Sign in with Google** (or **Connect Google**) and your browser opens on Google's sign-in page:

1. Choose the account you added as a test user.
2. Google warns that it hasn't verified the app, since the OAuth client is your own. Press **Advanced → Go to TimeParagon (unsafe)**.
3. Tick **every** permission (calendar, tasks, Gmail read-only, contacts, directory) and press **Continue**.
4. The browser shows *Signed in*. On Android, use **Return to TimeParagon**, or switch back to the app yourself.

The sign-in tokens stay on the device, in private storage (encrypted on Android), and are never put in the synced database. If sign-in times out (it waits three minutes), try again. Add more Google accounts later from the People screen.

## Notes folder

Notes are plain Markdown files, one per note, in a folder you choose on each device. They are **not** synced through the database: share the folder between devices with [Syncthing](https://syncthing.net) (free, peer-to-peer, and nothing in the cloud), or any other file sync.

1. Choose a notes folder on your first device.
2. Install Syncthing on each device. On Android, use a Syncthing app such as *Syncthing-Fork* from F-Droid or Google Play.
3. Pair the devices: on each, **Add remote device** with the other's device ID (Syncthing shows it as a QR code).
4. Share the notes folder from the first device. Accept it on the others and choose where it goes, such as `Documents/Notes` on a phone.
5. Wait for the first sync to finish.
6. Choose that folder in TimeParagon on each device. On Android, press **Choose folder** and pick it in the system folder picker, allowing access when asked; Android only lets the app into folders you grant it, so a typed path does not work there.

> [!TIP]
> Set the shared folder to *Send & Receive* on every device, so notes written anywhere reach the others.

## Mapbox (optional)

A [Mapbox](https://mapbox.com) access token lets you search for places when giving an event or task a location, and work out travel time between events. Without one, locations are plain text and there is no travel time. Mapbox's free tier is plenty for one person.

1. Create a Mapbox account and sign in.
2. Open [Access tokens](https://account.mapbox.com/access-tokens/).
3. Use the **Default public token**, or press **Create a token** for one just for TimeParagon (the default public scopes are enough). Public tokens start with `pk.`.
4. Paste it into the app.

The token is **shared between devices**: enter it once, and database sync brings it to the others. Leave it blank to go without.

## OpenRouter (optional)

An [OpenRouter](https://openrouter.ai) API key gives TimeParagon access to AI models, for turning speech and pictures into text and for pulling event and task details out of text. OpenRouter is paid per use; a few dollars of credit lasts a long time.

1. Create an OpenRouter account and add some credit under **Settings → Credits**.
2. Open **Settings → Keys** (or [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys)) and press **Create key**. Optionally give it a credit limit.
3. Copy the key, which starts with `sk-or-v1-`, and paste it into the app. On the desktop, that is **Settings → AI**.

The key is a secret: it is kept in private storage on the device it was entered on and is **never** synced, so each device needs its own copy. The rest of the AI configuration, such as which model handles voice, images and extraction, is set on the computer in **Settings → AI**.

Leave it blank to go without AI features.

## Notifications (optional)

On Android, reminders for events and tasks, alarms and bedtime notices are shown as notifications, and since Android 13 apps must ask before posting them.

Press **Allow notifications** and accept the prompt. If you said no before, Android may not ask again: turn them on in Android's **Settings → Apps → TimeParagon → Notifications**. The app warns you when notifications are switched off.

On Android 12 only, also allow **Alarms & reminders** for TimeParagon in the same app settings, so alarms ring exactly on time (Android 13 and later grant this to calendar apps). On phones that are aggressive about battery saving, consider setting TimeParagon's battery usage to **Unrestricted**.

Reminder settings are under **Settings → Reminders**.

## When you are done

Your day loads from Google, and habits, goals and settings arrive from your other devices through database sync; the first sync can take a minute. Every setting here can be changed later in **Settings**. On Android, long-press the home screen and add the **TimeParagon** widget to see the day at a glance.
