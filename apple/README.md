# Yap for iPhone, iPad and Mac

The native SwiftUI client for Yap: text channels, voice channels, custom
emoji, KLIPY GIFs, reactions, the waiting room, and peer-to-peer Opus voice
that talks to the web client. One multiplatform target builds a real iOS
app and a native macOS app from the same code. The wire protocol is
`../PROTOCOL.md`; the web client (`../web/js/`) is the reference.

## Layout

| Path | What |
|---|---|
| `Yap.xcodeproj` | One app target `Yap` (iphoneos + iphonesimulator + macosx), `YapTests`, and `YapBroadcast` (the iPhone/iPad screen-broadcast extension, embedded in the iOS app only). Schemes: **Yap** (Debug), **Yap DEV** (Dev sandbox), **Yap RELEASE**. |
| `Config/` | `Yap.xcconfig`: the team and bundle id behind every target and configuration, with neutral defaults. Your own values go in `Local.xcconfig` (git ignores it; copy `Local.example.xcconfig`). |
| `ExportOptions.example.plist` | Upload settings for TestFlight. Copy it to `ExportOptions.plist` (git ignores it). |
| `Yap/` | The app. Synced folder: new files join the target automatically. |
| `YapBroadcast/` | The ReplayKit broadcast upload extension (`SampleHandler`): iPhone and iPad screen sharing. Links only `YapScreenLink`. |
| `Yap/App/` | What the screens share: `AppModel`, `Theme`, `AppIdentity`, `Look` (one look; the status bar); `CallWatch` (a call ending off screen on iPhone: "You left Lounge"), `Notifier`, `BroadcastPicker` (iOS's screen-broadcast sheet); `ChannelController` (an open channel's behaviour: opening, jumps, edit/reply/staged picture, Send/Esc/↑, the channel menu's sheets), `SuggestionEngine`, `MessageMenu` and `ChannelMenuItems`, `InlineText` (message text), `GifSearch` / `EmojiBrowser` (the pickers' logic), `Prompts` (Ping everyone, Sign out others, copy), wording (`VoiceOptions`, `Stamps` for dates, `ColorNames`), `MediaViews` (looping GIF video, animated images), `SystemControls` (Paste, date pickers, `FlowLayout`). |
| `Yap/Views/`, `Yap/Components/` | The screens and their **Windows 98** building blocks. |
| `YapKit/` | Local Swift package, everything that isn't a view. |
| `YapKit/Sources/YapCore` | Protocol models and frames, `YapConnection` (WebSocket + heartbeat + backoff + network changes), `SendLine` (sends the server turns away), `YapClient` (identity and all the state screens read; `YapClient+Account/Events/Messages/People/Search/Vault/Extras.swift` by topic), `Mentions` (the server's mention rule), emoji codec and catalog, message formatter, KLIPY client, Keychain. No WebRTC. |
| `YapKit/Sources/YapVoice` | WebRTC mesh: `VoiceEngine` + `Peer` (perfect negotiation, Opus tuning, transmit gate, speaking levels, ICE restarts, link quality), `VoiceGate` (the transmit gate table), `VoiceHealth` (`PathWatch` ICE restart timing, `LinkQuality` "weak connection", `BoostLimiter`), `CallKeeper` (keeping calls across reconnects), `LevelMonitor` (the mic level while you're alone), `VoiceController` (join → own echo → offer, hints, the audio session, watching and sharing a screen), `MicCheck` (Hear yourself), `SyntheticAudioDevice` (fake mic for tests); screen sharing: `ScreenShare` (`ScreenQuality` presets, the `ScreenWatch` viewer states, `ScreenAudience` "Watching: …", H.264-first `ScreenCodecs`), `ScreenRenderer` (decoded frames → `AVSampleBufferDisplayLayer`), `ScreenFeeder` (+ `FrameRelay`, `SyntheticScreenFeeder`), `MacScreenFeeder` (ScreenCaptureKit + the system picker), `BroadcastScreenFeeder` (iPhone/iPad: the extension's pictures); `BackgroundCall` (the rules for a call off screen on iPhone). |
| `YapKit/Sources/YapScreenLink` | The extension ↔ app link: a Unix domain socket in the App Group's container, the framing (`ScreenLinkWire`), both ends, and `FrameScaler` (VideoToolbox). No WebRTC. |
| `Tools/` | `web-voice-peer.mjs` + `run-voice-interop.sh` (voice interop test), `seed-demo.mjs` (screenshot data), `make-icons.swift` (app icons, Win98 window + speech bubble). |

## Stack

- Swift 6, default MainActor isolation in the app (`YapKit` marks its own
  actors). `@Observable` everywhere, SwiftUI only; one look, Windows 98 /
  AIM, in light or dark (see One look).
- **WebRTC:** [`stasel/WebRTC`](https://github.com/stasel/WebRTC) 154.0.0
  (binary xcframework, iOS + macOS slices), pinned in `YapKit/Package.resolved`.
- No other dependencies. GIF videos use AVPlayer; animated WebP tiles use ImageIO.
- Deployment target: iOS 26.2 / macOS 26.2.

## Build and run

You need Xcode 27 and a Yap server to connect to (`../SELF_HOSTING.md` sets
one up; a local dev server works too, see Tests).

1. Copy `Config/Local.example.xcconfig` to `Config/Local.xcconfig`.
2. In it, set `YAP_TEAM` to your Apple Developer team ID (Xcode › Settings ›
   Accounts) and `YAP_BUNDLE_ID` to a bundle id you own, like
   `com.yourname.yap`. Git ignores this file.
3. Open `Yap.xcodeproj` and run the **Yap** scheme on an iPhone, iPad or
   "My Mac". Signing is automatic.

Without `Local.xcconfig` the app builds as `com.example.yap` with no team.
The simulator and the Mac still work (Xcode signs them locally); a real
iPhone needs a team.

Every bundle id comes from `YAP_BUNDLE_ID`:

| Scheme | Configuration | App | Tests |
|---|---|---|---|
| **Yap** | Debug | `<id>` | `<id>.tests` |
| **Yap DEV** | Dev | `<id>.dev` | `<id>.dev.tests` |
| **Yap RELEASE** | Release | `<id>` | `<id>.tests` |

**Yap DEV** installs beside the real app as `<id>.dev` ("Yap Dev", the
DEV-banded icon) with its own Keychain items, so testing never signs the
real app out. Nothing in the code names an identifier: `AppIdentity` derives
the Keychain service and the Dev flavor from the running bundle id, the logs
use it as their subsystem, and the Info.plist carries `YAP_BUNDLE_ID` as
`YapBundleIdentifier`.

From the command line (keep DerivedData outside the repo):

```sh
cd apple
# iOS simulator (use your own simulator's UDID)
xcodebuild build -project Yap.xcodeproj -scheme Yap \
  -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath /tmp/yap-dd
# macOS
xcodebuild build -project Yap.xcodeproj -scheme Yap -destination 'platform=macOS' \
  -derivedDataPath /tmp/yap-dd
# macOS, ad-hoc signed even when Local.xcconfig sets a team
xcodebuild build -project Yap.xcodeproj -scheme Yap -destination 'platform=macOS' \
  -derivedDataPath /tmp/yap-dd CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=
```

First run: paste an invite link, like `https://chat.example.com/#key=…`
(the `https://` is optional). There's no default server, so a bare key isn't
enough. Then "I'm new" (name + 6-digit PIN + color, then the waiting room)
or "I have an account". The device token and the invite key live in the
Keychain; name, color and read marks in UserDefaults.

**Optional server features.** Minecraft status and the board are settings
of each server. The app shows them only when the server has them: no
`minecraft` in `/api/config` means no Minecraft row or status panel (and
`/api/minecraft` is never asked), no board in Places (`ready.links`) means no
board. The board's page title is its link's name.

## TestFlight (optional)

The project is ready to upload: version 1.0, an opaque iOS icon (App Store
Connect refuses an alpha channel; `Tools/make-icons.swift` draws it opaque)
plus every Mac size, the microphone usage string, the `audio` background
mode, and a privacy manifest (`Yap/PrivacyInfo.xcprivacy`: no tracking;
name, messages, pictures and user id for app functionality; UserDefaults
`CA92.1`, boot time `35F9.1`). Both Info.plists set
`ITSAppUsesNonExemptEncryption` to NO: HTTPS and CryptoKit are the system's
encryption, and WebRTC's DTLS-SRTP is standard. The Personal space uses
AES-GCM end-to-end encryption, so check that answer for where you publish.

1. Set your team and bundle id in `Config/Local.xcconfig` (see above). The
   screen-broadcast extension signs with the same team (`<id>.broadcast`,
   `<id>.dev.broadcast` under Yap DEV) and shares the App Group
   `group.<id>` (`group.<id>.dev`) with the app (`YAP_APP_GROUP` in
   `Config/Yap.xcconfig`).
2. Copy `ExportOptions.example.plist` to `ExportOptions.plist` and put your
   team ID in `teamID`. Git ignores the copy. It says: `method`
   app-store-connect, `destination` upload, `signingStyle` automatic,
   `uploadSymbols` true.
3. Archive and upload:

```sh
cd apple
xcodebuild archive -project Yap.xcodeproj -scheme "Yap RELEASE" -destination 'generic/platform=iOS' \
  -archivePath /tmp/yap/Yap-iOS.xcarchive -derivedDataPath /tmp/yap/dd -skipMacroValidation \
  -skipPackagePluginValidation -allowProvisioningUpdates
xcodebuild -exportArchive -archivePath /tmp/yap/Yap-iOS.xcarchive -exportPath /tmp/yap/export \
  -exportOptionsPlist ExportOptions.plist -allowProvisioningUpdates
```

Or use Product › Archive › Distribute in Xcode. The Mac archives the same
way with `-destination 'generic/platform=macOS'`; a Mac TestFlight build is
a second upload of that archive. Every new upload needs a higher build
number (`CURRENT_PROJECT_VERSION`).

The account holder does these once, in this order (the upload stops at the
first one missing):
1. Xcode › Settings › Accounts: sign in. Command-line exports need a login
   that has an Xcode token.
2. The App Group, for iPhone/iPad screen sharing: automatic signing
   registers `<id>.broadcast` and the `group.<id>` App Group on the first
   archive with `-allowProvisioningUpdates` (or when you open the project in
   Xcode with your team set). If the archive says a profile doesn't support
   App Groups, open Certificates, Identifiers & Profiles › Identifiers, make
   sure the App Group `group.<id>` exists, and turn on App Groups with it for
   both `<id>` and `<id>.broadcast`; then archive again. The extension's
   build number must match the app's: bump them together (every
   `CURRENT_PROJECT_VERSION = N;` in the project).
3. App Store Connect › Apps › +: create the app with your bundle id, a SKU
   (e.g. `yap-ios`) and a name that's free on the store. The bundle id gets
   registered on the first signed export if it isn't already. Add macOS to
   the same record for a Mac build.
4. Agreements, Tax and Banking: the Free Apps agreement must be active.
5. App Privacy (the store's privacy labels): match `PrivacyInfo.xcprivacy`
   and your server's `/privacy`. The extension collects nothing itself
   (`YapBroadcast/PrivacyInfo.xcprivacy`): the screen goes to the app on the
   same device, and from there only to the people in your call who ask.
6. TestFlight › testers: internal testers right away. External testers
   need Beta App Review, which needs a way in: Yap is invite-only, so put an
   invite link and a test account with its PIN in the review notes.

## Tests

Start a local test server first, from the repo root. Any free port works;
the tests default to 8789. `test/wrangler.test.jsonc` is the server's test
config: a stand-in board (`test/fixtures/board`) and a Minecraft address
that never answers.

```sh
npx wrangler dev -c test/wrangler.test.jsonc -c test/fixtures/board/wrangler.toml --port 8789 \
  --var YAP_KEY:testkey --var KLIPY_KEY:test --var ADMIN_KEY:testadmin \
  --var PIN_PEPPER:testpepper --persist-to /tmp/yap-test-db
```

**Protocol client (macOS, ~3 s):** unit tests plus integration tests against
the dev server (they skip themselves without `YAP_TEST_URL`):

```sh
cd apple/YapKit
YAP_TEST_URL=http://127.0.0.1:8789 YAP_TEST_KEY=testkey YAP_TEST_ADMIN=testadmin \
  swift test --scratch-path /tmp/yapkit-build
```

They cover: signup → `pending` (with `uid` + token) → admin approval →
`ready`; login on a second "device"; token reconnect; sign-out revoking only
that device; friendly `bad_login` / `name_taken` / `bad_pin` / `bad_token`;
send/receive text with formatting and `:shortcode:` conversion; typing;
unread; history paging; delete; custom emoji token encode/decode and jumbo;
GIF messages (and the server refusing non-KLIPY URLs); reactions; @mentions;
pings; avatar upload/remove; the waiting room seen by members (let in, turn
away → blocked); KLIPY request shapes against a fake KLIPY; Opus SDP tuning;
invite links (https assumed without a scheme; a bare key or a server
without a key is refused); `/api/config` without the optional settings.
Sessions (`SessionTests`): signing out here and being signed out from another
device (`bad_token`) both leave nothing behind; a new invite key for the same
server keeps the device; a delete leaves pins, search and the shelf.
Mentions (`MentionsTests`): the web's own examples from `test/unit.test.mjs`
(custom emoji tokens included), for who gets pinged and who the formatter
highlights. The web's catch-up and waiting-room polish (`WebPolishTests`,
`WebPolishIntegrationTests`): `unread.counts` mentions (nil from older
servers) seeding the badges, Catch up's order and Mark all read, a device
that never read anything starting caught up, a reused link's `usedBy` and
its sentence, the session-only "joined" line and "You're in", and against
the dev server mentions-while-away via a second device and a link used twice.
Since v0.13: editing (own only, `editedAt` in history), pictures end to end
and the upload token (no `ut`, a wrong one or someone else's → refused, for
`/api/image` and `/api/avatar`; emoji upload with it), personal invites
(one-time link, `invitedBy` in the waiting room, the newcomer adopting the
main key from `ready`, still getting in after the invite is turned off),
server Places add/remove + the mod log, shared voice quality, OBS listeners
not counting as online, skin tones, and the picture encoder (compressed ≤ 1600 px by
default, Full res keeps the original or the sharpest copy that fits, HEIC →
JPEG, small GIFs as they are, size labels). Search and pins: the query parser (words, `from:`,
`has:`), `search.results` / `pinned` / `pins` frames, encrypted notes
searched on the device, live search against the server (words, `from:`,
`has:link`), pins live and listed, and `ensureLoaded` paging back to an old
message. v0.36 extras (`ExtrasTests`, `ExtrasIntegrationTests`): every new
frame both ways, spoilers, YouTube ids, `/poll`, quiet hours (the server's
rule), the chime table and synth, the soundboard gain cap, WAV round trip /
trim / resample, `SoundClip` decoding an AAC voice-memo-like file, doodle
GIFs (pixels survive LZW), and against the dev server: polls, dice, away +
auto-reply, soundboard + chimes, doodles, the shelf, catch-up counts, save
to Personal, quiet hours across devices, hangs (start the server with
`--var TEST_HANGS:1` for `hang.now`) and the Watch queue.

**Voice interop with the web client (iOS simulator, ~1 min):** the real web
client in headless Chromium (`Tools/web-voice-peer.mjs`, needs the repo's
`node_modules`) and the native engine on a simulator, with a synthetic mic on
both sides (the web's `getUserMedia` and `getDisplayMedia` are replaced by
WebAudio and canvas streams, so no device is opened and the OS never asks for
the microphone or the screen), so no hardware or permission prompts are involved:

```sh
apple/Tools/run-voice-interop.sh <simulator-udid>   # YAP_URL / YAP_KEY / YAP_ADMIN_KEY default to the values above
```

Four scenarios: native joins second and **offers**; native sits first and
**answers** the web's offer; the web **shares** a screen and the native
watches; the native **shares** and the web watches. Each must reach `connected` with Opus flowing
both ways at ~128 kbps, the tuned fmtp on both sides
(`maxaveragebitrate=128000;usedtx=0;useinbandfec=1;stereo=1…`), speaking
levels, mute showing on the web side with silence, and voice activation
opening on speech and closing on silence. Alone in the channel, the mic meter
still reads (the loopback monitor) and voice activity opens at the default
−50 dBFS. In the second scenario the native test drops its socket mid-call,
then the web drops its own: both times the same connection must carry on
under the new session ids, with no gap over 300 ms in either direction
(measured: ~80 ms, the sampling interval) and no chimes on either side.
Screens: the web shares through its own `voice.startScreen` (a moving canvas
and a 440 Hz tone as its sound); the native client, outside the channel,
uses Watch screen (it joins Lounge), and must decode H.264 constrained
baseline with packetization-mode=1, play it (`ScreenWatch` reaches playing)
and receive the share's sound on its second audio track; Stop watching must
make the web stop sending. Then the native shares a `SyntheticScreenFeeder`:
nothing goes out until the web clicks the real LIVE badge, "Watching:
<web user>" shows, the web decodes H.264 (VideoToolbox encodes it), Sharp
applies without stopping, and the web's Stop watching takes it off the list
and stops our encoding for it (measured: first frames ~1.2 s after asking).
`YapVoiceTests` also cover the transmit gate's table (hysteresis, the
450 ms hold, the noise floor, the push-to-talk tail, digital silence), the
reconnect state machine (`CallKeeperTests`: a fake mesh and a hand-cranked
clock), ICE restart timing, the "weak connection" thresholds, the boost
limiter, Hear yourself's gate and Opus round trip, Opus tuning of every
audio section and transport-cc leaving audio m-lines (video keeps it), and
screen sharing (`ScreenShareTests`: the viewer's states on a hand-cranked
clock — connecting, playing, paused after 3 s without a frame, stopped,
closed 2 s later — "Watching: …", the presets, H.264 first, the renderer
counting frames and converting software-decoded I420; on the iOS simulator,
`ScreenLoopbackTests` runs a real share between two engines in one process:
want → H.264 decoded → stop → nothing more encoded → want again on the same
transceiver → the sharer stops).

Round 4 added: `SendLineTests` (a message the server turns away: which send
an `error {re: "send"}` is about, `slow_down` re-sends with the same nonce
1.1 s apart up to 5 times, everything behind going one at a time in the
written order, other errors dropping the copy and returning its text; with
the dev server, twelve at once plus two more all arriving in order),
`HeartbeatTests` (every ping has its own deadline; extra pings never drop a
healthy socket; frames answered in the order sent), `BackgroundCallTests`
(a call off screen on iPhone, on a hand-cranked clock), `AudioAnchorTests`
(iOS simulator: the call's audio device never goes quiet while people come
and go), `BroadcastFeederTests` and `YapScreenLinkTests` (the extension ↔ app
link: framing, sizes, rotation, the scaler, a real socket both ways, either
side hanging up). ReplayKit itself can't run in a test.

**App target:** `xcodebuild test -project Yap.xcodeproj -scheme Yap -destination 'platform=iOS Simulator,id=<UDID>'`
(identity, bundled font, a stored Apple/Arch skin loading as Windows 98 with light/dark kept, the status bar, edit drafts, skin tone,
color names, away-message ends, composer suggestions, the web's words for the sign-on hint,
the wrong-PIN help, the join bars, the questions, screen sharing's words, and who gets the voice
roster's popover and which owner/mod actions it offers). It imports YapCore `@testable`
for `loadDemo` / `receiveForTesting`.

## Screenshots and DEBUG launch switches

Debug builds read a few environment variables so the simulator can open on a
given screen against a dev server, with no synthetic taps (pass them with the
`SIMCTL_CHILD_` prefix to `xcrun simctl launch`). Credentials stay in memory;
the Keychain isn't touched. See `Yap/App/DebugLaunch.swift`.

```sh
node apple/Tools/seed-demo.mjs --stay 60       # Sam / Cleo / Ada / Ben, PIN 582039, a short chat
SIMCTL_CHILD_YAP_INVITE='http://127.0.0.1:8789/#key=testkey' SIMCTL_CHILD_YAP_LOGIN='Sam:582039' \
SIMCTL_CHILD_YAP_SCREEN='channel:general' xcrun simctl launch <UDID> <your bundle id>
xcrun simctl io <UDID> screenshot chat.png
```

`YAP_SCREEN` = `channel:<id>`, `members`, `settings`, `invite` (Settings ›
Invite), `voice`, `gifs`, `emoji`, `staged` (general with a picture waiting
in the tray), `board`, `draw`, `space`, `voicepanel:<channel>` (the call sheet without
joining — the Mac can't use the synthetic mic). `YAP_DEMO_INVITE=label` makes
a personal invite once online, to show the one-time link. The seed also posts
a picture, edits a message and makes two invite links (one turned off).
`YAP_SCREEN=members` opens People; `YAP_VAULT_PASSCODE=…` unlocks the
encrypted Personal space once online (the seed's passcode is "purple tiger
river"); `YAP_DEMO_REPLY=text` starts a reply to the newest message
containing it. `YAP_SIGNUP=Name:PIN` lands in the waiting room;
`YAP_SYNTHETIC_AUDIO=1` + `YAP_AUTOJOIN=lounge` joins voice with the fake
mic; `YAP_FAKE_KLIPY=1` fills the GIF picker without a key;
`YAP_SEARCH=words` opens search from the open channel (everywhere) with
those words; `YAP_SHOW_PINS=1` opens the open channel's pins; `YAP_JUMP=text`
jumps to the newest message containing it and flashes it;
`YAP_APPEARANCE=charcoal|standard|system` (stored, so later launches keep it);
`YAP_WATCH=Name` watches Name's screen once they share (with
`YAP_SYNTHETIC_AUDIO=1`; `YAP_SCREEN_FULL=1` then goes full screen);
`YAP_PERSON=Name` opens Name's popover under the voice channel;
`seed-demo.mjs --voice --screen` makes Ada say she's sharing (a LIVE badge,
no video). Voice screenshots without a call:
`YAP_DEMO_VOICE=reconnecting:Ada,weak:Ben,relay:Cleo,cantconnect:Name,resuming,silent`
(the notes next to people, "Reconnecting… voice is still on", "Your mic is
sending silence"; with `voicepanel:lounge` and `seed-demo.mjs --stay 600
--voice`, which sits Cleo, Ada and Ben in Lounge) and
`YAP_DEMO_MICCHECK=recording|playing|done|failed|run` (Settings › Voice ›
Hear yourself; `run` runs it, silently with `YAP_SYNTHETIC_AUDIO=1`). The v0.36 extras: `YAP_SCREEN=switcher` (with
`YAP_SWITCHER=text` typed in), `catchup`, `hang` (Plan a hang), `doodle` (a
doodle for general's newest message); `YAP_SETTINGS_PAGE=look|profile|voice|notifications|account|mods|invite|about`
with `YAP_SCREEN=settings` (on iPhone the page sits on top of Settings' list;
without it you get the list); `YAP_SHOW_SHELF=1` / `YAP_NEW_POLL=1` on the open
channel; `YAP_LAST_READ=0` (or `general:0,games:1791…`) gives the in-memory store read
marks to start from — a scripted launch has none, so it would start caught up;
`0` makes everything unread, as before; `YAP_DEMO_EDIT=text` starts editing my
newest message containing it; `YAP_ASK=ping|archive` asks the open channel's
question in its Win98 box; `YAP_SETTINGS_PAGE=voice-advanced` opens Voice ›
Advanced (its page on iPhone, unfolded on the Mac); `YAP_SNAPSHOT_HEIGHT=900`
makes the Mac's Settings window that tall before the snapshot;
`YAP_UNREAD_LAST=N` pretends the open channel's last N messages are
new (the "New since" line, and the Jump to first unread pill when it's off
screen); `YAP_DRAFT=text` starts the open channel's composer with that text
(`\n` for a new line); `YAP_DEMO_PINRESET=Name` resets that (throwaway, local)
account's PIN once online, to show the temporary PIN;
`YAP_ORIENTATION=landscape` turns the iPhone app to landscape once it's up; `YAP_USE_PASSWORD=1` starts the sign-on form on "Use a password";
`YAP_DEMO_CALL_ENDED=soundTaken|soundFailed|stopped|offline|channelGone` (with `YAP_AUTOJOIN` and
`YAP_SYNTHETIC_AUDIO=1`) ends the call as if that had happened off screen; `YAP_DEMO_BURST=N` sends N
numbered messages to the open channel at once; `YAP_DEMO_SHARE=1` shares a moving test card once in the
call (the sharing UI on iPhone without the extension). The seed also posts a spoiler, a YouTube link, a poll with votes,
`/roll` and `/flip`, three soundboard clips, Ada's `tada` chime, Cleo's away
message, a hang in three hours and a doodle reaction. On the Mac, `YAP_SNAPSHOT=name` saves each
window after 8 s and quits — ScreenCaptureKit's current-process capture (no
screen-recording permission; includes materials and glass), AppKit's
`cacheDisplay` as the fallback — to the app's temporary directory; add
`YAP_SCREEN=settings` for Settings. The sandboxed build's container can be
off-limits to a terminal (macOS app-data protection), so for scripted
snapshots build an unsandboxed copy with its own bundle id and run its
binary directly; the PNGs land in `$TMPDIR`:

```sh
xcodebuild build -project Yap.xcodeproj -scheme Yap -destination 'platform=macOS' \
  -derivedDataPath /tmp/yap-snap CODE_SIGN_ENTITLEMENTS= ENABLE_APP_SANDBOX=NO \
  ENABLE_HARDENED_RUNTIME=NO PRODUCT_BUNDLE_IDENTIFIER=com.example.yap.snapshot \
  CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=
YAP_INVITE='http://127.0.0.1:8789/#key=testkey' YAP_LOGIN='Sam:582039' YAP_APPEARANCE=standard \
  YAP_SNAPSHOT=mac /tmp/yap-snap/Build/Products/Debug/Yap.app/Contents/MacOS/Yap
```

## Since v0.13 (web parity)

Windows 98 keeps its look (new bits use its own components).

- **Edit your own messages:** Edit in the message's menu, or ↑ in an empty
  composer on the Mac. The text comes into the composer (custom emoji as
  `:name:`); Send saves (`edit`), Cancel/Esc puts your draft back. Edited
  messages say "(edited)" (`editedAt`, live via `edited`).
- **Pictures:** Photos on iPhone; a file, ⌘V or the picture button on the
  Mac. Nothing goes out when picked: a GIF or picture waits in a tray above
  the composer (thumbnail, × or Esc to drop) and what you type is its
  caption. Meanwhile `PictureEncoder` prepares it in the background, like the
  web's `preparePicture` (v0.34): **compressed** by default — upright JPEG
  ≤ 1600 px at ~0.78, stepping to 1280 / 1024 px (a phone photo ≈ 150–300
  KB; an original that's already smaller goes as is) — or **Full res** (the
  tray's switch): the original PNG/JPEG/WebP when it fits 1.9 MB, else the
  sharpest JPEG that does (full size at 0.92, then 3072 / 2048 / 1600 px).
  HEIC always becomes JPEG (browsers can't show it); GIFs that fit stay as
  they are. The tray says the size ("47 KB, compressed"). No WebP: ImageIO
  can't write it. Upload = `POST /api/image?sid=&ut=`
  then `send … image`. Shown ≤ 360 pt in a sunken frame, downsampled in
  memory; tap for full size (pinch or double-click / double-tap to zoom, up
  to 4×).
- **Upload token:** every upload (`/api/image`, `/api/avatar`, `/api/emoji`,
  `/api/sound`, `/api/doodle`, `/api/blob`) sends `ut` = `ready.uploadToken`
  next to `sid` (`YapClient.uploadTarget()`).
- **Invites:** `ready.key` (the main key) replaces whatever key we came in
  with, so a revoked personal invite never locks anyone out. Settings ›
  Invite makes a personal link (`invite.create`; shown once, Copy / Share),
  lists yours (everyone's for mods) with Turn Off. Only the owner and mods
  see the server's main link (with the web's warning about sharing it);
  everyone else invites with personal links, as on the web. A new invite
  link for the same server swaps the key and keeps you signed in.
  Waiting-room rows say whose invite, and when the same personal link
  already brought someone else in (`invite.usedBy`, PROTOCOL.md › Waiting
  room) they say so: "This link was also used by Dee — check with Ada."
  (People and the join bars above each text channel's chat.)
- **Join bars** (the web's `renderRequests`, `JoinRequests`): one bar per
  person waiting — "Dee wants to join · invited by Ada", the passed-on-link
  warning, Let in / Turn away — until they'd crowd the chat (iPhone: from
  two, or one during a call; Mac: three, or two), then one "3 people want
  to join" bar with Review (People), tooltip yellow with a black edge.
- **Let in:** when someone is let in, the first text channel gets a quiet
  line for this session only — "Cleo joined — invited by Ada" (or "let in
  by you") — instead of a popup (`YapClient.localNotes`, placed among the
  messages by time by `MessageRows`); someone turned away is still a notice
  for everyone else. The newcomer's own first look says "You're in. Say hi
  👋", with the mention chime.
- **Places** come from the server (`ready.links`, live `links`; a malformed
  link is skipped, not the whole list): the board opens inside, other links
  in the browser. "New place…" adds an https link; a row's menu removes it.
- **Shared voice quality:** under the call sheet's ⋯ (next to Leave), "Each person's own
  (yours: 128k)" or "N k for everyone" sets `channel.bitrate`. The call only
  says "Sound: 256k for everyone" when someone has set one for the channel.
  In a call the engine uses the channel's bitrate, else yours
  (`VoiceController.callBitrate`), and renegotiates when it changes.
- **OBS listeners** (`client: "obs"`) show in voice as "📺 Name's OBS",
  secondary, no actions, and don't make that person online.
- **Owner & mods:** an Activity list (`modlog`, same wording as the web).
- **Skin tones:** a tone row in both emoji pickers (`yap.emojiTone`, 0–5);
  picks, suggestions, reactions and typed `:shortcodes:` use it.
- **Personal space** (v0.20, `kind: "personal"`, id `p_<uid>`): the first
  row of the channel list (a lock). Works like a text
  channel (send, history, edit, delete, react, pictures with preview) but
  only your own devices get it. Its start says what it's for and warns it
  isn't encrypted; the header says "only you · not for private info"; no
  typing, no ping/archive menu. Your own message without your nonce (sent
  from another device) counts as unread. iPhone: the system Paste button in
  its composer drops a copied picture in the tray or text into the field —
  no share sheet, no "Allow Paste?" prompt. `YAP_SCREEN=personal` opens it.
- **Nameplate at the top:** your profile card, Mute / Deafen and the call
  card sit right under the header (above the channels pane on the Mac,
  under the title bar on iPhone). On the iPhone the tab bar's call accessory hides on the channel
  list itself, where the card already is.

## v0.21: no presence, replies, encrypted Personal space

- **No online/offline anywhere** (PROTOCOL.md › Presence): no dots, counts,
  "last seen" or Online/Offline groups, no member column and no People tab.
  A **People** button (iPhone: the channel list's toolbar; Mac: the window
  toolbar / the Win98 chat pane's title bar) opens a sheet with the waiting
  room (Let in / Turn away) and everyone A–Z with role badges and who's in
  voice; its badge counts people waiting. The server only shows sessions in
  voice or on a shared view (`session.join` → `voice`/`view`, then
  `voice`/`view` → `session.leave`); `YapClient.members` / `voiceChannel(of:)`.
  iPhone is one navigation stack now; during a call the call bar sits at the
  top of the channel list's nameplate and of every chat or place.
- **Replies:** "Reply" in a message's menu; "Replying to Name ×" above the
  composer (Esc cancels on the Mac); `send … replyTo`. A reply shows
  "↪ Name: first line…" above it (or "Original message was deleted"); tap it
  to scroll to the original when it's loaded. A reply to you is a mention.
- **Mentions** follow the server's rule (`Mentions`, ported from
  `web/js/mentions.js`): a member's name in any case, longest names first and
  each mention used once (so "@Sam Lee" never also pings Sam), not right
  after a letter, digit, `_` or `@`, never inside code, links or custom
  emoji tokens — which count as words, so "<:wave:…>@Ben" or "`x`@Ben"
  pings nobody. The highlight, the mention badge and the server's push agree.
  None in the Personal space. (No swipe-to-reply: the chat is a plain
  scroll view, where a horizontal drag fights scrolling.)
- **40,000-character messages**; over 3,000 characters or 30 lines they start
  collapsed behind Show more / Show less (`MessageLength`).
- **Deletes are erased server-side.** Deleting an encrypted note with a
  picture also sends `blobs: [id]` (`YapClient.delete(_:)`).
- **Encrypted Personal space** (`Vault`, byte-compatible with
  `web/js/vault.js`, checked both ways against it in Node by `VaultTests`):
  CryptoKit AES-GCM, CommonCrypto PBKDF2-SHA256 (600k when made here), NFKC
  passcodes. The space key lives in the Keychain (`vault.<uid>`, this device
  only, after first unlock) tagged with `wrap.ct`; a different `ready.vault`
  — or a live `vault` change while unlocked — forgets it and asks again.
  Sheets: set up (≥ 8 characters, repeated; digits only need 12+), unlock
  (with "Forgot it? Start over…"), change passcode; Lock in the space's
  menu and Settings › Profile › Personal space. Notes show decrypted; text,
  GIF and picture notes are sealed (pictures via `/api/blob`, opened on the
  device); edits re-seal and keep the GIF/picture. Locked: the composer is
  disabled ("Unlock your Personal space to write") with a locked card.
  Header: "end-to-end encrypted" / "locked" / "only you · not encrypted".
- **Sealing earlier notes:** turning encryption on (or starting over) asks
  for `personal.plain`, seals each note (re-uploading its picture through
  `/api/blob`) and hands them back with `personal.seal` (≤ 50 notes and
  ≤ 300 KB a frame); a note counts as sealed once its `edited {sealed}`
  comes back (within 20 s), which also drops the plain GIF/picture, and
  `editedAt` 0 isn't an edit.
- **When's good** (Places) and **Privacy** (Settings › About) open
  `/#key=…&place=when` and `/privacy` in the browser.
- **Draw** and **Space** (Places) are the web app in an in-app web view at
  `/#key=…&place=draw|space`. The web keeps its own sign-in, so every web
  open (these two, Watch, When's good, a LIVE badge) first asks for a
  one-time pass (`device.handoff`, waiting up to 8 s for the connection)
  and adds `&handoff=…&name=…`; a web app that's already signed in ignores
  it. Without the pass it asked for name + PIN and landed on chat.
- **No "online" light of your own:** the connection only shows while it's
  connecting or reconnecting; the nameplate says "Profile & settings".
- **Accounts:** sign-in and Change PIN take a password (10–128 characters)
  via "Use a password" (text keyboard instead of the number pad); the
  server's `weak_pin` reason is shown as is. `ready.weakPin` puts a gentle
  "Your PIN is easy to guess — change it" bar above the nameplate;
  `signin.alert` puts "New sign-in: <device>, just now. Not you? Sign Out
  Everywhere Else" there. Settings › Account lists your devices
  (`account.devices`) with Sign Out (`account.revoke`) and Sign Out
  Everywhere Else (`account.signoutOthers`, after a confirmation). Signing
  out forgets the Personal space key on this device. Signed out from
  another device (the token stops working, `bad_token`): the same clean
  slate as signing out here, and the sign-in screen says why. Test/seed PIN: 582039 (424242 is now
  refused as weak).

## Sending, and messages the server turns away

A message shows dimmed until its echo (my nonce) comes back; one written
while offline goes when the socket is back, in the order written. The
server answers sends in order, so an `error {re: "send"}` is about my
oldest send not answered yet (`SendLine`; nothing on the wire). `slow_down`
(more than 10 in a burst, refilled one a second): the same message goes
again with the same nonce 1.1 s later, up to 5 times, and everything behind
it goes one at a time, 1.1 s apart, until nothing waits, so the order is
kept; no notice unless it finally gives up. Any other refusal drops the
stuck copy, shows the reason, and puts the text back in the message box if
that's empty and you're still on that channel. The socket asks the server
at once when the network changes (Wi-Fi ↔ cellular), and every ping has its
own deadline (`YapConnection`).

## Search and pins

- **Search** (`search` → `search.results`, newest first): a Find window
  (menu › Find…, ⌘F on the Mac and on an iPad with a keyboard) with In
  <channel> / Everywhere; it searches as you type (after 600 ms), on Find /
  Return, and again after a reconnect if you typed while offline. Words
  plus `from:Name` and `has:image|gif|link`
  (`SearchQuery` in YapCore parses them the same way the server does).
  Results show channel, author, time and an excerpt with the matches marked;
  tap one to jump. ⇧⌘F (the web's shortcut; Go › Search messages…) opens
  it on Everywhere from anywhere, like the switcher's last row.
- **Encrypted notes** never reach the server's search: while the Personal
  space is unlocked, `runSearch` also matches the decrypted notes on the
  device and merges them in.
- **Jumping** (search results, pins, reply quotes): `model.jump(to:)` opens
  the channel; the channel view calls `ensureLoaded`, which pages back (up to
  20 older pages) until the message is in the history, then scrolls to it and
  flashes it. The lazy list only knows where the rows it has laid out are and
  guesses the rest; from the bottom of a channel with tall rows (polls, dice,
  long messages) the guess for the rows just above them lands past the end,
  so `scrollTo` doesn't move. When the row isn't on screen after the first
  scroll (the rows report it, `reportsOnScreen`), the controller
  walks there a screenful at a time, each step to the message just past the
  screen's edge, which is laid out; then the last scroll is exact.
- **Pins** (`pin` / `pinned` / `pins`, `Message.pinnedAt`): Pin / Unpin in
  every message menu, a 📌 marker on the message, and Pinned Messages from
  the channel's menu — tap to jump, Unpin on each row. `pinned` updates live; a pins
  list asked for while offline is fetched again on `ready`.

## Screen sharing (watch and share on iPhone, iPad and Mac)

PROTOCOL.md › Screen sharing, unchanged on the wire: a sharer's
`voice.state {screen: true}` puts a red **LIVE** badge next to them in
voice (sidebar and call sheet); a viewer sends `signal {screen: "want"}` /
`"stop"`; the sharer adds or removes a video track on that one peer
connection (perfect negotiation renegotiates). The web's words throughout.

- **Watching** (every device): the LIVE badge, or **Watch screen** in the
  person's popover (below), opens their screen big in the main area
  (`Destination.screen`; joins their voice channel first if you aren't in
  it). The picture is fit to the window — letterboxed, never cropped or
  stretched (`ScreenRenderer`: decoded frames into an
  `AVSampleBufferDisplayLayer`, `.resizeAspect`). The bar has **Pop out**
  (picture-in-picture from the same layer; on iPhone it also starts by
  itself when you leave the app while watching), **Full screen** (or
  double-click / double-tap the picture; Mac: the window goes full screen and
  the channels pane steps aside; iPhone: no bars, turn the phone),
  **Stop watching**, and a volume slider for the share's sound (0–100 %,
  remembered per person on this device, `VoiceSettings.screenVolumes`,
  separate from their voice volume). Over the picture: `Connecting to Ana's
  screen…` until the first frame, `Ana's screen is paused.` after 3 s without
  one, `Ana stopped sharing.` when they stop or leave, and then the viewer
  closes itself 2 s later (`ScreenWatch`). Leaving the viewer stops watching
  unless it's popped out. One share at a time, like the web. A reconnecting
  sharer is followed to their new session (`onRekey`); after our own
  reconnect the want is sent again.
- **Sharing** (Mac): **Share screen** (the display button next to Mute in
  the call bar, or the call sheet's Screen group) opens the system picker
  (`SCContentSharingPicker`: a window, an app or a display; Yap itself is
  left out) and ScreenCaptureKit captures at the quality's size and frame
  rate (NV12, cursor shown). **Smooth / Sharp / Max** are the web's
  `SCREEN_QUALITY` numbers (1080p60 8 Mbps, 1440p30 6 Mbps, 4K60 25 Mbps,
  per viewer, with frame rate / resolution / balanced as what gives first)
  and change while sharing, without stopping. **Share something else**
  opens the picker for the running stream (same track, new content).
  **Stop sharing** (or the system's own stop) ends it; leaving voice does
  too. A strip under the call bar says "Sharing · Smooth" and **Watching:
  Ana, Ben** / **No one's watching yet** (from the wants and stops received;
  someone whose connection goes is dropped). Hardware H.264 (VideoToolbox,
  stasel/WebRTC's default factories); a viewer gets its own sender only after
  asking, nothing is encoded for a viewer who stopped and nothing at all
  while nobody watches (`FrameRelay` stops feeding the source), the newest
  picture is sent again at once for a new viewer and every second while the
  screen holds still (ScreenCaptureKit only delivers changes), so a picture
  arrives within about a second. **No Share sound:** stasel/WebRTC's SDK
  sends the microphone on every audio track and can't feed one from other
  audio, so the share's sound can't ride on its own track from the app (see
  Known gaps).
- **Sharing** (iPhone and iPad): **Share screen** in the call sheet's
  Screen group opens iOS's "Screen Broadcast" sheet with Yap selected
  (`RPSystemBroadcastPickerView`, `BroadcastPicker`; starting it from
  Control Center works too, while you're in a call). iOS runs the
  `YapBroadcast` extension and shows the red pill; the extension scales each
  picture to the quality (`FrameScaler`, VideoToolbox; a phone held upright
  at Smooth is 884×1920, its long side to 1920) and hands it to the app over
  a Unix domain socket in the App Group's container (`YapScreenLink`: a
  12-byte header, then tightly packed NV12 planes). The app feeds it into
  the same screen source as the Mac (`BroadcastScreenFeeder` → `FrameRelay`),
  so Watching: …, Smooth / Sharp / Max, the still picture every second and
  "nothing encoded while nobody watches" all behave the same. The app tells
  the extension the quality and the frame rate (the quality's while someone
  watches, one picture a second while nobody does, so a new viewer gets one
  at once); one picture is in flight at a time, the rest are skipped, so the
  extension stays far under its 50 MB (it doesn't link WebRTC; ~140 KB).
  A turned phone's pictures carry their rotation (WebRTC turns them, or
  tells the far side to). **Stop sharing** in Yap (or leaving the call)
  closes the link and the extension ends the broadcast ("Yap stopped sharing
  your screen."); stopping from Control Center or the red pill closes it
  from the other side and the share stops in Yap. The app listens only
  while it's in a call, so a broadcast started outside one says "Join a voice
  channel in Yap first, then share your screen from the call." Picture
  only: no app sound (stasel/WebRTC can't put it on its own track) and
  never the call. iOS shares the whole screen, notifications included.
- **Codecs:** H.264 constrained baseline, packetization-mode=1 is put first
  on every screen transceiver, sending and answering (`ScreenCodecs`);
  VP8/VP9/AV1 stay as fallbacks. A web sharer on Sharp (VP9 first) still
  sends H.264 to the app, since the app's answer asks for it first.
- **The share's sound** from a web sharer arrives on a later audio section,
  tuned as music (`SDPTuner`, ≥ 160 kbps, stereo) and played at its own
  volume; deafen silences it too.

## In the call: someone's volume and actions

Click (Mac), tap (iPhone, iPad), right-click / Control-click (Mac) or
long-press someone under a voice channel — or in the call sheet — for a
small popover (`VoicePersonMenu`): their name and role, their away note,
**Volume** 0–200 % in steps of 5 ("Past 100% boosts quiet people", the same
setting as People › person), **Watch screen** while they share, and what the
web offers owner and mods: **Make mod / Remove mod** (owner), **Reset PIN**
and **Ban** (mods, not on the owner; asked first in the usual box). Your own
row and OBS listeners have none. SwiftUI has no right-click gesture, so one
local event monitor hands a right-click to the row under the pointer
(`RightClicks`).

## Watch together (on the web)

- **Watch together** (PROTOCOL.md › Watch together): a **Watch** row in
  Places opens `<server>/#key=…&place=watch` in the browser. Under it: "N
  watching", "▶ <title>" and the audience (sessions with `watching`).
  `watching` updates live; the first time anyone is in the audience the app
  sends `watch.get` once (nothing to ask for before), and `watch.state`
  broadcasts keep the title current.

## One look

Windows 98 / AIM only (see Design), in **Charcoal** (dark, the default),
**Windows Standard** (light) or following the system: Settings › Look ›
Theme (`yap.appearance`). Earlier versions also had an Apple and an Arch
skin, stored per device as `yap.skin`; `Look.settle()` forgets that at
launch, so a device that used one opens in Windows 98 with its light/dark
choice kept (LookTests). The look follows Dynamic Type up to the
accessibility sizes and small controls keep ≥ 44 pt hit targets on iPhone.
The sidebar's footer names the server's version (`/changelog.json` ›
`current`); the app's own version is in Settings › About. Before the
Apple and Arch skins went, what only they had came to Windows 98: zoom in
the full-size picture, ⇧⌘F opening Find on Everywhere, Find searching as
you type and again after a reconnect, ⌘F on an iPad keyboard, Return
pressing the ✓ in Mac sheets, New poll asking before throwing away typed
answers, the profile name saving when you leave the field, the waiting
room's "Anyone already in Yap can let people in" line, and OBS listeners
not counting as company in the call sheet.

- **Status bar (iOS):** always white (every screen has a navy title bar
  or the teal desktop on top). The iOS 27 SDK ignores run-time changes to
  the app-wide style, so `StatusBarStyler` (in `Look.swift`) gives the
  window's root hosting controller class its own `preferredStatusBarStyle`
  while it's the root. Don't wrap the root in a container controller
  instead: that detaches SwiftUI's view from its scene and `scenePhase`
  never reaches `.active` (the app would think it's in the background: no
  read marks, mention notifications while open).
- **In-app web pages** (Draw, Space, the board) get the app's light/dark
  (`yap.settings.theme`) before the web app paints; the web has one look too.
- **Settings:** the web's pages in the web's order — Look ·
  Profile · Voice · Notifications · Account · Owner & mods · Invite ·
  About (`SettingsPage`). Mac: one tab each. iPhone: a list of the pages,
  each pushed (`AppModel.showSettings(_:)`); Account shows five devices,
  then "Show all". Voice is grouped like the web's: **Mic & output** (Hear
  yourself; iPhone: high-quality headphone audio) · **How you talk** (input
  mode, sensitivity) · **Sounds** (join & mute sounds, your chime,
  soundboard volume and mute; the app also lists the soundboard's clips) ·
  **Advanced** (quality, Hide my IP address) — on iPhone a row that opens
  its own page (`Destination.voiceAdvanced`), on the Mac a disclosure
  folded again each time Settings opens. No device pickers, music mode,
  RNNoise or OBS links: the app doesn't have them. Notifications: **On this
  device** (the system's notification settings) · **On your phone (ntfy)**
  · **Quiet hours**. Owner & mods › Activity and Invite › Your links show
  five, then "Show all N" (`ShortList`); each opening of Settings starts
  short again (the Mac's Settings window resets when it closes).
- **Questions** (delete, archive, ban, reset PIN, sign out, sign out
  everywhere else, turn off a link, remove a place, delete a sound, cancel
  a hang, discard changes, the ping note) are asked in a Windows 98 message
  box (`.question`, the web's `confirmSheet`), in the web's words
  (`Question.archive(_:)` etc.; the ping note keeps the server's 140
  characters): a question or warning icon, the action and Cancel
  centered, Cancel the default for destructive ones. On iPhone the box sits
  over the dimmed screen without sliding in; on the Mac it's the window's
  sheet.
- **Temporary PIN** (after an owner/mod resets someone's): the web's words
  in the box with the PIN in a selectable field and Copy PIN. Whichever
  screen is on top shows it — the person's sheet, People, or the root
  (`PinResetNotice`).
- **Reply and edit:** the reply bar quotes the first words of the message
  you're answering (or "📷 A picture", "GIF", "🔒 Encrypted note"); the
  message you're editing gets a dotted focus box and is brought into view
  if the edit bar pushed it off (scrolling as little as it takes).
- **`YapIcon.imageset`** (the DEBUG `staged` demo picture) is a 512 px copy
  of `AppIcon.appiconset/ios-1024.png`; refresh it when the icon changes
  (`Tools/make-icons.swift`).

## Design (Windows 98)

Windows 98 / AIM. `Yap/App/Theme.swift` holds every color, font and metric
(the same tokens as the web and desktop clients), so a reskin happens there;
`Yap/Components/Win98.swift` has the building blocks.

- **Schemes:** **Charcoal** (dark, the default) and **Windows Standard**
  (light), switchable in Settings › Look (or follow the system).
- **Chrome:** gray face with two-ring 1pt bevels (raised buttons and
  windows; pressed buttons invert and nudge the label 1pt; sunken white
  wells for fields, lists and the chat log; one-ring status panels), navy
  gradient title bars with bold white titles, etched group boxes,
  property-sheet tabs, a status bar (connection · voice · Minecraft, if
  the server has it).
  Square corners, no soft shadows, no pixel fonts.
- **Type:** Tahoma where the system has it (the Mac: chrome 11 pt, chat
  13 pt), otherwise the system font at small Dynamic Type sizes (iPhone:
  chrome = footnote, chat = subheadline).
- **Chat:** AIM lines — `(time) Name: message`, my name red, everyone else
  blue, bold. Mentions get the tooltip-yellow row, @names the selection
  color; reactions are small Win98 buttons (mine stay pushed in).
- **People:** a Win98 window (sheet) with the waiting room and everyone A–Z
  in a sunken list (role, "(you)", who's in voice); the selected person is
  solid selection color. No online/offline.
- **iPhone:** native navigation (push and sheets; the call bar under the
  title bar) dressed as Win98: navigation bars become title bars
  (gradient drawn under a transparent bar; white status bar app-wide), bar
  buttons are Win98 buttons, hit targets stay ≥ 44 pt.
- **Mac:** one window of two Win98 panes (channels · chat, `HSplitView`)
  over the status bar; People opens from the chat pane's title bar;
  Settings is a property sheet (tabs). Sheets are Win98 windows: a title bar
  with a close box and an OK row (`win98Sheet`), no system toolbar.
  Buttons are at least 75×23.

Sheet and list conventions: sheets close
with an X that asks "Discard changes?" when there are edits and confirm with
a ✓ (drawn as the Win98 default button); one "…" menu per screen; lists end
in "New …" rows; Settings is the last row of the channel list on iPhone (⌘,
on the Mac).

## v0.36 extras (PROTOCOL.md › Extras)

iPhone and Mac; the views are in `Views/Extras`, the logic is shared
(`App/Extras.swift`, `App/Sounds.swift`, `Components/Doodles.swift`,
YapCore's `Extras.swift`, `Chimes.swift`, `WAV.swift`, `TextRules.swift`,
`SoundClip`, `DoodleDrawing`).

- **Spoilers:** `MessageFormatter.parse(hideSpoilers:)` turns everything
  inside `||…||` into plain `.spoiler` runs (links can't be tapped, nothing
  leaks), drawn as an opaque block; a tap on the text reveals that message
  (`spoilerReveal`, also a VoiceOver action). One-line previews, reply
  quotes, search snippets and notifications show `▒▒▒`. The regex is the one
  every client shares (`Spoilers.regex`).
- **▶ Watch together** under a message with a YouTube link (watch, youtu.be,
  shorts, music, m.): `watch.queue {add}` + the web's Watch page, signed in.
- **Quick switcher:** ⌘K (Mac, iPad keyboard; the app's Go menu), a
  magnifying glass on the iPhone channel list (Win98: the list's menu ›
  Run…). Channels, voice (joins), places, links, people, actions; the last
  row searches messages (`model.searchRequest` → the open channel's search).
- **Since you left:** the unread line reads "New since 9:41 PM"; a "Jump to
  first unread" pill shows while it's off screen (pages back if it's older
  than the loaded page) until you've seen it. Counts are asked for on every
  `ready` (when something's unread): their `mentions` (mentions of you while
  you were away, replies included; nil from older servers) become the
  channel's mention badge, never lower than what came live. A device that
  has never read anything (a newcomer, or you on a new device) starts caught
  up: the first `ready` marks every channel read at its newest message.
  **Catch up** (sidebar, only
  while something's unread) lists channels with `unread.counts` and the
  first new line, those that mention you first ("@2 for you"), then the
  newest; picking one jumps there, and **Mark all read** marks every channel
  read up to its newest message. It shows from two unread channels and
  says how many; the counts are asked for again when it opens.
- **Save to Personal** in every message menu; the note shows "Saved from
  #general" and its quote jumps across channels. With encryption on, the
  first save says the server can see which message you saved.
- **Away messages:** Settings › Profile (text, suggestions, until when); shown
  in People, the person card and your nameplate; the `away` auto-reply is a
  quiet line under your message. Never in the channel list.
- **Sounds** (`SoundPlayer`, one AVAudioEngine → a −12 dB limiter): the
  web's join/leave/mute blips, everyone's **join chime** (the spec's table,
  synthesized by `ChimeTone.render`; or an uploaded `s:` clip), the
  **soundboard** (the call sheet's grid). Board clips play at measured gain
  × my volume (Settings › Voice › Sounds, default 50 %, never above the
  gain), two at once, nothing while deafened, nothing from people at 0 %, a
  mute switch.
  Uploading: pick a file (Voice Memos via Files, m4a/mp3/wav…), trim with a
  waveform and preview, `SoundClip` decodes with AVFoundation, mixes to
  mono 24 kHz PCM16 WAV, `POST /api/sound`.
- **Polls:** `/poll Question | A | B`, or New Poll in the channel menu; vote,
  change, tap yours to take it back. **`/roll` `/flip`** go as text; the
  server rolls and `Message.roll` draws as a die card. `/` suggests them.
- **Doodle reactions:** Doodle Reaction… in a message's menu: a 48 × 48
  pixel box, inks, brush sizes, eraser, wiggle (3 frames); exported at 2× as a
  GIF by our own encoder (`GIFEncoder`, round-trip tested), `POST
  /api/doodle`, `<d:ID>`. Shown bigger than emoji; tap for a big one (yours
  can be removed).
- **Media shelf:** Media / Pictures… in the channel menu: a grid, newest
  first, paging; tap to jump.
- **Quiet hours:** Settings › Notifications › Quiet hours (the push mode
  and topic are under On your phone (ntfy)): a nightly window, let
  @mentions through, snooze until tomorrow morning. The app's own notifications and their chimes follow the server's
  rule (`QuietHours.holds`).
- **Hangs:** upcoming ones in Places (cancel from the row's menu), Plan a
  Hang (half-hour times, up to 3 weeks, optional voice channel), and when
  `hang.now` comes a calm notice with Join voice (a notification if Yap is
  in the background).

## How voice works here

Same as `web/js/voice.js` and PROTOCOL.md › Voice: a mesh, the newcomer
offers to everyone already in the channel after its own `voice` echo,
perfect negotiation (the larger sid is polite, recomputed whenever a sid
changes; explicit rollback), signals applied strictly in order, the Opus
fmtp of every audio section rewritten on every local description (the
first is the voice; a later one, a web sharer's screen audio, is tuned as
music: ≥ 160 kbps, stereo where we send it), `maxBitrate` on the sender.
Audio m-lines drop `transport-cc` (`SDPTuner`): with it, WebRTC's bandwidth
estimator sends probe padding through an audio-only call. Video keeps it.

- **Calls survive reconnects** (PROTOCOL.md › Keeping calls across
  reconnects; `CallKeeper`): `voice.join` says `resume: true`. When our
  socket drops mid-call nothing hangs up: every connection stays, its
  signals wait in an outbox, and the call shows "Reconnecting… voice is
  still on". On `ready` we match who's back (`voice.prev` → the connection
  moves to the new sid), keep whoever isn't (after a deploy everyone
  reconnects), and rejoin with `prev` + `proof` (`ready.resumeKey`). Our echo
  with `prev` hands the call over: queued signals (and an unanswered offer)
  go to the current sids, apps that don't resume get a newcomer's offer, and
  a kept connection not `connected` 4 s later is rebuilt. Without `prev` we
  start over; not back within 20 s, we give up. Someone else's `lost` socket
  keeps their connection up to 30 s (or until it fails / stays down 5 s),
  listed as "reconnecting…" (`YapClient.holdSession`); `session.leave`
  doesn't close it; an offer with a new DTLS fingerprint is a fresh start.
  A lost connection's leave chime waits 5 s, and neither chime plays if
  they're back. Leaving, signing out and quitting send `voice.leave` first.
  Apps that don't resume (desktop 0.8.0) still work: they hang up and
  rejoin as before (measured: back in ~0.7 s, no chimes on our side).
- **ICE restarts** (`PathWatch`): when the path dies (Wi-Fi → cellular, a
  NAT rebinding), the impolite side restarts ICE after 3.5 s of
  `disconnected` (at once on `failed`), the polite side after 10 s, both
  with `/api/ice` fetched again (relay credentials expire; at most every
  30 s) and the relay-only policy kept; every 10 s while it's down; after
  the third try the impolite side rebuilds the connection if the other app
  can follow. iOS turns on WebRTC's `NWPathMonitor` field trial (network
  changes are noticed at once) and `WebRTC-Audio-iOS-Holding` (the audio
  unit restarts after an interruption); the SDK only offers the deprecated
  global setter, so `FieldTrials` looks it up at run time.
- **Connection hints** (nothing on the wire): "weak connection"
  (> 3 % of their audio concealed or a round trip over 300 ms, every 2 s
  while they send ≥ 30 packets/s, not in a connection's first 10 s, three
  bad readings on and three good off), "needs the voice relay" / "can't
  connect" (ICE failed, without / with a relay), "reconnecting…". The web's
  explanation is the Mac tooltip and the VoiceOver hint (`VoiceHintText`).

- **Audio session (iOS):** WebRTC's voice-processing unit with
  `.playAndRecord` + `.voiceChat` (Apple's echo cancellation), Bluetooth HFP
  allowed, speaker by default; an AirPlay/route button in the call panel.
  The `audio` background mode keeps the call and the socket alive, but
  only while the call's sound runs: WebRTC stops its voice-processing unit
  (and lets go of the session) the moment its last audio stream goes, which
  happened for a split second whenever the people in the call changed
  (someone joining you alone, the last one leaving, a connection rebuilt);
  in the background iOS then paused Yap and wouldn't start the mic again,
  so the call dropped with the screen off. The engine now keeps an
  **anchor** for the whole call on iOS (a receive-only audio stream between
  two in-app peer connections, nothing encoded), so the sound never stops
  mid-call (`AudioAnchorTests`). What Yap can't prevent (a phone call or an
  alarm keeping the sound, iOS refusing it back, a minute without the
  server, iOS stopping Yap) ends the call for real and says so: a "You
  left Lounge" notification with the reason, one tap to rejoin
  (`BackgroundCall`, `CallWatch`; an interruption gets ~20 s of background
  time first; a note scheduled 40 s ahead and pushed back every 5 s fires by
  itself if Yap is stopped, and Yap leaves when it wakes up after that).
  Outside calls the app's session is `.ambient`, so muted GIF videos never
  stop someone's music. **High-quality headphone audio** (Settings › Voice › Mic & output,
  off by default) keeps Bluetooth headphones in A2DP (stereo, full band;
  mode `.default`, the phone's own mic) instead of switching them to HFP.
- **Hide my IP address in calls** (Settings › Voice › Advanced, the web's
  `relayOnly`): `iceTransportPolicy = .relay`, so peers only see the
  server's TURN relay. Offered when the server's ICE servers include one
  (`IceServerConfig.hasRelay`); otherwise the option says the relay isn't on.
  Both voice options apply from the next join.
- **Transmit gate** (`VoiceGate`, PROTOCOL.md's table): voice activity,
  push to talk (hold the button) or always on, by enabling/disabling the
  track (no renegotiation). The level comes every 25 ms from the sender's
  media-source stats (`totalAudioEnergy` / `totalSamplesDuration`, measured
  before the mute gate, so it can reopen). Those counters add up each 10 ms
  frame's *peak*², not its mean square, so their "RMS" reads ~9 dB hotter
  than the real thing on speech (8.7 dB measured; 10 on noise, 3 on a pure
  tone): `VoiceGate.peakToRMSDb` brings it to the web's RMS scale, and a
  sensitivity picked on the old scale moves down with it (the −50 default
  stays; the older −45 default became −50). Voice activity opens above max(sensitivity, room + 12 dB, capped
  at −30), stays open within 6 dB (hysteresis, never into the room's
  noise) and 450 ms after; the room's noise is the quietest reading per
  500 ms, the quietest of the last 8, digital silence ignored. Push to talk
  keeps 200 ms after release. Mute and deafen switch the track off whatever
  the gate says. libwebrtc ramps each switch over 128 samples (~3 ms), so
  there's no click, but the web's 5/15 ms fades and its 40 ms pre-roll need
  a hook on the audio path that stasel's SDK doesn't have (only a custom
  `RTCAudioDevice`). Readings stopping while something should be sending
  fail open (voice activity sends everything until they're back). Alone in
  a channel there's no peer to send to, so `LevelMonitor` connects two peer
  connections inside the app (nothing leaves the device, nothing plays) to
  keep the meter and voice activity working; it closes when a real call
  starts.
- **Never silently lose the mic:** five seconds of digital silence (or no
  audio at all) while unmuted shows "Your mic is sending silence". iOS: WebRTC's
  session object reactivates the session after a call or Siri, and the
  iOS-Holding field trial restarts its audio unit; route changes (AirPods)
  restart it on a new sample rate; `AudioSessionWatch` logs all of it and
  puts the configuration back if the media server resets. The Mac uses
  WebRTC's Core Audio device, which doesn't follow a new default device
  mid-call (the warning shows).
- **Volume past 100 %** (`BoostLimiter`): WebRTC applies a person's gain
  with plain saturation, so the boost is capped from their own RTP audio
  level every 25 ms (speech peaks ~15 dB over it, kept under −1 dBFS): down
  at once, back at 6 dB/s, never under 100 %.
- **Hear yourself** (Settings › Voice › Mic & output, `MicCheck`): the SDK
  can't tap what a call sends, so outside a call it records 4 s through
  Apple's voice processing (`AVAudioEngine` with voice processing on; on
  iPhone the same unit WebRTC's calls use, in `.voiceChat`), runs the gate
  on it the way the engine does (decided every 25 ms, a 128-sample ramp),
  then Apple's Opus encoder at the call's bitrate, and plays it back; the
  meter shows the level while it records. In a call it asks you to leave
  first (it needs the mic to itself). On the Mac calls use WebRTC's own
  processing rather than Apple's, so it's close, not identical.
- **Speaking rings:** remote RTP audio levels (`RTCRtpReceiver.sources`),
  your own from the send-side stats.
- **Swift 6:** WebRTC delegates are nonisolated and hop to the main queue
  in order; nothing hands a main-actor closure to a framework callback.

## Known gaps

- Custom emoji **upload** and **rename/delete** have no UI yet (the picker
  shows and sends them; `YapClient.uploadEmoji` exists and is tested).
- OBS links (`obs.create`) and browser/ntfy push settings are made on the web.
- Watch together plays on the web: the apps show who's watching and open
  the browser.
- No **Share sound** from the Mac: stasel/WebRTC's SDK sends the
  microphone on every audio track (one recording feeds every sender) and has
  no way to push other audio into a track, so ScreenCaptureKit's
  `excludesCurrentProcessAudio` audio has nowhere to go without a different
  WebRTC build (one with an audio source you can push into). Watching a web
  share's sound works.
- iPhone/iPad sharing is the whole screen and picture only (no app sound),
  and needs the App Group set up once (TestFlight above).
- No mic/speaker device picker on the Mac (system default devices), no
  global push-to-talk key; a call doesn't follow a new default device.
- Voice activity has no 40 ms pre-roll and libwebrtc's ~3 ms switch ramp
  instead of 5/15 ms fades (no audio-path hook in the SDK); Hear yourself
  works outside calls only. No music mode yet (so no "music + voice
  activity = always on").
- Live cursors on the board, Draw and Space are web-only: those open in an
  in-app web view (which keeps its own sign-in).
- The app shows local notifications for @mentions, pings and hangs only
  while it's running (during a call in the background); ntfy covers the rest
  (Settings › Notifications sets the mode and shows the topic).
- Soundboard clips and chimes can't be recorded in the app yet: pick a
  file (a Voice Memo shared to Files works).
- Settings › Voice has no Test mic button: the meter reads in a call, or
  while Hear yourself records.
