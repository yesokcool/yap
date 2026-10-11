# Yap

A tiny Discord for a group of friends: **text channels and voice channels**,
plus a few calm extras. It runs for free on Cloudflare, and voice goes straight
between people as high-quality Opus (128 kbps by default).

Yap is built to be a nice place to hang out, not a product to keep you hooked:
- **No presence.** No online dots, no "last seen", no read receipts. People only see you in a voice channel or on a shared place.
- **Deleting a message erases it** from the server.
- **No ads, analytics or tracking.** Your server is yours.

## What's in it

- **Chat:** channels, replies, reactions (and tiny doodle reactions), @mentions, pins, search, GIFs, pictures, custom emojis, polls, `/roll` and `/flip`, spoilers, edits.
- **Voice:** peer-to-peer calls with noise suppression, push-to-talk, per-person volume, screen sharing, a soundboard with a loudness cap, and join chimes.
- **Places** next to the chat, so voice keeps going:
  - **Draw**, a shared WigglyPaint-style board with layers;
  - **Space**, a little top-down world where rooms are voice zones;
  - **Watch together** for YouTube;
  - **When's good**, a weekly when2meet with hang reminders.
- **Personal space:** notes only you can see, optionally end-to-end encrypted with a passcode.
- **Accounts that fit friends:** a name and a 6-digit PIN (or a password), a waiting room for newcomers, sign-in alerts, device lists, owner and mods.
- **One look:** Windows 98 / AIM, in light or dark.
- **Clients:**
  - the web app (any modern browser; it can be added to a phone's Home Screen);
  - a native desktop app in Rust (Windows, macOS, Linux; no Electron);
  - a SwiftUI app for iPhone, iPad and Mac.

## Run your own

You need a free Cloudflare account and Node.js. It takes about ten minutes:
[SELF_HOSTING.md](SELF_HOSTING.md).

## How it works

- **Server:** one Cloudflare Worker with SQLite-backed Durable Objects (`worker/`). It's the whole backend: accounts, history, signaling for voice.
- **Web client:** plain JavaScript and CSS (`web/`), no framework and no build step.
- **Desktop:** Rust and egui (`desktop/`). **iPhone/Mac:** SwiftUI (`apple/`).
- **Voice:** a WebRTC mesh, so audio never touches the server. That's what keeps it free, and it suits groups of up to about eight.

[PROTOCOL.md](PROTOCOL.md) describes the wire protocol every client speaks.
[SECURITY.md](SECURITY.md) explains what protects what, in plain words.
[PHILOSOPHY.md](PHILOSOPHY.md) explains what Yap is and refuses to become.

## Developing

```sh
npm install
npm run test:all     # every test, on a test server of its own
# A local server with the test settings. NTFY_SERVER keeps its phone pushes
# on this machine (where the protocol test's fake ntfy listens), never on
# ntfy.sh; TEST_HANGS lets a hang start a second from now.
npm run dev:test -- --port 8787 --persist-to .wrangler/dev-test \
  --var NTFY_SERVER:http://127.0.0.1:8790 --var TEST_HANGS:1
```

Open http://localhost:8787/#key=testkey. The first person to sign up becomes
the owner. For `npm run dev` (your own `wrangler.jsonc`), copy
`.dev.vars.example` to `.dev.vars` first. `CLAUDE.md` has the full list of
commands, conventions and design decisions, written for both people and
coding agents.

## License

Yap is free software under the [GNU Affero General Public License v3.0](LICENSE).
You can use it, change it and run it for anyone. If you run a changed version
for other people, you have to offer them your changed source code. Set
`SOURCE_URL` and the app's Source link will point to it.

Copyright © 2026 the Yap authors.
