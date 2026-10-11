# Yap security: what it protects, what it doesn't

> **In plain words:** Yap is as safe as a well-run private Discord server that *you* own. Outsiders can't get in, members can't pretend to be each other, and voice is encrypted between friends. But Yap is **not** Signal: messages are readable on the server (that's you, and Cloudflare), the same way they are on Discord.

## Compared with Signal

| | Signal | Yap |
|---|---|---|
| Message text | End-to-end encrypted; the server can't read it | Encrypted in transit (HTTPS), but **stored readable** in your Cloudflare Durable Object, and in your backups |
| Voice | End-to-end encrypted | **Encrypted peer-to-peer** (WebRTC DTLS-SRTP); audio never touches the server. Keys are exchanged through the server, though, so whoever runs the server *could* intercept a call in theory. |
| Who runs the server | Signal Foundation | You (on Cloudflare) |
| Metadata | Minimised (sealed sender, etc.) | The server sees who talks when, and voice peers see each other's IP addresses |
| Accounts | Phone number + device keys | Name + password (8+ characters) + per-device tokens |
| Audited | Yes, many times | No |

Making channel text Signal-grade means end-to-end encryption: per-group keys, multi-device, key verification. The server could then no longer search, back up or moderate text. It's a possible future project, and a big one.

**The exception: your Personal space** can be end-to-end encrypted (since 2026-10-05). Notes and pictures are sealed in your browser/app with AES-256-GCM under a random key that's wrapped by your passcode (PBKDF2-SHA256, 600,000 iterations); the server and backups only ever hold the wrapped key and ciphertext. Turning it on also seals your earlier notes and erases their plain copies. Limits: a weak passcode can be guessed offline by someone holding the database (so the app asks for 8+ characters and refuses short number-only ones), the web client is served by the same server (a malicious server could ship code that steals the passcode — true of every web-based E2E app), and anyone with your unlocked device can read it (Lock it in Settings). Details: PROTOCOL.md › Encrypted Personal space.

## What protects Yap today

**Getting in**
- **Invite key:** a random invite key (128 bits, made the way SELF_HOSTING.md shows). Without it, nothing connects. It travels in a header or the WebSocket subprotocol, so not in URLs or logs. (A key with characters other than letters, digits, `-` and `_` can't ride in the subprotocol, so the apps put it in the socket's address; keys made the way SELF_HOSTING.md shows never do.)
- **Waiting room:** a new account can't see or post anything until the owner or a mod lets them in; only they see who's waiting (and get a push about it). "Turn away" bans them.
- **Accounts:** a name plus a password of 8 or more characters (since 2026-10-11). Accounts from before then may still have a 6-digit PIN: it keeps signing in until its owner chooses a password, and nothing new can be set as a PIN.
  - **No guessable passwords:** the most common passwords (about 10,800 of 8+ characters, from SecLists' list of the 100,000 most common, MIT; `worker/common-passwords.ts`), your own name, one character repeated, and keyboard rows or counting are refused, so a curious friend (or a stranger with a list) can't just try the obvious. Someone still on a PIN, a guessable password or a temporary one is asked once on each device to choose a password ("Later" hides it there for good).
  - **New sign-in alerts:** signing in with your name + password on a new device alerts your other devices (and your phone, if pushes are on), with "Sign out everywhere else".
  - **Your devices:** Settings › Account lists every signed-in device (browser and system, when added, last used); sign any of them out.
  - **Lockout:** wrong passwords lock the name (5 free tries, then waits that double up to 24 h), with a per-IP cap on top.
  - **Hashing:** passwords (and old PINs) are hashed with PBKDF2-SHA256 at 100,000 iterations, mixed with a server secret (`PIN_PEPPER`) that isn't in the database or the backups. A leaked backup can't be brute-forced offline. **The exception:** a PIN set before the pepper existed (2026-10-05) keeps the old hash (20,000 iterations, no pepper) until its owner next signs in with name + PIN, which upgrades it; staying signed in on a device never does. Those few can be guessed offline from a leaked backup, so keep backups private, and ask such people to choose a password (Settings › Account › Set a password).
- **Device tokens:** 256-bit random secrets; the server only stores their hashes. Signing out revokes this device's token.
- **Old accounts:** signing in with an account id alone is gone (since 2026-10-06). An account that never set a password gets back in when an owner or mod resets it, and no new password-less accounts can be made.

**Inside**
- **Moderation:** the owner (proved by the admin key) and mods let people in, ban, delete any message, and reset passwords. A reset hands out a temporary password (three short words and two digits, like `maple-river-stone-42`) to pass on; the person is asked to choose their own after signing in with it.
- **Rate limits:**
  - per connection: messages, uploads, pings and more;
  - per IP: at the Worker;
  - per IP, for new accounts.
- **Uploads:** pictures only as PNG, GIF, WebP or JPEG, checked by their actual bytes (never SVG); soundboard clips only as WAV, which the server reads sample by sample to cap their loudness; encrypted Personal-space pictures as opaque bytes. All size-capped, and served with `nosniff` and a no-script policy. Each upload must come from a signed-in connection, proven by a token only that connection received (session ids alone are visible to everyone), checked before anything is stored.
- **Invite links:** each person can make their own, revocable link; only a hash is stored, and newcomers still wait for the owner or a mod to let them in.
- **OBS links:** listen-only tokens that can do nothing but join voice muted. Since 2026-10-11 they don't show up in the channel (anyone in a call can record it anyway); keep the link private, and turn them all off in Settings.
- **Message text:** always escaped before display. Links only allow `http(s)`. GIFs only come from KLIPY's CDN, so nobody can post an IP-logging image.

**The page**
- **Content-Security-Policy:**
  - only Yap's own scripts run;
  - no inline scripts (WebAssembly may compile, for the RNNoise noise filter; that allows no JavaScript `eval`);
  - images and media only from Yap and KLIPY (plus YouTube's video thumbnails), and the only frame from elsewhere is YouTube's privacy-enhanced player, for Watch together;
  - Yap can't be framed by other sites.
- **The embedded board (if a server has one):** it runs on Yap's origin, so it gets its own strict policy. Only its own scripts are allowed (by hash), so even a malicious card link can't run code.
- **Other headers:** `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`, COOP, and a Permissions-Policy that allows only the microphone and screen sharing (for Yap's own page; no camera, location, payments or USB).

**Data**
- Daily backups with `tools/backup.mjs` (kept 60 days; put them somewhere off-site), plus Cloudflare's 30-day point-in-time restore (tested 2026-10-05).
- **Deleted posts are erased** (2026-10-05): text, GIF, picture bytes, reactions and edit history go immediately; an empty row stays so ids aren't reused. Pictures stay cached at Cloudflare's edge for up to a day and in viewers' browsers. Everything else (channels, emojis, drawings) is still soft-deleted.
- **No presence:** the server never tells anyone who's online; sessions only become visible in voice or on a shared place, and `lastSeen` is never sent. (It still records when each device last connected, for account security.)
- What's stored and who can see it, in plain words for members: `/privacy` (`web/privacy.html`). Keep it true when anything here changes.

## Known gaps
- **Flooding from outside** can still use up the free 100k requests/day, because requests rejected at the Worker still count. The fix is a Cloudflare rate-limiting rule in front of the Worker, a 2-minute dashboard job:
  - Dashboard → your domain → **Security → Security rules** → **Create rule → Rate limiting rule**.
  - Expression: `(starts_with(http.request.uri.path, "/api/")) or (http.request.uri.path eq "/ws")`; same IP; **60 requests per 10 s**; action **Block** for 10 s.
  - The free plan allows one rule, path-only, so it covers every subdomain of that domain. Don't add `/emoji/` or `/media/`: opening the emoji picker alone loads ~60 images.
- **Voice IP addresses:** voice is peer-to-peer, so people in a call see each other's IP addresses, unless someone turns on Settings › Voice › "Hide my IP address in calls". That sends their voice through the server's voice relay (TURN), so it only works where the owner set one up (SELF_HOSTING.md), and the relay's operator sees the address instead.
- **Not end-to-end encrypted** (see above).
- **Third parties:** phone pushes pass through ntfy (ntfy.sh unless the server sets its own; topics are long random names); browser pushes go through your browser maker's push service, encrypted so only your browser can read them; GIF search talks to KLIPY directly from your browser; Watch together loads YouTube's player.
- **The owner's keys:** whoever holds the Cloudflare account or the admin key can read everything. Guard both, and keep the admin key and the password pepper (`PIN_PEPPER`) in a password manager or keychain, never in the repo.

## If something leaks
- **Invite link leaked:** `npx wrangler secret put YAP_KEY` (old links stop working; share the new one).
- **Admin key leaked:** `npx wrangler secret put ADMIN_KEY`, then update wherever your tools read it (`tools/instance.mjs`).
- **Someone's password leaked:** an owner or mod resets it (member menu › Reset password…) and sends them the temporary one; they choose a new password after signing in.
- **A bad actor got in:** ban them (member menu › Ban); everything they posted can be deleted by mods.
