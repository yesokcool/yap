# Yap protocol v1

Everything a client (web, native desktop, SwiftUI) needs to talk to a Yap
server. The TypeScript source of truth is `worker/protocol.ts`; keep the two in
sync. The web client (`web/js/`) is the reference implementation.

## HTTP

Calls marked *key* need the invite key (below): the `X-Yap-Key` header on HTTP,
the subprotocol on the WebSocket.

| Endpoint | Purpose |
|---|---|
| `GET /api/check` (*key*) | `{ ok, keyRequired, protocol }`. Call before connecting so you can show "wrong invite key" (a failed WebSocket upgrade's 401 isn't readable from browsers). |
| `GET /api/config` (*key*) | `{ gifs: { provider: "klipy", appKey } \| null, links: [{ id, name, url, view?, description? }], minecraft?: { name, address }, push: { publicKey } \| null, ntfy: { server }, source: string \| null }`. Client-side config for invited users. `links` is the same list as `ready.links` (older clients read it here); a link with `view` opens inside the app (today only `"board"`, and only on servers with a board). `minecraft` is there only when the server has one set (`MINECRAFT_SERVER`). `push.publicKey` = the VAPID key for browser pushes (null when the server has none). `ntfy.server` = the ntfy server this Yap pushes through (`NTFY_SERVER`, else `https://ntfy.sh`): subscribe to your topic there. `source` = where this server's source code is (the Source link; Yap is AGPL-3.0). |
| `GET /api/minecraft` (*key*) | `{ name, address, online, checkedAt, ms?, version?, motd?, players?: { online, max, names }, error? }` for the server's Minecraft server, if it has one; 404 otherwise (Server List Ping from the Worker, cached ~30 s). `error` says why a ping failed. Poll ~once a minute. |
| `GET /board` (+ the root paths in the server's `BOARD_PATHS`) | The optional board (another Worker, bound as `BOARD`), served same-origin so pointers can be drawn over it. 404 when the server has none. |
| `GET /api/ice` (*key*) | `{ iceServers: [...] }` in `RTCIceServer` shape. STUN always; plus a TURN relay when the server has one (a fixed login, or short-lived Cloudflare credentials). Fetch right before joining voice. |
| `GET /ws` (*key*) | WebSocket upgrade. 401 on a bad key, 426 without an upgrade. |
| `POST /api/image?sid=S&ut=U` (*key*) | A picture for a message: PNG, GIF, WebP or JPEG (sniffed from the bytes; no SVG), ≤ 1.9 MB → `{ ok: true, id, mime }`; then `send { image: { id, width, height } }`. |
| `POST /api/avatar?sid=S&ut=U` (*key*) | Your picture: image bytes ≤ 512 KB (clients square-crop to 256 px) → `{ ok: true }`. Broadcast as a `user` update; `User.avatar` = media id. |
| `POST /api/drawing?sid=S&ut=U` (*key*) | Save a picture of the drawing board to the gallery (a PNG, or an animated GIF), ≤ 1.9 MB → `{ ok: true, drawing }`, and `drawing` to everyone. |
| `POST /api/blob?sid=S&ut=U` (*key*) | An encrypted picture for your Personal space: opaque bytes, at most 1.9 MB + 28 → `{ ok: true, id }`. See *Encrypted Personal space*. |
| `POST /api/sound?sid=S&ut=U&kind=board\|chime&name=N&emoji=E` (*key*) | A soundboard clip or your own join chime: a WAV the client made, ≤ 480 KB → `{ ok: true, sound }`. See *Extras* › Soundboard. |
| `POST /api/doodle?sid=S&ut=U` (*key*) | A doodle reaction: PNG or GIF, ≤ 64 KB and ≤ 256 × 256 → `{ ok: true, id }`; react with `<d:id>`. See *Extras* › Doodle reactions. |
| `POST /api/emoji?sid=S&ut=U&name=N` (*key*) | Upload a custom emoji. Body = raw image bytes: PNG, GIF, WebP or JPEG, ≤ 512 KB. → `{ ok: true, emoji }` (also broadcast). Up to 30 at once, then one every 2 s. Clients scale static images to 128 px first. |
| `GET /media/<id>` (or `HEAD`) | Any stored file by its id: pictures, avatars, drawings, doodles, clips and encrypted pictures (`application/octet-stream`). No key needed (ids are 128-bit content hashes). Cached for a day at most: deleting a post erases its picture. |
| `GET /emoji/<id>` (or `HEAD`) | The same store. A custom emoji's image is cached for a year (`immutable`), and stays available after the emoji is deleted, so old messages keep rendering; any other id asked for this way gets the one-day cache too. |
| `/api/admin/*` | The admin API (backups, restores, bulk changes): `Authorization: Bearer <ADMIN_KEY>`. Every call is listed under *Backups*. |
| everything else | the web client's static files |

**Uploads** all take `sid` (one of your live sessions) and `ut` (the
`uploadToken` from that session's own `ready`: session ids are visible to
everyone, the token isn't), and the raw bytes as the body. The server checks
the session before it stores anything, then the bytes themselves (never the
filename or `Content-Type`). Each session may upload 10 files at once, then
one every 6 s; emojis have their own limit (above).

**Failures.** Upload routes answer `{ ok: false, code, message }` with the
status: 400 `bad_name` (emoji names: 2–32 letters, digits or `_`) or
`too_big` (a doodle bigger than 256 × 256), 401 `no_session` (`/api/emoji`),
403 `no_session` (the other uploads: no such session, or past the upload
limit), 409 `name_taken` or `duplicate`, 413 `too_big`, 415 `bad_type` or
`bad_sound`, 429 `slow_down`. `/api/avatar`, `/api/drawing` and `/api/sound`
can also be refused after the bytes were checked (the session went, or it's
changing its picture or saving too fast): `{ ok: false, message }`, status
400, no `code`. A wrong or missing invite key is a plain-text 401, a missing
admin key too, a wrong method a plain-text 405, and too many requests from
one address a plain-text 429 with `Retry-After`.

**Invite links** look like `https://host/#key=K`. The key lives in the URL
*fragment* so it never hits server logs. Clients send it as the **`X-Yap-Key`
header** on HTTP calls and as the **WebSocket subprotocol** `["yap", "key.K"]`
(the server answers with `yap`), so it never appears in a URL. A subprotocol
can only carry letters, digits, `-` and `_`: for a key with anything else,
clients put it in the socket's address instead (`/ws?key=K`), so keys should
stick to those characters (SELF_HOSTING.md). The server accepts `?key=K` on any
call. A server without a key is open (local dev only).
Everything that reaches the Worker is capped per IP (HTTP 429).

## WebSocket framing

- Every frame is UTF-8 JSON with a `"t"` (type) field. Max 384 KB.
- **Heartbeat:** send the bare text `ping` every ~15 s; the server answers
  `pong` without waking up. No pong within ~10 s of a ping → treat the socket
  as dead and reconnect.
- **Reconnect** with exponential backoff (0.5 s → 10 s cap, jittered). Each
  connection is a new **session** with a new `sid`; on reconnect, re-send
  `hello`, reload history, re-send unconfirmed messages, and rejoin voice
  (ideally keeping the call itself: *Voice* › Keeping calls across reconnects).

## Identity

**Accounts are a name + a 6-digit PIN** (or a password of 10–128 characters).
Names are unique (case-insensitive).
Each signed-in device gets a **device token** (a long random secret) — store
it and use it to reconnect. Internally an account has a `uid`; `ready.uid` is
always your canonical one.

`hello` is one of (fields besides `t`, `v`, `client`):

| Form | When | Reply |
|---|---|---|
| `token` | normal reconnect | `ready` (or `pending`); `error bad_token` → token revoked: sign in again |
| `login: { name, pin }` | new device / signed out | `ready`/`pending` **with `token`**; `bad_login`, `locked` (too many wrong PINs: 5 free tries, then waits that double up to 24 h), `no_pin` (the account never set a PIN: an owner or mod resets it) |
| `signup: { name, pin, color }` | brand-new person | `pending { uid, token }` (waiting room); `name_taken` (suggest signing in), `bad_pin` |

The old uid-only `hello` (devices from before PINs) is gone: it's refused with
`pin_required` ("ask the owner or a mod to reset your PIN"). An account with
no PIN gets one through `member.resetPin`.

- `account.pin { pin, current? }` sets/changes your PIN (`current` required
  when changing) → `account { hasPin, token?, kind }` (`token` is never sent any
  more: every signed-in device already has one; clients may ignore it). `pin` may also be a **password** (10–128 characters) — `kind: "password"`;
  sign-in (`login.pin`) accepts either. **Weak PINs are refused** (`weak_pin`):
  repeats, runs, patterns, common PINs, date-like ones (signup too).
- `ready.weakPin` (the PIN you last signed in with is guessable: nudge to change
  it) and `ready.secretKind` (`pin` | `password`).
- **Devices:** `account.devices` → `devices { devices: [{ id, client, label,
  createdAt, lastSeen, scope, current? }] }`; `account.revoke { id }` signs one
  other device out (its sockets close with 4001); `account.signoutOthers` signs
  out every other device, OBS links included. Both reply with `devices`.
  `client` is what the device said in `hello` (`obs` for an OBS link);
  `label` is made by the server from `hello.client` and the User-Agent
  ("Yap for iPad", "Chrome on Windows"); if a device later connects with a
  different `client` while its label is still the default for the old one
  (an iPad app that used to say `ios`), the label follows. Devices can't be
  renamed.
- **Sign-in alerts:** a name+PIN sign-in on a new device sends `signin.alert
  { device }` to your other devices (and a push). Clients offer "Sign out
  everywhere else".
- `account.signout` revokes this device's token and closes the socket.
- Owner/mod: `member.resetPin { uid }` → `pin.reset { uid, pin }` (a temporary
  PIN to hand over). Admin API: `POST /api/admin/reset-pin?uid=`.
- **Waiting room:** a new account gets `pending` instead of `ready` and can't
  do anything else. Members get `join.request`; **any member** answers with
  `member.approve` / `member.deny`. Someone who came with a personal invite
  link carries `invitedBy` ("Ada (for Cleo)") and `invite: { by, label,
  usedBy }`: the link's maker, its label, and the names of the other accounts
  that signed up with the **same link** before (oldest first, ≤ 10; any role,
  so turned-away ones too). Links can be passed on, so a non-empty `usedBy`
  is worth showing ("this link was already used by Ben — check with Ada").
  Approved → the waiting socket receives a normal `ready`. Denied → `error
  { code: "denied" }` and the account is banned. (The first person on a
  brand-new server becomes `owner`.) On `error` `banned`/`denied`, stop
  reconnecting.
- **Roles:** `User.role` is `owner | mod | member` in lists (`pending`/`banned`
  users aren't listed; a `user` event with those roles means "remove").
  Owner + mods can `delete` anyone's message, `member.ban`, and reset PINs;
  only the owner sets mods (`member.role`). `owner.claim { key }` with the
  admin key makes you owner.
- **Rate limits:** sends 10 burst / 1 per s (`error slow_down`), channel
  creation, emoji renames/uploads, new accounts per IP; typing, cursors and
  views over the limit are dropped silently.
- **A send the server turns away:** each `send` gets exactly one answer, in
  the order sent: its `message` echo (with your `nonce`) or an `error` with
  `re: "send"`, so an error is about your oldest send still waiting for an
  answer (track that order locally). On `slow_down`, keep the message pending
  and send the same frame (same nonce) again after 1.1 s, up to 5 times, and
  send everything behind it one at a time, 1.1 s apart, until nothing waits;
  no notice unless it finally gives up. On any other `re: "send"` error, drop
  the pending copy, show the error, and put its text back in the message box
  if that's empty and the channel is still open. Resend an outbox in the order
  written, never by timestamp. A frame over 384 KB gets `bad_frame` with no
  `re`, so check a send's size before sending it.
- **Merged accounts:** an admin can merge a duplicate uid into the real one;
  devices adopt `ready.uid`.

## Client → server

| `t` | Fields | Notes |
|---|---|---|
| `hello` | `v: 1, client?` + `token` \| `login` \| `signup` | Must be first. See *Identity*. `client` is a short label (≤ 16 chars): `web`, `desktop`, `ios` (iPhone), `ipad`, `mac`. Any other value is fine too — it only names the device in *Your devices* ("Yap for iPad", "Chrome on Windows"…), and an unknown one is named from the User-Agent. |
| `account.pin` / `account.signout` | `pin, current?` / — | See *Identity*. |
| `member.resetPin` | `uid` | Owner/mod: temporary PIN comes back as `pin.reset`. |
| `history` | `channelId, before?, beforeId?, limit?` | Newest page when `before` is omitted; older pages pass the oldest message's `ts` and `id`. `limit` ≤ 100 (default 50). A page also stops at ~512 KB of text, so it can hold fewer (`hasMore` stays true). |
| `send` | `channelId, body, nonce?, gif?, image?, replyTo?` | Text channels only. ≤ 40,000 chars (encrypted Personal-space notes: ≤ 240,000); `replyTo` = a message id in the same channel (in your Personal space: any message you can read, which is *Save to Personal*) — see *Replies*; trailing whitespace trimmed. `body` may be empty when `gif` or `image` is set. `image` = `{id, width, height}` from `POST /api/image` (*HTTP* › Uploads: every upload needs your `sid` and `uploadToken`). `nonce` is echoed back to you only, so you can swap your optimistic copy. |
| `delete` | `id, blobs?` | Own messages (owner/mods: anyone's). **Erased, not hidden:** its text, GIF, picture, reactions and earlier versions are wiped from the server; only an empty row stays so the id is never reused. `blobs` (your own Personal space only, ≤ 10): ids of the note's encrypted pictures to erase too. |
| `invite.create` | `label` | Make a personal invite link → `invite.created` (the key is shown once; only its hash is stored). |
| `invite.list` | | → `invites` (yours; everyone's for owner/mods). |
| `invite.revoke` | `id` | Maker or owner/mods. Stops new joins; people already in keep working (they hold the main key from `ready`). |
| `link.add` | `name, url` | Any member. Sidebar Places; `https://` only, ≤ 20 links. → `links` to everyone. |
| `link.remove` | `id` | Any member. → `links`. |
| `channel.bitrate` | `channelId, bitrate \| null` | Voice quality for everyone in that channel (32k–510k from a fixed list); null = each person's own setting. → `channel`. |
| `modlog` | | → `modlog` (last 200: who did what to whom; never message text). |
| `push.subscribe` / `push.unsubscribe` | `endpoint, keys {p256dh, auth}` / `endpoint` | Browser pushes (Web Push, `/sw.js`). The server encrypts per RFC 8291 and signs with VAPID (`VAPID_PRIVATE_JWK` secret; public key in `/api/config` as `push.publicKey`). Pushes follow your push mode (subscribing a new browser with it off turns on pings & mentions; sending the same subscription again leaves the mode alone) and go to ntfy and every browser you subscribed; 404/410 from a push service forgets that browser. |
| `device.handoff` | | A one-time sign-in pass (15 min) → `device.handoff {token}`. Hello with it as `token`: it's spent and `ready` carries a new lasting token for that device. Used by Add to Home Screen (`/#key=&handoff=&name=`), since home-screen apps don't share Safari's storage, and by the native apps whenever they open the web app (Draw, Space, Watch…: `/#key=&place=&handoff=&name=`; in-app pages add `&embed=1`, which hides the web's own sidebar and header for that tab). A pass that arrives while the browser still has a token is kept and used if that token turns out to be signed out. Limited to 5 at once, then one per 20 s; a refusal comes back as `error {re: 'device.handoff'}`. |
| `obs.create` / `obs.revoke` | | A listen-only device token for an OBS Browser Source (`/obs#key=&listen=&channel=[&only=Name][&overlay=0]`) → `obs.created {token}` / turn all of yours off → `obs.revoked {count}` (their sockets close). A listen-only session may only send `hello`, `voice.join/leave/state` (always muted) and `signal`; it shows to everyone with `client: "obs"` and never gets the main key. |
| `stats` | | Owner/mods: → `stats {chatBytes, mediaBytes, limitBytes}`. |
| `edit` | `id, body` | Your own messages only (mods can delete but not edit). Earlier versions are kept in `message_edits` until the message is deleted. Empty body → `empty` unless the message has a GIF or a picture. |
| `typing` | `channelId` | Send at most every ~3.5 s while the user types. Display for ~6 s. |
| `profile` | `name?, color?` | |
| `channel.create` | `name, kind: "text" \| "voice"` | Any member. The name is kept as typed (≤ 32 characters). |
| `channel.archive` | `channelId` | Any member. Hides it for everyone; messages stay. The last text channel can't be archived. Voice members are removed first. |
| `voice.join` | `channelId, muted?, deafened?, resume?, prev?, proof?, screen?` | Joining another channel while in one = leave + join. `resume: true` = you keep calls across reconnects; `prev` + `proof` (+ `screen`) = this join continues an earlier session's call — see *Voice* › Keeping calls across reconnects. |
| `voice.leave` | — | |
| `voice.state` | `muted, deafened, screen?` | `screen: true` = you're sharing your screen in your voice channel (see *Screen sharing*). |
| `watch.join` / `watch.leave` | | Join/leave the Watch together audience → `watching { sid, on }` to everyone (+ `watch.state` to you on join). You stay in it while browsing the app. |
| `watch.get` | | → `watch.state`. |
| `watch.set` | `videoId?` \| `playing?, position?` | New video (YouTube id; starts at 0, playing; the server looks up its title) / play, pause or seek (seconds). Anyone can. → `watch.state` to everyone. |
| `watch.queue` | `add?: videoId` \| `remove?: index` | Up to 20 queued; adding with nothing playing starts it. |
| `watch.next` | `expect: videoId` | The video ended: play the next queued one (ignored unless `expect` is still what's playing, so many clients can report the end at once). |
| `emoji.delete` | `id` | Anyone can. Soft delete. |
| `emoji.rename` | `id, name` | Anyone can. Broadcast as `emoji`; old `<:oldname:id>` tokens still render. |
| `member.approve` / `member.deny` | `uid` | Any member, for someone in the waiting room. |
| `member.ban` | `uid` | Owner/mod. Kicks every session; they can't come back. |
| `member.role` | `uid, role: "mod" \| "member"` | Owner only. |
| `owner.claim` | `key` | Becomes owner if `key` is the server's admin key. |
| `view` | `view: "board" \| "draw" \| "space" \| null` | Which shared view you're looking at (null = chat). Being on one makes your session visible (see *Presence*). When's good is not a view. |
| `search` | `q, channelId?` | → `search.results { q, channelId?, messages }` (newest first, ≤ 50, full messages with `reply`/`reactions`). Words must all appear (case-insensitive); `from:Name` (name prefix), `has:image` / `has:gif` / `has:link`; quotes keep a phrase together. Scope: one channel, or every live text channel plus your Personal space. Encrypted notes (`e1:`) never match — clients search those on the device after decrypting. 6 burst / 1 per 2 s. |
| `pin` | `id, on` | Pin/unpin a message (anyone who can see it; ≤ 50 per channel → `too_many`) → `pinned { id, channelId, pinnedAt \| null, by }` to everyone who can see it. Deleting a message unpins it. |
| `pins` | `channelId` | → `pins { channelId, messages }` (most recently pinned first). Messages carry `pinnedAt` everywhere (history, live, search). |
| `space.game` | `g, d` | Space mini-games: `g` = game name (`[a-z]{1,16}`), `d` = any JSON ≤ 512 bytes. Relayed to everyone else on Space as `space.game { sid, g, d }`; never stored. Games today: `sync`, `ball`, `tag`, `fx`, `throw`, `lap` — see *Space*. |
| `when.get` | | → `when.all` (see *When's good*). |
| `when.set` | `slots: number[]` | Your whole availability (replaces it). → `when` to everyone. |
| `personal.plain` / `personal.seal` | — / `notes: [{id, body}]` (≤ 50) | Sealing notes written before encryption was turned on: → `personal.plain { notes: [{id, body, gif?, image?}] }` (yours that aren't `e1:` yet, oldest first, ≤ 2,000); encrypt each (pictures re-uploaded via `/api/blob`), then hand back `e1:` bodies. The server swaps them in place (no "edited" mark), erases the plain text, its earlier versions and its plain picture, and tells your devices `edited { …, sealed: true }`. **One batch must fit one frame (384 KB)**: batch by size, not just count (the web sends ≤ 50 notes and ~300 KB per `personal.seal`). A note is sealed once its `edited { sealed: true }` arrives; a frame that's too big gets `error bad_frame` and seals nothing, so count what never came back as not sealed. |
| `vault.set` | `wrap: {v, iter, salt, iv, ct} \| null` | Turn on / change the passcode of / reset your encrypted Personal space → `vault` to your connections. See *Encrypted Personal space*. |
| `notify.settings` | `mode: "off" \| "pings" \| "all"` | Phone pushes via ntfy → `notify { mode, topic }`. `pings` = 📣 pings + @mentions; `all` = also every message while you're offline (≤ 1 per 20 s). |
| `ping` | `text?` | 📣 Everyone online gets `ping`; everyone with pushes on gets one. 2 per 30 s per person. |
| `draw.state` | — | → `draw.state { canvas, strokes, gallery }` (send when opening the board). |
| `draw.gallery` | — | → `draw.gallery { gallery }`: just the saved drawings (newest first, ≤ 60; PNGs and animated GIFs at `/media/<media>`). New saves still arrive as `drawing`. |
| `draw.seg` / `draw.stroke` | `sk, c, w, e, p, l?` | Live pieces of a stroke (relayed, not stored) / the finished stroke (stored). `sk` = your stroke id, `c` "#rrggbb", `w` width, `e` eraser, `p` flat `[x,y,…]` in board units (10000 × 6250). Optional `k`: `"poly"` = straight segments (lines, boxes), `"text"` = `text` (≤ 200 chars) drawn at `p[0],p[1]` (exactly 2 numbers), size from `w`. Optional `l` = layer 0–2 (absent = 0; see *Draw layers*). |
| `draw.undo` / `draw.clear` | `id` / — | Undo your own stroke (mods: any) / fresh page for everyone (old strokes kept). |
| `draw.wipe` | `l` | Empty one layer of the page for everyone (its strokes are kept, marked deleted like an undo) → `draw.wipe {l, by}`. |
| `space.state` / `space.move` | — / `x, y, d` | Space view: everyone's last spot / your position (world 2400 × 1600, `d` = facing in radians). |
| `space.emote` | `e` | One of 👋 😂 ❤️ 👍 🎉 💤 (`SPACE_EMOTES`), relayed to everyone on Space as `space.emote {sid, e}`. Sitting needs no message: seats are fixed spots, and standing exactly on one draws you seated. |
| `profile` | `avatar: null` / `look: "🐸" \| null` | Removes your picture (upload with `POST /api/avatar`) / your Space character (an emoji; null = your picture). |
| `react` | `id, emoji, on` | Add/remove your reaction. `emoji` = a character, or a custom token `<:name:id>`. ≤ 20 different emojis per message. |
| `cursor` | `view, a: { k: "card" \| "lane" \| "page", id?, x, y } \| null` | Your pointer on a shared view, ≤ ~15/s; `x`,`y` are fractions of the anchor element's box (page = whole document). Relayed, never stored. |
| `signal` | `to: sid, data` | WebRTC signaling, relayed verbatim, only to someone in your voice channel (or to/from someone sharing their screen); anyone else is `no_peer`. Up to 200 at once, then 20 a second (a call's setup is a burst); beyond that they're dropped. See *Voice*. |
| `poll.vote` | `id, option \| null` | See *Extras* › Polls. `send` also takes `poll: { options }`. |
| `sound.play` / `sound.delete` | `id` | See *Extras* › Soundboard. |
| `shelf` | `channelId, before?` | Pictures + GIFs in a channel. See *Extras* › Media shelf. |
| `unread.counts` | `since: { [channelId]: ts }` | See *Extras* › Since you left (never counts the Personal space). |
| `notify.quiet` | `quiet \| null` | See *Extras* › Quiet hours. |
| `hang.set` / `hang.cancel` | `at, title, channelId?` / `id` | See *Extras* › Hang reminders. |

## Server → client

| `t` | Fields | Notes |
|---|---|---|
| `ready` | `v, sid, uid, token?, needsPin, notify, channels, users, sessions, emojis, pending, build?, key?, links?, storageWarning?, uploadToken?, resumeKey?, vault?, weakPin?, secretKind?, sounds?, hangs?` | Full state snapshot. `uid` = your canonical id (adopt it if it differs: merged accounts). `users` = the members (≤ 500, most recently connected first); `sessions` = only the visible ones (see *Presence*); `emojis` = live custom emojis; `build` = `{ app, style }` of the web client now being served (see Updates). |
| `build` | `build` | A new deploy started while you were connected: the web client now being served (see Updates). |
| *(Personal space)* | | `ready.channels` includes **your** Personal space: `{id: "p_<your uid>", kind: "personal", name: "Personal space"}` (never anyone else's). `send`/`history`/`edit`/`delete`/`react` work in it for you only (others get `no_channel`/`no_message`, mods too); its messages, edits and reactions go only to your own connections; no typing, no pushes; a `replyTo` there is *Save to Personal*. Encrypted once you turn it on — see *Encrypted Personal space*. |
| `history` | `channelId, messages (oldest first), hasMore` | Refused (`no_channel`) for archived channels. Messages carry `editedAt` once edited and `reply` when they answer another message. |
| `message` | `message, nonce?` | Broadcast to all; `nonce` only on your own echo. |
| `deleted` | `id, channelId` | |
| `edited` | `id, channelId, body, editedAt, sealed?` | The author edited a message; replace its text. `sealed`: a Personal-space note was just encrypted in place — also drop its `gif`/`image` (they're inside the note now); `editedAt` 0 = never edited. |
| `invite.created` | `invite, key` | Only to the maker. Link = `<origin>/#key=<key>`. |
| `invites` | `invites` | `{id, label, by, createdAt, uses, lastUsedAt, revoked}` |
| `links` | `links` | The whole list, after any change. `{id, name, url, view?, description?}`; `view` opens inside Yap (`board`). |
| `modlog` | `entries` | `{ts, actor (uid or "admin"), action, target, detail}` |
| `stats` | `chatBytes, mediaBytes, limitBytes` | |
| `typing` | `channelId, uid` | Never sent back to the typer. |
| `user` | `user` | Name/color change or a new user. |
| `session.join` / `session.leave` | `session` / `sid` | A session became visible (joined voice or a shared view) / stopped being visible or disconnected. See *Presence*. |
| `when.all` / `when` | `people: [{uid, slots}]` / `uid, slots` | When's good: everyone's availability / one person's changed (empty `slots` = cleared). |
| `devices` / `signin.alert` | `devices` / `device` | See *Identity* › Devices / Sign-in alerts. |
| `search.results` / `pinned` / `pins` | see `search` / `pin` / `pins` | |
| `space.game` | `sid, g, d` | Another Space player's game update. |
| `vault` | `wrap \| null` | Your Personal space's wrapped key changed (on another device, or just now). |
| `channel` | `channel` | Created (or later: updated). |
| `channel.archived` | `channelId` | |
| `watch.state` / `watching` | `state, now` / `sid, on` | See *Watch together*. |
| `voice` | `sid, channelId \| null, muted, deafened, screen?, resume?, prev?, lost?` | Broadcast to everyone **including the subject** — your own echo is how you learn your join landed. `lost`: it left because its socket closed (it may come back); `resume`/`prev`: see *Voice* › Keeping calls across reconnects. |
| `emoji` / `emoji.deleted` | `emoji` / `id` | `emoji` = added or renamed (replace by id). |
| `pending` | `message, uid, token?` | You're in the waiting room (instead of `ready`). Store `token` and `uid`. |
| `account` / `pin.reset` | `hasPin, kind, token?` / `uid, pin` | See *Identity*. |
| `join.request` / `join.resolved` | `user` / `uid, approved, by` | Someone's waiting / was let in or turned away. |
| `view` | `sid, view` | A session switched views. |
| `cursor` | `sid, view, a` | Another person's pointer (only sent to people on that view). |
| `notify` | `mode, topic, quiet` | Your push settings; subscribe to `topic` in the ntfy app. Also in `ready.notify`. `quiet`: see *Extras* › Quiet hours. |
| `ping` | `from, text` | Play a ping sound; show who and what. |
| `draw.*` / `drawing` | see above / `drawing { id, media, uid, ts }` | Board updates for people on the draw view (`draw.seg` carries `l` when it isn't 0; `draw.wipe {l, by}`) / a PNG saved to the gallery (`/media/<media>`). |
| `space.state` / `space.pos` | `players[], me` / `sid, x, y, d` | Positions on the space view; `me` = where you stood last time (null the first time) — spawn there. |
| `reaction` | `messageId, channelId, emoji, uid, on` | Someone added/removed a reaction. History messages carry `reactions: [{ emoji, uids }]`. |
| `signal` | `from: sid, data` | |
| `poll` / `sound` / `sound.added` / `sound.deleted` / `away` / `shelf` / `unread.counts` / `hang` / `hang.cancelled` / `hang.now` | | See *Extras*. `ready` also carries `sounds` and `hangs`; `notify` carries `quiet`. |
| `error` | `code, message, re?` | `re` = the request type that failed. Codes: `already_hello bad_away bad_bitrate bad_chime bad_emoji bad_frame bad_game bad_gif bad_hang bad_image bad_index bad_json bad_key bad_kind bad_layer bad_login bad_name bad_pin bad_poll bad_position bad_quiet bad_role bad_roll bad_seal bad_slots bad_stroke bad_subscription bad_token bad_url bad_vault bad_version bad_video banned busy denied empty forbidden internal last_channel locked name_taken no_channel no_device no_emoji no_hang no_hello no_invite no_link no_message no_peer no_pin no_request no_sound no_user no_video not_in_voice pending pin_required slow_down too_long too_many unavailable unknown_type weak_pin` (`test/unit.test.mjs` checks this list against the server). Uploads have their own: *HTTP* › Failures. |

Shapes: `Channel { id, name, kind, position, lastMessageAt, bitrate? }` (`bitrate` on voice channels: bits/s for everyone in it, null = each person's own),
`User { id, name, color, lastSeen, role, avatar, look, away?, chime?, invitedBy?, invite? }` (`lastSeen` is always 0 now — there's no presence; `invitedBy`/`invite` only on waiting-room entries, see *Identity* › Waiting room),
`Session { sid, uid, client, voice: { channelId, muted, deafened, screen?, resume?, prev? } | null, view, watching? }`,
`WatchState { videoId | null, title | null, playing, position, at, by, queue: [{ id, title, by }] }`,
`Message { id, channelId, uid, body, ts, gif?, image?, editedAt?, reactions?, reply?, pinnedAt?, poll?, roll? }`,
`Reply { id, uid, channelId?, body (first 200 chars), gif?: true, image?: true, deleted?: true }` (`channelId` is missing only when the original is gone entirely),
`Stroke { id, uid, c, w, e, p, k?, text?, l? }` (a stored stroke, in `draw.state` and `draw.stroke`: `id` is the `sk` it was drawn with),
`Gif { provider: "klipy", slug, title, width, height, mp4?, webm?, webp?, gif?, still? }`,
`Emoji { id, name, mime, uid, createdAt }` (`id` = first 32 hex chars of the image's SHA-256).
Times are ms since the epoch.
Message `ts` is strictly increasing server-wide, so it's a safe sort key.

## Presence (there is none)

Yap doesn't show who's online, on purpose: people should be able to drop in
and chill without being watched. The server only reveals a session while it
does something others share:
- `ready.sessions` and `session.join`/`session.leave` cover sessions **in a
  voice channel, on a shared view** (board, draw, space) **or in the Watch
  together audience** — plus your own current session in `ready`.
- When a session joins voice or a view, everyone gets `session.join` (with
  `voice: null, view: null`) followed by the usual `voice` / `view` message.
  When it leaves both, `voice`/`view` then `session.leave`.
- Connecting, idling in chat and disconnecting from chat send nothing.
  `User.lastSeen` is always 0.
- Clients show no online/offline dots, counts or "last seen", and no member
  sidebar; the member list is a People sheet opened on demand.

## Mentions

`@Name` mentions a member (the highlight, unread badge and sound in the app;
a push to them; their away reply to you). One rule on every side, in
`web/js/mentions.js` (the Worker imports it): the name in any case, not right
after a letter, digit or `@` (so no e-mail addresses), not followed by a
letter or digit, and never inside `code`, a link or a custom emoji token
(`<:name:id>`). Those three are covered up first and count as a word, so one
right against the name blocks it like a letter would: ``"`x`@Ben"`` and
`"<:wave:…>@Ben"` mention nobody, `"<:wave:…> @Ben"` mentions Ben. Longer names go first and
each mention counts once, so `@Sam Lee` mentions Sam Lee and never also Sam.
`@everyone` and `@here` mention everyone. Other clients should match it.

## Replies

`send { replyTo }` answers a message in the same channel. The message then
carries `reply: { id, uid, body, gif?, image?, deleted? }` — the original's
author and first 200 characters, filled in by the server (also in history),
so clients can show the quote without having the original loaded. If the
original is deleted later, `reply` comes back as `{ id, uid, body: "",
deleted: true }`. A reply counts as an @mention of the original's author
(highlight, unread badge, push) unless you're answering yourself. In your
Personal space, `replyTo` may point at any message you can read — that's
*Save to Personal* (see *Extras*); `reply.channelId` says where it lives.

## When's good

A shared when2meet-style planner, Monday–Sunday. Times are **slots** =
half hours since the Unix epoch (`floor(ms / 1_800_000)`), so everyone sees
the same moments in their own time zone.
- `when.get` → `when.all { people: [{ uid, slots }] }`.
- `when.set { slots }` replaces your whole set (≤ 1000 slots, only from the
  start of last week to the end of the week after next are kept; empty =
  clear) →
  `when { uid, slots }` to everyone.
- **Clears every week:** slots before the start of last week (UTC Monday) are
  dropped by the server, and clients only show this week and next, so every
  Monday starts fresh.
- The web client is the reference: two grids (yours to paint, everyone's as
  a heat map), tap a cell to see who's free. Native apps open
  `/#place=when` in the browser.

## Space

A 2400 × 1600 top-down world. `web/js/space-map.js` has the map (rooms,
walls, furniture, seats, the fire, the water, the instruments, the track, the
gallery's frames) and `web/js/space.js` runs it; nothing about the map lives
on the server. Newcomers appear on the Plaza's left wing (around 450, 900).

**Rooms are voice zones.** Top row (y 100–556) Lounge, Cinema, Music, Arcade;
bottom row (y 1044–1500) Garden, Pond, Pitch, Campfire; the Gallery, a long
hall between them (720–1680 × 636–964, doors at both ends); everywhere else
(the wings and corridors round the Gallery, and the track round the edge) is
the Plaza, a zone of its own. A spot on a room's wall or in
its doorway counts as inside. Space voice is a per-device choice
(`yap.spaceVoice`): **"This room only"** (`room`, the default) = people in
your zone at 100 %, everyone else on Space at 0 %; `near` = full volume
within 260 units, fading to 12 % at 1100; `everyone` = no change. Use the
other person's last reported spot, ease the volume over ~0.4 s so doorways
crossfade, and leave people who are in the call but not on Space alone.

**Mini-games** (`space.game { g, d }`). Nothing is stored, so whoever owns a
piece of state answers `sync`. People are named by the first 8 characters of
their sid (`id8`).

| `g` | `d` | Who sends it |
|---|---|---|
| `sync` | `{}` | You, each time `space.state` arrives (opening Space, reconnecting). Everyone who owns something answers: the ball's owner, whoever is "it", anyone with an effect or splashes (`fx`), anyone with a lap time (`lap`). You also send your own `fx`/`lap` then. |
| `ball` | `x, y, vx, vy, k, s: [blue, red], goal?` | The ball's owner: right after a kick, ~10×/s while it rolls, once when it stops, and to answer `sync`. |
| `tag` | `it, k, left, t: {id8: deciseconds}, by?` | Whoever is "it": on start, on each pass, every 3 s, at the end (`left: 0`), and to answer `sync`. |
| `fx` | `fire, wet` (ms left, 0–10000), `n?`, `by?`, `hit?` | You, about yourself: when you catch fire, get wet, or one puts the other out; every 1.5 s at most while you stand in it; and to answer `sync`. `n` = water balloons you've landed. `by` + `hit` = the balloon that just hit you (thrower's `id8`, the throw's `id`). |
| `throw` | `x, y, a, id` | You threw a water balloon from `x, y` at angle `a` (radians); `id` 0–999, counting up. |
| `lap` | `best, n, ms?` | You finished a lap (`ms`), or just your best and lap count (to answer `sync`). |

- **Ball** (radius 14, starts at the Pitch's centre spot 1490, 1272): walking
  into it kicks it (power ≈ 150 + 1.9 × your speed into it, ≤ 1100 units/s);
  the kicker sets `k + 1` and becomes the owner. Everyone rolls it forward
  locally (friction `v × e^(−1.3 t)`, walls and goal posts bounce at 0.72,
  stops under 12 units/s); only the owner's copy counts and only the owner
  scores. Accept an update when its `k` is higher, or equal from the current
  owner, or equal from a lower sid (two kicks at once) or when the owner has
  left. Goals: the mouths are 160 units tall at each end of the Pitch; into
  the right (Red's) net is a goal for Blue (`s[0]`), the left net for Red
  (`s[1]`). The scoring update carries `goal: 0|1` (which net); 1.8 s later
  the owner puts the ball back on the spot. When the owner leaves Space, the
  lowest sid still there takes the ball where it lies and sends it. Clients
  throw confetti over whoever kicked a goal in.
- **Tag** (90 s rounds): pressing T / the Tag button or stepping on the TAG
  spot (2080, 800) starts a round with you "it" (needs someone else on
  Space; `k` = max(k + 1, Unix seconds)). "It" passes it by touching someone
  (centres ≤ 52 apart, not in the first 0.6 s), never to someone sitting, and
  not straight back to `by` within 2.5 s; every pass is `k + 1` and adds
  "it"'s time to `t`. `left` = ms remaining; receivers count down locally and
  show `t` (plus the running time of whoever is it). Least time as it wins.
  Accept a higher `k`, or an equal one from "it" (or from a lower sid). If
  "it" leaves or goes 10 s without a beat, the lowest sid among the rest hands
  it to whoever was nearest (`k + 1`, no `by`); with nobody left to chase, the
  round ends.
- **Fire and water** (`fx`). Each client is the authority on its own person
  and says so; others just draw it until it runs out. Walking within 32 of the
  campfire (2100, 1272) sets you on fire for 5 s after you step off (not while
  sitting). Water soaks you for 6 s: inside the Garden fountain (within 78 of
  340, 1272), wading in the Pond (ellipse at 870, 1300, radii 160 × 160,
  except on the dock: x 845–895 from y 1066 to 1260, then x 815–925, y 1260–1320),
  or a water balloon. Water puts fire out; fire dries you off instead of
  lighting you if you're wet (then you're safe from it for 1.2 s). Draw
  flames and embers on burning people, drips and a wet sheen on wet ones,
  puddles behind wet people, and a puff of steam when an announcement cancels
  a running effect.
- **Water balloons** (`throw`): F / the Throw button throws one the way you
  face, from 36 units in front of you; 650 units/s, 520 units of range, one
  per 0.7 s. Everyone flies it the same way; it pops on a wall or at the end
  of its range. Only the person it hits decides it hit (within 32 of their
  centre, never the thrower): they get wet and send `fx` with `by` and `hit`,
  so everyone pops that balloon, splashes it on them and adds one to the
  thrower's count.
- **Race laps** (`lap`): the track is the lane between the outer wall and the
  rooms (x < 100, x > 2300, y < 100 or y > 1500), run clockwise. Crossing the
  start line (y = 800 on the left lane, going up) starts the clock; pass the
  corners in order (top-left, top-right, bottom-right, bottom-left: the
  100 × 100 squares in each corner) and cross the line again to finish a lap,
  which starts the next. Off the lane for 4 s, or 3 minutes on one lap, and
  the lap is dropped. Each runner times their own laps; the lap records board
  on the Plaza (where newcomers appear, centered at 440, 680) shows everyone's best.
- **Instruments** need no messages: everyone sees where everyone stands, so
  each client plays a key or pad when someone steps onto it, if *you* are in
  the Music room too (and not deafened). The floor piano: 12 white keys
  C4–G5, 32 wide × 150 tall from (1328, 150), black keys 20 wide × 92 tall
  centred on the line after white keys 0, 1, 3, 4, 5, 7, 8, 10. Drum pads
  (radius 32, y 420): kick at x 1400, snare 1480, tom 1560, hi-hat 1640.
- **The Gallery** shows what's been saved in Draw and needs no `space.game`:
  send `draw.gallery` when Space opens (and on reconnect), then add each
  `drawing` as it arrives, newest first. Twelve frames to a wall, six on the
  top wall and six on the bottom one, left to right; more than twelve and you
  page through walls (your own page; nobody else's walls turn). Each frame
  shows the drawing at 112 × 70 with nearest-neighbour scaling (it's pixel
  art; a GIF shows its first frame) and a plate under it: the artist's name in
  their colour, the date. Frames: 122 × 80, x = 774 + 146·i for i = 0–5; the
  top row at y 664 (plate under it to 757), the bottom row at y 843. Stand in
  front of one (the aisle, y 757–843, over its width) without moving for
  0.45 s, or press E, and a viewer opens with the real image (GIFs play):
  previous/next through every drawing (← →), closes on Esc or when you walk
  more than 110 away; after Esc it won't reopen until you step to another
  frame.
- Validate everything you receive (numbers clamped, ids `[0-9a-f-]{8}`) and
  keep payloads under the 512-byte cap; the relay drops updates past
  ~15/s per sender without telling you.

## Draw layers (v0.38)

The page has three layers, shared by everyone: 0 = bottom (the default, and
every stroke from before layers), 1 = middle, 2 = top. A stroke's `l` says
which one it's on.

- **Drawing:** each layer is its own transparent sheet. Draw a layer's
  strokes in order on it (an eraser removes ink from **its own layer only**),
  make it crisp, then stack the sheets bottom to top on white paper. Live
  strokes go on their own layer too, so ink under a top layer stays under it
  while it's being drawn.
- **Which layer you draw on** is yours alone (default: bottom), and so is
  **hiding** one: a hidden layer just isn't shown on your screen. Choosing a
  layer, or drawing on it, shows it again.
- **Save, GIF and Share** export what you see (hidden layers left out).
  Clear's archive and a wipe's archive always keep every layer.
- **Wipe:** `draw.wipe {l}` empties one layer for everyone. Like Clear, the
  client saves the whole page to the gallery first. Undo can't bring wiped
  strokes back.
- Older clients ignore `l` and draw everything as one layer until they update.

## Extras (v0.36)

Thirteen small things picked on 2026-10-06. Everything here is
additive: older clients ignore the new message types and fields (they show
`/roll` results and poll questions as plain text, doodle reactions as a
`<d:…>` token).

### Spoilers (client only)
`||like this||` in a message body is a spoiler: drawn as a solid block that
reveals on tap/click (per message, not remembered). Works inside replies,
search results, pins and quotes. Pushes say `▒▒▒` instead of the text.
Unclosed `||` is plain text. Not inside code spans.

### YouTube links → Watch together (client only)
A message whose body has a YouTube link (`youtube.com/watch?v=ID`,
`youtu.be/ID`, `youtube.com/shorts/ID`, `music.youtube.com/watch?v=ID`,
`m.youtube.com/…`; ID = `[A-Za-z0-9_-]{11}`) gets a small **▶ Watch
together** button under it (one per message, first link). Clicking sends
`watch.queue { add: ID }` (plays now if nothing is on) and opens Watch
together (native apps: `/#place=watch` signed in, see *Identity* ›
`device.handoff`).

### Quick switcher (client only)
Ctrl+K (⌘K on Mac) opens a small palette: type to filter channels, Places
(Draw, Space, Watch, When's good, the board, links), people (opens their
profile/People sheet) and a few actions (Settings, Search…). Enter opens the
top hit; ↑/↓ move; Esc closes. The last row is always **Search messages for
"…"**, which opens search with that text (search's own shortcut moves to
Ctrl/⌘+Shift+F; ⌘F on Mac in native apps).

### Since you left
Clients already remember, per channel, the newest message time they've read
(`lastRead`). On opening a channel with unread messages, show a quiet
**"New since <time>"** divider above the first unread message, and a small
**Jump to first unread** pill when that divider is off screen. A **Catch up**
button (sidebar, only shown while something is unread) lists the unread
channels with counts and the first unread line; clicking one opens it at the
divider. Never a popup, never a badge for being away.
- `unread.counts { since: { [channelId]: ts } }` (≤ 50 channels) →
  `unread.counts { counts: { [channelId]: { n, first: { id, ts, uid, body } | null, mentions } } }`
  — `n` = messages after `ts` by other people (capped at 999), `first` = the
  oldest of them (body cut to 200), `mentions` = how many of them mention you
  (the *Mentions* rule: @you, `@everyone`/`@here`; or a reply to one of your
  messages). Channels you can't read are left out. Older servers send no
  `mentions` (treat it as unknown, not 0).
- Mentions that arrived while you were away: ask for counts on every `ready`
  (for the channels with something unread) and show `mentions` as the
  channel's mention badge and "for you" in Catch up, like a live mention.
  Catch up lists the channels that mention you first and offers **Mark all
  read** (every channel read up to its `lastMessageAt`).
- A device that has never read anything (a newcomer, or you on a new device)
  starts caught up: set `lastRead` to each channel's `lastMessageAt` on the
  first `ready`, rather than showing every channel unread since the beginning.
- When someone is let in (`join.resolved` with `approved`), the web shows a
  quiet line in the first text channel — "Cleo joined — invited by Ada"
  (from the waiting entry's `invite.by` / `invitedBy`) — for that session
  only: nothing is stored, and it's never a popup.

### Save to Personal
Bookmark any message into your Personal space: send a note there with
`replyTo` = that message's id (any text channel you can read, or your own
Personal space). The body is your optional comment and **may be empty**. The
note carries `reply` like any reply, plus `reply.channelId` (every `reply` now
has `channelId`), so clients show the quote and jump to it. It's a reference,
not a copy: if the original is deleted, the quote says so. Encrypted Personal
space: the comment is sealed as usual, but the server still sees which
message you saved (say so in the UI the first time).

### Away messages (AIM style)
`profile { away: { text, until } | null }` — `text` 1–80 chars, `until` = ms
or null ("until I'm back"). → `user` to everyone; `User.away = { text, until,
since }` while it's on. It ends by itself: at `until` (clients hide it once
past; the server drops expired ones), or when you send a message 2+ minutes
after setting it (the server clears it and broadcasts `user`). Shown only in
context — in the People sheet, on someone's name/profile card, and as an
auto-reply: when you @mention or reply to someone who's away, **you alone**
get `away { uid, text, until }` (at most once per person per 10 min), shown
as a quiet line under your message: "Ada is away: at work til 6". Never a
dot, never in the channel list.

### Join chimes
Everyone hears *your* chime when you join their voice channel (instead of the
generic join blip). `profile { chime }`: a built-in id, `s:<soundId>` (an
uploaded clip, see Soundboard; `kind: "chime"`, ≤ 2 s), or null = `default`.
`User.chime` (absent = default). Built-ins are synthesized (no audio files),
identically in every client — each is a list of notes (Hz) played `step` s
apart, each `length` s long with a 12 ms attack and an exponential decay:

| id | notes | step | length | volume | wave |
|---|---|---|---|---|---|
| `default` | 659.25, 880 | 0.075 | 0.11 | 0.05 | sine |
| `ding` | 1318.51 | — | 0.5 | 0.08 | sine |
| `chord` | 523.25, 659.25, 783.99 | 0 (together) | 0.6 | 0.05 | triangle |
| `tada` | 392, 523.25, 659.25, 783.99, 1046.5 | 0.07 | 0.25 | 0.06 | triangle |
| `bubble` | 440, 660, 990 | 0.04 | 0.08 | 0.06 | sine |
| `knock` | 160, 160 | 0.16 | 0.09 | 0.14 | sine |
| `arcade` | 523.25, 783.99, 1046.5, 1567.98 | 0.05 | 0.09 | 0.04 | square |
| `none` | — | | | | |

Chimes play through the same volume cap and limiter as the soundboard, and
not at all while you're deafened or have "Sounds" off.

### Soundboard
Short clips anyone in a voice channel can play for everyone in it.
- **Upload:** `POST /api/sound?sid=&ut=&kind=board|chime&name=&emoji=` with a
  **WAV** body: PCM 16-bit, mono, 8–48 kHz, ≤ 5.0 s (`board`) / ≤ 2.0 s
  (`chime`), ≤ 480 KB. Clients decode whatever the person picked (mp3, m4a,
  ogg…), trim, mix to mono, resample to 24 kHz and encode WAV themselves. The
  Worker checks the header and length and **measures loudness**: `gain =
  min(1, 0.25 / rms, 0.71 / peak)` (≈ −12 dBFS RMS, −3 dBFS peak; never
  louder than uploaded). `name` 1–24 chars (board; chimes are named after
  you), `emoji` optional (one emoji). → `{ ok, sound }` and `sound.added
  { sound }` to everyone; a chime also sets your `User.chime` to `s:<id>`.
  ≤ 60 board sounds. Bytes at `/media/<media>` (`audio/wav`).
- `Sound { id, kind, name, emoji | null, media, gain, ms, uid, createdAt }`.
  `ready.sounds` = every live board sound and every chime someone uses.
- `sound.play { id }` (you must be in voice; board sounds only) →
  `sound { id, sid, uid }` to everyone in that voice channel, you included.
  1 per 3 s per person (2 burst), 4 per 10 s per channel; else `slow_down`.
- `sound.delete { id }` (uploader or owner/mod) → `sound.deleted { id }`
  (soft: chimes and old plays aren't affected).
- **Volume limit (every client):** play at `gain × your soundboard volume`
  (Settings › Voice, default 50 %, max 100 % — never above the clip's measured
  gain), through a limiter (threshold −12 dB, fast attack); at most 2 clips at
  once (a third stops the oldest); nothing while deafened; a "Mute soundboard"
  switch; ignore plays from people you've muted (per-person volume 0).

### Polls, /roll and /flip
- **Polls:** `send { channelId, body: question, poll: { options } }` — 2–10
  options, each 1–80 chars, no duplicates; question ≤ 300 chars; not in a
  Personal space. `Message.poll = { options, votes }` where `votes[i]` = uids
  who picked option `i` (one vote each; change any time). `poll.vote { id,
  option }` (`option` = index, or null to take your vote back) → `poll { id,
  channelId, votes }` to everyone. Deleting the message erases its votes.
  Clients turn `/poll Question | A | B | C` into this.
- **Dice:** a message whose whole body is `/roll [spec]` or `/flip` is rolled
  **by the server** (so nobody can fake it): body becomes the result text and
  `Message.roll = { kind: "roll" | "flip", spec, rolls, total }`. Specs:
  nothing = 1–100; `N` = 1–N (2–1,000,000); `XdY` or `XdY+Z`/`XdY-Z` (X 1–20,
  Y 2–1000, Z ≤ 1000). `/flip` → `rolls: [0|1]` (0 = heads), `spec: "coin"`.
  Body text: `🎲 2d6: 3 + 5 = 8`, `🎲 1–100: 42`, `🪙 Heads`. Bad spec →
  `error bad_roll`.

### Doodle reactions
React with a tiny drawing: draw in a small pixel box (default 48 × 48, drawn
at 2×, up to 3 wiggle frames like Draw), save as GIF/PNG ≤ 64 KB and ≤ 256 ×
256 via `POST /api/doodle?sid=&ut=` → `{ ok, id }`, then `react { id, emoji:
"<d:ID>", on: true }`. One doodle per person per message (a new one replaces
yours). Rendered like a reaction, bigger (about 2 lines tall), click to see it
large. Removing it, or deleting the message, erases the image. Doodle
*replies* are just a reply with a picture.

### Media shelf
`shelf { channelId, before? }` → `shelf { channelId, items: Message[],
hasMore }`: messages with a picture or GIF, newest first, ≤ 60 per page
(`before` = oldest `ts` you have). Shown as a grid (header button / channel
menu); clicking one jumps to the message. Encrypted Personal-space pictures
aren't found (they're inside sealed notes).

### Quiet hours
`notify.quiet { quiet: { from, to, tz, mentions, until? } | null }` — `from`
/ `to` = minutes after midnight (0–1439) in time zone `tz` (IANA, e.g.
`Europe/Rome`); the window may wrap past midnight (23:00 → 08:00); `mentions`
= let @mentions and replies through; `until` = ms: snooze **all** pushes until
then ("until tomorrow morning"), on top of the window. → `notify { mode,
topic, quiet }`; also `ready.notify.quiet`. The server holds back pushes
during quiet time (sign-in alerts always go through). Clients apply the same
rule to their own notifications and sounds-for-notifications.

### Hang reminders
Turn a When's good slot into a plan: `hang.set { at, title, channelId? }` —
`at` = a half-hour boundary (ms) between 5 minutes and 3 weeks from now,
`title` 1–80 chars, `channelId` = a voice channel or omitted → `hang
{ hang }` to everyone. `hang.cancel { id }` (maker or owner/mod) → `hang.cancelled
{ id }`. `Hang { id, at, title, channelId | null, uid, createdAt }`;
`ready.hangs` = upcoming ones (≤ 10; ones that started < 2 h ago included).
≤ 5 upcoming at once. **At `at`** the server (a Durable Object alarm) sends
`hang.now { id }` to everyone connected and **one** push — "🗓 Game night is
starting · Lounge" — to the people who marked that slot free in When's good
plus the maker (everyone with pushes on if nobody marked it), quiet hours
respected. Clients show a calm notice with **Join voice** while it's on.

## Encrypted Personal space

Optional end-to-end encryption for your Personal space. The server stores
only a wrapped key and ciphertext; reference code in `web/js/vault.js`
(WebCrypto only, tested in `test/vault.test.mjs`).
- **Space key K:** 32 random bytes, AES-256-GCM. Made once, when you turn it on.
- **Wrap** (`ready.vault`, `vault.set`, `vault`): `{ v: 1, iter, salt, iv, ct }`,
  base64url without padding. KEK = PBKDF2-HMAC-SHA256(passcode NFKC-normalized
  as UTF-8, salt (16 bytes), `iter` (600,000 when made by the web)) → 32 bytes;
  `ct` = AES-GCM(KEK, iv (12 bytes), K, AAD `"yap:wrap:v1"`) = 48 bytes.
  Wrong passcode = the GCM tag fails. Changing the passcode re-wraps the same K
  (old notes keep opening); "start over" makes a new K (old notes stay locked).
  Other devices can't tell those two apart without the passcode, so **any wrap
  change locks every other device** until the passcode is entered again.
- **Note body:** `"e1:" + base64url(iv(12) ‖ AES-GCM(K, iv, JSON, AAD "yap:note:v1"))`,
  JSON = `{ text, gif?, image? }` (`image` = `{ id, width, height, mime }`).
  Encrypted notes go in `body`; `gif`/`image` on the message stay empty.
- **Pictures:** `iv(12) ‖ AES-GCM(K, iv, bytes, AAD "yap:blob:v1")`, uploaded
  with `POST /api/blob?sid=&ut=` (opaque bytes, ≤ 1.9 MB) → `{ ok, id }`;
  fetch from `/media/<id>` and decrypt.
- **Devices:** each asks for the passcode once and keeps K in the platform's
  secure store (web: a non-extractable key in IndexedDB; desktop: OS keychain;
  Apple: Keychain), tagged with `wrap.ct` — if `ready.vault.ct` differs (reset
  elsewhere), forget the stored key. Wrap null = encryption off.
- Notes that don't start with `e1:` are plain. Turning encryption on (and
  "start over") seals them: `personal.plain` → encrypt → `personal.seal`.
  Do the same in every client that can set a passcode.

## Screen sharing

Inside a voice channel, over the same peer-to-peer connections as audio.
- The sharer sets `voice.state { …, screen: true }`; everyone sees `voice { …, screen: true }`.
- Video only goes to people who ask: a viewer sends `signal { to: sharer, data: { screen: "want" } }`
  (and `"stop"` when they close it); the sharer adds/removes a video track on
  that one peer connection (renegotiation is the usual perfect-negotiation dance).
  So a share costs upload only per actual viewer.
- `getDisplayMedia` in browsers (a window by default, or a whole screen / a
  tab; tab audio, or system audio for a whole screen on Windows). The web asks
  what to share and picks a preset (`SCREEN_QUALITY` in `web/js/voice.js`):
  **Smooth** 1080p 60 fps ~8 Mbps per viewer (`contentHint: motion`,
  maintain-framerate, H.264 first for hardware encoding), **Sharp** 1440p
  30 fps ~6 Mbps (detail, maintain-resolution, VP9 first), **Max** up to 4K
  60 fps ~25 Mbps. Bitrate is per viewer (mesh), so Max suits one or two.
- Tab/system audio rides on its own audio track (a second audio section
  after the voice) and is music, not speech: stereo, `contentHint = "music"`,
  at least 160 kbps (see *Voice* › Opus tuning — every audio section is
  tuned, not just the first).
- Quality (Smooth / Sharp / Max) and Share sound can change while sharing;
  so can what's shared ("Share something else": a `replaceTrack`, no
  renegotiation).
- **Codecs:** every client offers and accepts H.264 constrained baseline
  (`profile-level-id=42e01f`, `packetization-mode=1`) for screen video. The
  web's presets may put another codec first but never drop H.264; the
  native apps may offer only H.264 (hardware encoding on Macs).
- **Receiving:** don't rely on stream ids. After each negotiation, the screen
  is every transceiver except the voice one (the first audio section) that
  the far side sends on: its video, plus a later audio section for its sound.
- **Sending:**
  - Encode only for people who asked, and nothing at all while nobody watches.
  - When a viewer watches again, reuse their transceivers (flip the direction)
    rather than adding new ones, so the description doesn't grow.
  - Share sound off = `replaceTrack(null)` on a sound track that stays
    negotiated. A native sharer's sound must leave out Yap's own audio, so
    the call never goes back into the call (macOS `excludesCurrentProcessAudio`,
    Windows process loopback); where that's impossible, no Share sound.
  - Keep sending a still picture at least once a second: viewers read 3 s
    without a frame as paused. Send a keyframe to a new viewer at once.
  - Forget who asked when the share stops.
- **What people see** (same words in every client): the sharer sees
  `Watching: Ana, Ben` or `No one's watching yet`; a viewer sees
  `Connecting to Ana's screen…` until the first frame, `Ana's screen is
  paused.` after 3 s without one, and `Ana stopped sharing.`, after which the
  viewer closes itself 2 s later. Viewers get Full screen (or double-click),
  Pop out, Stop watching, and their own volume for the share's sound (0–100 %,
  per person, separate from that person's voice volume).
- **Who can share:** the web (`getDisplayMedia`), the desktop app (macOS
  ScreenCaptureKit, through Apple's own picker on macOS 14+ and its own list
  on 12.3–13; Windows Graphics Capture; Linux X11; not Wayland yet), the Mac
  app (ScreenCaptureKit, picture only for now) and the iPhone/iPad app (a
  ReplayKit broadcast extension: the whole screen, picture only; the
  extension hands each scaled picture to the app, which encodes and sends it
  like any share). Every client can watch.
- Leaving voice stops sharing.

## Watch together

One shared YouTube player for the whole server (a place in the sidebar).
- `WatchState` is stored in the Hub. Where the video is now =
  `position + (playing ? (serverNow − at) / 1000 : 0)`; estimate `serverNow`
  as `Date.now() + (msg.now − Date.now() at receipt)`.
- Clients play it in a `youtube-nocookie.com/embed/<id>?enablejsapi=1` iframe
  controlled with postMessage (no YouTube script runs in Yap's page), seek when
  they drift more than ~1.5 s, and report `watch.next` at the end.
- Volume is each person's own. The audience = sessions with `watching: true`
  (they count as visible, like voice); they stay in it while browsing the
  app (a mini player keeps going), and joining voice is independent.
- Titles come from YouTube's oEmbed, fetched by the Worker (not the browser).
- The embed's postMessage protocol (JSON strings, each with `id` and
  `channel: "widget"`; web reference: `web/js/watch.js`): after the iframe
  loads, send `{event: "listening"}` every 250 ms until the player answers
  (`initialDelivery`, then `onReady`); then `{event: "command", func, args}`
  (`playVideo`, `pauseVideo`, `seekTo [s, true]`, `loadVideoById [id, s]`,
  `setVolume [0–100]`, `mute`/`unMute`). It reports `infoDelivery`
  (`currentTime`, `playerState`, `duration`, `videoData.video_id`, ~1×/s while
  playing) and `onStateChange` (-1 unstarted, 0 ended, 1 playing, 2 paused,
  3 buffering, 5 cued). Pass `origin=<your origin>` in the embed URL, and send a
  Referer (web: `referrerpolicy="strict-origin-when-cross-origin"` on the
  iframe; Yap's pages send none) or YouTube refuses to play (error 153).
- Only a change the person made in their own player is shared: ignore state
  changes for ~1.5 s after commands you sent, and anything while the app is
  in the background (phones pause video on their own).

## Custom emojis in text

A custom emoji inside `body` is the token `<:name:id>` (Discord's format).
Clients turn `:name:` into the token when sending (known names,
case-insensitive, never inside code), and render tokens as inline images
from `/emoji/<id>`. Render as ~22 px inline, or 48 px when the message is only
1–8 emoji. Because the token carries the id, renaming or deleting an emoji
never breaks old messages. Elsewhere (notifications, previews) show `:name:`.

## GIFs

GIFs come from [KLIPY](https://docs.klipy.com) (Tenor's API shut down
2026-06-30). Get the app key from `/api/config`; `null` means the server has
no key — hide or disable the picker.

KLIPY's integration rules, which every client must follow:
- Call `https://api.klipy.com/api/v1/{appKey}/gifs/{trending|search|recent/{customer_id}}`
  **directly from the client** — never through the Yap server.
- Load media straight from the returned `static*.klipy.com` URLs, unmodified.
  Don't cache or re-host it.
- Show search and trending results in the returned order, in their own grid.
- Show KLIPY branding in the picker ("Powered by KLIPY").
- `customer_id` = first 16 bytes (hex) of `SHA-256("yap:" + uid)`. That gives
  a stable, anonymous id, shared across all clients so "Recent" follows the user.
- After sending, `POST /gifs/share/{slug}` with `{ customer_id, q? }`. That feeds
  "Recent".

To send, map the item's `file.md` to a `Gif` (`md.mp4.url` → `mp4`, … ,
`md.jpg.url` → `still`; `width`/`height` from the same size) and send it as
`gif`. The server rejects any media URL that isn't `https://static*.klipy.com/…`,
because an arbitrary image URL could log the IP of everyone who views the
message. Render it at its aspect ratio (Yap uses a 300 px box): video (mp4,
then webm) when available, otherwise webp or gif, with `still` as the poster.

## Voice

Voice is a **WebRTC mesh**: each pair of people in a channel holds one direct
`RTCPeerConnection` with a single bidirectional audio transceiver. Audio never
goes through the server. Any WebRTC stack works (libwebrtc, webrtc-rs, Pion,
libdatachannel) as long as it speaks Opus.

**Who connects to whom**

1. Fetch `/api/ice`, open the mic, send `voice.join`.
2. When **your own** `voice` echo arrives with the channel, create a
   connection to every session already in that channel and **offer** (add one
   `sendrecv` audio transceiver). The server orders events, so everyone
   agrees on who was there first.
3. People already in the channel **don't** offer to a newcomer; they create a
   connection when the newcomer's offer arrives and answer it, putting their
   mic on the offered transceiver (`sendrecv`).
4. When a session's `voice` event moves it out of your channel (or
   `session.leave`), close that connection — unless it's a call you're keeping
   across a reconnect (next section).

**Renegotiation & glare:** use *perfect negotiation*. The session whose `sid`
is lexicographically **larger is polite** (rolls back on a colliding offer);
the smaller one ignores colliding offers.

**ICE restarts** (the path died: Wi-Fi → cellular, a NAT rebinding, a laptop
waking up). The **impolite** side restarts ICE once `connectionState` has been
`disconnected` for **3.5 s**, or at once on `failed` — after fetching
`/api/ice` again (relay credentials can expire) and applying it with
`setConfiguration`. The **polite** side does the same after 10 s, in case the
other app only restarts on `failed`; perfect negotiation sorts out a clash.
Keep trying every 10 s while it's down. After the third try, if the other
side keeps calls across reconnects (`Session.voice.resume`), the impolite
side throws the connection away and offers a new one (the other side sees a
new DTLS fingerprint and follows: *Keeping calls across reconnects* ›
fresh start). Measured with the old path killed for good: audio back after
~10 s (it was ~16 s, waiting for `failed`). *(Older apps restart only on
`failed`, impolite side only; they still interoperate.)*

**`signal.data`** is exactly one of:

```json
{ "sdp": { "type": "offer" | "answer", "sdp": "v=0..." } }
{ "candidate": { "candidate": "candidate:...", "sdpMid": "0", "sdpMLineIndex": 0, "usernameFragment": "..." } }
```

Apply signals from a peer strictly in order (queue them); don't send
end-of-candidates (`null`).

**Opus tuning.** Before setting each local description (offer *and*
answer), rewrite the Opus `a=fmtp` line of **every audio section** (keep
existing params, override these). These describe what *you* want to receive:

```
minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=<1 if you send stereo there>;
maxaveragebitrate=<your quality setting, bps>;maxplaybackrate=48000;sprop-maxcapturerate=48000
```

The first audio section is the voice (`sprop-stereo=1` only in music mode).
Any later audio section is a screen share's audio (see *Screen sharing*):
`maxaveragebitrate` = max(your setting, 160000), and `sprop-stereo=1` where
you're the one sending it (the section is `sendrecv`/`sendonly` in your
description).

**No `transport-cc` on audio.** In every audio section, also drop the
`a=rtcp-fb:<pt> transport-cc` lines and the
`a=extmap:<id> http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01`
line. With it, libwebrtc's bandwidth probing pads audio-only calls (measured:
~18 kbps per direction per person on a clean network, ~500 kbps at 5 % loss)
and gains nothing, since Opus runs at a fixed target. Video sections (screen
sharing) keep it. An answer can't add back what an offer left out, so
stripping your own description is enough whoever offers. *(All three clients
strip it: the web in `tuneOpus`, Apple in `SDPTuner`, and the desktop by
registering the transport-cc interceptor for video sections only, since
webrtc-rs refuses an edited local description. A native client built on another stack must do the
same, or native ↔ native calls pad again.)*

Also cap your own sender: `encodings[0].maxBitrate = <your setting>`,
`priority`/`networkPriority = "high"`. Net effect: each direction runs at the
lower of the two people's settings. Default 128 kbps; offer 64/96/128/256/510.

**Redundancy (RED, RFC 2198) on the voice.** In the voice section (the
first audio section), offer and answer `red/48000/2` with
`a=fmtp:<red pt> <opus pt>/<opus pt>`, listed **before** Opus in the `m=`
line, so each voice packet also carries the 20 ms frame before it. A lost
packet is then rebuilt from the next one, word for word, instead of being
patched over (Opus FEC only rebuilds a rough version, and nothing survives two
lost in a row). Rules:
- Keep Opus in the list and never require RED: with an app that doesn't offer
  it, the call is plain Opus, exactly as before.
- Voice only. A screen share's audio sections stay plain Opus (don't put RED
  first there).
- One earlier frame (distance 1). Leave the copy out of a packet when it would
  be over 1023 bytes (RFC 2198's block length; only near 510 kbps) or make
  the packet's payload over 1200 bytes; a RED packet may carry the primary
  block alone.
- Receiving: use a copy only for a frame whose own packet never arrived (by
  RTP timestamp, before its turn to play), and never twice. A copy for a frame
  already played or patched over is dropped.
- Opus keeps your quality setting; the copy rides on top, so a 128 kbps voice
  takes about 256 kbps on the wire per person you talk to. Where a stack counts
  the copy against `maxBitrate`, raise the cap so the Opus target itself stays
  at the setting (measure it).
*(Web: browsers send RED only when it's first in the negotiated list, so the
web puts it first with `setCodecPreferences`. Apple: libwebrtc packs and
unpacks it once negotiated. Desktop: webrtc-rs only carries the packets; the
app packs and unpacks them itself.)*

**Transmit gate** (mute, voice activity, push-to-talk) = stop sending audio
frames / disable the track, *not* renegotiation. Mute + deafen state is
broadcast via `voice.state` purely for display. Gate on the audio path itself
(the capture callback / an audio-thread processor), never from a UI timer, and
measure the level *before* the gate so the meter keeps working. What every
client should do (the web's is `web/js/gate-worklet.js`, with tests):

| | |
|---|---|
| Level | RMS over 10 ms blocks of the processed mic (after noise removal), in dBFS |
| Voice activity opens | above `max(sensitivity, noise floor + 12 dB)`, the latter capped at −30 dBFS. Sensitivity default −50 dBFS RMS. |
| …stays open | while above `max(sensitivity − 6 dB, noise floor + 8 dB)` (hysteresis), then a **450 ms hold** |
| Noise floor | the quietest 10 ms block in each 500 ms, the quietest of the last 8 of those (ignore digital silence) |
| Pre-roll | voice activity only: send audio **40 ms late**, so opening includes the 40 ms before the trigger (first syllables aren't clipped). Never in push-to-talk or always-on. |
| Push-to-talk | open while held, plus a **200 ms tail** after release |
| Fades | 5 ms in, 15 ms out (no clicks) |
| Music mode + voice activity | behaves as always-on (a gate cuts quiet passages out of music) |
| Mute / deafen | a hard switch outside the gate, so it holds even if audio processing stalls |

*(Reading the level from WebRTC stats instead of the samples: a media
source's `totalAudioEnergy` adds up each 10 ms frame's **peak**, not its RMS,
so on speech it reads about 9 dB hotter than the table's RMS — 10 dB on noise,
3 dB on a pure tone. Correct it by −9 dB, as the Apple app does, or the same
sensitivity means a different thing. Where a stack has no hook on the send
path (Apple's WebRTC SDK), the 40 ms pre-roll and the fades can't be done;
libwebrtc's own ~3 ms ramp keeps it click-free.)*

**Speaking indicators** come from each receiver's RTP audio level
(`getSynchronizationSources()[0].audioLevel` in browsers; the
`ssrc-audio-level` header extension elsewhere) — nothing goes over the
socket.

**Connection hints** (each client on its own; nothing on the wire). Next to a
person in your call, a quiet word, and only when something's off:
- *weak connection* — more than 3 % of their audio concealed (patched over
  after lost or late packets: `concealedSamples − silentConcealedSamples`
  over `totalSamplesReceived`) or a round trip over 300 ms, judged every 2 s
  while they're actually sending (≥ 30 packets/s), never in a connection's
  first 10 s (the jitter buffer is still settling); shown after 3 bad
  readings in a row, gone after 3 good ones.
- *needs the voice relay* — ICE `failed` and the server's `/api/ice` has no
  TURN server (*can't connect* when it has one).
- *reconnecting…* — their socket dropped and you're keeping the call (next
  section).

**Call report** (each app on its own; nothing on the wire). Every app keeps
notes on the call you're in and on the last one you left, so a call that
sounded off can be looked into afterwards. **Copy call report** (in the call's
⋯ menu or voice panel) and **Copy last call report** (Settings › Voice) put it
on the clipboard as plain text; the toast says "Call report copied." It
never leaves the device by itself and holds no IP addresses, no message text
and no keys. Kept after you leave: the web in `localStorage` (the last one),
the apps in their own data folder (the last three). Contents, in this order:
1. **Header:** `Yap call report`; the app, its version and build; OS and
   device; the channel's name; when the call started (local time) and how long
   it lasted; your settings (quality, and the channel's if set; input mode:
   voice activity with its sensitivity / push-to-talk / always-on; noise
   suppression; echo cancellation; automatic gain; music mode; hide my IP;
   iPhone: high-quality headphone audio and the mic mode iOS reports); your
   audio devices or route as the OS names them.
2. **People** (everyone who was in the call with you), one block each: name;
   how you were connected (direct or relay, and the local ↔ remote candidate
   *types* only: host, srflx, prflx, relay); the codec each way (`red+opus` or
   `opus`) and the Opus fmtp you sent; kbps sent and received (median); round
   trip (median, and the worst 2 s reading); lost packets (% of expected);
   audio patched over (`concealedSamples − silentConcealedSamples` as a % of
   `totalSamplesReceived`); how many packets RED rebuilt, where the stack says;
   jitter buffer delay (median); ICE restarts, connection rebuilds and
   reconnects kept; how long *weak connection* showed.
3. **Timeline**, as `+mm:ss` since you joined (`+h:mm:ss` past an hour):
   joining (and who was there), people joining and leaving, connection
   states, ICE restarts, your socket dropping and coming back, settings
   changed mid-call, audio route changes, interruptions (iPhone), network
   changes (Wi‑Fi ↔ cellular), *weak connection* on and off, and why the call
   ended. Plus a line per person every 10 s for the first minute, then every
   minute: `Ana: rtt 38 ms, lost 0.4 %, patched 0.1 %, 129 kbps in, 131 out`.
   At most 2,000 timeline lines (the oldest go first; the summary never does).

### Keeping calls across reconnects

Audio goes peer to peer, so a dropped WebSocket doesn't have to cut a call:
every deploy restarts the server and drops every socket, Wi-Fi blips, phones
wake up. Clients that support it keep their peer connections while the socket
is down and move them to the new session id afterwards — same
`RTCPeerConnection`, no new ICE or DTLS, no gap (measured: a 0.5–0.7 s
silence and 3–5 chimes before; nothing now). It's optional and per pair: a
client that does none of this works exactly as before, and two people only
hand over when **both** do it.

**On the wire**
- `ready.resumeKey` — this session's resume key: base64url (43 chars) of
  HMAC-SHA256(server secret, `"voice-resume:<sid>:<uid>"`). Keep it to
  yourself; it's only good for this session and this account. The secret is
  random, made once and kept by the server, so keys still check out after a
  deploy, when the server remembers nothing else about old sessions.
- `voice.join { channelId, muted, deafened, resume: true }` — "I keep calls
  across reconnects". Echoed as `resume: true` in `voice` broadcasts and in
  `Session.voice`, so everyone knows who can hand over.
- `voice.join { …, resume: true, prev: <old sid>, proof: <old session's resumeKey>, screen? }`
  — this join continues `prev`'s call. The server accepts it only if `proof`
  is `prev`'s key **for your account** (another account can never take a
  session over, even holding its key). Accepted: `voice { sid: <new>,
  channelId, …, resume: true, prev: <old> }` to everyone, you included — your
  echo **with** `prev` = handed over; **without** `prev` = refused, join as a
  newcomer. `prev` stays in that session's `voice` messages and
  `Session.voice.prev` until it leaves voice, so people who connect later can
  still match it. `screen: true` keeps a screen share going.
- If the server still holds the old socket (half-dead), it announces the
  resume first, then drops the old session as usual (`voice {sid: old,
  channelId: null, lost: true}`, `session.leave`) and closes it with 4000.
  Until then your new `ready.sessions` may still list your own old session:
  that's you, not someone to call. Leave it out of the people you offer to
  (and of the list), and ignore its later `voice`/`session.leave`, or you
  call yourself and then show yourself as "reconnecting…".
- `voice { sid, channelId: null, lost: true }` — that session's socket closed
  while it was in voice (no `voice.leave`). It may come back as a `prev`. A
  deliberate `voice.leave` never carries `lost`.
- Nothing else changes: signals still go `to` a sid, the server never sees
  audio, and a resume costs one HMAC check.

**Your socket drops while you're in a call**
1. Don't hang up. Keep the mic and every peer connection; queue each peer's
   outgoing signals (ICE candidates, offers) instead of sending them.
   Remember `{ sid: <your old sid>, key: <its resumeKey>, at: now }`.
2. Not back in voice within **20 s**: give up — close every connection and
   join as a newcomer when you're connected again.
3. On `ready` (a new sid): first *match* `ready.sessions` (below). Any kept
   connection whose far side isn't in `ready.sessions` stays, waiting: they
   may be reconnecting too (after a deploy everyone is). Then send
   `voice.join { channelId, muted, deafened, resume: true, prev, proof, screen? }`.
4. Your own `voice` echo **with `prev`**: for each kept connection whose far
   side is in your channel now — `Session.voice.resume` set: keep it (send its
   queued signals to its current sid; if an offer of yours is still
   unanswered, send it again: the answer may have gone to your old sid);
   not set: close it (that's an older client, which hung up on you when your
   socket dropped). Then offer to everyone in the channel you have no
   connection with, exactly like a newcomer. No join chime.
5. A kept connection that isn't `connected` 4 s after the handover: close it
   and offer a fresh one (rule *fresh start* below lets the far side follow).
6. Echo **without `prev`**: close everything and join as a newcomer.

**Someone else's socket drops**
- `voice { sid: X, channelId: null, lost: true }` for someone in your channel
  whose `Session.voice.resume` was set and whom you have a connection with:
  keep the connection and keep showing them ("reconnecting…"), no leave
  chime, for up to **30 s** — or until the connection goes `failed`/`closed`
  or stays `disconnected` for 5 s. Then close it (and chime). The
  `session.leave X` that follows doesn't close it either.
- Anyone else, or a leave without `lost`: close at once, as before.

**Matching (moving a connection to a new sid)** — whenever a session in your
channel has `voice.prev` = a sid you have a connection with (in
`ready.sessions`, or a `voice` broadcast): if it's the same account, move that
connection (and anything else you keep per sid: speaking state, the screen
you're watching) to the new sid and forget the old session. *Polite* is
recomputed from the current sids (both may have changed). Send that peer's
queued signals, and re-send an unanswered offer.
- A plain join (no `prev`) by the same account while you're keeping a
  connection for one of its old sessions = they couldn't resume (or their app
  can't): close the kept one quietly; they offer as a newcomer.
- *Fresh start:* an offer whose DTLS `a=fingerprint` differs from the one
  you already have for that sid means the far side started a new connection:
  close yours and answer with a new one.

**Chimes** (every client): a `lost` leave plays its leave chime only after
**5 s**, and if the same account joins your channel again within those 5 s,
neither chime plays. A deliberate leave chimes at once. **Leaving for real**:
send `voice.leave` before closing (the web does on `pagehide`), so nobody
waits for you.

**With older clients** (no `resume`): they see `voice {…, lost}` as a plain
leave and hang up; they see a resumed session as a plain join and wait for
its offer — which it sends (step 4). When *they* reconnect they hang up and
join as newcomers, offering to everyone; resuming clients close the kept
connection on that plain join (above). So a call between an older client and
a resuming one behaves exactly as it always has.

## Updates

**Web.** `tools/stamp.mjs` (run by `npm install`, `npm run dev`, `dev:test`,
`check` and `deploy`; the files aren't committed) hashes the web client into
`web/js/build.js` and `worker/build.ts`: `style` = the three skins'
stylesheets (`style.css`, `apple.css`, `arch.css`), `app` = every other file
under `web/`, folders included (`js/app/*` too), except `_headers` and
`downloads/`. A deploy restarts the Durable Object, which drops
every socket; pages reconnect and get the new `build` in `ready`. Sockets that
survive get a `build` message when the new version first starts.
- Only `style` differs → the page points its stylesheet
  (`link[rel=stylesheet]`) at `?v=<style>`: swapped in place, no reload.
- `app` differs → an "update ready" bar, and the page reloads itself once it's
  safe: not in voice, nothing typed, no dialog open, and the tab hidden or
  idle for 5 minutes. The draft, channel and open place survive the reload.

Native clients ignore `build`, and so does the OBS page (`/obs`).

**Staying compatible.** A page can run an older build for a long time: in a
call, with a draft or a dialog open, or as an OBS source, which never reloads.
So the server keeps accepting what the previous web build sends for at least
a day after a deploy, and the OBS page's few messages (`hello` with a token,
`voice.join`, `voice.leave`, `voice.state`, `signal`) for good. A change that
can't do that is a breaking one (`PROTOCOL_VERSION`).

**Desktop apps.** `GET /downloads/latest.json` (a static file, so it costs no
Worker requests):
```json
{ "version": "0.2.0", "date": "2026-10-05", "notes": ["…"],
  "files": { "windows-x86_64": { "path": "/downloads/<name>", "size": 123, "sha256": "<hex>", "sig": "<base64>" } } }
```
Platforms: `linux-x86_64`, `windows-x86_64`, `macos-aarch64`. `sig` is an
Ed25519 signature over the file's raw bytes; the apps carry the publisher's
public key (compiled in at build time: `YAP_UPDATE_PUBKEY` or
`desktop/update-pubkey.txt`) and refuse anything it doesn't verify, so even a
compromised server can't push a binary. Builds without a key don't
self-update. `tools/publish-desktop.mjs` signs (with `YAP_UPDATE_SIGNING_KEY`,
or a macOS Keychain item) and publishes.
Apps check at launch and every 6 h and install only when the person clicks.

## Backups

All admin calls take `Authorization: Bearer <ADMIN_KEY>` (Worker secret; the
tools read it as `tools/instance.mjs` says). Unset key = admin API off.

| Endpoint | |
|---|---|
| `GET /api/admin/stats` | `{ hub: { databaseBytes, messages, users, channels, emojis, sessions, pending }, media: { blobs, bytes, databaseBytes } }`: row counts and database sizes (each database's limit is 1 GB on the free plan). |
| `GET /api/admin/export?after=<ts>` | The chat database, paged 5000 messages at a time (follow `next` until `null`): archived channels, deleted messages (`deletedAt`, as empty rows), deleted emojis, and on the first page reactions, edits, strokes, drawings, poll votes, sounds, hangs, invites, links, the mod log, device-token hashes and Draw's page number. **Not in it:** When's good, browser push subscriptions, sign-in lockouts, the media trash, and the server's own bookkeeping (Watch together's state, its address, the voice resume secret). After a restore, When's good and Watch start empty, and browser pushes come back as each browser opens Yap again. |
| `GET /api/admin/media` | Every stored file `{ id, mime, size }` (emojis, pictures, avatars, drawings, doodles, clips, sealed pictures). |
| `GET /api/admin/media/<id>` | One stored file's bytes, `Cache-Control: no-store` (backups use this, so nothing a post's deletion should erase lingers in a cache). |
| `POST /api/admin/emoji?name=N` | Add an emoji without a live session (bulk imports); body = image bytes. Uploader shows as `admin`. |
| `POST /api/admin/role?uid=&role=owner\|mod\|member\|banned` | Set a role (also lets someone out of the waiting room). |
| `POST /api/admin/reset-pin?uid=` | A temporary PIN for someone who forgot theirs (or never set one) → `{ ok, name, pin }`. |
| `POST /api/admin/channels?names=a,b[&kind=voice]` | Add channels by name (names that exist are skipped); `?archive=a,b` archives instead (messages kept), `?unarchive=a,b` brings archived ones back with their history. Clients get the usual `channel` / `channel.archived`. |
| `POST /api/admin/merge-users?from=<dup uid>&into=<real uid>` | Merge a duplicate account: `from` reads as `into` everywhere (messages, emojis, deletes) and leaves the member list; its devices adopt `into` on reconnect. Nothing is rewritten. Everyone is bounced to reconnect. |
| `POST /api/admin/unmerge-user?alias=<uid>` | Undo a merge. |
| `POST /api/admin/media-put` | Store a file from a backup (pictures, clips, sealed pictures; content-addressed) → `{ ok, id, mime }`. For restores. |
| `POST /api/admin/import` | Rebuild from a backup: `{phase:'check'}`, `{phase:'begin', data}` (all tables but messages), `{phase:'messages', messages}` (pages), `{phase:'finish'}`. **Only into an empty server** (no users, no messages) — 409 otherwise. |
| `POST /api/admin/restore?to=<ISO time>` | **Point-in-time recovery** of the chat database to any moment in the last 30 days (Cloudflare keeps this history). Restarts the server; clients reconnect. Returns `undo` — `POST ?bookmark=<undo>` reverses it. |

`tools/backup.mjs` pulls a full snapshot (`backups/yap-YYYY-MM-DD.json.gz` +
`backups/media/`), keeps 60 days; `tools/install-backup.sh` runs it daily via
launchd (macOS). Put `backups/` (or `YAP_BACKUP_DIR`) somewhere synced for an
off-site, versioned copy. `backups/media/` keeps a file while the server has it
or a kept snapshot refers to it (`referencedMedia()` in `tools/lib.mjs`), so a
deleted post's picture leaves the backups once no kept snapshot shows it.

**Disaster recovery** (Cloudflare account gone, database lost beyond the 30-day
window): deploy Yap to the new place with the **same `PIN_PEPPER`** (keep a copy
somewhere safe; without it nobody's PIN works) and the same `YAP_KEY`, then
`node tools/restore.mjs backups/yap-<date>.json.gz --to https://<new server>`.
Files go back first (only those the snapshot refers to, plus sealed pictures,
whose ids are inside encrypted notes), then every table, then messages. A
deleted post comes back as an empty row even from a backup taken before
deleting erased everything. `npm run test:all` runs this round trip every time
(`test/roundtrip.test.mjs`). Admin calls skip the per-IP limit.
