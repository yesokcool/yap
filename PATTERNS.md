# Yap patterns & gotchas

## Techniques

### Durable Object that survives hibernation
Accept sockets with `ctx.acceptWebSocket(ws)` (hibernation API), keep
per-socket state in `ws.serializeAttachment()`, and rebuild everything else
from `ctx.getWebSockets()` + SQLite. Heartbeats use
`ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping','pong'))`
so idle connections cost nothing.

### A Durable Object in feature modules
`worker/hub.ts` is one `Hub` class (Durable Objects need one), but its
features live in `worker/hub-*.ts` as plain functions:
`export function onDrawStroke(hub: HubHost, ws, me, msg)`. `HubHost`
(`worker/hub-host.ts`) is `Pick<Hub, …>`: exactly the members the features
use, which are public on `Hub` for that (TypeScript's `private` never stopped
anything at runtime, and only Yap's own Worker holds a stub). `env` and `ctx`
are re-declared public with `declare`, so no field is emitted that would
reset them after `super()`. Rules that keep it safe:
- The dispatch, `fetch`, `webSocket*` and `alarm` stay on `Hub`, and every
  Worker RPC stays a method there as a one-line delegation.
- Calls between features are imports (`deliver(hub, …)`); a function that
  never needs the Hub doesn't take it.
- State that belongs to the object (`lastPush`, `lastJoinPush`, caches)
  stays an instance field. A module-level variable would outlive the object
  and be shared by every object in the isolate.
- Keep every `await` where it was (see *handlers interleave at an outgoing
  fetch()* below), and handlers keep returning `void`.

### The web client's modules (no bundler)
The page is native ES modules: `web/js/app.js` (the entry) imports its parts
from `web/js/app/`, and they import each other. Cycles are fine because the
rules keep them harmless:
- `app/state.js` imports nothing from `app/`, so it always runs first and
  everything can use `state`, `$` and `conn` while the rest load.
- Parts call each other's `function` declarations (hoisted, so a cycle
  can't hit one before it exists) and read each other's `let`s only inside
  functions, never while loading. A `let` lives in the module that assigns
  it; others read it (imports are live bindings) but can't assign it, so
  `start()` sets the socket with `setConn()`.
- Singletons (the Voice object, the Watch player) are made at the top of
  their part; everything that listens to the page (document, window, the
  message box) is registered in `bindX()` functions that `app.js` and
  `bindUi()` call in a fixed order, so the order listeners are added in
  never depends on which module happened to load first.
- A new file is covered by the build hash on its own (`tools/stamp.mjs`
  hashes all of `web/`); add it to `index.html`'s `modulepreload` list so it
  loads next to `app.js` instead of after it.

### Append-only migrations
`MIGRATIONS` array in `worker/schema.ts`, schema version in a `meta` table,
applied by `migrate()` in `blockConcurrencyWhile` in the Hub's constructor.
New schema = push a new function, and extend `test/migrations.test.mjs`
(Node's SQLite, from every older version, with data in it).

### Opus tuning by SDP munging
`tuneOpus()` in `voice.js` rewrites the Opus `a=fmtp` line of *every audio
section* (the voice, and a screen share's audio after it) on every local
description (offer *and* answer) before `setLocalDescription`, and drops
`transport-cc` from audio (it only buys bandwidth-probe padding). fmtp params
describe what the *receiver* wants, so each side tunes its own description,
and the sender also caps itself with `RTCRtpSender.setParameters`
(`maxBitrate`). Verify with `getStats()` → `codec.sdpFmtpLine` and
`inbound-rtp.bytesReceived` over time (see `test/e2e.test.mjs`; the SDP
rewriting itself is unit-tested in `test/voice.test.mjs`).

RED on the voice isn't munged: `preferAudio()` puts `audio/red` first with
`setCodecPreferences` on the voice transceiver (when offering, and before
answering), and a share's sound gets Opus first with RED left out. Chrome
*sends* the first codec of the other side's description, so each direction
follows the other person's order; an app that doesn't offer RED gets plain
Opus. Chrome's stats name the Opus inside RED as the codec both ways
(`audio/opus`, and `fecPacketsReceived` doesn't count RED's copies), so check
RED from the descriptions (the voice section's first payload type) and from
bytes per packet: ~2× Opus's, while `outbound-rtp.targetBitrate` stays at
`maxBitrate` (the copy isn't counted against it; at 510 kbps the copy doesn't
fit and is left out).

### Voice gate on the audio thread, without killing the meter
The mic goes through an `AudioWorklet` (`web/js/gate-worklet.js`) into a
`MediaStreamAudioDestinationNode`, and *that* track is what peers get. The
worklet measures its input (so voice activation can always reopen, and the
meter keeps moving while the gate is shut) and gates its output with fades,
hold, hysteresis, a push-to-talk tail and a 40 ms pre-roll delay line. Flipping
`track.enabled` from the main thread (the old way) clipped first syllables by
20–50 ms, and more whenever the page was busy. Mute and deafen still use
`track.enabled` on the sent track: they must hold even if the audio thread
stalls. Keep the logic in a plain class (`Gate`) with the processor as thin
glue, so Node tests can run the exact code on synthetic speech.

### A call that outlives its WebSocket
Media is peer to peer, so only signalling needs the socket. When it drops
(every deploy drops them all), the web keeps every `RTCPeerConnection`,
queues their signals, and on reconnect joins with `prev` = its old sid +
`proof` = that session's `ready.resumeKey`. The server checks
HMAC(secret, sid:uid) — stateless, so it works after a deploy restarted the
Hub — and broadcasts the join with `prev`; everyone moves the same
connection to the new sid (`Voice.rekey`). People whose socket dropped stay
listed as "reconnecting…" for up to 30 s (less if their connection dies).
Three things make it robust: handover only between clients that both say
`resume` (older apps get the old leave/join/offer dance, untouched); an offer
with a new DTLS fingerprint means "they started over", so start over too; a
kept connection not `connected` 4 s after the handover is rebuilt. The test
for a deploy locally: change a file in a *copy* of the repo that a dev
server is serving — wrangler reloads, which restarts the Hub and drops every
socket, like a deploy (a mere `touch` doesn't always reload). PROTOCOL.md ›
Voice › Keeping calls across reconnects.

### Never silently lose the mic
Browsers drop audio without telling the page: a mic is unplugged or another
app grabs it (the track ends), the system default device changes (the old
track keeps running on the old device), Safari interrupts the AudioContext for
a phone call, or a tab's audio graph is suspended. `voice.js` reopens the mic
on `ended`, follows `devicechange` (the device in use vanished, your picked one
came back, or Chrome's `default` entry has a new `groupId`), resumes the
context on `statechange` (and on the next tap if a gesture is needed), and
watches the gate's 20 ms reports: 1.5 s without one = stalled, so peers get a
plain clone of the mic (fail open: heard without processing beats silently
muted) until reports resume. Mute keeps working on the fallback. The gate
also flags 5 s of exact digital silence (a hardware mute switch), shown as
"Your mic is sending silence".

### Keyed message list
`renderMessages` builds `{key → html}` per row and `reconcile()` reuses the
existing element whenever a row's HTML is unchanged, only inserting/removing
what changed. Appends and prepends never move existing rows, so GIF videos
keep playing. Use this for any list holding media.

### GIF videos
KLIPY's mp4/webm play as muted, looping, inline `<video>` elements (much
lighter than a .gif). An `IntersectionObserver` plays them only while
on screen. With reduced motion they stay on the poster until hovered or tapped.

### Faking third-party APIs in e2e
`context.route()` in Playwright stubs `api.klipy.com` and `static*.klipy.com`
(SVG tiles, 404 videos → poster fallback), so the picker is tested without a
key or network, and the stub logs every request for assertions.

### Headless voice testing
Playwright Chromium with `--use-fake-device-for-media-stream`,
`--use-fake-ui-for-media-stream`, and `--use-file-for-fake-audio-capture=<wav>`
(generated music-like noise, so Opus actually spends the bits). Two browser
contexts = two users with separate localStorage. On some macOS builds headless
Chromium hangs forever opening *any* audio capture (the fake mic, a tab's
sound) while video capture still works: `test/e2e.test.mjs` probes the fake mic
first and otherwise injects `test/synth-capture.js`, a WebAudio mic playing the
same signal plus video-only `getDisplayMedia` with WebAudio "tab sound"
(`YAP_E2E_SYNTH_AUDIO=1` forces it). Each synthetic track has its own
AudioContext, so suspending Yap's audio graph (the fail-open test) doesn't
silence the "mic".

### A screen share any WebRTC stack can send
The viewer never trusts stream ids: a native stack may send the video with no
`a=msid` stream, or its sound in a stream of its own. After every negotiation
(`signalingstatechange` → `stable`), the screen is every transceiver but the
mic's whose `currentDirection` says the far side sends (`recvonly`/`sendrecv`),
merged into one `MediaStream` (`Peer.syncRemoteScreen` in `web/js/voice.js`).
The sharer keeps each viewer's transceivers and flips them (`replaceTrack` +
`direction`) instead of `removeTrack`/`addTrack`, which would add a new m-line
every time someone watches again. A new source is a `replaceTrack` (no
renegotiation); Share sound off is `replaceTrack(null)` on a sound transceiver
that stays negotiated. Codec preferences only reorder (`orderCodecs`), so H.264
constrained baseline, all the native apps send and receive, is always offered.

### Games over a relay that stores nothing
Space's mini-games (`space.game`) have no server state, so each game has an
**owner** (the ball's last kicker, whoever is "it") whose copy is the truth.
Every update carries a counter `k`: a higher `k` wins, a tie goes to the lower
sid, so two kicks at once still converge. Newcomers send `sync` and only the
owner answers; when the owner leaves (`view`/`session.leave`, or a 10 s
silence), the lowest sid still there takes over, which everyone can work out
without talking. Everyone simulates locally between updates (dead reckoning)
and draws corrections as a decaying offset instead of a jump. Hidden tabs get
no `requestAnimationFrame`, so a 250 ms interval keeps voice zones and games
ticking while the tab is in the background.
Two cheaper cases: things about **you** (on fire, dripping, a balloon hit
you, your lap time) are announced by you alone, since you see exactly where
you stand; and things that follow from where people stand (the floor piano)
need no message at all, because every client already has everyone's
position.

### A video that keeps playing while you browse
Watch together's YouTube iframe lives in one fixed-position "dock" appended
to `<body>` and is never moved in the DOM. On the Watch view a
`requestAnimationFrame` loop keeps the dock exactly over the 16:9 slot (so
banners and resizes can't misalign it); anywhere else the same element
becomes the mini player. The player is driven by postMessage alone (no
iframe_api script), and the e2e test stubs `youtube-nocookie.com` with a fake
player page that speaks the same protocol (`test/e2e.test.mjs`).

### One push at a future time, on the free plan
Hang reminders need "wake up at 8 PM Friday". A Durable Object **alarm** does
it with no cron and no polling: `armHangs()` sets `storage.setAlarm()` to the
earliest pending hang after every change (and after a restore); `alarm()`
marks due hangs `notified_at` first, sends, then re-arms. Anything hours late
(the object slept through a deploy) is marked but not pushed. Tests start the
server with `TEST_HANGS=1` so a hang can start a second from now.

### Loudness you can't fake
Clients upload clips as plain PCM WAV (they decode/trim/resample with
`OfflineAudioContext` first) so the Worker can read every sample and store
`gain = min(1, 0.25 / rms, 0.71 / peak)`. A modified client can't make a clip
louder than that: every client plays at `gain × your volume` through a
`DynamicsCompressor` limiter, two clips at most.

## Gotchas & lessons

- **An RTP check by payload type misses RED.** The impairment proxy treated
  only payload types 96–127 as RTP, so Chrome's RED (63) skipped the loss and
  every RED number looked perfect (2026-10-11). Tell RTCP apart by RFC 5761
  (second byte 192–223) and treat everything else as RTP; send media back to
  the socket it last came from, not to whoever pinged last.

- **webrtc-rs `get_transceivers()` comes out of a HashMap.** "The first one"
  can be a screen share's transceiver: find the voice by transceiver id 0.
  Answering an offer, webrtc-rs lists exact-fmtp matches first, so set codec
  preferences on every audio section you care about (a share's sound came out
  RED-first).

- **SwiftPM builds a package without `DEBUG` under a custom configuration.**
  The app's "Dev" configuration defines `DEBUG`, but YapKit is built as
  release there, so app code under `#if DEBUG` that calls YapKit's
  `#if DEBUG` API fails only in Yap DEV (2026-10-11). Keep debug hooks the
  app calls outside `#if DEBUG` in the package.

- **Test data leaks between suites on a reused dev database.** e2e matched
  `:has-text("Horn")` against an older "Airhorn" and a hang titled "Game
  night" from a screenshot run (2026-10-06). Use exact matches
  (`:text-is()`), unique names, and a fresh `--persist-to` before full runs.

- **webrtc-rs only fills `remote-inbound-rtp` loss from Receiver Reports.** A
  peer that is also sending reports our loss inside its Sender Reports, so in a
  two-way call the stat never moves. The desktop reads the report blocks itself
  (`desktop/src/voice/loss.rs`) to drive the encoder's packet-loss hint.
- **Chrome sends padding-only RTP packets** in calls (hundreds per call). A
  native client that drops them as "empty" sees false packet loss and glitches
  every few seconds (the desktop client did, until fixed).
- **`wrangler dev` hot-reloads on every file save** and drops all sockets —
  don't edit worker/web files while a test run is going; and background
  agents running their own dev servers must use their own ports and must not
  kill other `wrangler`/`workerd` processes.

- **Never build file paths from `new URL(import.meta.url).pathname`**: if the
  repo's path has a space, `.pathname` keeps it as `%20`, and Node happily
  creates a *new* `…/My%20Folder/…` tree next to the real one (in a synced
  folder that can land somewhere read-only). Use `fileURLToPath(import.meta.url)`
  (or pass the URL object straight to `fs`). Bit the first backup run, 2026-10-05.

- **`Date.now()` is frozen within a Worker event.** Two sends in quick
  succession got the same millisecond and came back out of order (random
  UUID tiebreak). Fix: `nextTs()` hands out strictly increasing timestamps.
  Caught by the paging test on day one.
- **`webSocketMessage` must return `void | Promise<void>`** — returning a
  helper's value from a `switch` breaks the `DurableObject` type.
- **Signals must be applied in order.** An ICE candidate processed while
  `setRemoteDescription(offer)` is still pending throws. Each `Peer` chains
  signal handling on a promise queue.
- **Timers are throttled in background tabs** (down to 1/min). Voice
  activation runs in an `AudioWorklet` (audio thread, not throttled) and
  gates the audio there too; the heartbeat compares against when the ping
  was actually sent, not wall-clock intervals.
- **A fixed voice-activity threshold near the room's noise is always open.**
  With fan noise at −52 dBFS a −50 dBFS gate sat open 59–99 % of the time,
  and hysteresis made it worse (closing 6 dB lower = inside the noise). The
  gate tracks a noise floor (quietest 10 ms per 500 ms, over 4 s) and opens
  12 dB / closes 8 dB above it when that's higher than your setting: 4 % open
  on noise alone, speech still gets through.
- **Grid gutter timestamps wrap** at small widths ("8:27 PM" in 38 px) and
  silently double every grouped message's height. `white-space: nowrap` +
  flex end-alignment.
- **`MediaStreamTrack.stop()` never fires `ended`** — only the browser ending
  it does (unplug, revoke). Tests simulate an unplug with `stop()` plus
  `dispatchEvent(new Event('ended'))`.
- **Peer state must be deleted on close**, not stored as `'closed'`, or the
  voice panel shows "Connecting…" forever after someone leaves.
- **Workers secrets aren't in `wrangler types` output** — they're declared
  by hand in `worker/env.d.ts` (both the global `Env` and `Cloudflare.Env`).
- **Moving an iframe in the DOM reloads it** (and restarts a video). Keep it
  in place and move its container with CSS instead.
- **YouTube embeds refuse to play without a Referer** (error 153). Yap's
  pages send `Referrer-Policy: no-referrer`, so the Watch iframe sets
  `referrerpolicy="strict-origin-when-cross-origin"` (YouTube gets the origin,
  which it already has from `origin=`).
- **Durable Object handlers interleave at an outgoing `fetch()`.** Read state,
  `await fetch(...)`, write state = a lost update when two messages arrive
  together (two quick `watch.queue` adds kept only one: found 2026-10-05,
  `onWatchQueue`/`onWatchSet` await the oEmbed title between reading and
  saving the state). Read the state again after the await.
