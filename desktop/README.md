# Yap desktop

The native desktop app for Yap: text, voice and screen sharing, for Windows,
Linux and macOS.
One Rust binary. No Electron, no webview: egui on OpenGL, about 60 MB of RAM
and 0% CPU when idle.

It works with any Yap server. It speaks the same protocol as the web client
(`../PROTOCOL.md`) and joins the same voice calls: WebRTC peer-to-peer Opus,
tuned exactly like `web/js/voice.js`. Nothing about a server is built in. The
app asks the server what it has (`/api/config`), and whatever the server
leaves out (GIFs, a board, a Minecraft status) doesn't show.

## Build

```sh
# Rust (once): curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
cd desktop
cargo build --release          # → target/release/yap (yap.exe on Windows)
cargo test                     # unit tests (SDP tuning, markup, parsing, resampler, echo cancellation…)
```

A plain build has no update key, so it doesn't update itself. See
Self-update below to give your builds one.

If the checkout is in a synced folder (Dropbox, iCloud Drive…), keep `target/`
out of the sync: it grows to gigabytes.

**Native build dependencies:** Windows — the MSVC toolchain (the CRT is linked
statically, see `.cargo/config.toml`, so users don't need the VC++
redistributable). Linux — `libasound2-dev` and `pkg-config` (X11, Wayland,
xkbcommon and GL are loaded at runtime, not linked).

### Cross-compiling Windows + Linux from a Mac

```sh
brew install zig pkgconf llvm nasm   # nasm: OpenH264's x86 SIMD (else plain C++)
rustup target add x86_64-pc-windows-msvc x86_64-unknown-linux-gnu
cargo install --locked cargo-zigbuild cargo-xwin
./scripts/build-cross.sh            # or: linux | windows | macos
# → target/dist/yap-linux-x86_64 (glibc ≥ 2.28), target/dist/yap-windows-x86_64.exe,
#   target/dist/yap-macos-aarch64 (native, ad-hoc signed; on a Mac only)
```

The script prints which update key the builds carry, if any (Self-update
below).

Sizes (0.12.0 as published, 2026-10-11): Linux 25,374,480 bytes,
Windows 19,828,736, macOS 16,388,400. The download page serves each file as a
static asset of at most 25 MiB (26,214,400 bytes; `tools/publish-desktop.mjs`
refuses bigger), and Linux is the closest. To make room, add code that's rarely
hot to the `[profile.release.package.*] opt-level = "s"` list in `Cargo.toml`
(file decoding, fonts, TLS, the OS glue, WebRTC's once-per-call setup, sonora's
unused parts); voice, video and the UI keep opt-level 3. That list's WebRTC and
sonora lines took 156 KB off Linux with no change to the echo canceller's CPU.

The script fetches Debian's `libasound2`/`-dev` into `target/linux-sysroot`
for linking (nothing is installed system-wide). The Linux binary needs only
glibc 2.28+ and `libasound.so.2` at link time; at runtime it also wants X11 or
Wayland (+ libxkbcommon) and an OpenGL driver (Mesa), which every desktop has.

## Run

```sh
yap                                  # the app; first run asks for the invite link
yap --server 'https://chat.example.com/#key=K'   # pre-fill your server
yap --config /tmp/other.json         # a second identity / test profile
yap --list-devices
RUST_LOG=yap=debug yap               # more logging
```

Screenshot aids (any build): `YAP_DEBUG_OPEN=settings[:tab]|channel|update|gifs|react|people|people-away|unlock|encrypt|reply|edit|search|pins|draw|space|switcher|plan|catchup|shelf|soundboard|voice-more|ping|link|password|delete|archive|viewer|volume|signin-alert|weak-pin|weak-password|temp-password|password-reset|hang-now`
opens that dialog, picker, bar or reply bar at start (the ones that need
something from the server — a message to delete, a picture to view, someone
away — wait until it's there). Settings tabs: 0 Look, 1 Profile, 2 Voice,
3 Notifications, 4 Account, 5 Owner & mods, 6 Invite, 7 About.
`people-away` opens People with someone who's away expanded; `password` opens
the "Choose a password" dialog; in debug builds `signin-alert`, `hang-now` and
the "Choose a password" bar (`weak-pin`, `weak-password`, `temp-password`: an
old PIN, a guessable password, a reset's temporary one) show as if the server
had said so, and `password-reset` shows an owner/mod's "Cleo's temporary
password: …" result.
(debug builds: `YAP_DEBUG_SPACE_AT=x,y` puts you there when you walk into Space)
(`YAP_DEBUG_QUERY=<words>` fills the search, Everywhere; `YAP_DEBUG_HERE=1`: this channel only)
Debug builds also take `YAP_DEBUG_OPEN=emoji` (+ `YAP_DEBUG_FILE=<picture>`,
else a made-up one), `YAP_DEBUG_OPEN=burst` (once connected, sends twelve
notes at once and a reply to a message that's gone: what a `slow_down` and a
refusal look like), `YAP_DEBUG_SETTINGS_SCROLL=<points>|end` (Settings' tab
scrolled down that far, or to its end), `YAP_DEBUG_SETTINGS_ADVANCED=1`
(Voice › Advanced open), and
`YAP_DEBUG_VAULT=<passcode>` (keep the Personal space key in memory, not the
keychain, and unlock with it once connected; `-` = stay locked),
`YAP_DEBUG_FOCUSED=1` (draw the title bars as if the window had focus),
`YAP_DEBUG_DRAW_LAYERS=<layer>[:<hidden>…]` (Draw: the layer you're on and
hidden ones, 1–3, e.g. `3:1`), `YAP_DEBUG_DRAW_CONFIRM=1` (Clear's choices
open), `YAP_DEBUG_DRAW_SAVE=1` (archive the board to the gallery once),
`YAP_DEBUG_AUDIO=silence` + `YAP_DEBUG_OPEN=voice` (join the first voice
channel with no real mic or speakers; `soundboard` does that too and opens
it; `watch` watches the first person sharing their screen; `share` shares
and `share-dialog` opens Share Screen, both with `YAP_DEBUG_SCREEN=pattern`:
the only thing offered is then a synthetic test pattern, never the real
screen; without it, on macOS 14+ `share-dialog` shows the dialog that leads to
Apple's picker, which only opens on a click), `YAP_DEBUG_OPEN=doodle|sound[:chime]` (+ `YAP_DEBUG_SOUND=<file>`),
`YAP_DEBUG_RESET_MAIN=ask` with `YAP_DEBUG_OPEN=settings:6` (Invite with the
"Reset main link?" popup open; `press` also presses Reset link once
connected as the owner, so only ever against a test server),
`YAP_DEBUG_HINT=weak|relay|cant|reconnecting` (in voice, show that
connection hint on everyone else; `reconnecting` also says your own call is
reconnecting), `YAP_DEBUG_HEAR=1` with `YAP_DEBUG_AUDIO=silence` +
`YAP_DEBUG_OPEN=settings:2` (start Hear yourself once),
`YAP_DEBUG_AUDIO_FAIL=mic|speakers[@secs]` (fake a stream error once, 5 s
in by default: the recovery path without real devices),
`YAP_DEBUG_STAGE=<picture>` (put a picture in the preview tray),
`YAP_DEBUG_TAB=<n>` (move the keyboard focus on n times, as Tab
would, 3 s in), `YAP_DEBUG_WINDOW=WxH[+X+Y]` (that window size, at the top left of the main
screen, or at X,Y over other apps' windows: a window that's entirely covered
isn't drawn, so parallel shots each need a spot of their own)
and `YAP_DEBUG_SHOT=<file.png>` (on a Mac that window never takes the focus
from what you're typing in; the app saves its own window
`YAP_DEBUG_SHOT_AFTER` seconds in — default 6 — and quits; needs no
screen-recording permission and works with the screen locked).

Settings live in `~/.config/yap/config.json` (Linux),
`%APPDATA%\Yap\config.json` (Windows), `~/Library/Application Support/Yap/config.json`
(macOS); device tokens in `tokens.json` next to it, the last three call
reports in `calls.json`. Custom emoji images are
cached in the OS cache dir (they're immutable); KLIPY GIFs are never cached on
disk (their terms).

## Self-update for your own builds

Yap can update itself, but only a build that carries an update key does. The
key is an Ed25519 key pair: the public half goes into the build, the private
half signs each release. A build without a key never checks for updates and
has no install button. People using it get new versions from the server's
`/download` page, or by building it again.

1. **Make a key pair** (once), from the repo root. The public key goes to
   `desktop/update-pubkey.txt`, the private key to a file only you can read:

   ```sh
   node -e '
     const c = require("node:crypto"), fs = require("node:fs"), os = require("node:os");
     const { publicKey, privateKey } = c.generateKeyPairSync("ed25519");
     fs.writeFileSync(os.homedir() + "/.yap-update-signing-key", privateKey.export({ format: "der", type: "pkcs8" }).toString("base64"), { mode: 0o600 });
     process.stdout.write(publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("base64"));
   ' > desktop/update-pubkey.txt
   ```

   Keep the private key secret and backed up. Never commit it. If you lose it,
   or change keys later, the installed apps refuse every new update: people
   have to download a new build once.
2. **Build with the public key.** `scripts/build-cross.sh` takes it from the
   `YAP_UPDATE_PUBKEY` environment variable, else from
   `desktop/update-pubkey.txt` (gitignored: it's not a secret, just yours).
   It prints which one it used, or "no update key: this build won't
   self-update". `./scripts/build-cross.sh key` only prints that. A plain
   cargo build reads only the variable:
   `YAP_UPDATE_PUBKEY=$(cat update-pubkey.txt) cargo build --release`.
3. **Sign and publish** with `tools/publish-desktop.mjs`. It takes the builds
   from `desktop/target/dist/` (where `build-cross.sh` leaves them), or
   `--from <dir>`. It reads the private key
   from `YAP_UPDATE_SIGNING_KEY` (base64 PKCS#8 DER, or PEM). On a Mac it can
   use the Keychain instead: a generic password with the service name
   `yap-update-signing-key`
   (`security add-generic-password -a "$USER" -s yap-update-signing-key -w`
   asks for it). It refuses a binary that doesn't contain the
   matching public key, so you can't ship an update the apps would reject. It
   never prints the private key. It writes `web/downloads/` (gitignored);
   deploy the server from the same machine afterwards.

   ```sh
   YAP_UPDATE_SIGNING_KEY="$(cat ~/.yap-update-signing-key)" node tools/publish-desktop.mjs --local   # try it, no git tag
   YAP_UPDATE_SIGNING_KEY="$(cat ~/.yap-update-signing-key)" node tools/publish-desktop.mjs           # publish + tag desktop-v<version>
   ```

## What it does

- **One look: Windows 98 / AIM** (light or dark, on every OS). The Apple
  and Arch skins are gone: a config.json that still says
  `"skin": "apple"` or `"arch"` loads as Windows 98, keeping your light or
  dark choice, and the key is dropped the next time settings are saved.
- **Look:** the same tokens as `web/style.css`. Two
  windows on a teal desktop — Channels and the chat ("general - Yap") —
  with two-ring 3D bevels, navy gradient title bars (every window's, always:
  swapping navy / grey with focus made the sidebar's bar flash on each click),
  white sunken wells, square corners, no soft shadows, Tahoma (system sans as
  the fallback: Segoe UI / DejaVu Sans). Buttons ≥ 75×23 that press in, the
  dialog's default button with a black ring, 98-style check boxes, radio
  buttons, combo boxes, trackbars, group boxes, property-sheet tabs (Settings),
  16 px scroll bars (dithered track, bevelled thumb; no arrow buttons — egui
  can't add them), pale-yellow tooltips. The chat log reads like AIM: square
  buddy icon, **Name:** message, time on the right; your name red, everyone
  else's blue. A status bar along the bottom shows the server (a light only
  while connecting or reconnecting), voice and, if the server has one, its
  Minecraft server. Charcoal (dark, default) or Windows Standard (light). Drawn in
  `ui/theme.rs` (egui's own widgets can't do bevels, so most are replaced).
  Motion is instant, like the real thing.
- **Sign in:** paste an invite link, then *I'm new* (name + password →
  waiting room) or *I have an account*. One **Password** field, masked, with
  a Show box; "8 characters or more." under it when signing up (Sign Up waits
  for 8). Signing in takes whatever was set, so an account from before
  2026-10-11 still signs in with its old 6-digit PIN. The server judges the
  rest (too short, too easy to guess, too many wrong tries) and its words show
  under the form as it puts them; a wrong name or password adds "An old PIN
  works here too. Forgot it? The owner or a mod can reset it." and too many
  wrong tries "Forgot your password? The owner or a mod can reset it." (the
  web's words). The wire still says `pin`
  (`login.pin`, `signup.pin`, `account.pin`, `member.resetPin`, `pin.reset`,
  `bad_pin`, `weak_pin`): PROTOCOL.md › Identity. The device token is kept
  per server in `tokens.json` (owner-only permissions on Unix), never the
  password. The WebSocket upgrade carries `User-Agent: yap-desktop/<version>
  (<OS>)`, so your devices list says "Desktop app (Mac)". (The pre-PIN
  uid-only sign-in is gone: the server answers it with `pin_required`.) An
  account with no password at all that's still signed in with an old device
  token gets the "Set a password" bar and dialog.
- **Choose a password:** `ready.weakPin` (you signed in with an old PIN, a
  guessable password or a reset's temporary one) shows a quiet bar:
  **Choose a password** and, by `ready.weakReason`, "Yap uses passwords now.
  Your PIN still works until you choose one." (`pin`), "You signed in with a
  temporary password. Choose your own." (`temporary`) or "Your password is
  easy to guess. Choose a stronger one." (`guessable`); an older server
  without `weakReason` gets the PIN's words if you signed in with a PIN
  (`ready.secretKind`), else the guessable one. [Choose a password] opens a
  small dialog (the bar goes once it's saved); [Later] puts that reason off
  on this device for good (`"password_later"` in config.json, like the web's
  `passwordLater`), so a reset's temporary password is still asked about
  after a PIN's prompt was put off.
- **Account protection:** Settings › Account says **Change password** (**Set
  a password** while you're still on a PIN): Current password ("Current
  password or PIN" while on one), New password with "8 characters or more.",
  Show; a refusal shows under the fields in the server's words and keeps
  them. It also lists where you're signed in (`account.devices`): sign one
  out (`account.revoke`) or all the others (`account.signoutOthers`, asked
  first in the web's words). "Sign out of this computer" (a red danger
  button) asks once first. A `signin.alert` shows "New sign-in: <device>,
  just now. Not you? [Sign out everywhere else…] [It was me]" (the first
  opens Settings › Account asking, with the devices in view; and a desktop
  notification if they're on). In a narrow window the bars over the chat wrap
  their words short of their buttons, or put the buttons on a row below. The invite key
  goes in the `X-Yap-Key` header and the `["yap", "key.K"]` WebSocket
  subprotocol, never in a URL. **A wrong invite key** shows the sign-on
  window's "The invite key changed. Paste the new invite link." only when
  it's certain: a 4003 close (a server that lets a wrong-key socket through
  for `hello { token }` only, and closes anything else) or a 401 on the
  upgrade (an older server). Three failed connections in a row ask
  `/api/check` only when the app has no device token, so a signed-in
  computer whose stored link is old (the owner reset it while it was off)
  keeps connecting with its token and saves the new key from `ready.key`
  (`core::key_verdict`). The app keeps trying meanwhile.
- **No presence, on purpose:** nobody is shown online or offline — no dots,
  no counts, no "last seen", no member sidebar — so people can drop in and
  chill. The server only reveals sessions in voice or on a shared view
  (`session.join` → `voice`/`view`; `voice`/`view` → `session.leave`), and
  that's all the client keeps. The **People** button (chat header, both
  a badge counts people waiting) opens a sheet with everyone A–Z, role
  badges, who's in voice, the waiting room (Let in / Turn away, for the owner
  and mods) and, click someone, their volume plus the owner/mod actions
  (make/remove mod; Reset password… and Ban… ask first, in the web's words;
  a reset ends with "Cleo's temporary password: maple-river-stone-42. Send it
  to Cleo; they'll choose their own after signing in." and Copy).
- **Chat:** channels (no `#`), bold unread channels, history paging (a page
  can be short and still have more), grouping, day dividers, typing ("Ready"
  when nobody is), yellow information bars (connection lost, choose a
  password, join requests), reconnect with backoff + heartbeat. Markdown subset (bold/italic/
  strike/code/blocks/quotes), clickable links, **@mention highlights**, custom
  emoji images inline (and big when alone), `:shortcodes:` for standard emoji.
  Every snippet of a message (reply quotes and the reply bar, pins, search,
  Catch up, notifications) is plain text the web's way (`core::plain_text`,
  a port of `plainText`: no formatting marks, spoilers `▒▒▒`) and long ones
  are cut at a word with one "…" (`core::clip`). Keyboard: Tab moves through
  buttons, check boxes, options and tabs (Space / Enter press them), and the
  focused one is marked with a dotted box.
- **Replies:** the reply arrow in the hover toolbar (next to the pencil; not
  in the Personal space). A "Replying to Name" bar, quoting the first words
  of that message, sits over the message box (Esc or Cancel drops it) and
  the message goes out with `replyTo`. Replies
  show a quote line over them — "↪ Name first line…" (or "Original message
  was deleted"); click it to scroll to the original and flash it, if it's
  loaded. A reply to you counts as a mention (highlight, badge, chime,
  "Name replied" notification).
- **Long messages:** up to 40,000 characters. Anything over 3,000 characters
  or 30 lines folds, with "Show more (N characters)" / "Show less".
- **Sending:** your message shows grey until the server's echo (with its
  `nonce`) replaces it. The server answers sends in order, so an
  `error {re: "send"}` is about the oldest one not answered yet
  (`core/outbox.rs`). `slow_down` (more than 10 at once): it goes again with
  the same nonce 1.1 s later, up to 5 more times, and until nothing is
  waiting everything goes one at a time, 1.1 s apart, in the order written
  (no error shown unless it finally gives up). Any other refusal (too long,
  the replied-to message is gone…) takes the grey copy away and, if that
  channel's message box is empty, puts the text back in it; the server's
  words are the toast. After a reconnect, grey messages go again as sent.
- **Deletes are erasures:** the server wipes the text, picture, reactions and
  edits; deleting an encrypted note with a picture also names its blob
  (`delete { id, blobs }`) so that's erased too.
- **Personal space:** your own channel (`kind: "personal"`, `p_<uid>`), first
  in the list with a lock: notes, links and pictures to grab on your other
  devices. Only you see it. Notes from your other devices show as unread; no
  typing or replies there. The header says "only you · not encrypted",
  "end-to-end encrypted" or "locked".
- **Encrypted Personal space** (PROTOCOL.md › Encrypted Personal space,
  `src/vault.rs`, byte-compatible with `web/js/vault.js`): "Encrypt it with a
  passcode" (≥ 8 characters, typed twice, numbers-only under 12 refused) makes
  a space key K (AES-256-GCM), wraps it with PBKDF2-SHA256 (600,000 rounds,
  ~0.15 s here) and sends `vault.set`; then the notes written before are
  sealed (`personal.plain` → encrypt, pictures re-uploaded sealed →
  `personal.seal` in batches that fit one frame: at most ~300 KB of JSON and
50 notes, like the web; a note counts as sealed only once the server's
`edited { sealed: true }` comes back, and the toast says how many
couldn't be). Notes go out as `e1:` bodies (text, GIF or
  picture inside; pictures encrypted and uploaded to `/api/blob`), edits
  re-seal and keep the GIF/picture, and they show decrypted (sealed pictures
  are fetched and decrypted in the image workers). K lives in the OS keychain
  (macOS Keychain, Windows Credential Manager, Secret Service on Linux —
  service "Yap Personal space"), tagged with `wrap.ct`; when a `vault` /
  `ready.vault` arrives with a different `ct` (passcode changed or started
  over elsewhere) the key is dropped and the space locks until the passcode
  is entered again. No keychain → the key stays in memory for the run.
  Locked: "Unlock your Personal space to write" (message box disabled), a
  "Locked on this device. [Unlock…]" card, notes as "🔒 Encrypted…" (click to
  unlock). Unlock / Change passcode / "Forgot it? → Start over" (a new key;
  old notes stay locked) / Lock (forgets the key here) are in the dialog, the
  header and Settings › Account. Clicking a sealed picture opens it full size
  in the app (decrypted, up to 2560 px, **Save to Downloads**); plain pictures
  still open in the browser.
- **You at the top:** your nameplate (opens Settings › Profile), the big
  Mute / Deafen buttons and the voice panel sit right under the sidebar's
  header, AIM-style.
- **Edit your messages:** the pencil in the hover toolbar, or ↑ in an empty
  message box (your last one); Enter saves, Esc cancels and gives your draft
  back. The message being edited is marked with a dotted focus box and
  brought into view. Edited messages say "(edited)".
- **Pictures:** attach (the picture button), paste (Cmd/Ctrl+V with a picture
  on the clipboard) or drop one on the window. **Preview before sending:** it
  waits in a tray above the message box with a thumbnail and ×; whatever you
  type goes along as its caption when you press Send / Enter; Esc drops it.
  While you type it's prepared in the background (`src/picture.rs`, like the
  web's `preparePicture()`): **compressed by default** — ≤ 1600 px on the long
  side, JPEG quality 80 (then 1280 px / 74, 1024 px / 68 if needed; no
  pure-Rust lossy WebP encoder exists), turned upright from EXIF first; a
  phone photo lands ~150–300 KB (a 6016² 9.3 MB JPEG → 159 KB). Pictures with
  transparency try PNG at that size first; a small picture our copy wouldn't
  shrink goes as it is. **"Full res"** (a check box in the tray) sends the
  original bytes if they fit the 1.9 MB cap (JPEG / PNG / WebP), else the
  sharpest version that fits (any size / 92, 3072 / 90, 2048 / 88, 1600 / 82).
  Small GIFs (≤ 1.9 MB) always go as they are, still animated. The tray says
  the size: "47 KB, compressed" / "1.2 MB, original". Then it's uploaded to `POST /api/image?sid=&ut=` — every upload (pictures, emojis)
  carries `ready.uploadToken` as `ut`. Pictures show at most 360 px; click one
  to open it full size. GIFs from the picker wait in the same tray.
- **Invites:** Settings › Invite makes personal links (`invite.create`; the
  link is shown once, with Copy), lists them (`invites`) and turns them off
  (`invite.revoke`, asked first); the waiting room shows who invited someone, and when
  others already signed up with the same link (`invite.usedBy`) it says
  "This link was also used by Noor — check with Priya." (the request bar
  and the People sheet). The server's
  main key from `ready.key` replaces the stored one, so a revoked personal
  link never locks a member out.
- **Reset main link** (the owner only; PROTOCOL.md › HTTP › Resetting the
  main link): Settings › Invite › Main link › **Reset main link…** asks in a
  popup over Settings, in the spec's words ("Reset main link?" "The old link
  stops working for anyone who hasn't joined yet. Everyone already signed in
  keeps working, and their apps get the new link by themselves. Personal
  invite links keep working." [Reset link] [Cancel], Windows' order like
  every dialog here, Reset link in red and never the default), sends
  `invite.resetMain`, then shows the new link with Copy (a refusal shows the
  server's words under the warning). Every app connected gets `key { key }`
  and saves it as its stored link at once (config.json's `"key"`), and the
  socket's next attempt uses it; `ready.key` does the same for anyone who
  was away. The mod log says "reset the main link".
- **Owner & mods:** Settings › Owner & mods shows the moderation log
  (`modlog`) in the web's plain words ("The owner (admin key) made Rosa a
  member", "set the voice quality in "Lounge" to 256 kbps for everyone"),
  the five newest and then "Show all N" (Invite's links too; five again
  each time Settings opens). Settings' tabs follow the web's order: Look,
  Profile, Voice, Notifications, Account, Owner & mods, Invite, then About.
- **Settings › Voice and Notifications in the web's groups:** Voice is
  *Mic & output* (microphone, output, Hear yourself, Call report), *How you talk*
  (input mode; the sensitivity meter, slider and Test mic in voice
  activity only; the push-to-talk key in push-to-talk only; noise
  suppression, echo cancellation, automatic gain), *Sounds* (join & mute blips — and the
  @mention / ping chimes —, your join chime, soundboard volume and mute),
  then *Advanced*, folded each time Settings opens (quality, Hide my IP
  address). Notifications is *On this device*, then *Quiet hours*. Groups
  are etched group boxes; Advanced is an "Advanced >>" button.
- **Reactions:** pills under messages (click to toggle yours), a picker
  (quick picks, custom emojis, search by shortcode) from the hover toolbar.
- **@mentions & pings:** who's mentioned follows the one rule of
  `web/js/mentions.js` (`core::find_mentions`): names longest first, never
  right after a letter, digit or `@`, never inside `code`, a link or a
  custom emoji, and those count as words (so "`x`@Ben" and
  "<:wave:…>@Ben" mention nobody); the highlight agrees exactly (a unit
  test runs the shared examples through both). An unread-mention badge on
  the channel — also for mentions that came while you were away
  (`unread.counts` › `mentions`, asked on every connect) —, a chime, and a
  desktop notification when Yap isn't focused (Settings › Notifications: every
  message / only @mentions and pings / off); 📣 "Ping everyone" (megaphone in
  the title bar) with an optional note; incoming pings chime + notify.
- **GIFs:** messages render (still frame, plays on hover); the **GIF picker**
  talks to KLIPY directly (trending, recent, search; "Powered by KLIPY"; shares
  reported), per PROTOCOL.md › GIFs.
- **Search** (the magnifier in the chat header, Ctrl+Shift+F, or ⌘F / ⌘⇧F on a Mac): a
  panel on the right (a third window in Win98; on a narrow window it floats
  over the chat, under its header). Type words (all must appear),
  `from:Name`, `has:image|gif|link`, "a phrase"; "This channel" / "Everywhere";
  it searches after a short pause. Results are newest first: channel · author ·
  time, then the text with the matches highlighted. Encrypted Personal-space
  notes never match on the server, so while the space is unlocked its history
  is loaded (up to 20 older pages) and the decrypted notes are searched here
  too (`core::local_matches`). Clicking a result opens its channel and jumps
  to the message: older pages load until it's there (≤ 20), it scrolls into
  view and flashes, or a toast says it's further back. Reply quotes jump the
  same way.
- **Pins:** the pin in the hover toolbar pins/unpins (`pin`); pinned messages
  carry a small pin; the header's pin button shows how many and opens the
  channel's pinned list (`pins`; click → jump, × → unpin). `pinned` updates
  arrive live (and "Name pinned a message." for others' pins).
- **Draw** (Places › Draw, inside the app; voice keeps going): the shared board
  of `web/js/draw.js` — Paint's colour box, sizes S/M/L/XL, pen / line / box /
  circle / text, eraser, the "wiggle" line boil (the same position-seeded noise
  as the web, checked by a unit test against values from its code), undo
  (Ctrl/⌘+Z), live strokes from everyone (`draw.seg`) with their name at the
  tip, finished strokes (`draw.stroke`), and the gallery (click to open full
  size). **Pixelly, like WigglyPaint:** the page is drawn at a fixed 640 × 400
  by our own rasterizer (`src/drawing.rs`: round-capped segments, the web's
  quadratic smoothing, eraser, text from the hand font's glyph outlines via
  `skrifa`), every pixel on or off (the web anti-aliases then snaps alpha ≥ 110;
  here a pixel is on when its centre is inside the shape, which comes to the
  same), and shown blown up nearest-neighbour. **Three layers**
  (PROTOCOL.md › Draw layers; a stroke's `l`, 0 = bottom): each layer is its
  own transparent sheet per wiggle frame (frames × layers textures; a new
  stroke is drawn on top of its layer; undo / clear / a wipe redraw that layer,
  Wiggle redraws all), its strokes in order so an eraser only clears its own
  layer, stacked bottom to top on white paper. Strokes in progress go on their
  own layer too: a layer with live strokes shows a copy of its sheet with them
  drawn on (the eraser really erasing), so ink under a top layer stays under it
  while it's drawn. The toolbar's Layers group has a chip per layer (1 bottom …
  3 top): a little picture of it, its number, pressed for the layer you draw
  on (keys 1 2 3), and an eye that hides it on your screen only (choosing or
  drawing on a layer shows it again); neither is ever sent. **Save** = a
  640 × 400 PNG on white, straight lines, to
  Downloads and the gallery (`/api/drawing`); **GIF** = the 3 wiggle frames,
  140 ms each, looping (NETSCAPE2.0; one frame with Wiggle off), encoded with
  the pure-Rust `gif` crate (one global palette, like `web/js/gif.js`), to
  Downloads and the gallery; **Share** posts the GIF with Wiggle on, the PNG
  otherwise (image size 640 × 400) — Save, GIF and Share export what you see
  (hidden layers left out); **Clear** asks "Clear layer N" (the one you're on)
  or "Clear the page" (red danger buttons, neither the default): both archive the whole page (every layer) to the gallery
  as that GIF, then send `draw.wipe {l}` or `draw.clear` — only once the upload
  landed. Encoding and
  uploading run off the UI thread. Gallery thumbnails are nearest-neighbour
  too (and play on hover).
- **Space** (Places › Space): the world of `web/js/space.js` / PROTOCOL.md ›
  Space, map and all (`space-map.js`, drawn like `space-art.js`) — nine
  rooms: Lounge, Cinema (its screen and couches), Music, Arcade along the
  top; Garden (the fountain), Pond (a dock out to the middle, lily pads,
  ducks), Pitch, Campfire along the bottom; the Gallery, a long hall between
  them; the Plaza round it and the race track round the edge. WASD / arrows
  (Shift runs) or click where to go, E sits on the nearest free seat (or looks
  at the drawing in front of you), 1–6 emote, T tag, F throws a water balloon,
  characters (an emoji or your picture), everyone else gliding to their
  `space.pos`, and the radar (the track, room names — short ones on a small
  radar — yours lit, the ball, everyone, "it" ringed red, burning orange,
  dripping blue). **Rooms are voice zones:** "This room only" (the default:
  your zone at 100 %, the rest of Space at 0 %), "Nearby is louder",
  "Everyone" — eased over ~0.4 s so doorways crossfade, applied as per-person
  factors on top of your own volume settings (people in the call but not on
  Space are left alone). The "your room" panel shows the zone, who's here (in
  the call? 🔥 / 💧?), the voice picker and a hint. **Games over
  `space.game`:** `sync` on every `space.state` (we also announce our own
  effects and best lap then); the **ball** (walk into it to kick; the last
  kicker owns and broadcasts it ~10×/s while it rolls; friction, bouncy walls
  and posts, goals for Blue/Red with the scoreboard and confetti over the
  scorer, back to the spot after 1.8 s; `k` / owner / lower-sid acceptance),
  **tag** (T, the Tag button or the TAG spot; 90 s rounds, touch to pass it,
  2.5 s no tag-backs, sitting is safe, least time as "it" wins; beats every
  3 s; if "it" leaves or goes quiet the lowest sid hands it on), **fire and
  water** (`fx`: walk into the campfire and you burn for 5 s after you step
  off; the fountain, wading in the pond or a balloon soaks you for 6 s; water
  puts fire out, fire dries you instead (then 1.2 s safe); you announce your
  own, re-announced when it drifts 1.5 s; flames and embers, a wet sheen with
  drips, puddles behind wet people, steam), **water balloons** (`throw`:
  650 units/s, 520 range, one per 0.7 s; only the person hit decides, `fx`
  with `by` + `hit`, and the splash board counts) and **laps** (`lap`: run the
  lane clockwise past the four corner flags; your own clock in the game line,
  everyone's best on the LAP RECORDS board on the Plaza where you start, at
  the web's spot, 440, 680). The **floor
  piano and drum pads** in the Music room need no messages: whoever steps on a
  key lights it and you hear it if you're in the Music room too (not
  deafened, Sounds on) — synthesized like `space-sound.js`
  (`src/audio/space_sound.rs`) and played through the clip bus. **The
  Gallery** hangs Draw's saved drawings (`draw.gallery` on every
  `space.state`, then each `drawing`; "🖼 Name hung a new drawing"), twelve
  to a wall, newest first, with your own ◀ Newer / Older ▶ paging in the room
  panel; frames show the pixel art nearest-neighbour with a name plate (the
  artist in their colour, the date) under hard beams of light.
  Stand still in front of one for 0.45 s (or press E) and a viewer opens with
  the real image (GIFs play): ‹ › / ← → through every drawing, Esc or ✕
  closes it (it won't reopen until you step to another frame), walking away
  closes it. Small windows: the room panel leaves room for the radar, game
  news moves under the radar, the HUD gets compact. Decorative motion is
  stepped (4 fps), like Windows 98. Being on a view makes
  you visible: the sidebar lists who's on Draw and Space.
- **Places & Minecraft:** the server's links (`ready.links`, live `links`
  updates; a board link, `view: "board"`, only comes from a server that has a
  board and opens `<server>/board`; [+] adds an https link, right-click
  removes one), Draw and Space (in the app), **Watch** (Watch together, a
  shared YouTube player: opens the browser signed in — a one-time
  `device.handoff` pass, `#key=…&place=watch&handoff=…&name=…`; under it
  "▶ title" from `watch.state` — asked for with `watch.get` on every connect —
  and the audience, sessions with `watching: true`, kept live by `watching`),
  When's good (the browser, signed in the same way, `place=when`), and, when
  `/api/config` names a Minecraft server, its status in the status bar
  (`/api/minecraft`, polled once a minute while focused; hover for who's on,
  click to copy the address). A server without one shows no Minecraft at all
  and is never polled.
- **Sidebar footer:** pinned at the very bottom of the left panel, outside
  the scrolling list (like the web's): "v0.x · what's new" (the changelog),
  Updates (this app's updates: checks and opens the update dialog) and Privacy
  (`<server>/privacy`, also in Settings › About), under an etched line.
- **Also:** create/archive channels, delete messages (yours; mods: anyone's),
  avatars (pictures from `/media/<id>`), add a custom emoji with the emoji
  picker's "Add emoji…", right-click one in the picker to delete it.
- **Voice:** join/leave, mute/deafen, speaking rings, per-person volume
  (0–200%, a limiter past 100 %; click or right-click someone in a voice
  channel: the popup is the web's — volume, **Watch screen** while they
  share, and the owner/mod actions: make or remove mod, Reset password…, Ban…,
  asking first; nothing for your own row), voice
  activity with a threshold (the web's gate: it opens a little *before* you
  speak and ignores a steady fan), push-to-talk (global, with a 200 ms tail),
  always on, input/output device pickers (the system default is followed
  when it changes), a level meter (the mic only opens in a call or while
  **Test mic** is on, and the test stops when you leave Settings › Voice),
  **Hear yourself** (Settings › Voice › Mic check: 4 s of exactly what a call
  sends — noise removal, gain, voice activity, Opus at the call's quality —
  played back; in a push-to-talk call, hold the key), quality
  64/96/128/256/510 kbps (default 128), RNNoise noise suppression (default on),
  **Echo cancellation** (default on; it applies at once, in a call too, and
  Hear yourself includes it), **Automatic gain** (default on), calls that survive reconnects and server
  restarts (no gap, no chimes), a quiet word next to someone when their
  connection is off (*weak connection*, *needs the voice relay* /
  *can't connect*, *reconnecting…*), "Your mic is sending silence" in the
  voice panel,
  **Hide my IP address in calls** (`"relay_only"` in config.json, like the
  web's `relayOnly`: every peer connection gathers TURN relay candidates only,
  `iceTransportPolicy: relay`, and never falls back to host candidates; it's
  greyed out until `/api/ice` hands out a `turn:` relay — this stack can't use
  `turns:` — and a change applies from the next call),
  join/leave/mute blips. **Channel quality:** the voice panel's ⋯ (next to
  the channel name) sets one bitrate for everyone in the channel
  (`channel.bitrate`) or "each person's own"; the panel only says "Sound:
  256k for everyone" when one is set. A call uses `channel.bitrate ?? your
  setting` and renegotiates when it changes. An OBS link (`client: "obs"`)
  listens without showing up while its account is in the call, like on the
  web: it isn't listed under the channel and gets no join or leave chime,
  but the call connects to it as to anyone (`Model::voice_members`; the call
  report names it "Name's OBS"). While only the OBS link is there (its
  account left; a newer server lets it listen for 5 more minutes,
  PROTOCOL.md › Voice › OBS links in a call), the channel shows the account
  greyed out — a faded picture and "Ada's OBS is still listening" in grey
  italics, the tooltip saying why — so nobody's heard without knowing. It's
  an in-between state, not a call member: no volume on click, no speaking
  ring, no marks, not counted (`Model::obs_listening`). The voice track goes out in no stream
  (`a=msid:- <track>`, like a browser's `addTransceiver(track)`), so it's
  never taken for a screen share.
- **Audio redundancy (RED, RFC 2198)** (PROTOCOL.md › Voice › Redundancy;
  `voice/red.rs`): every voice packet also carries the 20 ms frame before
  it, so a lost packet is rebuilt word for word from the next one instead
  of roughed in by Opus FEC or patched over. Offered and answered first on
  the voice, both ways, with every client that offers it (browsers send it
  because it leads *our* description); plain Opus with apps that don't.
  Opus stays at your quality; the copy rides on top (a 128 kbps voice is
  ~260 kbps on the wire per person). At 256 and 510 kbps the copy doesn't
  fit in a packet (1200 bytes), so those go as RED with their own frame
  only. `YAP_RED=0` leaves it out (a test switch: plain Opus both ways,
  like an older app). Measured against Chrome through the interop's proxy
  (60 s calls at 128 kbps, the same drops with RED off and on): audio
  Chrome patched over from us, 2.45 → 0.21 % Wi‑Fi-like, 4.08 → 0.53 %
  cell-like, 5.30 → 0.27 % at 5 % loss, 2.36 → 0.10 % at 2 %; what we had
  to rough in or patch over from Chrome (FEC + PLC), 2.02 → 0 %, 4.74 →
  0.23 %, 4.35 → 0.16 %, 2.01 → 0.06 %.
- **Call report** (PROTOCOL.md › Voice › Call report; `core/report.rs`, in
  the web's words): **Copy call report** in the voice panel's ⋯, and
  Settings › Voice › Mic & output › **Copy last call report** (Copy call
  report while in one; disabled with "No call yet." before the first) put
  what the app noticed about the call on the clipboard ("Call report
  copied."): the app, computer, channel, start and length, your settings
  and devices; per person the connection (direct or relay, candidate
  *types* only), codec each way, the Opus fmtp, kbps, round trip, lost
  packets, audio patched over, frames RED rebuilt, the jitter buffer,
  ICE restarts / rebuilds / reconnects kept, how long *weak connection*
  showed; then a timeline (`+mm:ss`, ≤ 2,000 lines: joins and leaves,
  connection states, socket drops and the call kept, mute, settings and
  device changes, weak connection, a line per person every 10 s for the
  first minute then every minute, why it ended). Never an IP address
  (types only, and the text is scrubbed of anything that looks like one),
  message text or a key. The last three stay in `calls.json` next to
  `config.json`; nothing leaves the computer unless you paste it.
- **Screen sharing** (PROTOCOL.md › Screen sharing; `src/screen/`, the UI in
  `ui/screen.rs`, `ui/dialogs/share.rs` and the voice panel):
  - *Watching:* the red LIVE badge by someone in the voice list (or
    **Watch screen** in the popup you get by clicking their name) asks them
    for it (`signal {screen: "want"}`; joins their voice channel first if
    need be) and shows it big in the chat's window, fit to it with black
    bars, never cropped or stretched: "Connecting to Ana's screen…" until
    the first picture, "Ana's screen is paused." after 3 s without one,
    "Ana stopped sharing." when they stop (the view closes itself 2 s
    later). **Pop out** (a window of its own; the chat comes back),
    **Full screen** (or double-click the picture; Esc leaves), **Stop
    watching** (`"stop"`), and **Sound**: the share's sound volume, 0–100 %,
    per person, remembered (`"screen_volumes"` in config.json), apart from
    their voice volume. Opening another channel or Draw/Space stops watching
    unless it's popped out (like the web). Their video is H.264 only
    (`profile-level-id=42e01f;packetization-mode=1`, which every client
    offers), reordered and decoded by OpenH264 on a thread of its own while
    you watch, and nowhere else; a missing keyframe is asked for with a PLI
    (twice a second at most), so a picture shows within about a second.
    The share's sound is the second audio track (the voice is always the
    first transceiver), played through its own mixer stream. Transport-cc is
    negotiated for video only (the web's sender ramps up fast); audio
    sections still never carry it.
  - *Sharing* (in voice): **Share screen** in the voice panel opens Share
    Screen: your screens and windows (listed by the OS: ScreenCaptureKit on
    macOS 12.3–13, Windows.Graphics.Capture on Windows 10 1903+, X11 on Linux;
    Wayland says "Screen sharing on Wayland isn't supported yet."),
    **Smooth / Sharp / Max** (the web's presets and numbers: 1080p60 at
    8 Mbps, 1440p30 at 6 Mbps, up to 4K60 at 25 Mbps, per viewer) and
    **Share sound** (the computer's sound without Yap's own, so the call
    never goes back into the call: ScreenCaptureKit's
    `excludesCurrentProcessAudio` on macOS 13+, process loopback leaving out
    Yap's process tree on Windows 10 2004+; not on Linux, where nothing can
    leave Yap out). `voice.state.screen` says you share; whoever asks gets an
    H.264 track (and a stereo 160 kbps Opus track for the sound) in the
    share's own stream on their existing connection, by renegotiation; a
    viewer who comes back reuses the same transceivers. Nothing is captured
    or encoded until someone watches, and it all stops when the last one
    leaves: one encoder serves every viewer, keyframes when someone joins or
    asks (PLI/FIR), a still picture is resent twice a second (viewers never
    think it's paused), and it backs off when viewers lose packets (from
    their receiver reports). While sharing: **Stop sharing**, "Watching:
    Ana, Ben" / "No one's watching yet" (from who asked; people drop out
    when their connection goes), the quality (no restart), **Share sound**,
    and **Share something else** (another window or screen, same tracks).
    Leaving voice stops sharing and watching; a call kept across a
    reconnect keeps the share (`voice.join {…, screen: true}`).
  - *Sharing on macOS 14+: Apple's own picker* (`SCContentSharingPicker`,
    like the iPhone/Mac app). Yap never lists your screens and windows
    itself there, which is what made macOS 15+ ask again and again whether
    Yap may "bypass the private window picker". **Share screen** opens Share
    Screen with just the quality and **Share sound**, and its **Share…**
    button opens Apple's picker: a window, an app or a whole screen (Yap's
    own windows aren't offered; a whole screen does show Yap's window, as on
    the iPhone/Mac app and in browsers). Cancelling it does nothing; if it
    can't open at all, Yap's own list shows instead (as on 12.3–13). The
    picked thing gets one ScreenCaptureKit stream for the whole share (macOS
    hands over its consent for that stream, so it's never started twice):
    idle at 1 fps with nothing copied while nobody watches (the newest
    picture is kept by reference, so a new viewer sees one at once on a
    still screen), the share's quality while someone does; it always takes
    the computer's sound, which only goes out with Share sound on.
    **Share something else** opens Apple's picker for that stream and swaps
    what it shows in place (no new stream, viewers keep watching); macOS's
    own menu can do the same. Stopping it from the menu-bar indicator stops
    the share in Yap ("Screen sharing was stopped from the macOS menu bar.").
    On 15.2+ a watchdog also notices a shared window closing or a screen
    going. macOS 12.3–13 keep Yap's own list; `YAP_DEBUG_SCREEN=pattern` and
    headless tests always do (the test pattern, never the real screen).
  - H.264 (constrained baseline): on a Mac, the hardware encoder through
    VideoToolbox (`screen/vt.rs`: NV12 in, Annex B out, the SEI dropped,
    bitrate changes on the running session, no periodic keyframes);
    elsewhere, or with `YAP_SCREEN_ENCODER=openh264`, or if the hardware
    fails three times in a row, OpenH264 on the CPU (built from its bundled
    C++ source by the `openh264` crate). Decoding is always OpenH264. The x86
    builds use OpenH264's assembly only when `nasm` is on the build machine
    (`brew install nasm`); without it they're plain C++ (slower).
    `scripts/build-cross.sh` says which (`OpenH264 x86 assembly: NASM version …`
    or a warning); a build cached from before nasm was installed stays plain
    C++ until `cargo clean -p openh264-sys2 --release --target <triple>
    --target-dir target/cross`.
- **The v0.36 extras** (PROTOCOL.md › Extras; `ui/extras/`):
  - **Spoilers:** `||like this||` (one line, no `||` inside) is a solid block
    until you click the message (not remembered); quotes, search, Catch up
    and notifications show `▒▒▒`.
  - **Polls:** `/poll Question | A | B` (2–10 different options) posts one;
    each option is a bar with its count and share, yours ticked; click to
    vote, click yours again to take it back (`poll.vote`, live `poll`).
  - **Dice:** `/roll`, `/roll 20`, `/roll 2d6+3`, `/flip` are rolled by the
    server; the message shows a dice card (each die, then the total) or a coin.
  - **YouTube → Watch together:** a message with a YouTube link (watch, youtu.be,
    shorts, live, music.) gets **▶ Watch together**: `watch.queue` adds it
    (plays now if nothing is on) and the browser opens Watch together signed in.
  - **Quick switcher:** Ctrl+K (⌘K) — channels, Places, people (opens them in
    People), actions (Settings, Catch up, the shelf, pins, Ping, Plan a hang,
    away message, the soundboard in a call); ↑/↓, Enter, Esc; the last row
    always searches messages.
  - **Since you left:** opening a channel you hadn't read (or coming back after
    a restart) draws a quiet "New since 9:41 PM" line above the first new
    message — found with `unread.counts` when it's further back than a page —
    and an "↑ Jump to first unread" pill while that line is off screen.
    **Catch up** (sidebar, only while something is unread) lists those
    channels — the ones that mention you first ("1 for you · 5 new"), then
    the newest — with the first new line; click one to open it there.
    "Mark all read" reads every channel up to its newest message. A device
    that has never read anything (a newcomer, a new computer) starts with
    everything read.
  - **Save to Personal:** the bookmark in the hover toolbar sends a note to your
    Personal space that quotes the message (`replyTo`, empty body); its quote
    jumps back to the original in its own channel. The first save into an
    encrypted space says the server still sees which message you saved.
  - **Away messages:** Settings › Profile (until my next message / 1 h / 3 h /
    tomorrow morning; "I'm back"). Shown only in context: in People, the
    volume popup, your own nameplate, and the `away` auto-reply line under
    your message ("Ada is away: at work til 6 · back at 6:00 PM").
  - **Join chimes:** Settings › Voice: the built-ins (synthesized from the
    spec's table, at their own quiet volume) with ▶ Hear it, or upload your
    own (≤ 2 s). When someone joins your channel you hear their chime instead
    of the generic blip; nothing while deafened or with Sounds off.
  - **Soundboard:** the voice panel's Soundboard button opens the clips; click
    plays it for everyone in your channel (`sound.play`), right-click hears it
    here only or deletes it (yours; mods: any). Playback goes through one clip
    bus (`audio/clips.rs`): gain = the clip's measured gain × your soundboard
    volume (Settings › Voice, default 50 %, max 100 %), a limiter (−12 dBFS,
    20:1, 3 ms attack), at most two clips at once, silent while deafened, with
    the soundboard muted, or from people you've turned down to 0. Out of a
    call (previews, Space's piano) a short-lived output stream plays them and
    closes after a few idle seconds. **Add a sound…** (`ui/sounds.rs`) decodes
    WAV, MP3, M4A/AAC, OGG Vorbis/Opus, FLAC, AIFF, CAF (`audio/decode.rs`:
    Symphonia, pure Rust; Opus through `ropus`), shows the waveform, picks a
    ≤ 5 s part (chimes ≤ 2 s; leading silence skipped), plays it, then mixes
    it to mono 24 kHz 16-bit WAV with 10 ms fades and posts `/api/sound`.
  - **Doodle reactions:** "✎ Draw one…" in the reaction picker opens a 48 × 48
    pixel box (`ui/doodle.rs`: 10 colours, 3 pens, eraser, undo, clear, a
    3-frame Wiggle like Draw) saved at 96 × 96 as a looping GIF (or a PNG),
    ≤ 64 KB, to `/api/doodle`, then reacts `<d:ID>`. Doodles show about two
    lines tall; click one to see it big (and remove yours).
  - **Media shelf:** the pictures button in the chat header opens the
    channel's pictures and GIFs as a grid on the right (`shelf`, "Load more");
    click one to jump to it.
  - **Quiet hours:** Settings › Notifications: a nightly window, "Let
    @mentions and replies through", "Snooze until tomorrow morning"
    (`notify.quiet`, with your IANA time zone). This app's own notifications
    and their sounds follow the same rule; sign-in alerts always come through.
  - **Hangs:** Upcoming (sidebar) lists the plans (and one that's on "now";
    click joins its voice channel, right-click cancels yours); [+] / "Plan a
    hang…" picks a title, a day, a half hour and a voice channel
    (`hang.set`). When one starts (`hang.now`), a calm bar says "Game night is
    starting · Lounge" with Join voice (and a desktop notification if Yap is
    in the background and it isn't quiet time).
- **Waiting room:** only the owner and mods let people in. They see join
  requests (a bar each — three or more collapse into one "N people want to
  join [Review]" bar, like the web — the People sheet and the header's count,
  with a chime and a notification); members see none of it, even from an
  older server that still tells everyone. The newcomer's window says
  "Waiting for the owner or a mod to let you in." Someone let in shows up as a
  quiet "Theo joined — invited by Priya" line in the first channel, for
  this session only (never a popup); a turned-away person is a toast. The
  newcomer hears a chime and sees "You're in. Say hi 👋" there (not a line
  about themselves). Members never get the request, so their line comes
  from `join.resolved.user` (`{ id, name, color, invitedBy? }`): "Cleo
  joined — invited by Ada", or "— let in by Ben" without an invite (an older
  server without it: the member list).
- **Look:** theme (Charcoal / Windows Standard / Match system), **Size** — Auto (the
  default: 115 % on top of the OS scale, 130 % on a 4K-class monitor the OS
  runs at 100–125 %, worked out once the zoom has settled, since egui reports
  the monitor in points) or 90–175 % (`"size"` in config.json; 0.6.0's
  `"zoom"` moves into it). Cmd/Ctrl + and − step through 90 / 100 / 110 / 125 /
  150 / 175 % anywhere, Cmd/Ctrl 0 goes back to Auto (egui's own zoom keys are
  off). The window starts at 1120 × 740 *points*, so a bigger UI gets a bigger
  window, and its minimum (680 × 440 points) grows with it, kept on screen.
  Settings scroll inside the dialog and dialog buttons take one row, so
  nothing falls off a short window at 175 %.
- **Updates (signed, click to install):** see below. Builds without an update
  key don't have them (see Self-update above).

## Updates

A build with an update key (see Self-update above) fetches
`<server>/downloads/latest.json` at launch (once online) and every 6 hours:

```json
{ "version": "0.2.0", "date": "2026-10-05", "notes": ["…"],
  "files": { "linux-x86_64":   { "path": "/downloads/yap-linux-x86_64", "size": 123, "sha256": "<hex>", "sig": "<base64>" },
             "windows-x86_64": { … }, "macos-aarch64": { … } } }
```

If `version` is newer (semver) than the running one, an **Update to X** button
appears in the status bar (the notes are in its tooltip and in the dialog it
opens); Settings › About has **Check for updates**. Nothing installs until you
click **Install and restart**: Yap downloads the file next to its executable
(a hidden `.yap.update-*.tmp`), checks the size, the SHA-256 and an **Ed25519
signature over the raw file bytes** against the public key built into the
binary (`YAP_UPDATE_PUBKEY` at build time, read with `option_env!` in
`src/update.rs`), and only then swaps the executable (Windows: the running
`yap.exe` is renamed to `yap.exe.old` and the new one moved in; Linux/macOS:
replaced in place) and restarts with the same arguments. Leftovers
(`yap.old`, temp files) are deleted on the next start. A bad file leaves the
old executable untouched.

A build without a key never fetches the manifest and never shows Update.
Settings › About and the sidebar's Updates link say "This build doesn't
update itself", with a button for `<server>/download`. The log says
`self-update off` at start.

Debug builds only: `YAP_UPDATE_URL` (manifest URL) and `YAP_UPDATE_PUBKEY`
(base64 key) override the defaults at run time for tests; release builds
ignore both (the strings aren't even in the binary). From the command line:
`yap --server URL --update-check` / `--update-install` (`UPDATE off` without
a key).

Release artifacts (`./scripts/build-cross.sh` → `target/dist/`, which
`tools/publish-desktop.mjs` reads directly) are **bare executables**, named
after the manifest keys:
`yap-linux-x86_64`, `yap-windows-x86_64.exe`, `yap-macos-aarch64` (ad-hoc
signed, not an `.app`). The version comes from `Cargo.toml`.

`tests/update-e2e.mjs` checks the whole path against a local server and a
throwaway key: tampered bytes, a signature from another key and a size lie are
refused with the executable untouched; a good update is swapped in and
relaunched with the same arguments; leftovers are cleaned; a build without a
key never asks; a release build ignores the overrides
(`node desktop/tests/update-e2e.mjs`, port `YAP_UPDATE_PORT`, default 8796;
needs `target/debug/yap`, and `target/release/yap` for the last check;
`CARGO_TARGET_DIR`, `YAP_BIN` and `YAP_RELEASE_BIN` move them).

## Headless mode (tests)

No window, no audio devices: a WAV file is the mic and what's heard is measured
(optionally recorded). Logs events and, once a second, per-peer stats; ends with
one `SUMMARY {json}` line.

```sh
yap --headless --server http://localhost:8788 --name Bot --join-voice lounge \
    --input-wav file.wav --say "hello" --duration 20
```

`--password P` (sign in, or sign up if there's no such account; an old
6-digit PIN signs in too; `--pin P` is the same flag; prints `TOKEN …`,
`PENDING uid …` while waiting and `ACCOUNT uid=… secret=pin|password
weak=… prompt=none|Pin|Temporary|Guessable role=…` once in), `--signup`,
`--token T` (one of `--password` / `--token` is needed), `--set-password NEW`
(once in, change it with the sign-in one as the current: `PASSWORD saved` or
the server's `Error`), `--approve-all` (let everyone in from the waiting room:
a test doorman, which has to be the owner or a mod),
`--react-to TEXT --react EMOJI`, `--ping TEXT`, `--send-gif QUERY`,
`--edit-to TEXT`, `--send-image FILE`, `--add-link NAME=URL`,
`--make-invite LABEL` (prints `INVITE <label> <link>`), `--reset-main-link`
(as the owner, reset the main invite link once in; every app connected logs
`KEY saved sha256:<first 8 hex of the new key's SHA-256>`, never the key,
and so does one that saves a different `ready.key`), `--channel-bitrate N`,
`--reply-to TEXT [--reply-body TEXT]` (reply to someone's message containing
TEXT, live or already there; prints `REPLIED to <id>`; incoming replies log
`[reply to <id>]`), `--vault PASSCODE` (encrypt the Personal space or unlock
it; `--say`/`--send-image` then go there, sealed; prints `VAULT set|unlocked`,
`SEALED n earlier notes` and `NOTE <id> text=… [image <blob> …]` for every
encrypted note it can open — headless never touches the OS keychain),
`--delete-after S` (delete our last picture, else last message),
`--search Q [--search-here] [--jump-oldest]` (prints `SEARCH n results: …`,
`JUMPED <id> after n older pages`), `--pin-msg TEXT` (`PINNED`, `PINS`),
`--draw` (opens the board, draws a line on the top layer, a box and a word,
saves them to the gallery as the GIF; logs `DRAW …` incl. `DRAW wipe <l> by
<uid> (<n> gone)`; every bot also logs `WATCH …` and `SCREEN …`),
`--space` (walks in, moves, waves, sends `sync`, kicks the ball, throws a
water balloon, announces it's on fire and posts a lap; logs `SPACE …`
including `gallery <n>` and incoming `space.game`),
the extras: `--poll "Q|A|B"`, `--vote N` (in the first poll someone else
posts), `--say '/roll 2d6'`, `--away TEXT`, `--hang TITLE [--hang-in S]`,
`--sound-file WAV --sound-name NAME`, `--play-sound NAME` (once in voice),
`--chime ID`, `--shelf`, `--unread`, `--save-msg TEXT`, `--doodle-on TEXT`,
`--quiet`, `--reply-to-missing TEXT` (send TEXT as a reply to a message that
doesn't exist: `SEND refused …` and `RESTORED #channel "TEXT"`; many `--say`s
at once log `SEND retry <nonce> <n>` when the server slows them down; the
SUMMARY has `sendRetries`, `sendsRefused`, `pendingLeft` and `restored`),
`--last-read CH=MS` (start as if CH was read up to MS; the
counts asked on connect then log `UNREAD ch:n (m for you)` and `MENTIONS
ch m while away`) (they log `POLL`, `ROLL`, `SOUND`, `AWAY`, `HANG`,
`SHELF`, `UNREAD`, `MENTIONS`, `JOINED`, `QUIET` lines; join requests say
"link also used by …"),
`--mute-after S`, `--expect TEXT`, `--record-wav out.wav`, `--bitrate`,
`--mode open|vad|ptt`, `--denoise`, `--echo-cancel` (echo cancellation on
the WAV mic against what the bot hears; the stats line then adds `aec=` AEC3's
delay estimate, `erle=`, `realigned=`, `owed=`), `--speakers` (real output device),
`--drop-socket-at S[,S…]` (drop the socket then: the call is kept across it),
`--hear-yourself out.wav` (once in voice, run Hear yourself and save what came
out of the codec), `--share-screen [smooth|sharp|max]` (once in voice, share a
synthetic test pattern — 1280 × 720, colour bars, a moving square, a frame
counter; never the real screen), `--share-sound` (with a stereo test tone as
its sound), `--watch-screen` (watch the first person sharing in our channel;
`SCREEN frames <n> <w>x<h> sound <bytes>B` each second, and `screenFrames`,
`screenSize`, `screenSoundBytes`, `screenEncoded` in the SUMMARY). It logs `MENTION`, `REACTION`, `PING` lines, and for voice
`CHIME` (every join/leave chime it would play, sounds on or off),
`OBSLISTEN Lounge: Ada` / `OBSLISTEN none` (the greyed "<name>'s OBS is
still listening" rows, when they change), `RESUME` /
`GHOST` (calls kept across reconnects), `HINT` (connection hints), `MIC`
(sending silence), `AUDIO` (a stream reopened), `ICE`, `HEAR`; the stats line
each second has the jitter buffer (`jb=held/target ms`, `ins`/`drop`), the
frames RED rebuilt (`red=`), `red+opus` where a direction uses RED, the
encoder's loss hint and the round trip; the SUMMARY's peers add `sendRed`,
`opusBytesSent` (of `bytesSent`: the rest is RED's copies), `redPackets`,
`red` and `expected` (packets the sequence numbers say were sent).
`--call-report FILE` writes the call report there at the end (`CALLREPORT`
line). `yap --help` lists everything.

## Interop test (web ↔ native)

`YAP_IMPAIR=wifi|cell|loss5|loss2|none` runs the voice calls behind a UDP
impairment proxy (the voice audit's profiles, on every RTP packet of any
payload type, RED's 63 included; RTCP, STUN and DTLS go straight through)
and adds the native side's jitter-buffer numbers, and what crossed the
proxy each way (payload types, kbps), to the evidence. The proxy answers
each STUN check to the socket that sent it and sends media to the socket
media came from, so the web's checks on other pairs can't steal it.
`YAP_IMPAIR_SEED=<n>` makes its drops repeatable. `YAP_RED=0` turns RED
off in the native app (plain Opus both ways); `YAP_VOICE_SECS` makes the
voice calls longer (default 24).

`tests/interop.mjs` drives the web client in headless Chromium (fake mic) and
`yap --headless`: text both ways; sends the server turns away (native says 12
things at once: the server takes 10 and slows the rest down, native sends them
again 1.1 s apart with the same nonce, and the web gets all 12 in order, with no
error shown; a reply to a message that doesn't exist is refused, leaves no grey
copy, and its text goes back); @mentions (web highlight / native detection),
reactions (native reacts → web pill; web toggles → native sees), pings both
ways; a GIF sent from the native side through KLIPY (a local mock,
`tests/klipy-mock.mjs`, checks the search and share calls); edits both ways
(native edits → web shows "(edited)"; web edits with ↑ → native sees it);
search + jump (a web message buried under 60 newer ones: native finds it
everywhere and jumps to it, loading older pages); pins both ways (the web shows
the pin mark; native hears the web's pin); Draw (native's line, box and word
reach the web board, the line with `l: 2` and the word with no `l`; native's
GIF is a 640 × 400 GIF89a with 3 frames at 14 cs that loops and shows in the
web's gallery; a web stroke reaches native; the web's `draw.wipe` of the top
layer reaches native, which drops the line); Space (the web sees native
walk in and wave; native sees the web move; native's `space.game` ball reaches
the web's ball, the web's tag reaches native; native's balloon, fire and lap
reach the web (its lap on the web's board, native shown on fire), the web's
throw / fx / lap reach native, and native's `draw.gallery` matches the web's
Gallery);
mentions from while native was away (the web writes in a channel native
"read" just before: `unread.counts` says 4 new, 1 for you — `@Native` in
code or against a custom emoji doesn't count — and native's badge says
1); replies both ways (native replies → the web shows the quote and treats it as a
mention; the web replies with its Reply button → native sees `replyTo` and a
mention); the encrypted Personal space (the same account on both sides: native
sets the passcode and seals an earlier web note, writes a note and a sealed
picture; the web unlocks with the passcode and reads them, all bodies on the
server are `e1:`, the stored picture isn't the plain bytes; the web writes,
native reads; deleting the picture note erases its blob); a
3000×2000 picture the native client shrinks and uploads with its upload token
(and a `sid`-only upload refused with 403); a Places link added from native
showing in the web sidebar; a personal invite a newcomer joins with (the
waiting room says who invited them; the inviter, a member, gets no join
request but logs `JOINED New… joined — invited by Native…` from
`join.resolved.user`; they keep the main key; a second
newcomer with the same link: the join request says the link was also used
by the first); account sign-in with the test password + token reconnect
(no "Choose a password" prompt; a wrong password is `bad_login`); an old
PIN account (the test server's `TEST_PINS:1` lets the admin API's
`test-pin` give an account an old PIN; never set it anywhere else) signing
in with its PIN, asked once (`prompt=Pin`), choosing a password with the
PIN as the current, then signing in with the password while the PIN no
longer works, and after a reset signing in with the temporary password
(`prompt=Temporary`, from `ready.weakReason`) and replacing it; the waiting
room (a member's app hears nothing of a newcomer but logs who joined, "let in
by" the doorman; the doorman hears and lets them in; the newcomer logs
"we're in", not a line about itself); voice in
`lounge` in both join orders, checking audio both
ways, the negotiated Opus fmtp, bitrate, the speaking ring the browser shows for
us (RTP audio-level extension) and mute propagation, RED both ways (native's
description lists it first; native unpacks RED from the web and the web
receives about twice the Opus rate; through the proxy, payload type 63 on
every packet each way: Chrome's stats name Opus as the codec even then), and
native's call report (`--call-report`: the header, the web user's block with
the connection type and `red+opus` each way, the timeline, no IP address);
an OBS link (the web's `/obs` page, listen-only) in native's call with its
account (the web user) in it too: native connects to it and it hears native,
with no join or leave chime for it (the `CHIME` lines are the web user's:
join, leave, join), and native's call report names "Name's OBS"; when the
web user leaves, native shows them greyed out (`OBSLISTEN Lounge: Web…`)
while the OBS link listens on, and when they're back, nothing
(`OBSLISTEN none`);
and screen sharing +
Watch together (native asks `watch.get` at connect; the web's `screen: true`
in voice and its `watch.join` / `watch.leave` reach native, on and off);
screen sharing both ways with synthetic pictures (a web page of its own: its
getDisplayMedia is a moving 1280 × 720 canvas with a stereo tone): native
watches the web's share (it arrives as H.264 although the web's Sharp preset
prefers VP9, decodes at 1280 × 720, its sound on its own track; the web
stopping ends it on native) and the web watches native's test pattern from
the LIVE badge (H.264 at 1280 × 720 in the web's player, the share's sound
on its own track; Stop watching stops native's encoder); in both, RED
leads only native's voice section, the share's sound stays Opus-first; and
the extras: polls and dice both ways (native polls, the web votes and native
sees the votes; the web polls, native votes; native's `/flip` and the web's
`/roll 2d6` arrive rolled), away messages both ways (native @mentions the
away web user and gets the `away` reply; native's away reaches the web), the
soundboard (native uploads a WAV and plays it in voice: the web hears it; the
web plays it: native hears it), a hang a few seconds out (the web lists it,
both get `hang.now`; needs the dev server's `--var TEST_HANGS:1`), and a
doodle reaction + Save to Personal + shelf/catch-up requests from native.
Last, **the main link reset**: a native app made owner through the admin API
resets it; a native member connected meanwhile and the web page save the
same new key; a device that signed in before and was off connects with the
old key and its device token and saves the new key; a sign-in with the old
key is turned away (`BadKey`, from the 4003 close), and one with the new key
gets in. Then the admin API puts the server's own key back (`DELETE
/api/admin/main-key`; the member saves that too), so the suite can run again
against the same server.

```sh
# from the repo root: the local test server (port 8788, throwaway data, keyed;
# test/wrangler.test.jsonc adds a stand-in board and a Minecraft address that never answers):
npx wrangler dev -c test/wrangler.test.jsonc -c test/fixtures/board/wrangler.toml --port 8788 \
  --persist-to <scratch> --var YAP_KEY:testkey --var KLIPY_KEY:test --var ADMIN_KEY:testadmin \
  --var PIN_PEPPER:testpepper --var TEST_HANGS:1 --var TEST_PINS:1
cd desktop && cargo build --release && cd ..
YAP_URL=http://localhost:8788 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
  YAP_BIN=desktop/target/release/yap node desktop/tests/interop.mjs
```

Accounts sign in with the test password (`correct-horse-582039` — the server
refuses short and guessable ones) or sign up; a "doorman" bot
(`--approve-all`) lets newcomers in. Only the owner and mods hear about
newcomers, so the doorman is the owner on a fresh server and is otherwise
made a mod through the admin API (also when it's left over as a member from
an older run).
`YAP_ONLY=<regex>` runs just the matching tests. `YAP_WEB_MIC=synth` gives the
web pages a WebAudio mic instead of Chromium's fake one (headless Chromium on
macOS 27 hangs forever opening any audio capture in some sessions; the
screen tests always use it). `YAP_BITRATE=64000` checks another quality; `YAP_KLIPY_PORT` moves the mock
(default 8795). `YAP_KLIPY_API=http://localhost:8794/api/v1` points the app (or
`--send-gif`) at a mock; `node desktop/tests/klipy-mock.mjs 8794 local` serves
picture tiles for screenshots. Measured
(release build, localhost): web→native 128 kbps, native→web 123–129 kbps,
0 packets lost/concealed, fmtp
`maxaveragebitrate=128000;maxplaybackrate=48000;minptime=10;sprop-maxcapturerate=48000;sprop-stereo=0;stereo=1;usedtx=0;useinbandfec=1`.

`tests/echo-loop.mjs` checks echo cancellation end to end, through the whole
call path: two headless bots in `lounge`, one talking and recording what it
hears, the other silent but with its speakers fed back into its mic 60 ms
later at −6 dB (`YAP_DEBUG_ECHO_LOOP=<ms>[,<dB>]`, a headless test aid: the
null output's audio comes back into the WAV mic). Without `--echo-cancel` the
talker hears himself at about −21 dBFS; with it, about −80 dBFS (59 dB less;
AEC3 settles on a ~40 ms delay, the rest being its own backlog). Same
server and variables as the interop test:
`YAP_URL=http://localhost:8788 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin
YAP_BIN=desktop/target/release/yap node desktop/tests/echo-loop.mjs`.

`tests/vault-crosscheck.mjs` checks the encryption against the web's own code
both ways, at the real 600,000 rounds: Node makes a vault and a note (plus a
sealed picture) with `web/js/vault.js`, the Rust tests open them; Rust writes
its own, `vault.js` opens those (`node desktop/tests/vault-crosscheck.mjs`).

## How it's built

| Piece | Choice | Why |
|---|---|---|
| UI | `eframe`/`egui` 0.36, glow (OpenGL) backend | Immediate-mode, repaints only on events (0% idle CPU), ~10 MB; glow over wgpu for size and older GPUs. |
| Socket | `tokio` + `tokio-tungstenite` (rustls/ring) | No OpenSSL anywhere → trivial cross-compiles. |
| HTTP | `ureq` (rustls/ring) | Small, blocking, run on worker threads. |
| WebRTC | `webrtc` 0.21 (webrtc-rs, sans-IO core) | Full ICE/STUN/TURN, DTLS-SRTP, pure Rust. |
| Opus | `ropus` (pure-Rust port of libopus, bit-exact with the reference) | No C toolchain for any target. |
| Noise suppression | `nnnoiseless` (RNNoise) | Pure Rust. |
| Echo cancellation | `sonora` (WebRTC's AEC3, the canceller browsers use) | A pure-Rust port, BSD-3-Clause, checked against WebRTC's own C++ tests; builds for all three targets like everything else. The C++ original (`webrtc-audio-processing`) needs meson, ninja and abseil at build time, which `build-cross.sh` can't give cargo-xwin or zig; SpeexDSP's canceller is weaker, and the OS's (macOS voice processing) would cover one platform. |
| Audio I/O | `cpal` (WASAPI / ALSA / CoreAudio) + own sinc resampler | Prefers 48 kHz; resamples when a device can't. |
| Push-to-talk | own poller: `GetAsyncKeyState`, evdev `EVIOCGKEY`, X11 `XQueryKeymap` (dlopen), `CGEventSourceKeyState` | Sees key down *and* up globally without grabbing the key from games. |
| Screen video | H.264 constrained baseline: VideoToolbox's hardware encoder on a Mac (`objc2-video-toolbox`), else `openh264` (OpenH264 from its bundled source), which also decodes | Every client takes H.264; OpenH264 builds from C++ source for all three targets. |
| Screen capture | ScreenCaptureKit (`objc2-screen-capture-kit`, weakly linked: `build.rs`), Windows.Graphics.Capture + WASAPI process loopback (`windows`), X11 with MIT-SHM (`x11-dl`, dlopen) | Native APIs, no extra libraries to ship. |

Source map: `core/` (the protocol state machine, a port of `web/js/app.js`:
`mod.rs` the loop and connection, `model.rs` what the UI reads, `cmd.rs`
commands and events, `msg.rs` / `cmds.rs` server messages and commands by
part of the app, `vault.rs`, `search.rs`, `voice.rs`, `text.rs`), `net.rs`
(socket, heartbeat, backoff), `voice/` (peer connections, perfect
negotiation, Opus SDP tuning), `audio/` (capture → echo cancellation →
RNNoise → VAD/gate → 20 ms frames; jitter buffers → mixer → device and the
echo canceller), `ptt.rs`, `ui/` (egui:
`mod.rs` the app and frame; `sidebar.rs`, `chat.rs`, `statusbar.rs`;
`sections.rs` the pieces they share; `dialogs/`, `extras/`, `space/`,
`draw.rs`, `side.rs`, `screen.rs` (watching); `theme.rs` the widgets;
`debug.rs` the screenshot aids), `screen/` (screen sharing: `share.rs` our
share's capture → encode worker, `watch.rs` RTP → pictures, `codec.rs`
OpenH264 and the BGRA → I420 conversion, `vt.rs` the Mac's hardware
encoder, `capture/` the OS backends and the
test pattern), `headless.rs`.

### Voice details worth knowing

- **Opus tuning without SDP munging.** webrtc-rs refuses a local description
  that differs from what it generated, and answers with the codec parameters it
  negotiated from the *remote* side. So the tuned fmtp is registered on the Opus
  codec up front, and the remote description's Opus fmtp is rewritten to ours
  before it's applied (the peer's real wishes are read from the original). The
  result on the wire is exactly what `tuneOpus()` produces. Limitation: a
  quality change mid-call applies to what you *send* immediately, and to what you
  receive from the next join.
  Every audio section is tuned (a later one is a screen share's audio:
  ≥ 160 kbps), and audio sections carry no `transport-cc`: the interceptors
  are webrtc-rs's defaults minus transport-wide congestion control, which
  only makes libwebrtc pad audio-only calls with bandwidth probes
  (PROTOCOL.md › Voice › No transport-cc on audio).
- The encoder runs per peer at min(your setting, their `maxaveragebitrate`),
  DTX off, 20 ms frames, mono (`sprop-stereo=0`), and every packet carries
  the RFC 6464 audio level so browsers show our speaking ring. Its
  packet-loss hint follows the far side's receiver reports (libwebrtc's
  levels 0/1/5/10/20 % with hysteresis): 0 on a clean network keeps Opus in
  CELT (a third of Hybrid's CPU, better fidelity), anything else carries
  in-band FEC (Hybrid).
- **RED** (`voice/red.rs`; webrtc-rs only carries the packets): `red/48000/2`
  (fmtp `111/111`, payload type 63 like the browsers) is registered next to
  Opus and put first on the voice transceiver with `set_codec_preferences`
  before every offer and answer (the first offer lists both; later ones
  reorder what was negotiated, so an app without RED never finds it in an
  answer). A screen share's sound sections go the other way, Opus first
  (answering a browser's offer, webrtc-rs puts the codecs whose fmtp
  matches ours exactly first, and for a share's sound that's RED), and our
  share's sound always goes as plain Opus. The voice transceiver is the one
  with id 0, never "the first in `get_transceivers()`": that list comes out
  of a HashMap, so with a screen share's transceivers it's any of them.
  When RED was negotiated, each packet is the frame plus the one before
  (distance 1, timestamp offset 960); the copy is left out over 1023 bytes
  (only near 510 kbps) or past a 1200-byte payload (above ~239 kbps: so
  never at 256 or 510), and such a packet carries the primary block alone.
  Nothing counts the copy against a cap: the encoder still runs at the
  call's quality. Chrome's stats call the codec `audio/opus` even for RED
  packets (it names the one that decodes), so the tests look at the wire
  instead.
- The mic: an 80 Hz high-pass, echo cancellation (optional, below), RNNoise
  (optional), automatic gain (speech
  towards −23 dBFS while you talk, at most +20 dB, cuts fast, recovers
  ~3 dB/s, never clips; optional), then the transmit gate — a port of the
  web's `gate-worklet.js` (PROTOCOL.md › Voice › Transmit gate): hysteresis,
  a 450 ms hold, a 40 ms pre-roll in voice-activity mode only, a 200 ms
  push-to-talk tail, 5/15 ms fades, an adaptive noise floor. Mute/deafen
  are a hard switch after it; a muted mic sends silence rather than
  stopping packets, like a disabled browser track. If processing falls
  behind, noise removal is skipped until it catches up (fail open).
- **Echo cancellation** (`audio/echo.rs`): WebRTC's AEC3 through `sonora`,
  in libwebrtc's order (high-pass → AEC3 → noise suppression → gain; the web
  has the browser's AEC3 first and RNNoise after its gain) and at its
  default 32 kHz inside, like the browser: while it's on, the mic keeps
  everything up to 16 kHz. The far end is exactly what the call's output
  plays (voices, a screen share's sound, the soundboard, chimes, after the
  mixer's soft clip), tapped as each 10 ms block leaves the mixer. The two
  devices run on their own clocks and callback sizes, so the canceller takes
  one far-end block per mic block from a small backlog it keeps at 1–20 ms
  (judged once a second by its lowest point): silence for what hasn't been
  played yet (skipped when it arrives, so the two stay lined up), a fresh
  start when the output stream changes or stalls, and a correction when the
  clocks drift apart (one sample per block up to 2 ms, at once beyond, with a
  fresh AEC3, which settles faster than one hunting for a moved delay). AEC3
  estimates the rest of the delay itself (up to ~600 ms). Off, it's skipped
  entirely (bit-exact). Measured offline with synthetic speech, a room, and
  two simulated devices (`audio/echo_sim.rs`; `cargo test --release
  echo_report -- --ignored --nocapture` prints the table): echo 6 or 12 dB
  below what's played, 20/80/200 ms away, is down 20 dB within 0.25 s (1 s
  with Linux-sized 85 ms output periods) and 49–56 dB once settled, also with
  the clocks 50 ppm apart (five minutes of it: one 250 ms moment of 877
  under 20 dB, at a correction); new speakers mid-call with 70 ms more delay
  are back under 20 dB within a second. While both talk, AEC3 turns your
  voice down 4–6 dB (about half the half-seconds by more than 6 dB); with
  headphones (nothing comes back) 2–4 dB once it has heard a while that
  nothing does, more in a call's first seconds. Alone, your voice passes
  unchanged (0.1 dB). About 50 µs of CPU per 10 ms on an M4 Max (0.5 % of
  one core). Music mode, when the desktop gets it, must turn it off like the
  web's does.
- Receive: an adaptive jitter buffer per peer (`audio/jitter.rs`), keyed by
  RTP timestamp so padding-only probes are neither loss nor in FEC's way.
  RED packets (by payload type, ours or theirs) are unpacked: the primary
  goes in as usual, and a copy only for a frame whose own packet hasn't
  come and whose turn hasn't passed, never twice (its own packet arriving
  later replaces it); a frame played from a copy still counts as a gap, so
  the lossy 60 ms floor below holds. Plain Opus, RED with only a primary,
  and malformed RED (dropped whole) all work.
  Its target is the lateness 99 % of the last ~10 s of packets beat, +10 ms,
  40–200 ms (late packets raise it; at least 60 ms while packets are being
  lost, so FEC can repair them); clock drift and target changes are
  followed one frame at a time in quiet audio. Its tests are the voice
  audit's network simulation.
- One offer per join: webrtc-rs reports the offering side's `add_track` as
  `negotiationneeded` from its driver task, often after a fast answer has
  already come back; that late event is skipped when our track is already
  negotiated (it used to send a second, identical offer).
- Calls outlive the socket (PROTOCOL.md › Voice › Keeping calls across
  reconnects): a dropped socket keeps every connection, queues their
  signals and rejoins with `prev` + `proof`; people whose socket dropped
  stay "reconnecting…" up to 30 s; chimes wait 5 s; quitting sends
  `voice.leave`. Apps that don't resume (desktop 0.8.0 and older) are
  handled the old way. A dead path restarts ICE (impolite after 3.5 s
  `disconnected`, polite after 10 s, `/api/ice` fetched again, re-gathering
  on fresh sockets; rebuilt after three tries).
- The mic and speakers follow the system default (checked every 2 s in a
  call), come back to a picked device when it's plugged in again, and are
  reopened after a stream error or a 2 s stall. 5 s of digital silence
  while unmuted shows "Your mic is sending silence".
- Someone turned up past 100 % goes through a limiter (−3 dBFS, like the
  web's).
- Speaking rings come from decoded audio levels (yours from the mic gate).

### Push-to-talk by platform

- **Windows:** works everywhere, keyboard and mouse buttons 3–5.
- **Linux X11:** works everywhere (keyboard; middle mouse).
- **Linux Wayland:** global PTT reads `/dev/input` — add yourself to the `input`
  group (`sudo usermod -aG input $USER`, log out/in). Without that it only works
  while the Yap window is focused. (The XDG GlobalShortcuts portal is the
  future, permission-free route.)
- **macOS:** needs Input Monitoring permission for use outside the window.

## Not done yet

- When's good, Watch together and a server's board inside the app (they open
  in the browser).
- Screen sharing: hardware encoding on Windows and Linux (OpenH264 runs on
  the CPU there; Media Foundation / VA-API would do it), sharing on Wayland (needs xdg-desktop-portal + PipeWire),
  the share's sound on Linux, and a picker with thumbnails on Windows, Linux
  and macOS 12.3–13 (macOS 14+ uses Apple's).
- Stereo/music mode (so the gate never
  turns into always-on for music), phone-push (ntfy)
  settings, uploading your own avatar picture (shown, not uploaded).
- Colour emoji (unicode emoji draw in egui's monochrome emoji font; a few newer
  ones have no glyph).
- Screen-reader support (accesskit is compiled out to keep the binary small).
- Scroll-bar arrow buttons (egui's scroll area has no hook for them).

## Please test by hand

- **Voice on real hardware and networks** (the tests run on localhost with
  WAV mics): the gate and its pre-roll on a real mic (soft starts, a fan, a
  keyboard), Automatic gain with a quiet and a loud mic (and with RNNoise
  off), the push-to-talk tail, **Hear yourself** on real speakers and in a
  push-to-talk call, unplugging the mic / headphones mid-call and plugging
  them back, switching the system default (AirPods connecting), Bluetooth
  headsets (HFP vs A2DP), **echo cancellation** on real speakers (laptop
  speakers and a separate USB mic or speakers, on macOS, Windows and Linux,
  with a friend on the web and both talking at once: they shouldn't hear
  themselves, and you shouldn't cut out much; switching the output device
  mid-call; turning it off and on mid-call), a Wi-Fi ↔ cellular handover mid-call (ICE restart), a laptop
  sleeping in a call, calls with friends on CGNAT/cellular with and without
  the relay, someone at 200 % (the limiter), and a deploy mid-call with web,
  iPhone and desktop people in it.

- **The soundboard and join chimes on real speakers:** loudness at 50 % and
  100 %, two at once (a third cuts the oldest), the limiter on a loud clip,
  nothing while deafened / muted soundboard / someone turned to 0; a friend's
  chime when they join; previews out of a call (the idle output stream opens
  and closes); Bluetooth headsets and 44.1 kHz devices.
- **Adding a sound from real files** (MP3 with ID3 tags, M4A from a phone,
  OGG, FLAC, a long WAV): the waveform, trimming, Play, Upload.
- **Doodles with a real mouse/trackpad**, Wiggle on and off, and how they look
  on the web and the iPhone.
- **Quiet hours** against real OS notifications (and that sign-in alerts still
  come through), snooze until tomorrow, a different time zone on another device.
- **▶ Watch together / Watch / When's good / LIVE** opening the browser
  already signed in (`device.handoff`), on macOS, Windows and Linux browsers.
- **A hang starting** while Yap is in the background (the notification, then
  Join voice).
- **Reset main link** with real people: the owner resets it while friends
  are connected on the desktop app, the web and the iPhone, and while one
  computer is asleep or quit; everyone keeps working, Copy main invite link
  then gives the new link everywhere, the sleeping computer comes back
  without asking for a link, and the old link in a fresh profile
  (`--config`) says "The invite key changed…".
- **An OBS link in OBS Studio** while you leave and rejoin the call: friends
  see you greyed out ("…'s OBS is still listening") only while you're out,
  and after 5 minutes away it stops listening and the row goes.

- **Draw with a real mouse / pen / trackpad** (strokes, lines, boxes,
  circles, text, eraser, undo, the wiggle), **layers** (the chips, the eye,
  keys 1 2 3, drawing and erasing on a hidden or lower layer while someone
  draws on another), and **Save / GIF / Share / Clear (a layer or the page)**
  (files in Downloads, the gallery, the chat; tested through the headless bot
  and a debug trigger only, since the tests never click). Text uses Comic Sans
  where the system has it (Windows, macOS), else Comic Neue, else egui's sans.
- **Size:** Auto on a real 4K monitor at 100 % / 125 % on Windows and Linux
  (should start at 130 %), on a laptop (115 %), and Cmd/Ctrl + − 0. Moving the
  window to another monitor re-picks Auto.
- **Apple's picker on macOS 14+** (no test can open or click it): Share
  screen › Share… opens it, also for the bare `yap-macos-aarch64` binary (no
  app bundle) started from Finder and from Terminal (a window, an app, a
  whole screen; Yap's own windows not offered; if it can't open, Yap's own
  list shows), cancelling it does nothing, and on macOS 15+ no
  more "keep allowing Yap to bypass the private window picker?" asks
  (System Settings › Privacy & Security › Screen Recording may not even need
  Yap now: try with it off); someone watches a still screen and sees a
  picture at once; the quality and Share sound while nobody watches and
  while someone does; **Share something else** reopens Apple's picker and
  swaps what's shared while people keep watching (and cancelling it keeps
  the share); macOS's own screen-sharing menu changing what's shared; the
  menu-bar indicator's stop ends the share in Yap (the LIVE badge goes for
  everyone); a shared window closing (15.2+: "The shared window was
  closed."); leaving voice and quitting while sharing (the indicator goes);
  and the purple indicator staying up for the whole share even while nobody
  watches (that's the idle stream).
- **Screen sharing on real machines** (the tests share a synthetic pattern
  only): macOS 12.3–13 — the Screen Recording prompt on the first Share screen (and
  Yap showing up in System Settings › Privacy & Security › Screen
  Recording; macOS may ask to quit and reopen); every Mac — sharing a Retina display, an
  external one and a window, the share's sound on macOS 13+ (music in, no
  echo of the call), stopping from the menu-bar indicator, the hardware
  encoder (the log says "H.264: VideoToolbox, hardware encoder"; after sleep
  it starts a new session; `YAP_SCREEN_ENCODER=openh264` to compare CPU
  use); Windows 10 and 11
  — screens at 125–150 % scaling, windows (resize, minimise, close: "The
  shared window was closed."), no yellow border on 11, the share's sound
  without the call's own; Linux X11 — screens (RandR), windows, the cursor,
  and the Wayland message on a Wayland session. On every OS: Smooth / Sharp
  / Max on a real network (the bitrate backs off when a viewer loses
  packets), CPU use while sharing 1080p60 (and none at all while nobody
  watches), **Share something else**, watching a web, iPhone and Mac share
  (H.264), **Pop out** and **Full screen** (and double-click) on each OS,
  the Sound slider, and "paused" / "stopped sharing".
- **The voice list's popup:** click and right-click someone in a voice
  channel: volume (0–200 %), Watch screen while they share, and the owner /
  mod actions.
- **Space with people**: walking, sitting, the ball and tag with two or three
  players (web and desktop mixed), and the room voice zones / "Nearby is
  louder" with real voices — volume fades at doorways. Water balloons
  hitting someone (and them hitting you), fire and water both ways, a full
  lap of the track (the corners count, the board updates), the floor piano
  and drums with real speakers (two people on the keys; the first note opens
  a little output stream when you're not in a call), the Gallery viewer with
  an animated GIF, paging walls with more than twelve drawings.
- **Since you left with real people:** mentions that came while the app was
  closed (the badge and "for you" in Catch up), Mark all read, a brand-new
  install starting with everything read, someone being let in by the owner
  or a mod (the "joined" line, a member's too, with whose link it was; you
  being let in: "You're in"; a member sees no bar, count or chime), and a
  reused personal link's warning.
- **Passwords with real accounts:** an old PIN account signing in on the
  desktop (the "Choose a password" bar, Later remembered after a restart),
  choosing a password from the bar and from Settings › Account, a refusal's
  words under the fields ("That password is too easy to guess."), an
  owner/mod's Reset password… and the person signing in with the temporary
  one (the bar asks again, even after Later on the PIN's), Show in each
  field, and a password manager filling the field (egui's text fields don't
  tell the OS they're password fields, so autofill may not offer itself).
- **RED on real networks** (the tests impair a local proxy): a call on
  choppy Wi‑Fi or cellular with web, iPhone/Mac and desktop people should
  have far fewer robotic blips; the call report says `red+opus` each way
  and counts what RED rebuilt. With a desktop app from before RED the call
  is plain Opus both ways (the tests stand in for one with `YAP_RED=0`).
  At 256/510 kbps the copy doesn't fit (each packet goes with its own
  frame only), so those calls get no help from RED.
- **Copy call report / Copy last call report** on each OS (the clipboard
  on macOS, Windows, Linux X11 and Wayland): paste it somewhere and read it
  (no addresses; names, devices and settings right; "No call yet." on a
  fresh install), and a call where your Wi‑Fi drops or a friend's app
  restarts (the timeline shows it).
- **Settings › Voice** with real fonts: the groups, Advanced
  folding and unfolding (and folded again next time), the push-to-talk key
  and the sensitivity meter only in their modes; the voice panel's ⋯.
- **Joining voice** with web, iPhone and desktop people on real networks
  (one offer per join now; the late `negotiationneeded` is skipped).
- Search and jump on a long real history (older pages load until the message
  is there), pins from several people at once.

- **The OS keychain for the Personal space key** — macOS Keychain (an
  unsigned or freshly rebuilt binary may make macOS ask "yap wants to use
  your keychain"), Windows Credential Manager, Linux Secret Service (GNOME
  Keyring / KWallet); and with no Secret Service at all (it falls back to
  memory: the passcode is asked again after a restart). Locking, then
  quitting and relaunching, must ask again.
- Encrypt on one device and unlock on another (web ↔ desktop ↔ iPhone), change
  the passcode on one (the others lock and ask for the new one), "Forgot it? →
  Start over" (old notes stay locked everywhere).

- Windows and Linux builds on real machines: window, fonts (Segoe UI / your
  fontconfig default), audio devices (USB/Bluetooth headsets, 44.1 kHz devices),
  device switching mid-call.
- **Global push-to-talk while a game is focused** (Windows; X11; Wayland with
  and without the `input` group).
- Firewall prompts on first voice join (Windows Defender asks for UDP).
- Voice with friends on other networks (TURN), and with Safari/Firefox peers.
- **Hide my IP address in calls** against a server with a real TURN relay:
  the call connects (a little more delay), and the peer only ever sees the
  relay's address (chrome://webrtc-internals on a web peer shows no host or
  srflx candidates from us).
- Noise suppression quality and the voice-activity threshold on a real mic.
- **Updating on Windows and Linux** (the rename-the-running-exe swap and the
  relaunch are only exercised on macOS by the tests), and a first download
  through a browser: macOS quarantines it (right-click › Open, or
  `xattr -d com.apple.quarantine yap-macos-aarch64`), Windows SmartScreen warns
  about an unsigned exe. Updates Yap installs itself aren't quarantined.
- The 98 look on Windows (Tahoma) and Linux (DejaVu Sans unless Tahoma is
  installed), at 100% and 150% scaling.
- A config.json from 0.10 or older that says `"skin": "apple"` or `"arch"`
  opens in Windows 98 with the same light/dark theme.
- Desktop notifications: Windows toasts (registered as "Yap" under
  HKCU\Software\Classes\AppUserModelId\Yap.Desktop), Linux `notify-send`,
  macOS `osascript`; and the chime when not in a call.
