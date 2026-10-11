# Run your own Yap

Yap runs on Cloudflare's free plan: one Worker plus two SQLite-backed Durable
Objects. No server to look after, no card needed. These steps take about ten
minutes.

## What you need

- A free [Cloudflare](https://dash.cloudflare.com/sign-up) account.
- [Node.js](https://nodejs.org) 22.6 or newer, and this repository.
- Optional: a domain whose DNS is on Cloudflare (otherwise Yap runs at `yap.<your-account>.workers.dev`).

## 1. Set it up

```sh
npm install
npx wrangler login                          # opens the browser once
cp wrangler.example.jsonc wrangler.jsonc    # your server's settings; it's gitignored
```

Open `wrangler.jsonc` and read the comments. The main choice is the address:
- **Your own domain:** add `"routes": [{ "pattern": "chat.example.com", "custom_domain": true }]` and set `"workers_dev": false`. Cloudflare makes the DNS record and the certificate.
- **workers.dev:** leave it as it is. The address includes your Cloudflare account's subdomain, which is often your email's username. A custom domain avoids that.

Pick the address before inviting people. Moving later resets everyone's local settings, because browsers keep them per address.

## 2. Secrets

Secrets go into Cloudflare, never into files. Make up long random values and keep a copy of each in a password manager:

```sh
openssl rand -hex 16 | npx wrangler secret put YAP_KEY          # the first invite key: anyone with it can ask to join (letters, digits, - and _ only); reset it later in the app
openssl rand -hex 32 | npx wrangler secret put PIN_PEPPER       # mixed into PIN hashes; a restore needs the same one
openssl rand -hex 32 | npx wrangler secret put ADMIN_KEY        # backups, restores, claiming owner
```

To see a value again, you need your copy: Cloudflare won't show it.

Keep `YAP_KEY` to letters, digits, `-` and `_` (hex is fine). The apps send it
in the WebSocket's protocol header, which can't carry anything else; a key with
other characters ends up in the socket's address, and so in logs.

## 3. Deploy and invite

```sh
npm run deploy
```

Open `https://<your address>/#key=<your YAP_KEY>` and sign up. **The first person becomes the owner.**

Share that same link with friends. Newcomers wait in a waiting room until you or a mod lets them in (you get a push). Members can also make personal invite links (Settings › Invite), which you can revoke one by one.

If the main link leaks, reset it: Settings › Invite › **Reset main link…** (owner). Everyone signed in keeps working and gets the new link by themselves. Once reset, the server keeps its own key, so `wrangler secret put YAP_KEY` won't change it (`DELETE /api/admin/main-key` goes back to `YAP_KEY`).

If you ever lose owner rights, Settings › Owner & mods › "Claim owner" takes the `ADMIN_KEY`.

## 4. Strongly recommended

- **A rate-limit rule** in front of the Worker, so floods from outside can't use up the free plan's 100,000 requests a day. It's a two-minute dashboard job: [SECURITY.md › Known gaps](SECURITY.md#known-gaps).
- **Backups.** Cloudflare keeps 30 days of point-in-time history on its own. For a copy off Cloudflare:
  ```sh
  YAP_URL=https://<your address> YAP_ADMIN_KEY=<your ADMIN_KEY> node tools/backup.mjs
  ```
  - The tools also find the address in your `wrangler.jsonc` (`"PUBLIC_URL"` under `"vars"`). On a Mac they find the admin key in the Keychain item `yap-admin-key` (`tools/instance.mjs`).
  - `tools/install-backup.sh` runs a backup every night on a Mac.
  - `tools/restore.mjs` rebuilds an empty server from a backup; see [PROTOCOL.md › Backups](PROTOCOL.md#backups).

## Optional extras

Each of these is optional, and all but phone notifications are off until you set them:

| What | How |
|---|---|
| **GIF picker** | A free API key from [KLIPY](https://klipy.com): `npx wrangler secret put KLIPY_KEY`. GIF searches go from each person's device straight to KLIPY. |
| **Browser notifications** | `node tools/make-vapid.mjs \| npx wrangler secret put VAPID_PRIVATE_JWK` |
| **Phone notifications** | They work out of the box through [ntfy](https://ntfy.sh), using a private random topic per person. For your own ntfy server, set `NTFY_SERVER` (and `NTFY_TOKEN`). |
| **Voice behind strict networks** | Some mobile and office networks need a TURN relay. Use a fixed login (`TURN_URLS`, `TURN_USERNAME`, `TURN_CREDENTIAL`; [ExpressTURN](https://www.expressturn.com) has a free plan) or Cloudflare TURN (`TURN_KEY_ID`, `TURN_KEY_API_TOKEN`). A relay also enables "Hide my IP address in calls". |
| **Links in notifications** | `"PUBLIC_URL": "https://chat.example.com"` under `"vars"`. Without it, Yap uses the address people connect on. |
| **Minecraft server status** | `"MINECRAFT_SERVER": "mc.example.com:25565"` (and optionally `"MINECRAFT_NAME"`) under `"vars"`. |
| **A board** | Show another Worker of yours inside Yap at `/board`, with everyone's mouse pointers on it. See below. |

### A board

Yap can show another web app of yours as a Place, served from Yap's own
address so it can draw everyone's pointers on top.
1. Bind your board's Worker as `BOARD`: `"services": [{ "binding": "BOARD", "service": "<its name>" }]`.
2. Its page is served at `/board`. List the other root paths that page calls (its APIs, images) as a regex in `"BOARD_PATHS"`, and add them to `assets.run_worker_first`. Yap checks these paths before its own routes, so keep the regex narrow: something like `^/api/` would hide Yap's API.
3. Yap asks the board's Worker at Yap's own address. If it expects another one (its own `workers.dev` address, say), set `"BOARD_ORIGIN"`.
4. For pointers to land on the same thing for everyone, mark columns with `data-lane="<id>"` and the cards in them with `data-id="<id>"`.
5. It shows in the sidebar as "Board". To call it something else, set `"BOARD_NAME"`.

The board's page is on Yap's address, so it shares Yap's local storage: Yap
fills in `gk-name` (the person's name, unless the board already has one) and
keeps `gk-theme` (`dark` or `light`) up to date, for a board that wants them.
`test/fixtures/board/` is a complete minimal example.

## The apps

- **Web:** nothing to do. It's served with the server, and it updates itself after each deploy. On a phone, "Add to Home Screen" makes it feel like an app.
- **Desktop (Windows, macOS, Linux):** build it with `desktop/scripts/build-cross.sh` ([desktop/README.md](desktop/README.md)).
  - To offer downloads and click-to-update from your server, make a signing key and publish with `tools/publish-desktop.mjs`. The `/download` page then lists them.
  - Builds without your key work fine but don't update themselves.
- **iPhone, iPad and Mac:** build with Xcode under your own Apple team ([apple/README.md](apple/README.md)). Friends type in your invite link.

## Updating

```sh
git pull
npm run deploy
```

Database changes apply by themselves when the server wakes up. They add
tables and columns and never drop any; the one time one rewrote data was when
deleting a post started to erase it, and the posts deleted before then were
erased too. Open pages pick up the new version on their own at a quiet moment.

## Staying within the free plan

Cloudflare's free plan allows:
- 100,000 requests a day (every 20 chat messages count as one);
- 100,000 database writes a day;
- 1 GB per database. Text is about 250 bytes a message, so that's millions of messages.

Voice and screen sharing go directly between people, so they cost nothing.
A group of friends won't come close; `GET /api/admin/stats` shows the numbers.

## The license

Yap is AGPL-3.0. Run it for anyone you like. If you change the code and other
people use your changed version, offer them your changed source: put it
somewhere public and set `"SOURCE_URL"` under `"vars"`. The Source link in
the app's sidebar then points there.
