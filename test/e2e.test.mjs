// End-to-end in real (headless) Chromium against a running dev server:
// text chat, the GIF picker (KLIPY mocked at the network layer), and two
// users in a voice channel — checks audio actually flows peer to peer as
// tuned Opus inside RED at the real bitrate, mute propagation, the call
// report — plus screenshots.
// Easiest: npm run test:all (it starts a fresh test server). By hand:
//   YAP_URL=http://localhost:8787 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin node test/e2e.test.mjs [outDir]
// The dev server needs any KLIPY_KEY var set (npm run dev:test does).
// YAP_E2E_SYNTH_AUDIO=1: a synthetic WebAudio mic and screen sound instead of
// Chromium's fake capture devices (test/synth-capture.js). Used by itself when
// Chromium can't open audio capture on this machine (some macOS builds hang).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const BASE = process.env.YAP_URL ?? 'http://localhost:8787';
const KEY = process.env.YAP_KEY ?? '';
const OUT = process.argv[2] ?? fs.mkdtempSync(path.join(os.tmpdir(), 'yap-shots-'));
const BITRATE = Number(process.env.YAP_BITRATE ?? 128000);
// Text this run writes carries RUN, so a wait can never be satisfied by an
// older run's message on a server that kept its data.
const RUN = Math.random().toString(36).slice(2, 6);
fs.mkdirSync(OUT, { recursive: true });

// 12 s of chords + noise so Opus has something to spend bits on.
function writeTestWav(file) {
  const sr = 48000, n = sr * 12, data = Buffer.alloc(n * 2);
  let seed = 1;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
  for (let i = 0; i < n; i++) {
    const t = i / sr;
    const chord = Math.floor(t) % 2 ? [196, 246.94, 293.66] : [220, 277.18, 329.63];
    let v = chord.reduce((s, f) => s + Math.sin(2 * Math.PI * f * t) * 0.18, 0) + rnd() * 0.12;
    v *= 0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t);
    data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, v)) * 30000), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8);
  h.write('fmt ', 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(sr, 24); h.writeUInt32LE(sr * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write('data', 36); h.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([h, data]));
}
// Each run its own file, so two runs at once don't share (or delete) one.
const wav = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yap-audio-')), 'test-audio.wav');
writeTestWav(wav);

/** Can headless Chromium open its fake mic here? (On some macOS builds any audio capture hangs forever.) */
async function audioCaptureWorks() {
  const probe = await chromium.launch({ args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'] });
  try {
    const page = await (await probe.newContext({ permissions: ['microphone'] })).newPage();
    await page.goto(`${BASE}/privacy`);
    return await page.evaluate(() => Promise.race([
      navigator.mediaDevices.getUserMedia({ audio: true }).then((s) => (s.getTracks().forEach((t) => t.stop()), true), () => false),
      new Promise((resolve) => setTimeout(() => resolve(false), 5000)),
    ]));
  } finally {
    await probe.close();
  }
}
const SYNTH = process.env.YAP_E2E_SYNTH_AUDIO === '1' || !(await audioCaptureWorks());
if (SYNTH) console.log('e2e: a synthetic mic and screen sound (test/synth-capture.js) stand in for Chromium\'s fake capture devices');

const browser = await chromium.launch({
  args: [
    '--use-fake-device-for-media-stream',
    '--use-fake-ui-for-media-stream',
    `--use-file-for-fake-audio-capture=${wav}`,
    '--autoplay-policy=no-user-gesture-required',
    // Screen sharing for real (no stub): the picker picks the whole (fake) screen by itself.
    '--auto-select-desktop-capture-source=Entire screen',
    '--enable-usermedia-screen-capturing',
  ],
});

// ── Fake KLIPY ──────────────────────────────────────────────────────────
const klipyLog = [];
const PALETTE = ['#5b6cff', '#e5484d', '#f76b15', '#30a46c', '#0090ff', '#8e4ec6', '#d6409f', '#12a594'];
function klipyItem(q, i) {
  const [w, h] = [[498, 280], [498, 498], [360, 480], [480, 270]][i % 4];
  const file = (kind) => ({ url: `https://static.klipy.com/test/${q}-${i}.${kind}`, width: w, height: h, size: 1000 });
  const size = { gif: file('gif'), webp: file('webp'), jpg: file('jpg'), mp4: file('mp4'), webm: file('webm') };
  return { id: i, slug: `${q}-${i}`, title: `${q} ${i}`, type: 'gif', file: { hd: size, md: size, sm: size, xs: size } };
}
async function fakeKlipy(context) {
  await context.route('https://api.klipy.com/**', async (route) => {
    const url = new URL(route.request().url());
    klipyLog.push(`${route.request().method()} ${url.pathname.replace(/^\/api\/v1\/[^/]+/, '')}${url.search}`);
    if (route.request().method() === 'POST') return route.fulfill({ json: { result: true } });
    const q = url.searchParams.get('q') ?? (url.pathname.includes('recent') ? 'recent' : 'trending');
    const page = Number(url.searchParams.get('page') ?? 1);
    const items = Array.from({ length: 12 }, (_, i) => klipyItem(q, (page - 1) * 12 + i));
    return route.fulfill({ json: { result: true, data: { data: items, current_page: page, per_page: 12, has_next: page < 2 } } });
  });
  await context.route(/https:\/\/static\d*\.klipy\.com\/.*/, (route) => {
    const name = route.request().url().split('/').pop();
    if (/\.(mp4|webm)$/.test(name)) return route.fulfill({ status: 404 });
    const i = Number(name.match(/-(\d+)\./)?.[1] ?? 0);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="498" height="280" viewBox="0 0 498 280"><rect width="498" height="280" fill="${PALETTE[i % PALETTE.length]}"/><text x="249" y="160" font-size="64" font-family="sans-serif" fill="#fff" text-anchor="middle">${name.split('.')[0]}</text></svg>`;
    return route.fulfill({ contentType: 'image/svg+xml', body: svg });
  });
}

// ── Page errors ───────────────────────────────────────────────────────────
// Every page in the run reports uncaught exceptions and console errors here;
// the last test fails if there were any (a broken import or a TDZ mistake
// shows up as one, even when the page still half works).
const pageErrors = [];
/** Console errors that are expected, and why. */
const EXPECTED_CONSOLE = [
  // The fake KLIPY answers 404 for its videos on purpose (tiles fall back to the poster).
  (text, url) => /^Failed to load resource/.test(text) && /^https:\/\/static\d*\.klipy\.com\/.+\.(mp4|webm)$/.test(url),
  // Chromium reports this on every getDisplayMedia because the page says camera=(); sharing works all the same.
  (text) => text === 'Permissions policy violation: camera is not allowed in this document.',
];
/** browser.newContext, plus error reporting for every page it opens. */
async function newContext(options, label = 'page') {
  const context = await browser.newContext(options);
  if (SYNTH) await context.addInitScript({ path: fileURLToPath(new URL('synth-capture.js', import.meta.url)) });
  await fakeKlipy(context); // no page in the run ever reaches the real KLIPY
  const report = (text) => {
    console.log(`[${label}] ${text}`);
    pageErrors.push(`[${label}] ${text}`);
  };
  context.on('weberror', (e) => report(`page error: ${e.error().message}`));
  context.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const { url } = msg.location();
    if (EXPECTED_CONSOLE.some((expected) => expected(msg.text(), url))) return;
    report(`console error: ${msg.text()}${url ? ` (${url})` : ''}`);
  });
  return context;
}

async function user(name, color, viewport = { width: 1280, height: 800 }, colorScheme = 'light') {
  const context = await newContext({ viewport, colorScheme, permissions: ['microphone'] }, name);
  await context.addInitScript(([name, color, bitrate]) => {
    if (window !== window.top || !location.origin.startsWith('http')) return; // only Yap's page (not about:blank or iframes)
    if (!localStorage.getItem('yap.settings')) {
      // A fresh browser; settle() signs in (or signs up on a fresh server) like a person would.
      // Noise suppression off: the fake mic plays a test signal (chords + noise), not speech, and
      // RNNoise rightly removes nearly all of it once it has heard a few seconds of it (measured in a
      // call: 0.17 RMS in, 0.0005 out). Every check of what's sent (its level, its bitrate, the
      // push-to-talk tail, Space's left and right, Hear yourself, "heard again") then passed or
      // failed by how long the call had been going. The noise suppression test turns it on.
      localStorage.setItem('yap.settings', JSON.stringify({ color, bitrate, mode: 'open', noiseSuppression: false }));
    }
  }, [name, color, BITRATE]);
  const page = await context.newPage();
  await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await settle(page, name);
  return page;
}

// The made-up password every suite signs up with.
const TEST_PASSWORD = 'correct-horse-582039';
/** Type a password into the welcome form. */
const fillPassword = (page, value = TEST_PASSWORD) => page.fill('#welcome-form input[name=pin]', value);
/** The admin API (the test server's ADMIN_KEY). */
const adminPost = (p) => fetch(`${BASE}${p}`, { method: 'POST', headers: { Authorization: `Bearer ${process.env.YAP_ADMIN_KEY}` } });
/** Ada lets people in and resets passwords below: on the shared test server someone else came first, so she claims owner. */
async function ensureOwner(page) {
  if (!process.env.YAP_ADMIN_KEY || (await page.evaluate(() => yap.state.users.get(yap.settings.uid)?.role)) === 'owner') return;
  await page.evaluate((key) => yap.ui.send({ t: 'owner.claim', key }), process.env.YAP_ADMIN_KEY);
  await page.waitForFunction(() => yap.state.users.get(yap.settings.uid)?.role === 'owner', null, { timeout: 5000 });
}
/** Get a test user online like a person would: sign in (or sign up) with a password; admin lets newcomers in. */
async function settle(page, name) {
  let triedLogin = false;
  for (let i = 0; i < 8; i++) {
    const s = await page.waitForFunction(() => {
      if (document.querySelector('#welcome-dialog')?.open) return 'welcome';
      const status = globalThis.yap?.state.status;
      return status === 'online' || status === 'waiting' ? status : false;
    }, null, { timeout: 15000 }).then((h) => h.jsonValue());
    if (s === 'welcome') {
      // Sign in first; if there's no such account yet (fresh server), sign up.
      await page.click(`#welcome-form label:has(input[value=${triedLogin ? 'signup' : 'login'}])`);
      await page.fill('#welcome-form input[name=name]', name);
      await fillPassword(page);
      await page.click('#welcome-form button[type=submit]');
      triedLogin = true;
      await page.waitForFunction(() => !document.querySelector('#welcome-dialog')?.open || document.querySelector('#welcome-error:not([hidden])'), null, { timeout: 10000 });
    } else if (s === 'waiting') {
      const uid = await page.evaluate(() => yap.settings.uid);
      await fetch(`${BASE}/api/admin/role?uid=${uid}&role=member`, { method: 'POST', headers: { Authorization: `Bearer ${process.env.YAP_ADMIN_KEY}` } });
      await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
    } else if (await page.evaluate(() => !document.querySelector('dialog[open]'))) {
      return;
    }
  }
  throw new Error(`${name} never got online`);
}

const results = [];
// YAP_E2E_ONLY=<regex>: run just the tests whose names match (plus the setup above).
const ONLY = process.env.YAP_E2E_ONLY ? new RegExp(process.env.YAP_E2E_ONLY, 'i') : null;
async function test(name, fn) {
  if (ONLY && !ONLY.test(name) && !name.startsWith('no page errors')) return;
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
    // Leave evidence: what each user was looking at when it failed.
    const slug = name.replace(/\W+/g, '-').slice(0, 40);
    for (const [who, page] of [['a', globalThis.pageA], ['b', globalThis.pageB]]) {
      await page?.screenshot({ path: path.join(OUT, `FAIL-${slug}-${who}.png`) }).catch(() => {});
    }
    await globalThis.pageA?.keyboard.press('Escape').catch(() => {}); // don't cascade open pickers
  }
}

const a = await user('Ada', '#5b6cff');
const b = await user('Ben', '#30a46c');
globalThis.pageA = a;

/**
 * Back to how the voice tests expect someone (unmuted, undeafened, always-on, no push-to-mute key,
 * reminders on), so one failure in a test that changes these doesn't silence every test after it.
 */
const voiceReset = (page) => page.evaluate(() => {
  const click = (action) => document.querySelector(`[data-action="${action}"]`)?.click();
  if (yap.settings.deafened) click('toggle-deafen');
  if (yap.settings.muted) click('toggle-mute');
  Object.assign(yap.settings, { mutedReminder: true, pttClick: true, pushToMuteKey: '' });
  yap.voice.setPushToMute(false);
  if (yap.settings.mode !== 'open') {
    yap.settings.mode = 'open';
    yap.voice.releasePtt();
  }
  document.querySelector('#settings-dialog')?.open && document.querySelector('#settings-dialog').close();
}).catch(() => {});
globalThis.pageB = b;

await test('same person on a new device: sign in with name + password', async () => {
  const ctx = await newContext({ viewport: { width: 1280, height: 800 } }, 'Ben, 2nd device');
  const page = await ctx.newPage();
  await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await page.waitForSelector('#welcome-dialog[open]');
  await page.click('#welcome-form label:has(input[value=login])');
  // Signed out, the app isn't drawn behind the form; a wrong password keeps the typed name, says
  // who can help, and its error goes away when you switch to "I'm new" (it was about signing in).
  assert.equal(await page.evaluate(() => document.documentElement.classList.contains('signed-out')), true);
  const field = '#welcome-form input[name=pin]';
  assert.deepEqual(await page.$eval(field, (el) => [el.type, el.autocomplete]), ['password', 'current-password'], 'password managers fill the saved one');
  assert.equal(await page.isHidden('#welcome-form .field-note'), true, '"8 characters or more." is for signing up');
  await page.fill('#welcome-form input[name=name]', 'ben');
  await fillPassword(page, 'not-bens-password');
  await page.click('#welcome-form button[type=submit]');
  await page.waitForSelector('#welcome-error:not([hidden])', { timeout: 8000 });
  assert.equal(await page.textContent('#welcome-error'), 'Wrong name or password. An old PIN works here too. Forgot it? The owner or a mod can reset it.');
  assert.equal(await page.inputValue('#welcome-form input[name=name]'), 'ben');
  await page.click('#welcome-form label:has(input[value=signup])');
  assert.equal(await page.isHidden('#welcome-error'), true, 'no stale "Wrong name or password"');
  assert.equal(await page.$eval(field, (el) => el.autocomplete), 'new-password', 'and offer to make one when signing up');
  await page.waitForSelector('#welcome-form .field-note:text-is("8 characters or more.")');
  await page.click('#welcome-form label:has(input[value=login])');
  // Show / Hide what's typed.
  await fillPassword(page);
  await page.click('#welcome-form [data-action="reveal"]');
  assert.equal(await page.$eval(field, (el) => el.type), 'text');
  assert.equal(await page.textContent('#welcome-form [data-action="reveal"]'), 'Hide');
  await page.click('#welcome-form [data-action="reveal"]');
  assert.equal(await page.$eval(field, (el) => el.type), 'password');
  // The only PIN on this screen is the wrong-sign-in hint ("An old PIN works here too").
  const pins = await page.$eval('#welcome-dialog', (d) => [...d.querySelectorAll('*')].filter((el) => !el.closest('#welcome-error')
    && [...el.childNodes].some((n) => n.nodeType === 3 && /\bPIN\b/.test(n.textContent))).length);
  assert.equal(pins, 0, 'no PIN anywhere but the wrong-sign-in hint');
  await page.screenshot({ path: path.join(OUT, 'signin.png') });
  await page.click('#welcome-form button[type=submit]');
  await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  assert.equal(await page.evaluate(() => document.documentElement.classList.contains('signed-out')), false);
  const [uidB, uidNew] = [await b.evaluate(() => yap.settings.uid), await page.evaluate(() => yap.settings.uid)];
  assert.equal(uidNew, uidB, 'same account as the first device');
  assert.ok(await page.evaluate(() => yap.settings.token), 'device token saved');
  await ctx.close();
});

await test('text message shows up for the other person', async () => {
  const text = `hello from Ada ${Date.now()}`;
  await a.fill('#input', text);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body:text("${text}")`, { timeout: 5000 });
  await a.fill('#input', 'some **bold**, `code`, and a link https://example.com/x?a=1&b=2.');
  await a.press('#input', 'Enter');
  await b.waitForSelector('.msg .body a[href="https://example.com/x?a=1&b=2"]', { timeout: 5000 });
});

await test('edit your message: ↑ in an empty box, Enter saves, others see (edited)', async () => {
  const text = `edit me ${Date.now()}`;
  await a.fill('#input', text);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body:text("${text}")`, { timeout: 5000 });
  await a.waitForFunction((t) => yap.state.history.get('general')?.messages.some((m) => m.body === t), text, { timeout: 5000 });
  await a.press('#input', 'ArrowUp');
  await a.waitForSelector('#edit-bar:not([hidden])');
  assert.equal(await a.inputValue('#input'), text);
  await a.waitForSelector(`.msg.editing .body:text("${text}")`); // the one being edited is marked
  await a.fill('#input', `${text} (fixed)`);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body:has-text("${text} (fixed)") .edited`, { timeout: 5000 });
  assert.equal(await a.isHidden('#edit-bar'), true);
  assert.equal(await a.inputValue('#input'), '');
});

await test('Settings › Invite: make a personal link; Places: add a link everyone sees', async () => {
  await a.click('[data-action="open-settings"]');
  const inviteTab = await a.evaluate(() => [...document.querySelectorAll('#settings-tabs [data-tab]')].find((b) => b.textContent === 'Invite')?.dataset.tab);
  await a.click(`#settings-tabs [data-tab="${inviteTab}"]`);
  await a.fill('#invite-label', `for Dee ${RUN}`);
  await a.click('[data-action="create-invite"]');
  await a.waitForSelector('#invite-made:not([hidden])', { timeout: 5000 });
  assert.match(await a.inputValue('#invite-link'), /#key=yi_[A-Za-z0-9_-]{32}$/);
  await a.waitForSelector(`#invite-list li:has-text("for Dee ${RUN}")`, { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-settings-invite.png') });
  await a.keyboard.press('Escape');
  await a.click('[data-action="add-link"]');
  await a.fill('#link-form input[name=name]', `Wiki ${RUN}`);
  await a.fill('#link-form input[name=url]', 'https://example.com/wiki');
  await a.click('#link-form button[value=add]');
  await b.waitForSelector(`#links a.channel:has-text("Wiki ${RUN}")`, { timeout: 5000 });
  // Questions are Yap's own sheet now (js/sheet.js), not the browser's confirm(): Cancel keeps it…
  await b.click(`#links .channel-row:has(a:has-text("Wiki ${RUN}")) [data-action="remove-link"]`, { force: true });
  await b.waitForSelector('#confirm-dialog[open] #confirm-title:has-text("Remove Wiki")');
  await b.screenshot({ path: path.join(OUT, 'desktop-confirm.png') });
  await b.click('#confirm-cancel');
  await b.waitForSelector('#confirm-dialog', { state: 'hidden' });
  assert.equal(await a.locator(`#links a.channel:has-text("Wiki ${RUN}")`).count(), 1, 'Cancel keeps it');
  // …and the action does it.
  await b.click(`#links .channel-row:has(a:has-text("Wiki ${RUN}")) [data-action="remove-link"]`, { force: true });
  await b.click('#confirm-dialog[open] #confirm-ok');
  await a.waitForSelector(`#links a.channel:has-text("Wiki ${RUN}")`, { state: 'detached', timeout: 5000 });
});

await test('Settings › Invite › Reset main link… (owner only): the warning, then the new link; signed-in devices with the old key keep working and pick it up', async () => {
  await ensureOwner(a);
  const openInvite = async (page) => {
    await page.click('[data-action="open-settings"]');
    await page.click('#settings-tabs [role=tab]:text-is("Invite")');
  };
  // Ben isn't the owner: no reset for him.
  await openInvite(b);
  assert.equal(await b.isHidden('#reset-main-btn'), true, 'only the owner resets it');
  await b.keyboard.press('Escape');
  await openInvite(a);
  await a.waitForSelector('#reset-main-btn:not([hidden])');
  await a.locator('#main-invite').screenshot({ path: path.join(OUT, 'MW1-before.png') });
  let key;
  try {
    await a.click('#reset-main-btn');
    await a.waitForSelector('#confirm-dialog[open]');
    assert.deepEqual(await a.evaluate(() => ['confirm-title', 'confirm-text', 'confirm-ok', 'confirm-cancel'].map((id) => document.getElementById(id).textContent)), [
      'Reset main link?',
      "The old link stops working for anyone who hasn't joined yet. Everyone already signed in keeps working, and their apps get the new link by themselves. Personal invite links keep working.",
      'Reset link',
      'Cancel',
    ]);
    await a.locator('#confirm-dialog').screenshot({ path: path.join(OUT, 'MW1-warning.png') });
    // Cancel changes nothing.
    await a.click('#confirm-cancel');
    await a.waitForTimeout(300);
    assert.equal(await a.evaluate(() => yap.settings.key), KEY);
    await a.click('#reset-main-btn');
    await a.click('#confirm-dialog[open] #confirm-ok');
    await a.waitForSelector('#main-link-made:not([hidden])', { timeout: 5000 });
    key = new URLSearchParams(new URL(await a.inputValue('#main-link')).hash.slice(1)).get('key');
    assert.match(key, /^[A-Za-z0-9_-]{32}$/);
    assert.notEqual(key, KEY);
    assert.equal(await a.evaluate(() => yap.settings.key), key, 'the owner\'s own link is the new one');
    await a.locator('#main-invite').screenshot({ path: path.join(OUT, 'MW1-after.png') });
    await b.waitForFunction((k) => yap.settings.key === k, key, { timeout: 5000 }); // Ben's page got it by itself
    await a.keyboard.press('Escape');
    // A device that's signed in but still has the old key (it was closed during the reset) gets in, and saves the new one.
    await b.evaluate((old) => {
      const saved = JSON.parse(localStorage.getItem('yap.settings'));
      localStorage.setItem('yap.settings', JSON.stringify({ ...saved, key: old }));
    }, KEY);
    await b.reload();
    await b.waitForFunction(() => yap.state.status === 'online', null, { timeout: 15000 });
    assert.equal(await b.evaluate(() => yap.settings.key), key, 'ready.key is its link now');
    assert.equal(await b.isHidden('#welcome-dialog'), true, 'never asked for a link');
    // Someone new with the old link is told it doesn't work.
    const ctx = await newContext({ viewport: { width: 1280, height: 800 } }, 'old link');
    const page = await ctx.newPage();
    await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
    await page.waitForSelector('#welcome-dialog[open] #welcome-key-field:not([hidden])', { timeout: 10000 });
    assert.equal(await page.textContent('#welcome-error'), "That invite link didn't work. Ask for a new one.");
    await ctx.close();
  } finally {
    // Back to the test server's own key for everything after this.
    await fetch(`${BASE}/api/admin/main-key`, { method: 'DELETE', headers: { Authorization: `Bearer ${process.env.YAP_ADMIN_KEY}` } });
  }
  await a.waitForFunction((k) => yap.settings.key === k, KEY, { timeout: 5000 });
  await b.waitForFunction((k) => yap.settings.key === k, KEY, { timeout: 5000 });
});

await test('Settings: Advanced starts closed, long lists show 5 then all, and it reopens on your last tab', async () => {
  // Ada lists everyone's invite links as the owner. On the shared test server (npm run test:all runs
  // the protocol test first) someone else came first, so she claims it with the admin key.
  if (process.env.YAP_ADMIN_KEY && (await a.evaluate(() => yap.state.users.get(yap.settings.uid)?.role)) !== 'owner') {
    await a.evaluate((key) => yap.ui.send({ t: 'owner.claim', key }), process.env.YAP_ADMIN_KEY);
    await a.waitForFunction(() => yap.state.users.get(yap.settings.uid)?.role === 'owner', null, { timeout: 5000 });
  }
  // Enough invite links (and mod-log lines) for "Show all": 5 each at most (a rate limit), so Ada and Ben share.
  for (let i = 0; i < 4; i++) await a.evaluate((label) => yap.ui.send({ t: 'invite.create', label }), `extra ${i} ${RUN}`);
  for (let i = 4; i < 7; i++) await b.evaluate((label) => yap.ui.send({ t: 'invite.create', label }), `extra ${i} ${RUN}`);
  await a.waitForFunction((run) => (yap.state.invites ?? []).filter((x) => x.label.endsWith(run)).length >= 5, RUN, { timeout: 5000 });
  await b.waitForFunction((run) => (yap.state.invites ?? []).filter((x) => x.label.endsWith(run)).length >= 3, RUN, { timeout: 5000 });
  const tab = (name) => a.click(`#settings-tabs [role=tab]:text-is("${name}")`);
  await a.click('[data-action="open-settings"]');
  // Voice: everyday groups (one Noise suppression switch), then Advanced (closed) with quality, music mode, relay-only and OBS.
  await tab('Voice');
  assert.deepEqual(await a.$$eval('#settings-form section:not([hidden]) .settings-group > legend', (els) => els.map((e) => e.textContent)), ['Mic & output', 'How you talk', 'Sounds']);
  assert.equal(await a.evaluate(() => document.getElementById('voice-advanced').open), false, 'Advanced starts closed');
  assert.equal(await a.isVisible('#settings-form select[name=bitrate]'), false);
  await a.click('#voice-advanced > summary');
  await a.waitForSelector('#settings-form select[name=bitrate]', { state: 'visible' });
  assert.equal(await a.locator('#settings-form [name=rnnoise]').count(), 0, 'no second noise switch');
  assert.equal(await a.evaluate(() => document.querySelector('#settings-form [name=noiseSuppression]').closest('#voice-advanced')), null, 'Noise suppression is with the everyday ones');
  for (const name of ['music', 'relayOnly']) assert.ok(await a.evaluate((n) => document.querySelector(`#settings-form [name=${n}]`).closest('#voice-advanced'), name), `${name} is under Advanced`);
  // OK stays on screen however long the tab is (the tab scrolls inside).
  const ok = await a.locator('#settings-form .settings-actions .btn').boundingBox();
  assert.ok(ok && ok.y + ok.height <= 800, 'OK is on screen');
  // Invites: five, then "Show all".
  await tab('Invite');
  await a.waitForFunction(() => document.querySelectorAll('#invite-list li').length === 5);
  await a.click('[data-action="show-all"][data-list="invites"]');
  await a.waitForFunction(() => document.querySelectorAll('#invite-list li').length >= 8);
  assert.equal(await a.isVisible('[data-action="show-all"][data-list="invites"]'), false);
  // Activity: the same, in plain words.
  await tab('Owner & mods');
  await a.waitForFunction(() => document.querySelectorAll('#modlog li').length === 5);
  assert.match(await a.textContent('#modlog'), /Ben made an invite link \(extra 6/);
  await a.click('[data-action="show-all"][data-list="modlog"]');
  await a.waitForFunction(() => document.querySelectorAll('#modlog li').length > 5);
  await a.click('#settings-form .settings-actions .btn');
  // Reopened without asking for a tab (Ctrl/⌘+K › Settings): back on Owner & mods, lists short again.
  await a.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await a.waitForSelector('#switcher-dialog[open]');
  await a.keyboard.type('settings');
  await a.waitForSelector('.switcher-row.active:has-text("Settings")');
  await a.keyboard.press('Enter');
  await a.waitForSelector('#settings-dialog[open]');
  assert.equal(await a.textContent('#settings-tabs [aria-selected="true"]'), 'Owner & mods');
  assert.equal(await a.locator('#modlog li').count(), 5);
  await a.click('#settings-form .settings-actions .btn');
  // The profile card still opens on Profile.
  await a.click('[data-action="open-settings"]');
  assert.equal(await a.textContent('#settings-tabs [aria-selected="true"]'), 'Profile');
  await a.click('#settings-form .settings-actions .btn');
  await a.waitForFunction((run) => yap.state.invites.filter((x) => x.label.endsWith(run)).length >= 8, RUN);
  for (const page of [a, b]) {
    for (const i of (await page.evaluate((run) => yap.state.invites.filter((x) => x.label.startsWith('extra ') && x.label.endsWith(run) && !x.revoked).map((x) => x.id), RUN))) {
      await page.evaluate((id) => yap.ui.send({ t: 'invite.revoke', id }), i);
    }
  }
});

await test('iPhone: Add to Home Screen shows the steps; the shortcut opens signed in', async () => {
  // Safari on an iPhone (well: Chromium dressed as one).
  const iphone = await newContext({
    viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true,
    userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  }, 'iPhone Safari');
  const token = await a.evaluate(() => yap.settings.token);
  await iphone.addInitScript(([t, k]) => window === window.top && location.origin.startsWith('http') && localStorage.setItem('yap.settings', JSON.stringify({ token: t, key: k, mode: 'open' })), [token, KEY]);
  const p = await iphone.newPage();
  await p.goto(BASE);
  await p.waitForSelector('#home-bar:not([hidden])', { timeout: 10000 });
  await p.click('#home-bar [data-action="add-to-home"]');
  await p.waitForSelector('#home-dialog[open]');
  assert.match(await p.textContent('#home-steps'), /Add to Home Screen/);
  const shortcut = await p.evaluate(() => location.href);
  assert.match(shortcut, /#key=.+&handoff=.+&name=/);
  await p.screenshot({ path: path.join(OUT, 'phone-add-to-home.png') });
  await iphone.close();
  // The Home Screen app starts with empty storage and opens the address it was added from.
  const app = await newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true }, 'Home Screen app');
  const home = await app.newPage();
  await home.goto(shortcut);
  await home.waitForFunction(() => yap.state.status === 'online' && yap.settings.token, null, { timeout: 10000 });
  assert.doesNotMatch(await home.evaluate(() => location.hash), /handoff=|key=/, 'the pass (and the key) leave the address bar');
  assert.equal(await home.evaluate(() => document.querySelector('#welcome-dialog')?.open ?? false), false, 'no sign-in needed');
  await app.close();
});

await test('iPhone: no hover, so a tap on a message shows its actions; Reply works from there', async () => {
  const iphone = await newContext({
    viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true,
    userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  }, 'iPhone taps');
  const token = await b.evaluate(() => yap.settings.token);
  await iphone.addInitScript(([t, k]) => window === window.top && location.origin.startsWith('http') && localStorage.setItem('yap.settings', JSON.stringify({ token: t, key: k, mode: 'open' })), [token, KEY]);
  const p = await iphone.newPage();
  await p.goto(BASE);
  await p.waitForFunction(() => yap.state.status === 'online' && yap.state.history.get('general')?.loading === false, null, { timeout: 10000 });
  const text = `tap me ${RUN}`;
  await a.evaluate((body) => yap.ui.send({ t: 'send', channelId: 'general', body }), text);
  const row = p.locator('.msg', { has: p.locator(`.body:text-is("${text}")`) });
  await row.waitFor({ timeout: 5000 });
  // (Hidden, not gone: transparent and untappable, but there for Tab and screen readers.)
  assert.equal(await row.locator('.msg-actions').evaluate((el) => getComputedStyle(el).opacity === '0' && getComputedStyle(el).pointerEvents === 'none'), true, 'hidden until tapped');
  await row.locator('.body').tap();
  await p.waitForSelector('.msg.show-actions .msg-actions [data-action="reply-message"]', { state: 'visible' });
  const size = await row.locator('[data-action="reply-message"]').boundingBox();
  assert.ok(size.width >= 36 && size.height >= 30, `a finger-sized button (${size.width}×${size.height})`);
  await p.screenshot({ path: path.join(OUT, 'phone-msg-actions.png') });
  await row.locator('[data-action="reply-message"]').tap();
  await p.waitForSelector('#reply-bar:not([hidden])');
  assert.equal(await p.isHidden('#reply-bar .key-hint'), true, 'no "Esc to cancel" on a phone');
  await p.waitForSelector('.msg.show-actions', { state: 'detached' });
  await p.tap('#reply-bar [data-action="cancel-reply"]');
  await iphone.close();
});

await test('inside the iPhone/Mac app: Draw opens signed in, embedded (no web sidebar or header), even over a signed-out token', async () => {
  // What the app does: ask for a one-time pass, open /#…&place=draw&handoff=…&embed=1.
  const pass = await a.evaluate(() => new Promise((resolve) => {
    yap.state.handoffWaiter = resolve;
    yap.ui.send({ t: 'device.handoff' });
  }));
  const ctx = await newContext({ viewport: { width: 390, height: 700 } }, 'app web view');
  // A web view that remembers a token which was signed out since.
  await ctx.addInitScript((k) => window === window.top && location.origin.startsWith('http') && localStorage.setItem('yap.settings', JSON.stringify({ key: k, token: 'signed-out-long-ago', mode: 'open' })), KEY);
  const p = await ctx.newPage();
  await p.goto(`${BASE}/#key=${encodeURIComponent(KEY)}&place=draw&handoff=${encodeURIComponent(pass)}&embed=1`);
  await p.waitForFunction(() => yap.state.status === 'online' && yap.state.view === 'draw', null, { timeout: 10000 });
  assert.equal(await p.evaluate(() => yap.settings.uid), await a.evaluate(() => yap.settings.uid), 'signed in as the same person');
  assert.equal(await p.isVisible('.sidebar'), false);
  assert.equal(await p.isVisible('.chat-header'), false);
  assert.equal(await p.isVisible('#draw-view'), true);
  await p.screenshot({ path: path.join(OUT, 'embedded-draw.png') });
  await ctx.close();
});

await test('a sign-in link opened in a plain tab asks whose account it is (named by the server, not the link); “Not me” signs out again', async () => {
  const uid = await a.evaluate(() => yap.settings.uid);
  const name = await a.evaluate(() => yap.state.users.get(yap.settings.uid)?.name);
  for (const answer of ['ok', 'cancel']) {
    // A friend posts a link with a pass of their own (here: a's), naming someone else.
    const pass = await a.evaluate(() => new Promise((resolve) => {
      yap.state.handoffWaiter = resolve;
      yap.ui.send({ t: 'device.handoff' });
    }));
    const ctx = await newContext({ viewport: { width: 900, height: 640 } }, `someone's sign-in link (${answer})`);
    const p = await ctx.newPage();
    await p.goto(`${BASE}/#key=${encodeURIComponent(KEY)}&handoff=${encodeURIComponent(pass)}&name=Somebody%20Else`);
    await p.waitForSelector('#confirm-dialog[open]', { timeout: 10000 });
    assert.equal(await p.textContent('#confirm-title'), `Signed in as ${name}`);
    if (answer === 'ok') {
      await p.click('#confirm-ok');
      await p.waitForFunction(() => !document.getElementById('confirm-dialog').open);
      assert.equal(await p.evaluate(() => yap.settings.uid), uid, 'that’s me: stays signed in');
    } else {
      await p.click('#confirm-cancel');
      await p.waitForSelector('#welcome-dialog[open]', { timeout: 10000 });
      assert.equal(await p.evaluate(() => yap.settings.token), '', 'not me: signed out again');
    }
    await ctx.close();
  }
});

await test('one look: a saved Apple or Arch skin opens as Windows 98, keeping light or dark; Settings has no skin picker', async () => {
  // What an older page left in storage (or what an older iPhone/Mac app still writes before every load).
  for (const [skin, theme] of [['apple', 'light'], ['arch', 'dark'], ['apple', 'system']]) {
    const ctx = await newContext({ viewport: { width: 900, height: 640 }, colorScheme: 'light' }, `saved ${skin} skin`);
    await ctx.addInitScript(([k, skin, theme]) => {
      if (window !== window.top || !location.origin.startsWith('http')) return;
      const s = JSON.parse(localStorage.getItem('yap.settings') || '{}');
      localStorage.setItem('yap.settings', JSON.stringify({ ...s, key: k, skin, theme, mode: 'open' }));
    }, [KEY, skin, theme]);
    const p = await ctx.newPage();
    await p.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
    await p.waitForSelector('#welcome-dialog[open]', { timeout: 10000 });
    const look = await p.evaluate(() => ({
      theme: document.documentElement.dataset.theme,
      skinAttr: document.documentElement.dataset.skin ?? null,
      sheets: [...document.styleSheets].map((s) => s.href && new URL(s.href).pathname).filter(Boolean),
      title: document.querySelector('#welcome-dialog h2').textContent,
      submit: document.querySelector('#welcome-form button[type=submit]').textContent,
      font: getComputedStyle(document.body).fontFamily,
      saved: JSON.parse(localStorage.getItem('yap.settings')).skin ?? null,
      inMemory: yap.settings.skin ?? null,
    }));
    assert.deepEqual(look.sheets, ['/style.css'], 'only the Windows 98 stylesheet');
    assert.equal(look.theme, theme, 'their light/dark choice stays');
    assert.equal(look.skinAttr, null);
    assert.equal(look.title, 'Sign On', 'Windows 98 wording');
    assert.match(look.font, /Tahoma/);
    assert.equal(look.inMemory, null, 'the old skin is dropped');
    assert.equal(look.saved, null, 'and not saved back');
    await p.screenshot({ path: path.join(OUT, `saved-${skin}-${theme}.png`) });
    await ctx.close();
  }
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [data-tab="0"]');
  assert.equal(await a.locator('#settings-form [name=skin]').count(), 0, 'no skin picker');
  assert.equal(await a.locator('#settings-form [name=theme]').count(), 3, 'light, dark, match system');
  await a.keyboard.press('Escape');
});

await test('Personal space: notes to self stay private', async () => {
  const mine = await a.evaluate(() => `p_${yap.settings.uid}`);
  // Not encrypted yet (the test server keeps data between runs).
  await a.evaluate(() => yap.ui.send({ t: 'vault.set', wrap: null }));
  await a.waitForFunction(() => !yap.state.vault.wrap);
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a.waitForSelector('.personal-warning');
  const note = `note to self ${Date.now()}`;
  await a.fill('#input', note);
  await a.press('#input', 'Enter');
  await a.waitForSelector(`.msg .body:text("${note}")`, { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-personal.png') });
  await b.waitForTimeout(500);
  assert.equal(await b.evaluate((t) => [...yap.state.history.values()].some((h) => h.messages.some((m) => m.body === t)), note), false, 'Ben never gets it');
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('replies: Reply on a message, the answer quotes it, pings the author, the quote jumps back', async () => {
  const q = `anyone up for a game at 9? ${Date.now()}`;
  await a.fill('#input', q);
  await a.press('#input', 'Enter');
  const row = b.locator('.msg', { has: b.locator(`.body:text("${q}")`) });
  await row.waitFor({ timeout: 5000 });
  await row.hover();
  await row.locator('[data-action="reply-message"]').click();
  await b.waitForSelector('#reply-bar:not([hidden]):has-text("Replying to Ada")');
  assert.match(await b.textContent('#reply-bar .reply-snippet'), /^anyone up for a game at 9\?/, 'the bar says what you answer');
  await b.fill('#input', `count me in ${RUN}`);
  await b.press('#input', 'Enter');
  assert.equal(await b.isHidden('#reply-bar'), true);
  const answer = a.locator('.msg', { has: a.locator(`.body:text-is("count me in ${RUN}")`) });
  await answer.locator(`.reply-quote:has-text("${q.slice(0, 20)}")`).waitFor({ timeout: 5000 });
  assert.ok(await answer.evaluate((el) => el.classList.contains('mentions-me')), 'a reply to Ada pings Ada');
  await a.screenshot({ path: path.join(OUT, 'desktop-reply.png') });
  await answer.locator('.reply-quote').click();
  await a.waitForSelector('.msg.flash');
});

await test('long messages: up to 40,000 characters; long ones fold behind "Show more"', async () => {
  const long = Array.from({ length: 80 }, (_, i) => `line ${i + 1}: ${'all work and no play '.repeat(4)}`).join('\n');
  await a.fill('#input', long);
  await a.press('#input', 'Enter');
  const msg = b.locator('.msg', { has: b.locator('.body.folded') }).last();
  await msg.waitFor({ timeout: 5000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-long-folded.png') });
  await msg.locator('.show-more').click();
  await b.waitForFunction(() => ![...document.querySelectorAll('.msg .body')].at(-1).classList.contains('folded'));
  await a.fill('#input', 'x'.repeat(40_001));
  await a.press('#input', 'Enter');
  await a.waitForSelector('#toast:not([hidden]):has-text("40,000")');
  assert.equal((await a.inputValue('#input')).length, 40_001, 'nothing lost: the text stays in the box');
  await a.fill('#input', '');
});

await test('People: a sheet with everyone A–Z, no online/offline anywhere', async () => {
  await a.click('[data-action="people"]');
  await a.waitForSelector('#people-dialog[open] .member:has(.name:text-is("Ben"))');
  const sheet = await a.locator('#people-dialog').innerText();
  assert.ok(!/\b(online|offline)\b/i.test(sheet), sheet);
  assert.ok(!/\bonline\b/i.test(await a.locator('#user-panel').innerText()), 'no "Online" on the nameplate');
  assert.equal(await a.locator('.presence').count(), 0);
  assert.equal(await a.locator('aside.members').count(), 0, 'no member column');
  await a.click('#people-dialog .member:has(.name:text-is("Ben"))');
  await a.waitForSelector('#people-dialog .person-controls input[type=range]');
  await a.screenshot({ path: path.join(OUT, 'desktop-people.png') });
  await a.click('[data-action="close-people"]');
});

await test('waiting room: Let in from People moves them from "Waiting to join" to Everyone', async () => {
  // With Ben's personal link (Ada has used up her invite links for now): the second person on the
  // same link gets a "check first" note.
  await b.evaluate(() => {
    document.querySelector('#invite-link').value = '';
    yap.ui.send({ t: 'invite.create', label: 'for a friend' });
  });
  await b.waitForFunction(() => /#key=yi_/.test(document.querySelector('#invite-link')?.value ?? ''), null, { timeout: 5000 });
  const link = await b.inputValue('#invite-link');
  const name = `Wren ${RUN}`;
  /** A newcomer signs up with the link and waits. */
  const arrive = async (who) => {
    const ctx = await newContext({ viewport: { width: 1280, height: 800 } }, who);
    const page = await ctx.newPage();
    await page.goto(link);
    await page.waitForSelector('#welcome-dialog[open]');
    await page.fill('#welcome-form input[name=name]', who);
    await fillPassword(page);
    await page.click('#welcome-form button[type=submit]');
    await page.waitForFunction(() => yap.state.status === 'waiting', null, { timeout: 10000 });
    return { ctx, page };
  };
  await ensureOwner(a);
  const { ctx, page } = await arrive(name);
  await page.waitForSelector('#gate-text:has-text("Waiting for the owner or a mod to let you in.")');
  await a.waitForSelector(`#requests .request:has-text("${name}")`, { timeout: 5000 });
  // Ben is a member: only the owner and mods let people in, so he sees no request, no badge, no Let in.
  await b.waitForTimeout(500);
  assert.equal(await b.isHidden('#requests'), true, 'no request bar for members');
  assert.equal(await b.isHidden('#waiting-badge'), true, 'no waiting badge for members');
  assert.equal(await b.evaluate(() => yap.state.waiting.size), 0, 'the server never told him');
  await b.click('[data-action="people"]');
  await b.waitForSelector('#people-dialog[open] .member:has(.name:text-is("Ada"))');
  assert.equal(await b.locator('#people-dialog .member.waiting, #people-dialog [data-action="approve"]').count(), 0, 'nothing to let in from People');
  await b.click('[data-action="close-people"]');
  assert.equal(await a.locator(`#requests .request:has-text("${name}") .invite-warn`).count(), 0, 'a first use is fine');
  await a.click('[data-action="people"]');
  const waiting = `#people-dialog .member.waiting:has(.name:text-is("${name}"))`;
  await a.waitForSelector(waiting, { timeout: 5000 });
  await a.click(`${waiting} [data-action="approve"]`);
  await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  await a.waitForSelector(waiting, { state: 'detached', timeout: 5000 });
  await a.waitForSelector(`#people-dialog .member:not(.waiting):has(.name:text-is("${name}"))`, { timeout: 5000 });
  await a.click('[data-action="close-people"]');
  // Others see a quiet line in the first channel (no popup); the newcomer starts caught up and is welcomed.
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.waitForSelector(`.local-note:has-text("${name} joined — invited by Ben")`, { timeout: 5000 });
  await b.click('[data-action="open-channel"][data-id="general"]');
  await b.waitForSelector(`.local-note:has-text("${name} joined — invited by Ben")`, { timeout: 5000 });
  await page.waitForSelector('.local-note:has-text("You\'re in.")', { timeout: 5000 });
  assert.equal(await page.locator(`.local-note:has-text("${name} joined")`).count(), 0, 'no "joined" line about yourself');
  assert.equal(await page.locator('#text-channels .channel.unread').count(), 0, 'a new device starts with nothing unread');
  assert.ok(await page.isHidden('#catchup-row'));
  // The same link again: whoever lets them in is told who used it before.
  const second = await arrive(`Moss ${RUN}`);
  await a.waitForSelector(`#requests .request:has-text("Moss ${RUN}") .invite-warn:has-text("This link was also used by ${name} — check with Ben")`, { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-request-reused-link.png') });
  await a.click(`#requests .request:has-text("Moss ${RUN}") [data-action="deny"]`);
  await a.waitForSelector(`#requests .request:has-text("Moss ${RUN}")`, { state: 'detached', timeout: 5000 });
  await second.ctx.close();
  await ctx.close();
});

await test('passwords: sign-up says 8 or more; an old PIN is asked once to choose one (Later is for good); a reset hands out a temporary one', async () => {
  const name = `Pip ${RUN}`;
  const fresh = async (label) => {
    const ctx = await newContext({ viewport: { width: 1280, height: 800 } }, label);
    const page = await ctx.newPage();
    await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
    await page.waitForSelector('#welcome-dialog[open]');
    return { ctx, page };
  };
  const submit = async (page, mode, password) => {
    await page.click(`#welcome-form label:has(input[value=${mode}])`);
    await page.fill('#welcome-form input[name=name]', name);
    await fillPassword(page, password);
    await page.click('#welcome-form button[type=submit]');
  };
  // Sign up: too short is caught on the page; a guessable one gets the server's words.
  const first = await fresh(`${name}, phone`);
  await submit(first.page, 'signup', 'short');
  await first.page.waitForSelector('#welcome-error:not([hidden]):text-is("Use a password of at least 8 characters.")');
  await submit(first.page, 'signup', 'password123');
  await first.page.waitForSelector('#welcome-error:not([hidden]):has-text("That password is too easy to guess.")', { timeout: 8000 });
  await submit(first.page, 'signup', 'pip-meadow-cove-38');
  await first.page.waitForFunction(() => yap.state.status === 'waiting', null, { timeout: 10000 });
  const uid = await first.page.evaluate(() => yap.settings.uid);
  await adminPost(`/api/admin/role?uid=${uid}&role=member`);
  await first.page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  assert.equal(await first.page.isHidden('#weak-bar'), true, 'a good password: nothing to ask');
  // Pip was here before passwords: an old 6-digit PIN (the test server's TEST_PINS).
  const made = await adminPost(`/api/admin/test-pin?uid=${uid}&pin=582039`);
  if (made.status === 404) throw new Error('start the test server with --var TEST_PINS:1 (npm run test:all does)');
  // Signing in with the PIN on a laptop: asked once; Later hides it there for good (even after a reload).
  const laptop = await fresh(`${name}, laptop`);
  await submit(laptop.page, 'login', '582039');
  await laptop.page.waitForSelector('#weak-bar:not([hidden])', { timeout: 10000 });
  assert.equal(await laptop.page.textContent('#weak-bar b'), 'Choose a password');
  assert.equal(await laptop.page.textContent('#weak-bar .prompt-text'), 'Yap uses passwords now. Your PIN still works until you choose one.');
  await laptop.page.screenshot({ path: path.join(OUT, 'desktop-choose-password.png') });
  await laptop.page.click('#weak-bar [data-action="password-later"]');
  assert.equal(await laptop.page.isHidden('#weak-bar'), true);
  await laptop.page.reload();
  await laptop.page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  await laptop.page.waitForTimeout(300);
  assert.equal(await laptop.page.isHidden('#weak-bar'), true, 'Later is for good on this device');
  // On a tablet: Choose a password → "Set a password", the PIN counts as the current one.
  const tablet = await fresh(`${name}, tablet`);
  await submit(tablet.page, 'login', '582039');
  await tablet.page.waitForSelector('#weak-bar:not([hidden])', { timeout: 10000 });
  await tablet.page.click('#weak-bar [data-action="change-pin"]');
  await tablet.page.waitForSelector('#pin-dialog[open]');
  assert.equal(await tablet.page.textContent('#pin-title'), 'Set a password');
  assert.equal(await tablet.page.textContent('#pin-current-label'), 'Current password or PIN');
  assert.deepEqual(await tablet.page.$eval('#pin-form input[name=pin]', (el) => [el.type, el.autocomplete]), ['password', 'new-password']);
  await tablet.page.screenshot({ path: path.join(OUT, 'desktop-set-password.png') });
  await tablet.page.fill('#pin-form input[name=current]', '582039');
  await tablet.page.fill('#pin-form input[name=pin]', 'qwertyuiop');
  await tablet.page.click('#pin-save');
  await tablet.page.waitForSelector('#pin-error:not([hidden]):has-text("That password is too easy to guess.")', { timeout: 5000 });
  await tablet.page.fill('#pin-form input[name=pin]', 'otter-lagoon-kelp-71');
  await tablet.page.click('#pin-save');
  await tablet.page.waitForSelector('#toast:not([hidden]):has-text("Password saved.")', { timeout: 5000 });
  assert.equal(await tablet.page.isHidden('#weak-bar'), true, 'nothing more to ask');
  assert.equal(await tablet.page.textContent('#change-pin-btn'), 'Change password');
  // The owner resets Pip's password: a temporary one to pass on.
  await ensureOwner(a);
  await a.click('[data-action="people"]');
  await a.click(`#people-dialog .member:has(.name:text-is("${name}"))`);
  await a.click('#people-dialog .person-controls [data-action="reset-pin"]:text-is("Reset password…")');
  await a.click('#confirm-dialog[open] #confirm-ok');
  await a.waitForSelector('#confirm-dialog[open] #confirm-title:text-is("Temporary password")', { timeout: 5000 });
  const said = await a.textContent('#confirm-text');
  const temp = said.match(/: ([a-z]+-[a-z]+-[a-z]+-\d{2})\. /)?.[1];
  assert.equal(said, `${name}’s temporary password: ${temp}. Send it to ${name}; they’ll choose their own after signing in.`);
  await a.screenshot({ path: path.join(OUT, 'desktop-temporary-password.png') });
  await a.click('#confirm-dialog[open] #confirm-cancel');
  await a.click('[data-action="close-people"]');
  // Signing in with it: the prompt says it's temporary.
  const phone = await fresh(`${name}, new phone`);
  await submit(phone.page, 'login', temp);
  await phone.page.waitForSelector('#weak-bar:not([hidden])', { timeout: 10000 });
  assert.equal(await phone.page.textContent('#weak-bar .prompt-text'), 'You signed in with a temporary password. Choose your own.');
  for (const c of [first, laptop, tablet, phone]) await c.ctx.close();
});

await test("When's good: paint your times, the other person sees the heat map and the best time", async () => {
  for (const p of [a, b]) {
    await p.click('[data-action="open-view"][data-view="when"]');
    await p.waitForSelector('#when-view:not([hidden]) .when-mine');
    await p.click('[data-when="week"][data-week="1"]'); // next week: every cell is in the future
    await p.click('[data-when="clear"]');
  }
  // 2–8 AM fold away (unless someone is free then); the best times sit above the grids.
  const nightRows = () => a.locator('.when-mine .when-cell[data-h="3"]').count();
  if (await a.locator('.when-mine .when-fold').count()) {
    assert.equal(await nightRows(), 0, 'night hours folded by default');
    await a.click('.when-mine .when-fold');
    assert.equal(await nightRows(), 7, 'Show night hours');
    await a.click('.when-night');
    assert.equal(await nightRows(), 0, 'Hide night hours');
  }
  assert.ok(await a.evaluate(() => document.querySelector('.when-foot').compareDocumentPosition(document.querySelector('.when-grids')) & Node.DOCUMENT_POSITION_FOLLOWING), 'best times above the grids');
  const drag = async (p, [d1, h1], [d2, h2]) => {
    const box = async (d, h) => p.locator(`.when-mine .when-cell[data-d="${d}"][data-h="${h}"]`).boundingBox();
    const from = await box(d1, h1);
    const to = await box(d2, h2);
    await p.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await p.mouse.down();
    await p.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 6 });
    await p.mouse.up();
  };
  await drag(a, [1, 19], [3, 21]); // Tue–Thu, 7–9 PM
  await a.waitForFunction(() => document.querySelectorAll('.when-mine .when-cell.on').length === 9);
  await b.waitForFunction(() => [...document.querySelectorAll('.when-group .when-cell')].filter((c) => Number(c.style.getPropertyValue('--level')) > 0).length === 9, null, { timeout: 5000 });
  await drag(b, [2, 20], [2, 22]); // Wed 8–10 PM
  await a.waitForSelector('.when-best:has-text("Wed 8 PM–10 PM")', { timeout: 5000 });
  await a.click('.when-group .when-cell[data-d="2"][data-h="20"]');
  await a.waitForSelector('.when-foot:has-text("2/2 free")');
  // Turn it into a plan: everyone sees it (and one reminder goes out when it starts).
  await a.click('.when-foot [data-when="plan"]');
  const plan = `Game night ${Date.now() % 10000}`;
  await a.fill('.when-plan input[name=title]', plan);
  await a.click('.when-plan button[type=submit]');
  const ours = `.hang-list li:has(b:text-is("${plan}"))`;
  await b.waitForSelector(ours, { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-when.png') });
  await a.click(`${ours} [data-when="cancel-hang"]`);
  await a.click('#confirm-dialog[open] #confirm-ok');
  await b.waitForSelector(ours, { state: 'detached', timeout: 5000 });
  for (const p of [a, b]) await p.click('[data-when="clear"]');
  for (const p of [a, b]) await p.click('[data-action="open-channel"][data-id="general"]');
});

await test('encrypted Personal space: set a passcode, notes are sealed, the server sees only e1:, another device unlocks', async () => {
  const mine = await a.evaluate(() => `p_${yap.settings.uid}`);
  // Start from "not encrypted" (the test server keeps data between runs).
  await a.evaluate(() => yap.ui.send({ t: 'vault.set', wrap: null }));
  await a.waitForFunction(() => !yap.state.vault.wrap);
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  // Long notes from before: 8 × 40,000 characters need more than one frame to seal (25 at a time didn't fit, and said it worked).
  // (The earlier tests used some of the server's send allowance: what it turns away goes again by itself.)
  for (let i = 0; i < 8; i++) await a.evaluate(([id, body]) => yap.ui.send({ t: 'send', channelId: id, body }), [mine, `${i} ${RUN} `.padEnd(40_000, 'long note ')]);
  await a.waitForFunction(([id, run]) => yap.state.history.get(id)?.messages.filter((m) => m.body.length === 40_000 && m.body.includes(run)).length === 8, [mine, RUN], { timeout: 20000 });
  await a.click('.personal-warning [data-action="vault-setup"]');
  await a.waitForSelector('#vault-dialog[open]');
  await a.fill('#vault-form input[name=pass]', 'short');
  await a.fill('#vault-form input[name=again]', 'short');
  await a.click('#vault-ok');
  await a.waitForSelector('#vault-error:not([hidden])');
  await a.fill('#vault-form input[name=pass]', 'purple tiger river 42');
  await a.fill('#vault-form input[name=again]', 'purple tiger river 42');
  await a.click('#vault-ok');
  await a.waitForFunction(() => !document.querySelector('#vault-dialog').open && yap.state.vault.key, null, { timeout: 15000 });
  await a.waitForSelector('.personal-note:has-text("End-to-end encrypted")');
  // Notes from before encryption get sealed too (the earlier test wrote one).
  await a.waitForSelector('#toast:has-text("earlier notes are encrypted too")', { timeout: 20000 });
  const secret = `wifi at the cabin: cabin-${Date.now()}`;
  await a.fill('#input', secret);
  await a.press('#input', 'Enter');
  await a.waitForSelector(`.msg:not(.pending) .body:text("${secret}")`, { timeout: 10000 }); // stored, not just shown
  await a.screenshot({ path: path.join(OUT, 'desktop-personal-encrypted.png') });
  if (process.env.YAP_ADMIN_KEY) {
    const dump = await (await fetch(`${BASE}/api/admin/export?after=0`, { headers: { Authorization: `Bearer ${process.env.YAP_ADMIN_KEY}` } })).json();
    const notes = dump.messages.filter((m) => m.channelId === mine);
    assert.ok(notes.at(-1).body.startsWith('e1:'), 'stored sealed');
    assert.ok(notes.every((m) => m.deletedAt || m.body.startsWith('e1:')), 'earlier notes sealed too');
    assert.ok(!JSON.stringify(dump).includes(secret), 'the server never sees the text');
  }
  // Same person, new device: locked until the passcode. (And Ada's first device hears about the sign-in.)
  const a2 = await user('Ada', '#5b6cff');
  await a.waitForSelector('#signin-bar:not([hidden]):has-text("New sign-in")', { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-signin-alert.png') });
  await a.click('#signin-bar [data-action="dismiss-signin"]');
  await a2.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a2.waitForSelector('.msg .body.sealed:has-text("Unlock")');
  assert.equal(await a2.isDisabled('#input'), true, 'locked: nothing to write with');
  // A locked note is in the quiet grey.
  const [color, want] = await a2.evaluate(() => {
    const note = document.querySelector('.msg .body.sealed');
    const probe = note.parentElement.appendChild(Object.assign(document.createElement('span'), { style: 'color: var(--gray)' }));
    const out = [getComputedStyle(note).color, getComputedStyle(probe).color];
    probe.remove();
    return out;
  });
  assert.equal(color, want, 'a locked note is --gray');
  await a2.screenshot({ path: path.join(OUT, 'desktop-personal-locked.png') });
  await a2.click('.personal-note [data-action="vault-unlock"]');
  await a2.fill('#vault-form input[name=pass]', 'purple tiger river 4');
  await a2.click('#vault-ok');
  await a2.waitForSelector('#vault-error:not([hidden]):has-text("doesn’t open")', { timeout: 15000 });
  await a2.fill('#vault-form input[name=pass]', 'purple tiger river 42');
  await a2.click('#vault-ok');
  await a2.waitForSelector(`.msg .body:text("${secret}")`, { timeout: 15000 });
  // A new passcode on one device locks the others until they enter it.
  await a2.evaluate(() => document.querySelector('[data-action="vault-change"]').click());
  await a2.fill('#vault-form input[name=old]', 'purple tiger river 42');
  await a2.fill('#vault-form input[name=pass]', 'green ocean lamp 7');
  await a2.fill('#vault-form input[name=again]', 'green ocean lamp 7');
  await a2.click('#vault-ok');
  await a2.waitForFunction(() => !document.querySelector('#vault-dialog').open, null, { timeout: 15000 });
  await a.waitForFunction(() => yap.state.vault.wrap && !yap.state.vault.key, null, { timeout: 5000 });
  await a.waitForSelector('.msg .body.sealed:has-text("Unlock")');
  await a.click('.personal-note [data-action="vault-unlock"]');
  await a.fill('#vault-form input[name=pass]', 'green ocean lamp 7');
  await a.click('#vault-ok');
  await a.waitForSelector(`.msg .body:text("${secret}")`, { timeout: 15000 });
  await a2.context().close();
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.isDisabled('#input'), false, 'other channels never lock');
});

await test('a message the server turns away leaves no grey copy, and its text comes back to the empty message box', async () => {
  // Too long to keep encrypted (the box doesn't check sealed notes; the server does).
  const mine = await a.evaluate(() => `p_${yap.settings.uid}`);
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a.waitForFunction(() => yap.state.vault.key, null, { timeout: 5000 });
  const long = `${`packing list ${RUN}: `.padEnd(200_000, 'sleeping bag, torch, snacks, ').trimEnd()}.`;
  await a.fill('#input', long);
  await a.press('#input', 'Enter');
  await a.waitForSelector('#toast:not([hidden]):has-text("too long to keep encrypted")', { timeout: 10000 });
  await a.waitForFunction(() => !yap.state.pending.size && !yap.state.outbox.length, null, { timeout: 5000 });
  assert.equal(await a.locator('.msg.pending').count(), 0, 'no grey copy stuck as sending');
  assert.equal(await a.inputValue('#input'), long, 'the text is back in the box');
  await a.screenshot({ path: path.join(OUT, 'desktop-send-refused.png') });
  // The next one goes out as usual.
  const note = `short list ${RUN}`;
  await a.fill('#input', note);
  await a.press('#input', 'Enter');
  await a.waitForSelector(`.msg:not(.pending) .body:text-is("${note}")`, { timeout: 10000 });
  // A poll shows no copy while it's sent, but a refused one comes back too (questions are up to 300 characters).
  await a.click('[data-action="open-channel"][data-id="general"]');
  const poll = `/poll ${'Which day works best for the cabin trip? '.repeat(8)}| Friday | Saturday`;
  await a.fill('#input', poll);
  await a.press('#input', 'Enter');
  await a.waitForSelector('#toast:not([hidden]):has-text("up to 300 characters")', { timeout: 5000 });
  assert.equal(await a.inputValue('#input'), poll, 'the poll is back in the box');
  await a.fill('#input', '');
});

await test('your devices: the list shows each sign-in; sign one out', async () => {
  const extra = await user('Ada', '#5b6cff');
  await a.click('#signin-bar [data-action="dismiss-signin"]', { timeout: 5000 });
  await a.evaluate(() => document.querySelector('[data-action="devices"]').click());
  await a.waitForSelector('#devices-dialog[open] .device:has-text("(this device)")');
  const before = await a.locator('#devices-list .device').count();
  assert.ok(before >= 2, `${before} devices`);
  await a.screenshot({ path: path.join(OUT, 'desktop-devices.png') });
  const closed = extra.waitForFunction(() => yap.state.status !== 'online', null, { timeout: 8000 });
  await a.locator('#devices-list .device.new [data-action="revoke-device"]').first().click();
  await a.waitForFunction((n) => document.querySelectorAll('#devices-list .device').length === n - 1, before);
  await closed;
  await extra.context().close();
  await a.click('[data-action="close-devices"]');
});

await test('privacy page is there and linked from the sidebar', async () => {
  assert.equal(await a.locator('a[href="/privacy"]').count(), 1);
  const res = await fetch(`${BASE}/privacy`);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /Who can see what/);
  const p = await a.context().newPage();
  await p.goto(`${BASE}/privacy`);
  await p.screenshot({ path: path.join(OUT, 'privacy.png'), fullPage: true });
  await p.close();
});

await test('search finds a message and jumps to it; pins show up for everyone', async () => {
  const word = `kumquat${Date.now() % 100000}`;
  await a.fill('#input', `who wants a ${word} tart?`);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body:has-text("${word}")`, { timeout: 5000 });
  for (let i = 0; i < 6; i++) {
    await a.fill('#input', `filler line ${i}`);
    await a.press('#input', 'Enter');
  }
  // Ctrl/⌘+K is the quick switcher; its last row searches messages.
  await b.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await b.waitForSelector('#switcher-dialog[open]');
  await b.keyboard.type(word);
  await b.waitForSelector(`.switcher-row.active:has-text("Search messages for")`);
  await b.keyboard.press('Enter');
  await b.waitForSelector('#search-dialog[open]');
  assert.equal(await b.inputValue('#search-input'), word);
  // Nothing found in this channel: one click searches everywhere.
  await b.fill('#search-input', `zebra ${RUN}`);
  await b.waitForSelector('#search-results li.muted:has-text("Nothing for")', { timeout: 5000 });
  await b.click('#search-results [data-action="search-everywhere"]');
  await b.waitForSelector('#search-results li.muted:has-text("anywhere")', { timeout: 5000 });
  assert.equal(await b.evaluate(() => document.querySelector('#search-form').scope.value), 'all');
  await b.click('#search-form input[value=here]');
  await b.fill('#search-input', word);
  await b.waitForSelector(`#search-results .result-text mark:text("${word}")`, { timeout: 5000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-search.png') });
  await b.click('#search-results [data-action="open-result"]');
  await b.waitForSelector('.msg.flash', { timeout: 5000 });
  const row = b.locator('.msg', { has: b.locator(`.body:has-text("${word}")`) });
  await row.hover();
  await row.locator('[data-action="pin-message"]').click();
  await a.waitForSelector(`.msg:has(.body:has-text("${word}")) .pin-mark`, { timeout: 5000 });
  await a.click('[data-action="pins"]');
  await a.waitForSelector(`#pins-list .result-text:has-text("${word}")`);
  await a.screenshot({ path: path.join(OUT, 'desktop-pins.png') });
  await a.click('#pins-list [data-action="pin-message"]');
  await a.waitForSelector('#pins-list li.muted:has-text("Nothing pinned")', { timeout: 5000 });
  await a.click('[data-action="close-pins"]');
  await b.waitForFunction((w) => !document.querySelector('.msg .pin-mark') || ![...document.querySelectorAll('.msg')].some((m) => m.textContent.includes(w) && m.querySelector('.pin-mark')), word);
});

await test('extras: spoilers, a poll, server dice, a doodle reaction, the YouTube button, save to Personal', async () => {
  for (const p of [a, b]) await p.click('[data-action="open-channel"][data-id="general"]');
  const send = async (p, text) => {
    await p.fill('#input', text);
    await p.press('#input', 'Enter');
    await p.waitForTimeout(1100); // the send limit is 1 per second after a burst
  };
  await send(a, `the twist ${RUN}: ||it was a dream all along||`);
  const spoiler = `.msg .body:has-text("the twist ${RUN}") .spoiler`;
  await b.waitForSelector(`${spoiler}:not(.revealed)`, { timeout: 5000 });
  assert.equal(await b.locator(spoiler).evaluate((el) => getComputedStyle(el).color === 'rgba(0, 0, 0, 0)' || getComputedStyle(el).color === 'transparent'), true, 'hidden until clicked');
  await b.click(spoiler);
  await b.waitForSelector(`${spoiler}.revealed`);
  await send(a, `/poll Lunch ${RUN}? | pizza | tacos`);
  const pollOn = (p) => p.locator('.msg', { has: p.locator(`.body:text-is("Lunch ${RUN}?")`) }).locator('.poll-option:has-text("tacos")');
  await pollOn(b).waitFor({ timeout: 5000 });
  await pollOn(b).click();
  await pollOn(a).locator('.poll-count', { hasText: /^1\b/ }).waitFor({ timeout: 5000 }); // Ben's vote, on Ada's screen
  const rollsBefore = await a.evaluate(() => yap.state.history.get('general').messages.filter((m) => m.roll).length);
  await send(b, '/roll 2d6');
  await a.waitForFunction((n) => yap.state.history.get('general').messages.filter((m) => m.roll && !m.pending).length > n, rollsBefore, { timeout: 5000 });
  const rollMsg = await a.evaluate(() => yap.state.history.get('general').messages.filter((m) => m.roll).at(-1));
  assert.equal(rollMsg.roll.rolls.length, 2);
  const rowOf = (p, id) => p.locator(`.msg:has([data-action="pin-message"][data-id="${id}"])`);
  await rowOf(a, rollMsg.id).locator('.roll .roll-total').waitFor({ timeout: 5000 });
  await send(b, `trailer ${RUN} https://youtu.be/dQw4w9WgXcQ`);
  await a.waitForSelector(`.msg:has(.body:has-text("trailer ${RUN}")) .yt-watch`, { timeout: 5000 });
  // A doodle reaction on Ben's roll: draw, React; Ben sees the picture.
  const rollRow = rowOf(a, rollMsg.id);
  await rollRow.hover();
  await rollRow.locator('[data-action="doodle-open"]').click();
  const pad = await a.locator('#doodle-pad canvas').boundingBox();
  await a.mouse.move(pad.x + 30, pad.y + 30);
  await a.mouse.down();
  await a.mouse.move(pad.x + 150, pad.y + 150, { steps: 10 });
  await a.mouse.up();
  await a.click('[data-doodle="send"]');
  await rowOf(b, rollMsg.id).locator('.doodle-reaction img').waitFor({ timeout: 8000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-extras.png') });
  // Save to Personal: a bookmark that quotes and jumps back.
  const pollRow = b.locator('.msg', { has: b.locator(`.body:text-is("Lunch ${RUN}?")`) });
  await pollRow.hover();
  await pollRow.locator('[data-action="save-personal"]').click();
  const mine = await b.evaluate(() => `p_${yap.settings.uid}`);
  await b.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await b.waitForSelector(`.reply-quote.saved:has-text("Lunch ${RUN}?")`, { timeout: 5000 });
  await b.click(`.reply-quote.saved:has-text("Lunch ${RUN}?")`);
  await b.waitForFunction(() => yap.settings.channel === 'general', null, { timeout: 5000 });
  await b.waitForSelector('.msg.flash', { timeout: 5000 });
});

await test('quick switcher: type a few letters, Enter opens it', async () => {
  const mine = await a.evaluate(() => `p_${yap.settings.uid}`);
  await a.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await a.waitForSelector('#switcher-dialog[open]');
  await a.keyboard.type('persn');
  await a.waitForSelector('.switcher-row.active:has-text("Personal space")');
  await a.screenshot({ path: path.join(OUT, 'desktop-switcher.png') });
  await a.keyboard.press('Enter');
  await a.waitForFunction((id) => yap.settings.channel === id, mine, { timeout: 5000 });
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('away: set in Settings, everyone sees it in People, mentioning you shows the line, your next message clears it', async () => {
  await b.click('[data-action="open-settings"]');
  await b.click('#settings-tabs [role=tab]:text-is("Profile")');
  await b.fill('#settings-form input[name=awayText]', 'grabbing food 🍜');
  await b.click('[data-action="away-set"]');
  await b.waitForSelector('#away-now:has-text("grabbing food")');
  await b.click('#settings-form .settings-actions .btn');
  const ben = await b.evaluate(() => yap.state.users.get(yap.settings.uid).name);
  await a.fill('#input', `@${ben} you there?`);
  await a.press('#input', 'Enter');
  await a.waitForSelector('.away-note:has-text("grabbing food")', { timeout: 5000 });
  await a.click('[data-action="people"]');
  await a.waitForSelector('#members .away-sub:has-text("grabbing food")');
  await a.click('[data-action="close-people"]');
  await b.evaluate(() => yap.ui.send({ t: 'profile', away: null })); // (posting clears it too, after 2 minutes)
  const benUid = await b.evaluate(() => yap.settings.uid);
  await a.waitForFunction((uid) => !yap.state.users.get(uid)?.away, benUid, { timeout: 5000 });
});

await test('catch up: two unread channels → the list, one click starts at the first unread', async () => {
  const name = (n) => `catch-${n}-${Date.now() % 10000}`;
  const made = [];
  for (const n of [1, 2]) {
    const channel = name(n);
    await a.evaluate((c) => yap.ui.send({ t: 'channel.create', name: c, kind: 'text' }), channel);
    await b.waitForFunction((c) => [...yap.state.channels.values()].some((x) => x.name === c), channel, { timeout: 5000 });
    made.push(await b.evaluate((c) => [...yap.state.channels.values()].find((x) => x.name === c).id, channel));
  }
  await a.waitForTimeout(1100);
  for (const id of made) {
    await a.evaluate((id) => yap.ui.send({ t: 'send', channelId: id, body: `catch-up news in ${id}` }), id);
    await a.waitForTimeout(1100);
  }
  await b.waitForSelector('#catchup-row:not([hidden])', { timeout: 5000 });
  await b.click('#catchup-row');
  await b.waitForSelector('#catchup-list .result-open', { timeout: 5000 });
  assert.ok(await b.locator('#catchup-list .result-open').count() >= 2);
  await b.screenshot({ path: path.join(OUT, 'desktop-catchup.png') });
  await b.locator('#catchup-list .result-open').first().click();
  await b.waitForSelector('.msg.flash', { timeout: 5000 });
  for (const id of made) await a.evaluate((id) => yap.ui.send({ t: 'channel.archive', channelId: id }), id);
  for (const p of [a, b]) await p.click('[data-action="open-channel"][data-id="general"]');
});

await test('"Jump to first unread" shows as soon as the "New since" line is above the screen (not a header\'s height later)', async () => {
  await b.setViewportSize({ width: 1280, height: 360 });
  try {
    const seen = await b.evaluate(async () => {
      const { renderMessages, renderJumpPill } = await import('/js/app/messages.js');
      const channel = yap.settings.channel;
      const scroller = document.getElementById('messages');
      // The first message from someone else that leaves room to scroll its line off the top.
      let at = null;
      for (const m of yap.state.history.get(channel).messages.slice(2).filter((m) => m.uid !== yap.settings.uid)) {
        yap.state.since.set(channel, m.ts - 1);
        renderMessages();
        const divider = document.getElementById('new-divider');
        at = Math.ceil(divider.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop); // (scrollTop is whole pixels)
        if (at + 10 <= scroller.scrollHeight - scroller.clientHeight) break;
      }
      const pill = (top) => {
        scroller.scrollTop = top;
        renderJumpPill();
        return { top: scroller.scrollTop, shown: !document.getElementById('jump-pill').hidden };
      };
      const result = { above: pill(at + 10), visible: pill(Math.max(0, at - 40)), at };
      yap.state.since.delete(channel);
      renderMessages({ keep: 'bottom' });
      return result;
    });
    assert.ok(seen.above.top >= seen.at + 10, `the list scrolls far enough for this check (${JSON.stringify(seen)})`);
    assert.equal(seen.above.shown, true, 'the line is 10 px above the screen: the pill shows');
    assert.equal(seen.visible.shown, false, 'the line is on screen: no pill');
  } finally {
    await b.setViewportSize({ width: 1280, height: 800 });
  }
});

await test('each channel keeps its own draft: switching brings back what you typed there, and Enter never sends it to the wrong channel', async () => {
  const mine = await a.evaluate(() => `p_${yap.settings.uid}`);
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.fill('#input', `half a thought for general ${RUN}`);
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  assert.equal(await a.inputValue('#input'), '', 'the Personal space starts empty');
  await a.fill('#input', `a note to self ${RUN}`);
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.inputValue('#input'), `half a thought for general ${RUN}`, 'general’s draft is back');
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  assert.equal(await a.inputValue('#input'), `a note to self ${RUN}`);
  await a.press('#input', 'Enter');
  await a.waitForSelector(`.msg .body:text("a note to self ${RUN}")`, { timeout: 5000 });
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.inputValue('#input'), `half a thought for general ${RUN}`, 'sending one channel’s draft leaves the other alone');
  assert.equal(await a.locator(`.msg .body:text("half a thought for general ${RUN}")`).count(), 0, 'nothing went to general');
  await a.fill('#input', '');
  // A reply in progress stays with its channel.
  const last = await a.evaluate(() => yap.state.history.get('general').messages.filter((m) => !m.pending).at(-1).id);
  await a.locator(`.msg:has([data-action="reply-message"][data-id="${last}"])`).hover();
  await a.click(`[data-action="reply-message"][data-id="${last}"]`);
  await a.waitForSelector('#reply-bar:not([hidden])');
  await a.click(`[data-action="open-channel"][data-id="${mine}"]`);
  assert.equal(await a.isHidden('#reply-bar'), true, 'no reply bar in another channel');
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('"↓ New messages" shows when someone writes below while you’re scrolled up; a click goes down and it’s gone', async () => {
  await b.setViewportSize({ width: 1280, height: 420 });
  try {
    await a.click('[data-action="open-channel"][data-id="general"]');
    await b.click('[data-action="open-channel"][data-id="general"]');
    // Enough to scroll.
    for (let i = 0; i < 12; i++) await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `filler ${i} ${RUN}`);
    await b.waitForSelector(`.msg .body:text("filler 11 ${RUN}")`, { timeout: 10000 });
    // At the bottom: a new message just appears, no pill.
    await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `seen at once ${RUN}`);
    await b.waitForSelector(`.msg .body:text("seen at once ${RUN}")`, { timeout: 5000 });
    assert.equal(await b.isHidden('#new-pill'), true, 'at the bottom: no pill');
    await b.evaluate(() => (document.getElementById('messages').scrollTop = 0));
    await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `something new ${RUN}`);
    await b.waitForSelector('#new-pill:not([hidden])', { timeout: 5000 });
    assert.equal(await b.textContent('#new-pill'), '↓ New messages');
    for (const scheme of ['light', 'dark']) {
      await b.evaluate((t) => yap.ui.setTheme(t), scheme);
      await b.screenshot({ path: path.join(OUT, `desktop-new-messages-${scheme}.png`) });
    }
    await b.evaluate(() => yap.ui.setTheme('dark'));
    // Keyboard reachable, and it's in a polite live region.
    assert.equal(await b.locator('#new-pill').evaluate((el) => el.closest('[aria-live]')?.getAttribute('aria-live')), 'polite');
    await b.click('#new-pill');
    await b.waitForFunction(() => { const s = document.getElementById('messages'); return s.scrollHeight - s.scrollTop - s.clientHeight < 80; }, null, { timeout: 5000 });
    await b.waitForSelector('#new-pill', { state: 'hidden' });
    // Scrolling down yourself hides it too; switching channels clears it.
    await b.evaluate(() => (document.getElementById('messages').scrollTop = 0));
    await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `and again ${RUN}`);
    await b.waitForSelector('#new-pill:not([hidden])', { timeout: 5000 });
    await b.evaluate(() => { const s = document.getElementById('messages'); s.scrollTop = s.scrollHeight; });
    await b.waitForSelector('#new-pill', { state: 'hidden' });
    await b.evaluate(() => (document.getElementById('messages').scrollTop = 0));
    await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `once more ${RUN}`);
    await b.waitForSelector('#new-pill:not([hidden])', { timeout: 5000 });
    const mine = await b.evaluate(() => `p_${yap.settings.uid}`);
    await b.click(`[data-action="open-channel"][data-id="${mine}"]`);
    assert.equal(await b.isHidden('#new-pill'), true, 'another channel: no pill');
    await b.click('[data-action="open-channel"][data-id="general"]');
    assert.equal(await b.isHidden('#new-pill'), true, 'and none when you come back (you open at the bottom)');
  } finally {
    await b.setViewportSize({ width: 1280, height: 800 });
  }
});

await test('a tapped browser push opens its message: from a closed Yap (#channel=…&message=…) and from an open tab (the service worker asks it)', async () => {
  const target = await a.evaluate(() => yap.state.history.get('general').messages.find((m) => !m.pending && m.body?.startsWith('filler 2 ')));
  assert.ok(target, 'a message to open');
  // Closed: the service worker opens /#channel=…&message=…
  const page = await b.context().newPage();
  await page.goto(`${BASE}/#channel=general&message=${target.id}`);
  await page.waitForSelector(`.msg.flash:has([data-id="${target.id}"])`, { timeout: 10000 });
  assert.equal(new URL(page.url()).hash, '#channel=general', 'the address is tidied up: just where you are now');
  await page.close();
  // Open: the service worker posts { t: 'yap.open' } to the tab.
  const mine = await b.evaluate(() => `p_${yap.settings.uid}`);
  await b.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await b.evaluate((id) => navigator.serviceWorker.dispatchEvent(new MessageEvent('message', { data: { t: 'yap.open', channelId: 'general', messageId: id } })), target.id);
  await b.waitForSelector(`.msg.flash:has([data-id="${target.id}"])`, { timeout: 10000 });
  assert.equal(await b.evaluate(() => yap.settings.channel), 'general');
});

await test('the installed app’s badge counts your unread @mentions', async () => {
  const page = await b.context().newPage();
  await page.addInitScript(() => {
    window.badges = [];
    navigator.setAppBadge = (n) => (window.badges.push(n), Promise.resolve());
    navigator.clearAppBadge = () => (window.badges.push(0), Promise.resolve());
  });
  await page.goto(BASE);
  await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 15000 });
  const mine = await page.evaluate(() => `p_${yap.settings.uid}`);
  await page.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `@Ben badge me ${RUN}`);
  await page.waitForFunction(() => window.badges.at(-1) === 1, null, { timeout: 5000 });
  await a.evaluate((t) => yap.ui.send({ t: 'send', channelId: 'general', body: t }), `@Ben and again ${RUN}`);
  await page.waitForFunction(() => window.badges.at(-1) === 2, null, { timeout: 5000 });
  await page.click('[data-action="open-channel"][data-id="general"]');
  await page.waitForFunction(() => window.badges.at(-1) === 0, null, { timeout: 5000 });
  await page.close();
});

await test('mentions while you were away: the badge comes back after a reload; Catch up lists them first; Mark all read', async () => {
  const mine = await b.evaluate(() => `p_${yap.settings.uid}`);
  await b.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.fill('#input', `@Ben are you around? ${RUN}`);
  await a.press('#input', 'Enter');
  await b.waitForSelector('[data-action="open-channel"][data-id="general"] .mention-badge', { timeout: 5000 });
  // A fresh page has no live count: the server's count brings the badge back.
  await b.reload();
  await b.waitForFunction(() => yap.state.status === 'online', null, { timeout: 15000 });
  await b.waitForSelector('[data-action="open-channel"][data-id="general"] .mention-badge:text-is("1")', { timeout: 5000 });
  await b.evaluate(() => yap.ui.send({ t: 'channel.create', name: `quiet-${Date.now() % 100000}`, kind: 'text' }));
  const quiet = await b.waitForFunction(() => [...yap.state.channels.values()].find((c) => c.name.startsWith('quiet-'))?.id, null, { timeout: 5000 }).then((h) => h.jsonValue());
  await a.evaluate((id) => yap.ui.send({ t: 'send', channelId: id, body: 'nothing for anyone in particular' }), quiet);
  await b.waitForSelector(`[data-action="open-channel"][data-id="${quiet}"].unread`, { timeout: 5000 });
  await b.click('#catchup-row');
  await b.waitForSelector('#catchup-list .result-open', { timeout: 5000 });
  assert.match(await b.locator('#catchup-list .result-meta').first().innerText(), /^general · \d+ new · 1 for you$/, 'mentions first');
  await b.click('#catchup-read-all');
  await b.waitForSelector('#catchup-dialog', { state: 'hidden' });
  assert.equal(await b.locator('#text-channels .channel.unread').count(), 0);
  assert.equal(await b.locator('#text-channels .mention-badge').count(), 0);
  await a.evaluate((id) => yap.ui.send({ t: 'channel.archive', channelId: id }), quiet);
  await b.click('[data-action="open-channel"][data-id="general"]');
});

await test('quiet hours: Settings › Notifications saves them on the server', async () => {
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Notifications")');
  await a.check('#settings-form input[name=quietOn]');
  await a.waitForFunction(() => yap.state.notify?.quiet?.from === 23 * 60 && yap.state.notify.quiet.to === 8 * 60, null, { timeout: 5000 });
  await a.click('[data-action="quiet-snooze"]');
  await a.waitForFunction(() => yap.state.notify?.quiet?.until > Date.now(), null, { timeout: 5000 });
  await a.uncheck('#settings-form input[name=quietOn]');
  await a.click('[data-action="quiet-unsnooze"]');
  await a.waitForFunction(() => yap.state.notify?.quiet === null, null, { timeout: 5000 });
  // Phone pushes: subscribe on this server's ntfy (NTFY_SERVER), not always ntfy.sh.
  const ntfy = (await (await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': KEY } })).json()).ntfy.server;
  await a.selectOption('#settings-form select[name=notifyMode]', 'pings');
  await a.waitForSelector('#ntfy-help:not([hidden])', { timeout: 5000 });
  assert.ok((await a.getAttribute('#ntfy-open', 'href')).startsWith(`${ntfy}/yap-`), 'Open in ntfy');
  assert.equal(await a.textContent('#ntfy-host'), new URL(ntfy).host);
  await a.selectOption('#settings-form select[name=notifyMode]', 'off');
  await a.waitForFunction(() => yap.state.notify?.mode === 'off', null, { timeout: 5000 });
  await a.click('#settings-form .settings-actions .btn');
});

await test('unread: a channel you are not in lights up (dot, bright name) until you open it', async () => {
  const mine = await b.evaluate(() => `p_${yap.settings.uid}`);
  await b.click(`[data-action="open-channel"][data-id="${mine}"]`);
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.fill('#input', `news for Ben ${Date.now()}`);
  await a.press('#input', 'Enter');
  await b.waitForSelector('[data-action="open-channel"][data-id="general"].unread .unread-dot', { timeout: 5000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-unread.png') });
  await b.click('[data-action="open-channel"][data-id="general"]');
  await b.waitForFunction(() => !document.querySelector('[data-action="open-channel"][data-id="general"] .unread-dot'));
});

await test("What's new opens with highlights from the past week", async () => {
  const p = await a.context().newPage();
  await p.goto(`${BASE}/changelog`);
  await p.waitForSelector('#highlights:not([hidden]) li');
  assert.match(await p.locator('#highlights h2').innerText(), /Highlights from the past week/);
  await p.screenshot({ path: path.join(OUT, 'changelog-highlights.png'), fullPage: true });
  await p.close();
});

await test('typing indicator appears', async () => {
  await b.type('#input', 'typing...');
  await a.waitForSelector('#typing:has-text("Ben is typing")', { timeout: 5000 });
  await b.fill('#input', '');
});

await test('offline: a message you send waits, says so, and goes out when you are back', async () => {
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.context().setOffline(true);
  await a.evaluate(() => yap.ui.dropSocket());
  await a.waitForFunction(() => yap.state.status === 'offline', null, { timeout: 5000 });
  assert.match(await a.textContent('#banner'), /go out when you’re back/);
  const text = `sent while offline ${RUN}`;
  await a.fill('#input', text);
  await a.press('#input', 'Enter');
  await a.waitForSelector(`.msg.pending:has(.body:text-is("${text}")) .pending-note:has-text("Waiting to send")`, { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-offline-pending.png') });
  await a.context().setOffline(false);
  await b.waitForSelector(`.msg .body:text-is("${text}")`, { timeout: 15000 });
  await a.waitForSelector(`.msg:not(.pending):has(.body:text-is("${text}"))`, { timeout: 5000 });
  assert.equal(await a.locator('.pending-note').count(), 0);
});

await test('12 messages in a burst: the server takes 10 and says slow down, the rest go out a moment later, each once, in order', async () => {
  for (const p of [a, b]) await p.click('[data-action="open-channel"][data-id="general"]');
  // The account's whole allowance (10 at once, then one a second; per account, so a reconnect adds nothing): long refilled by now.
  const slowDowns = [];
  const listen = (ws) => ws.on('framereceived', ({ payload }) => String(payload).includes('"slow_down"') && slowDowns.push(payload));
  a.on('websocket', listen);
  await a.evaluate(() => yap.ui.dropSocket());
  await a.waitForFunction(() => yap.state.status === 'online' && yap.state.history.get('general')?.loading === false, null, { timeout: 15000 });
  const texts = Array.from({ length: 12 }, (_, i) => `burst ${i + 1} of 12 ${RUN}`);
  for (const text of texts) {
    await a.fill('#input', text);
    await a.press('#input', 'Enter');
  }
  await a.waitForFunction(() => !yap.state.pending.size && !yap.state.outbox.length, null, { timeout: 15000 });
  a.off('websocket', listen);
  assert.ok(slowDowns.length > 0, 'the server did turn some away');
  await a.waitForSelector(`.msg:not(.pending) .body:text-is("${texts.at(-1)}")`, { timeout: 5000 });
  assert.equal(await a.locator('.msg.pending').count(), 0, 'nothing left stuck as sending');
  const burst = (page) => page.evaluate((run) => yap.state.history.get('general').messages.map((m) => m.body).filter((t) => t.startsWith('burst ') && t.endsWith(run)), RUN);
  await b.waitForFunction((run) => yap.state.history.get('general').messages.filter((m) => m.body.startsWith('burst ') && m.body.endsWith(run)).length >= 12, RUN, { timeout: 5000 });
  await b.waitForTimeout(1500); // a copy sent twice would be here by now
  assert.deepEqual(await burst(b), texts, 'Ben has every one, once, in order');
  assert.deepEqual(await burst(a), texts);
  await a.screenshot({ path: path.join(OUT, 'desktop-send-burst.png') });
});

await test('@ suggests people first (with their pictures); @everyone only once you type it, and last', async () => {
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.fill('#input', '');
  await a.focus('#input');
  await a.keyboard.type('thanks @');
  await a.waitForSelector('#emoji-suggest:not([hidden]) .suggest-row');
  const rows = await a.locator('#emoji-suggest .suggest-row').allInnerTexts();
  assert.ok(!rows.some((r) => /everyone/.test(r)), `no @everyone for a bare @: ${rows.join(' | ')}`);
  assert.ok(await a.locator('#emoji-suggest .suggest-row.active .avatar').count(), 'people come with their picture');
  await a.keyboard.press('Enter');
  assert.doesNotMatch(await a.inputValue('#input'), /@everyone/, '@ + Enter picks a person');
  await a.fill('#input', '');
  await a.keyboard.type('hey @ev');
  await a.waitForSelector('#emoji-suggest:not([hidden]) .suggest-row:last-child:has-text("everyone")');
  await a.keyboard.press('Escape');
  await a.fill('#input', '');
});

await test('GIF picker: trending, search, send, share report', async () => {
  await a.click('[data-action="toggle-gifs"]');
  await a.waitForFunction(() => document.querySelectorAll('#gif-picker .gif-tile').length >= 12, null, { timeout: 5000 });
  assert.ok(klipyLog.some((l) => l.startsWith('GET /gifs/trending')), 'trending requested');
  await a.screenshot({ path: path.join(OUT, 'desktop-gif-picker.png') });
  await a.fill('#gif-picker .gif-search', 'cats');
  await a.waitForSelector('#gif-picker .gif-tile[title="cats 0"]', { timeout: 5000 });
  assert.ok(klipyLog.some((l) => l.startsWith('GET /gifs/search') && l.includes('q=cats')), 'search requested');
  const gifsBefore = await b.locator('.msg .gif-embed[title="cats 0"]').count();
  await a.click('#gif-picker .gif-tile[title="cats 0"]');
  assert.equal(await a.isHidden('#gif-picker'), true, 'picker closes after picking');
  // It waits in the preview tray: nothing sent yet; a caption goes with it.
  await a.waitForSelector('#staged:not([hidden]) .gif-embed[title="cats 0"]');
  await a.screenshot({ path: path.join(OUT, 'desktop-gif-preview.png') });
  await b.waitForTimeout(500);
  assert.equal(await b.locator('.msg .gif-embed[title="cats 0"]').count(), gifsBefore, 'not sent before you press Send');
  await a.fill('#input', `meow ${RUN}`);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg:has(.body:text-is("meow ${RUN}")) .gif-embed[title="cats 0"]`, { timeout: 5000 });
  assert.equal(await a.isHidden('#staged'), true, 'tray empties after sending');
  assert.ok(klipyLog.some((l) => l.startsWith('POST /gifs/share/cats-0')), 'share reported to KLIPY');
});

await test('GIF rows survive new messages (no re-render flicker)', async () => {
  await b.evaluate(() => (window.__gifEl = document.querySelector('.msg .gif-embed[title="cats 0"]')));
  const text = `after the gif ${Date.now()}`;
  await a.fill('#input', text);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body:text("${text}")`, { timeout: 5000 });
  assert.equal(await b.evaluate(() => document.querySelector('.msg .gif-embed[title="cats 0"]') === window.__gifEl), true);
});

const emojiName = `party_${Math.random().toString(36).slice(2, 7)}`;
await test('custom emoji: upload via picker, autocomplete, renders for others', async () => {
  // Draw a little emoji in the page and hand it to the hidden file input.
  const dataUrl = await a.evaluate(() => {
    const c = Object.assign(document.createElement('canvas'), { width: 256, height: 256 });
    const g = c.getContext('2d');
    g.fillStyle = '#f5a524'; g.beginPath(); g.arc(128, 128, 120, 0, Math.PI * 2); g.fill();
    g.fillStyle = '#1b1d23'; g.beginPath(); g.arc(88, 100, 16, 0, 7); g.arc(168, 100, 16, 0, 7); g.fill();
    g.lineWidth = 14; g.strokeStyle = '#1b1d23'; g.beginPath(); g.arc(128, 140, 60, 0.15 * Math.PI, 0.85 * Math.PI); g.stroke();
    // A random-coloured corner keeps the image unique per run (same bytes = 'duplicate');
    // big enough to survive the picker shrinking it.
    g.fillStyle = `rgb(${Math.random() * 255 | 0},${Math.random() * 255 | 0},${Math.random() * 255 | 0})`; g.fillRect(0, 0, 40, 40);
    return c.toDataURL('image/png');
  });
  await a.click('[data-action="toggle-emojis"]');
  await a.waitForSelector('#emoji-picker .emoji-empty:has-text("No custom emojis yet")', { timeout: 5000 }).catch(() => {});
  await a.setInputFiles('#emoji-picker input[type=file]', {
    name: 'Big Grin.png', mimeType: 'image/png', buffer: Buffer.from(dataUrl.split(',')[1], 'base64'),
  });
  await a.waitForSelector('#emoji-picker .emoji-upload:not([hidden])', { timeout: 5000 });
  assert.equal(await a.inputValue('#emoji-picker .emoji-upload input[name=name]'), 'big_grin');
  await a.fill('#emoji-picker .emoji-upload input[name=name]', emojiName);
  await a.click('#emoji-picker .emoji-upload button[type=submit]');
  await a.waitForSelector(`#emoji-picker .emoji-tile[title=":${emojiName}:"]`, { timeout: 5000 });
  const size = await a.evaluate(async (name) => {
    const id = [...yap.state.emojis.values()].find((e) => e.name === name).id;
    const blob = await (await fetch(`/emoji/${id}`)).blob();
    const bmp = await createImageBitmap(blob);
    return [bmp.width, bmp.height, blob.type];
  }, emojiName);
  assert.deepEqual(size.slice(0, 2), [128, 128], 'big images are scaled to 128 px');
  await a.screenshot({ path: path.join(OUT, 'desktop-emoji-picker.png') });
  await a.keyboard.press('Escape');

  await a.click('#input');
  await a.type('#input', `nice :${emojiName.slice(0, -1)}`); // unique prefix (older test emojis also start with "party")
  await a.waitForSelector(`#emoji-suggest:not([hidden]) .suggest-row:has-text(":${emojiName}:")`, { timeout: 5000 });
  await a.press('#input', 'Tab');
  assert.equal(await a.inputValue('#input'), `nice :${emojiName}: `);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body img.emoji[alt=":${emojiName}:"]`, { timeout: 5000 });
  await a.fill('#input', `:${emojiName}::${emojiName}:`);
  await a.press('#input', 'Enter');
  await b.waitForSelector(`.msg .body.jumbo img.emoji[alt=":${emojiName}:"]`, { timeout: 5000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-emoji-chat.png') });
});

await test('standard emojis: picker, :shortcode: autocomplete, convert on send', async () => {
  await a.fill('#input', '');
  await a.click('[data-action="toggle-emojis"]');
  await a.waitForSelector('#emoji-picker [data-section="0"] [data-uni="😀"]', { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-emoji-standard.png') });
  await a.click('#emoji-picker [data-jump="3"]'); // Food & drink tab
  await a.click('#emoji-picker [data-uni="🍕"]');
  assert.equal(await a.inputValue('#input'), '🍕');
  assert.equal(await a.isHidden('#emoji-picker'), true);
  await a.type('#input', ' :thumbsu');
  await a.waitForSelector('#emoji-suggest:not([hidden]) .suggest-row:has-text(":thumbsup:")', { timeout: 5000 });
  await a.press('#input', 'Tab');
  assert.match(await a.inputValue('#input'), /^🍕 👍️? $/u);
  await a.type('#input', `lol ${RUN} :joy: \`:joy:\``);
  await a.press('#input', 'Escape');
  await a.press('#input', 'Enter');
  const msg = await b.waitForSelector(`.msg .body:has-text("lol ${RUN} 😂")`, { timeout: 5000 });
  assert.match(await msg.textContent(), new RegExp(`lol ${RUN} 😂 :joy:$`, 'u'), 'converted outside code, left alone inside code');
  await a.click('[data-action="toggle-emojis"]');
  await a.waitForSelector('#emoji-picker [data-section="recent"] [data-uni="🍕"]', { timeout: 5000 });
  await a.fill('#emoji-picker .emoji-search', 'pizza');
  await a.waitForSelector('#emoji-picker [data-section="results"] [data-uni="🍕"]', { timeout: 5000 });
  await a.keyboard.press('Escape');
});

await test('Settings › Voice before any call: Copy last call report is disabled, "No call yet."', async () => {
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Voice")');
  await a.waitForSelector('#call-report-copy:disabled', { timeout: 3000 });
  assert.equal(await a.textContent('#call-report-copy'), 'Copy last call report');
  assert.equal(await a.isVisible('#call-report-none'), true);
  assert.equal(await a.textContent('#call-report-none'), 'No call yet.');
  assert.equal(await a.textContent('#call-report-field > small'), 'What Yap noticed about your last call: connections, lost audio, devices. No messages or IP addresses. Send it along when a call sounded off.');
  await a.locator('#call-report-field').screenshot({ path: path.join(OUT, 'call-report-settings-none.png') });
  await a.click('#settings-form .settings-actions .btn');
});

await test('Settings › Voice › Test mic: the button shows it\'s testing, the bar moves, Stop test lets the mic go; in every input mode; a blocked mic says so', async () => {
  const width = () => a.evaluate(() => parseFloat(document.querySelector('#meter-fill').style.width) || 0);
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Voice")');
  // Ada is on "Always on": the level and Test mic are there too (they used to be voice activity only).
  assert.equal(await a.evaluate(() => yap.settings.mode), 'open');
  await a.waitForSelector('#mic-test-btn:visible');
  assert.equal(await a.isVisible('[name=vadThreshold]'), false, 'the sensitivity is for voice activity');
  await a.click('#mic-test-btn');
  await a.waitForSelector('#mic-test-btn[aria-pressed="true"]:text-is("Stop test")', { timeout: 3000 });
  await a.waitForFunction(() => /^Listening/.test(document.querySelector('#mic-test-status').textContent), null, { timeout: 8000 });
  await until(async () => (await width()) > 20, 5000, 'the bar to move');
  await a.locator('#settings-form .mic-level').screenshot({ path: path.join(OUT, 'MW4-after.png') });
  await a.click('#mic-test-btn');
  await a.waitForSelector('#mic-test-btn[aria-pressed="false"]:text-is("Test mic")');
  assert.equal(await width(), 0, 'stopped: the bar goes back');
  assert.equal(await a.evaluate(() => yap.voice.rawStream), null, 'and the mic is let go');
  // The browser blocks the mic: the line says so, and how to fix it.
  await a.evaluate(() => {
    globalThis.__gum = navigator.mediaDevices.getUserMedia;
    navigator.mediaDevices.getUserMedia = () => Promise.reject(new DOMException('Permission denied', 'NotAllowedError'));
  });
  await a.click('#mic-test-btn');
  await a.waitForFunction(() => /blocked the microphone/.test(document.querySelector('#mic-test-status').textContent), null, { timeout: 5000 });
  assert.equal(await a.getAttribute('#mic-test-btn', 'aria-pressed'), 'false');
  await a.evaluate(() => (navigator.mediaDevices.getUserMedia = globalThis.__gum));
  await a.click('#settings-form .settings-actions .btn');
});

await test('Settings › Voice › Advanced › Quality: 256 and 510 kbps say what they cost; the note says each voice goes out at its own setting', async () => {
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Voice")');
  await a.click('#voice-advanced summary');
  const quality = '#voice-advanced label.field:has(select[name=bitrate])';
  assert.equal(await a.isHidden('#bitrate-warn'), true, `${BITRATE / 1000} kbps: nothing to warn about`);
  assert.equal(await a.textContent(`${quality} small.muted`), "Voice goes straight between you and your friends, so better quality costs the server nothing. Your voice goes out at your setting, and each friend's at theirs.");
  for (const value of ['256000', '510000']) {
    await a.selectOption('#settings-form select[name=bitrate]', value);
    assert.equal(await a.textContent('#bitrate-warn:visible'), 'No dropout protection at this quality (each bit of voice no longer fits twice in a packet), and several times the upload for each friend in the call.', value);
  }
  await a.locator(quality).screenshot({ path: path.join(OUT, 'TW9-quality.png') });
  await a.selectOption('#settings-form select[name=bitrate]', String(BITRATE));
  assert.equal(await a.isHidden('#bitrate-warn'), BITRATE < 256000);
  await a.click('#settings-form .settings-actions .btn');
});

await test('Settings › Voice › Test mic: a mic whose peaks keep hitting the top says "Too loud (clipping)"; a normal one never does', async () => {
  // The test mic as it is (its peaks at −4 dBFS): the browser's automatic gain lifts this test signal
  // to full scale (measured: 0 dBFS peaks), which RNNoise, on here before, used to hide.
  const autoGain = await a.evaluate(() => {
    const was = yap.settings.autoGain;
    yap.settings.autoGain = false;
    return was;
  });
  try {
    await a.click('[data-action="open-settings"]');
    await a.click('#settings-tabs [role=tab]:text-is("Voice")');
    await a.click('#mic-test-btn');
    await until(() => a.evaluate(() => parseFloat(document.querySelector('#meter-fill').style.width) > 20), 5000, 'the bar to move');
    await a.waitForTimeout(2500);
    assert.equal(await a.isHidden('#mic-clip'), true, 'the test mic peaks around −4 dBFS: fine');
    await a.click('#mic-test-btn');
    // The same mic turned up four times: it clips.
    await a.evaluate(() => {
      const md = navigator.mediaDevices;
      globalThis.__plainMic = md.getUserMedia;
      md.getUserMedia = async (c) => {
        const s = await globalThis.__plainMic.call(md, c);
        const ctx = new AudioContext({ sampleRate: 48000 });
        const gain = ctx.createGain();
        gain.gain.value = 4;
        const dest = ctx.createMediaStreamDestination();
        dest.channelCount = 1;
        ctx.createMediaStreamSource(s).connect(gain).connect(dest);
        return dest.stream;
      };
    });
    await a.click('#mic-test-btn');
    await a.waitForSelector('#mic-clip:visible:text-is("Too loud (clipping)")', { timeout: 6000 });
    assert.equal(await a.evaluate(() => document.querySelector('#meter-fill').classList.contains('clip')), true, 'the bar goes red');
    await a.locator('#settings-form .mic-level').screenshot({ path: path.join(OUT, 'TW8-clipping.png') });
    await a.click('#mic-test-btn');
    assert.equal(await a.isHidden('#mic-clip'), true, 'gone with the test');
    await a.evaluate(() => (navigator.mediaDevices.getUserMedia = globalThis.__plainMic));
    await a.click('#settings-form .settings-actions .btn');
  } finally {
    await a.evaluate((on) => {
      if (globalThis.__plainMic) navigator.mediaDevices.getUserMedia = globalThis.__plainMic;
      yap.settings.autoGain = on;
    }, autoGain).catch(() => {});
  }
});

await test('both join Lounge and connect peer to peer', async () => {
  await a.click('button.channel.voice:has-text("Lounge")');
  await a.waitForSelector('.vp-status.ok', { timeout: 10000 });
  await b.click('button.channel.voice:has-text("Lounge")');
  await a.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
  await b.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
  await b.waitForSelector('.vp-status.ok', { timeout: 5000 });
});

const inbound = (page) => page.evaluate(async () => {
  const peer = [...yap.voice.peers.values()][0];
  const report = await peer.pc.getStats();
  let rtp, codec;
  report.forEach((s) => {
    if (s.type === 'inbound-rtp' && s.kind === 'audio') rtp = s;
  });
  if (rtp) codec = report.get(rtp.codecId);
  return { bytes: rtp?.bytesReceived ?? 0, packets: rtp?.packetsReceived ?? 0, lost: rtp?.packetsLost ?? 0, mime: codec?.mimeType, fmtp: codec?.sdpFmtpLine, level: rtp?.audioLevel ?? 0, at: performance.now() };
});

let measured;
await test(`audio flows as tuned Opus at ~${BITRATE / 1000} kbps (inside RED)`, async () => {
  await b.waitForTimeout(3000); // let the bitrate ramp up
  const s1 = await inbound(b);
  await b.waitForTimeout(4000);
  const s2 = await inbound(b);
  const kbps = ((s2.bytes - s1.bytes) * 8) / (s2.at - s1.at);
  measured = { ...s2, kbps: Math.round(kbps) };
  assert.equal(s2.mime, 'audio/opus', 'Chrome names the Opus inside RED as the codec (next test: RED itself)');
  assert.match(s2.fmtp, /useinbandfec=1/);
  assert.match(s2.fmtp, new RegExp(`maxaveragebitrate=${BITRATE}`));
  assert.ok(s2.packets > 100, `packets received: ${s2.packets}`);
  assert.ok(s2.level > 0.001, `audio level: ${s2.level}`);
  // Payload bitrate should land near the target (VBR; allow a wide band).
  assert.ok(kbps > BITRATE / 1000 * 0.6, `measured ${kbps.toFixed(1)} kbps`);
});

await test(`RED on the voice: first in every description, both ways; each packet carries the frame before it; Opus stays at ${BITRATE / 1000} kbps`, async () => {
  const look = (page) => page.evaluate(async () => {
    const peer = [...yap.voice.peers.values()][0];
    // The codec each description lists first in its voice (first audio) section.
    const first = (d) => {
      const m = d.sdp.split(/\r\nm=/).find((x) => x.startsWith('audio'));
      return m.match(new RegExp(`a=rtpmap:${m.split('\r\n')[0].split(' ')[3]} ([^/]+)/`))?.[1];
    };
    const snap = async () => {
      let o, i;
      (await peer.pc.getStats()).forEach((x) => {
        if (x.type === 'outbound-rtp' && x.kind === 'audio') o = x;
        if (x.type === 'inbound-rtp' && x.kind === 'audio') i = x;
      });
      return { at: performance.now(), sent: o.bytesSent, sentPk: o.packetsSent, target: o.targetBitrate, got: i.bytesReceived, gotPk: i.packetsReceived };
    };
    const s1 = await snap();
    await new Promise((r) => setTimeout(r, 3000));
    const s2 = await snap();
    return {
      local: first(peer.pc.localDescription), remote: first(peer.pc.remoteDescription), target: s2.target,
      sentKbps: ((s2.sent - s1.sent) * 8) / (s2.at - s1.at),
      sentPerPacket: (s2.sent - s1.sent) / (s2.sentPk - s1.sentPk),
      gotPerPacket: (s2.got - s1.got) / (s2.gotPk - s1.gotPk),
    };
  });
  const [ada, ben] = await Promise.all([look(a), look(b)]);
  const opusPerPacket = BITRATE / 8 / 50; // a 20 ms Opus frame at the target
  // (Up to 256 kbps a frame and its copy fit a packet; at 510 kbps the copy is left out: RFC 2198 / PROTOCOL.md.)
  const copies = BITRATE <= 128000;
  for (const [who, s] of [['Ada', ada], ['Ben', ben]]) {
    assert.equal(s.local, 'red', `${who}'s description lists RED first`);
    assert.equal(s.remote, 'red', `${who} gets a description with RED first`);
    assert.equal(s.target, BITRATE, `${who}'s Opus target is the quality setting (RED's copy isn't counted against it)`);
    if (copies) {
      assert.ok(s.sentPerPacket > opusPerPacket * 1.7 && s.sentPerPacket < opusPerPacket * 2.3, `${who} sends ${s.sentPerPacket.toFixed(0)} bytes a packet (Opus alone: ${opusPerPacket})`);
      assert.ok(s.gotPerPacket > opusPerPacket * 1.7 && s.gotPerPacket < opusPerPacket * 2.3, `${who} gets ${s.gotPerPacket.toFixed(0)} bytes a packet`);
      assert.ok(s.sentKbps > (BITRATE / 1000) * 1.7, `${who} sends ${s.sentKbps.toFixed(0)} kbps of payload: Opus plus the copy`);
    }
  }
  measured.red = { sentKbps: Math.round(ada.sentKbps), perPacket: Math.round(ada.sentPerPacket), target: ada.target };
});

await test('no transport-cc on audio: 50 packets a second, no bandwidth-probe padding', async () => {
  const sdp = await a.evaluate(() => {
    const pc = [...yap.voice.peers.values()][0].pc;
    const audio = (d) => d.sdp.split(/\r\nm=/).filter((m) => m.startsWith('audio')).join('\n');
    return { local: audio(pc.localDescription), remote: audio(pc.remoteDescription) };
  });
  for (const side of ['local', 'remote']) assert.doesNotMatch(sdp[side], /transport-cc|transport-wide-cc/, `${side} audio has no transport-cc`);
  const sent = () => a.evaluate(async () => {
    let n = 0;
    (await [...yap.voice.peers.values()][0].pc.getStats()).forEach((s) => { if (s.type === 'outbound-rtp' && s.kind === 'audio') n += s.packetsSent; });
    return [performance.now(), n];
  });
  const [t1, n1] = await sent();
  await a.waitForTimeout(3000);
  const [t2, n2] = await sent();
  const pps = ((n2 - n1) * 1000) / (t2 - t1);
  assert.ok(pps > 45 && pps < 54, `${pps.toFixed(1)} packets/s (20 ms Opus frames, nothing else)`);
});

await test('push-to-talk keeps sending ~200 ms after the key is let go', async () => {
  await a.evaluate(() => {
    document.activeElement?.blur();
    // Watch what's actually sent: the gate's output track.
    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 256;
    ctx.createMediaStreamSource(new MediaStream([yap.voice.sendTrack])).connect(analyser);
    const buf = new Float32Array(256);
    globalThis.__sent = [];
    globalThis.__sentTimer = setInterval(() => {
      analyser.getFloatTimeDomainData(buf);
      __sent.push([performance.now(), Math.sqrt(buf.reduce((s, x) => s + x * x, 0) / buf.length)]);
    }, 5);
    addEventListener('keyup', () => (globalThis.__releasedAt = performance.now()), { once: true, capture: true });
  });
  try {
    // First, still always on: what's watched is what's sent, and there is something to send.
    await until(() => a.evaluate(() => __sent.length >= 40 && __sent.slice(-40).every(([, rms]) => rms > 0.001)), 3000, 'always on, it sends');
    await a.evaluate(() => (yap.settings.mode = 'ptt'));
    await a.waitForTimeout(500);
    await a.keyboard.down('Backquote');
    await a.waitForTimeout(700);
    await a.keyboard.up('Backquote');
    await a.waitForTimeout(800);
    const { tail, during } = await a.evaluate(() => {
      const after = __sent.filter(([t]) => t > __releasedAt);
      const lastLoud = after.filter(([, rms]) => rms > 0.001).at(-1)?.[0] ?? __releasedAt;
      const held = __sent.filter(([t]) => t > __releasedAt - 400 && t < __releasedAt);
      return { tail: lastLoud - __releasedAt, during: held.filter(([, rms]) => rms > 0.001).length / Math.max(1, held.length) };
    });
    assert.ok(during > 0.9, `sending while the key is down (${(during * 100).toFixed(0)}%)`);
    assert.ok(tail >= 150 && tail < 450, `still sending ${tail.toFixed(0)} ms after release`);
  } finally {
    await a.evaluate(() => {
      clearInterval(__sentTimer);
      yap.settings.mode = 'open';
    });
  }
});

await test('push-to-talk clicks (for you) when it opens and when it closes after the tail; a double-tap stays live ("Live — tap to stop") until the next press', async () => {
  try {
    await a.evaluate(() => {
      yap.settings.mode = 'ptt';
      document.activeElement?.blur();
      yap.state.pttClicks = [];
      addEventListener('keyup', (e) => e.code === 'Backquote' && (globalThis.__upAt = performance.now()), { capture: true });
    });
    const clicks = () => a.evaluate(() => yap.state.pttClicks.map((c) => ({ ...c, afterUp: c.at - (globalThis.__upAt ?? 0) })));
    const heard = async () => {
      await b.waitForTimeout(1200);
      return (await inbound(b)).level > 0.001;
    };
    await a.keyboard.down('Backquote');
    await a.waitForTimeout(400);
    await a.keyboard.up('Backquote');
    await a.waitForTimeout(700);
    let c = await clicks();
    assert.deepEqual(c.map((x) => x.open), [true, false], 'a click opening, a click closing');
    assert.ok(c[1].afterUp >= 150 && c[1].afterUp < 450, `the closing click after the tail: ${c[1].afterUp.toFixed(0)} ms after the key`);
    assert.equal(await heard(), false, 'closed again');
    // Double-tap: it stays open with the key up.
    await a.evaluate(() => (yap.state.pttClicks = []));
    await a.keyboard.press('Backquote');
    await a.waitForTimeout(120);
    await a.keyboard.press('Backquote');
    await a.waitForSelector('#user-panel .me-bubble:text-is("Live — tap to stop")', { timeout: 2000 });
    assert.equal(await heard(), true, 'live with the key up');
    assert.deepEqual((await clicks()).map((x) => x.open), [true], 'one click: it opened and stayed open');
    await a.locator('#user-panel').screenshot({ path: path.join(OUT, 'TW3-latched.png') });
    // One more tap: it closes (after the tail), with its click.
    await a.keyboard.press('Backquote');
    await a.waitForSelector('#user-panel .me-bubble', { state: 'detached', timeout: 2000 });
    assert.equal(await heard(), false, 'closed by the next tap');
    assert.deepEqual((await clicks()).map((x) => x.open), [true, false]);
    // Push-to-talk click off: no clicks (the latch still works).
    await a.evaluate(() => {
      yap.settings.pttClick = false;
      yap.state.pttClicks = [];
    });
    await a.keyboard.press('Backquote');
    await a.waitForTimeout(700);
    assert.deepEqual(await clicks(), [], 'switched off: silent');
    await a.evaluate(() => {
      yap.settings.pttClick = true;
      yap.settings.mode = 'open';
    });
  } finally {
    await voiceReset(a);
  }
});

await test('voice quality for the whole channel: picked by one, used by both', async () => {
  // It's under the voice panel's ⋯ (the panel only says so once it's set for everyone).
  assert.equal(await a.locator('#voice-panel .vp-shared').count(), 0);
  await a.click('#voice-panel [data-action="voice-more"]');
  assert.equal(await a.isHidden('#popover .vp-quality-warn'), true, 'no warning at each person\'s own');
  await a.selectOption('#popover .vp-quality', '256000');
  // Next to 256 and 510: what it costs (PROTOCOL.md › Voice).
  assert.equal(await a.textContent('#popover .vp-quality-warn:visible'), 'No dropout protection at this quality (each bit of voice no longer fits twice in a packet), and several times the upload for each friend in the call.');
  await b.waitForFunction(() => yap.state.channels.get('lounge')?.bitrate === 256000, null, { timeout: 5000 });
  await b.waitForSelector('#voice-panel .vp-shared:text-is("Sound: 256k for everyone")', { timeout: 5000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-voice-quality.png') });
  await a.selectOption('#popover .vp-quality', '');
  await b.waitForFunction(() => yap.state.channels.get('lounge')?.bitrate === null, null, { timeout: 5000 });
  await a.keyboard.press('Escape');
});

await test('Settings › Voice › Mic volume: 100 % sends what the mic gives, 50 % sends 6 dB less (and the bar shows it), live while you drag', async () => {
  // What goes out (the gate's output) against what the mic gives, both through an analyser, in back-to-back
  // ~0.7 s windows over 4 s: the test signal swells and fades every 2 s, and the two paths differ by a few ms.
  const sentVsMic = () => a.evaluate(async () => {
    const ctx = new AudioContext();
    const size = 32768;
    const tap = (track) => {
      const an = ctx.createAnalyser();
      an.fftSize = size;
      ctx.createMediaStreamSource(new MediaStream([track])).connect(an);
      return an;
    };
    const mic = tap(yap.voice.rawStream.getAudioTracks()[0]);
    const sent = tap(yap.voice.sendTrack);
    const buf = new Float32Array(size);
    const energy = (an) => {
      an.getFloatTimeDomainData(buf);
      return buf.reduce((s, x) => s + x * x, 0);
    };
    const windowMs = (size / ctx.sampleRate) * 1000;
    await new Promise((r) => setTimeout(r, windowMs));
    let micSum = 0;
    let sentSum = 0;
    let windows = 0;
    // The meter (the gate's own level, every 20 ms), over the same seconds.
    const meter = [];
    const reading = setInterval(() => meter.push(10 ** (yap.voice.localDb / 10)), 20);
    for (const end = performance.now() + 4000; performance.now() < end; windows++) {
      micSum += energy(mic);
      sentSum += energy(sent);
      await new Promise((r) => setTimeout(r, windowMs));
    }
    clearInterval(reading);
    ctx.close();
    const micPower = micSum / (windows * size);
    return { sent: 10 * Math.log10(sentSum / micSum), meter: 10 * Math.log10(meter.reduce((s, p) => s + p, 0) / meter.length / micPower) };
  });
  const setVolume = (pct) => a.evaluate((v) => {
    const slider = document.querySelector('#settings-form [name=micVolume]');
    slider.value = String(v);
    slider.dispatchEvent(new Event('input', { bubbles: true }));
  }, pct);
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Voice")');
  try {
    assert.equal(await a.textContent('#mic-volume-out'), '100%');
    const full = await sentVsMic();
    assert.ok(Math.abs(full.sent) < 1, `at 100 % what's sent is what the mic gives (${full.sent.toFixed(2)} dB)`);
    const tagged = () => a.evaluate(() => [yap.voice.rawStream, ...[...yap.voice.peers.values()].map((p) => p.pc)].map((o) => (o.__e2e ??= Math.random())));
    const before = await tagged();
    await setVolume(50);
    assert.equal(await a.textContent('#mic-volume-out'), '50%');
    assert.equal(await a.evaluate(() => yap.settings.micVolume), 0.5, 'saved for this device');
    const half = await sentVsMic();
    assert.ok(Math.abs(half.sent + 6.02) < 1, `at 50 % it's 6 dB down (${half.sent.toFixed(2)} dB)`);
    const bar = half.meter - full.meter;
    assert.ok(Math.abs(bar + 6.02) < 1.5, `the bar hears it too (${bar.toFixed(2)} dB; read every 20 ms, so a little rougher)`);
    assert.deepEqual(await tagged(), before, 'the same mic and the same connections (nothing reopened)');
    await a.locator('#settings-form label:has([name=micVolume])').screenshot({ path: path.join(OUT, 'mic-volume.png') });
  } finally {
    await setVolume(100).catch(() => {});
    await a.click('#settings-form .settings-actions .btn').catch(() => {});
  }
});

await test('speaking ring shows for the remote speaker', async () => {
  await b.waitForFunction(() => document.querySelector('.voice-member.speaking'), null, { timeout: 5000 });
});

await a.screenshot({ path: path.join(OUT, 'desktop-ada.png') });
await b.screenshot({ path: path.join(OUT, 'desktop-ben.png') });

await test('mute shows on the other side and silences audio', async () => {
  await a.click('[data-action="toggle-mute"]');
  await b.waitForFunction(() => document.querySelector('.voice-member .flags svg'), null, { timeout: 5000 });
  await b.waitForTimeout(1500);
  const level = (await inbound(b)).level;
  assert.ok(level < 0.001, `level while muted: ${level}`);
  await a.click('[data-action="toggle-mute"]');
});

await test("talking while muted: \"You're muted\" by the Mute button at most every 5 s, a soft blip only the first time since muting; the first time says what it means; not while another of your devices is live; the setting turns it off", async () => {
  try {
    await a.evaluate(() => {
      globalThis.__reminders = [];
      yap.voice.addEventListener('muted-talk', () => __reminders.push(performance.now()));
      yap.state.remindedMuted = 0;
      yap.state.mutedBlips = 0;
      yap.settings.mutedReminderExplained = false; // as on a device that never heard it
    });
    await b.evaluate(() => (yap.state.remindedMuted = 0));
    // Ada mutes and keeps talking (the test mic never stops).
    await a.click('[data-action="toggle-mute"]');
    // (Within 5 s of an earlier reminder — the mute test before this one talks while muted too — it waits its turn.)
    await a.waitForSelector('#user-panel .me-bubble:text-is("You\'re muted")', { timeout: 8000 });
    assert.equal(await a.textContent('#toast'), "That sound means you're muted. Turn it off in Settings › Voice.");
    assert.equal(await a.evaluate(() => yap.state.remindedMuted), 1);
    assert.equal(await a.evaluate(() => yap.state.mutedBlips), 1, 'the blip played once');
    await a.locator('#user-panel').screenshot({ path: path.join(OUT, 'TW1-muted-reminder.png') });
    await until(() => a.evaluate(() => yap.state.remindedMuted >= 2), 8000, 'a second reminder');
    const gaps = await a.evaluate(() => __reminders.slice(1).map((t, i) => t - __reminders[i]));
    assert.ok(gaps.every((g) => g >= 4990 && g < 6000), `5 s apart: ${gaps.map(Math.round).join(', ')} ms`);
    assert.equal(await a.evaluate(() => yap.state.mutedBlips), 1, 'the words again, without the blip');
    assert.equal(await a.evaluate(() => yap.settings.mutedReminderExplained), true, 'explained once on this device');
    assert.ok(await a.isHidden('#toast'), 'the explanation is not shown again (the first one went after 4 s)');
    // Another of Ada's devices in this call, unmuted: she's talking through that one, so nothing here.
    let before = await a.evaluate(() => {
      yap.state.sessions.set('s_other', { sid: 's_other', uid: yap.settings.uid, client: 'web', voice: { channelId: yap.state.voiceChannel, muted: false, deafened: false } });
      return yap.state.remindedMuted;
    });
    await a.waitForTimeout(6000);
    assert.equal(await a.evaluate(() => yap.state.remindedMuted), before, 'live on another device: no reminder');
    await a.evaluate(() => yap.state.sessions.delete('s_other'));
    // Off in Settings › Voice: nothing.
    await a.click('[data-action="open-settings"]');
    await a.click('#settings-tabs [role=tab]:text-is("Voice")');
    assert.equal(await a.isChecked('#settings-form [name=mutedReminder]'), true, 'on by default');
    await a.uncheck('#settings-form [name=mutedReminder]');
    await a.click('#settings-form .settings-actions .btn');
    before = await a.evaluate(() => yap.state.remindedMuted);
    await a.waitForTimeout(6000);
    assert.equal(await a.evaluate(() => yap.state.remindedMuted), before, 'switched off: no blip');
    assert.equal(await a.locator('#user-panel .me-bubble').count(), 0);
    await a.evaluate(() => yap.settings.mutedReminder = true);
    // Unmuted: the word goes at once; Ben (talking, never muted) never got one.
    await a.click('[data-action="toggle-mute"]');
    assert.equal(await a.locator('#user-panel .me-bubble').count(), 0);
    assert.equal(await b.evaluate(() => yap.state.remindedMuted), 0);
    // Muted again: the blip again, once.
    await a.click('[data-action="toggle-mute"]');
    await until(() => a.evaluate(() => yap.state.mutedBlips >= 2), 8000, 'a blip after muting again');
    await a.click('[data-action="toggle-mute"]');
  } finally {
    await voiceReset(a);
  }
});

await test('mute and deafen from the keyboard (Ctrl/⌘+Shift+M and +D, even while typing), listed on the buttons; push-to-mute mutes only while held, in any mode', async () => {
  try {
    const adaUid = await a.evaluate(() => yap.settings.uid);
    const adaOnBen = () => b.evaluate((uid) => {
      const v = [...yap.state.sessions.values()].find((s) => s.uid === uid && s.voice)?.voice;
      return v ? { muted: v.muted, deafened: v.deafened } : null;
    }, adaUid);
    assert.match(await a.getAttribute('[data-action="toggle-mute"]', 'title'), /^Mute your mic \(Ctrl\/⌘\+Shift\+M\)$/);
    assert.match(await a.getAttribute('[data-action="toggle-deafen"]', 'title'), /\(Ctrl\/⌘\+Shift\+D\)$/);
    await a.click('#input');
    await a.keyboard.type('half a thought');
    await a.keyboard.press('Control+Shift+M');
    await until(async () => (await adaOnBen())?.muted === true, 5000, 'Ben to see Ada muted');
    assert.equal(await a.evaluate(() => yap.settings.muted), true);
    assert.equal(await a.inputValue('#input'), 'half a thought', 'nothing typed into the message');
    await a.keyboard.press('Meta+Shift+M'); // ⌘ on a Mac
    await until(async () => (await adaOnBen())?.muted === false, 5000, 'unmuted');
    await a.keyboard.press('Control+Shift+D');
    await until(async () => (await adaOnBen())?.deafened === true, 5000, 'Ben to see Ada deafened');
    assert.equal(await a.evaluate(() => [...yap.voice.peers.values()][0].audio.muted), true, 'deafened: she hears nobody');
    await a.keyboard.press('Control+Shift+D');
    await until(async () => (await adaOnBen())?.deafened === false, 5000, 'undeafened');
    await a.fill('#input', '');
    // Push-to-mute: pick a key in Settings › Voice (not the push-to-talk one), then hold it.
    await a.click('[data-action="open-settings"]');
    await a.click('#settings-tabs [role=tab]:text-is("Voice")');
    assert.equal(await a.textContent('#ptm-key'), 'None (click to pick one)');
    await a.click('#ptm-key');
    await a.keyboard.press('Backquote');
    await a.waitForSelector('#toast:text-is("` is already your push-to-talk key.")', { timeout: 3000 });
    assert.equal(await a.evaluate(() => yap.settings.pushToMuteKey), '');
    await a.click('#ptm-key');
    await a.keyboard.press('F8');
    assert.equal(await a.textContent('#ptm-key'), 'F8');
    await a.locator('#settings-form fieldset.settings-group:has(legend:text-is("How you talk"))').screenshot({ path: path.join(OUT, 'TW2-settings.png') });
    await a.click('#settings-form .settings-actions .btn');
    await a.evaluate(() => {
      document.activeElement?.blur();
      yap.state.remindedMuted = 0;
    });
    await a.keyboard.down('F8');
    await b.waitForTimeout(1200);
    assert.ok((await inbound(b)).level < 0.001, 'silent while held');
    assert.equal(await a.textContent('[data-action="toggle-mute"] span'), 'Muted');
    assert.match(await a.getAttribute('[data-action="toggle-mute"]', 'title'), /^Muted while you hold F8/);
    assert.equal(await a.evaluate(() => yap.settings.muted), false, 'nothing saved');
    assert.equal(await a.evaluate(() => yap.state.remindedMuted), 0, 'a cough is no reason to remind');
    await a.keyboard.up('F8');
    await b.waitForTimeout(1200);
    assert.ok((await inbound(b)).level > 0.001, 'heard again once it is let go');
    assert.equal(await a.textContent('[data-action="toggle-mute"] span'), 'Mute');
    await a.click('[data-action="open-settings"]');
    await a.click('#settings-tabs [role=tab]:text-is("Voice")');
    await a.click('#ptm-clear');
    assert.equal(await a.evaluate(() => yap.settings.pushToMuteKey), '');
    await a.click('#settings-form .settings-actions .btn');
  } finally {
    await voiceReset(a);
  }
});

await test('the mic reopens by itself when it stops; a stalled audio graph fails open (mute still holds)', async () => {
  const level = async () => {
    await b.waitForTimeout(1200);
    return (await inbound(b)).level;
  };
  // Unplugged / revoked: the track ends on its own (stop() alone doesn't fire 'ended').
  const before = await a.evaluate(() => yap.voice.rawStream.id);
  await a.evaluate(() => {
    const t = yap.voice.rawStream.getAudioTracks()[0];
    t.stop();
    t.dispatchEvent(new Event('ended'));
  });
  await a.waitForFunction((id) => yap.voice.micOk && yap.voice.rawStream?.id !== id, before, { timeout: 5000 });
  assert.ok(await level() > 0.001, 'audio again after the mic came back');
  // A device change with nothing new is harmless.
  await a.evaluate(() => navigator.mediaDevices.dispatchEvent(new Event('devicechange')));
  // The browser suspends the audio graph and won't resume it: the plain mic goes out instead.
  await a.evaluate(() => {
    const ctx = yap.voice.context;
    globalThis.__resume = ctx.resume.bind(ctx);
    ctx.resume = () => Promise.resolve();
    return ctx.suspend();
  });
  await a.waitForFunction(() => yap.voice.stalled, null, { timeout: 5000 });
  await a.waitForSelector('.vp-warn:has-text("plain mic")', { timeout: 3000 });
  assert.ok(await level() > 0.001, 'still heard while the graph is stalled (fail open)');
  await a.click('[data-action="toggle-mute"]');
  assert.ok(await level() < 0.001, 'mute holds while stalled');
  await a.click('[data-action="toggle-mute"]');
  await a.evaluate(() => {
    yap.voice.context.resume = globalThis.__resume;
    return globalThis.__resume();
  });
  await a.waitForFunction(() => !yap.voice.stalled, null, { timeout: 5000 });
  assert.equal(await a.locator('.vp-warn:has-text("plain mic")').count(), 0);
});

await test('Test mic in a call: says the mic is already on, the bar moves, and stopping it keeps the call', async () => {
  await a.click('[data-action="open-settings"]');
  await a.click('#settings-tabs [role=tab]:text-is("Voice")');
  await a.click('#mic-test-btn');
  await a.waitForFunction(() => /you’re in a call/.test(document.querySelector('#mic-test-status').textContent), null, { timeout: 5000 });
  await until(() => a.evaluate(() => parseFloat(document.querySelector('#meter-fill').style.width) > 20), 5000, 'the bar to move');
  await a.click('#mic-test-btn');
  await a.waitForSelector('#mic-test-btn[aria-pressed="false"]');
  await a.click('#settings-form .settings-actions .btn');
  assert.ok(await a.evaluate(() => yap.voice.rawStream && [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected')), 'still in the call, mic open');
});

await test('one Noise suppression switch: RNNoise in the call (not the browser\'s too); music mode turns it off; an old "stronger" setting keeps it on', async () => {
  const mic = () => a.evaluate(() => ({ rnnoise: yap.voice.rnnoise, asked: window.__micAsked?.noiseSuppression ?? null }));
  const reopen = (patch) => a.evaluate(async (p) => {
    Object.assign(yap.settings, p);
    await yap.voice.restartMic();
  }, patch);
  try {
    await reopen({ noiseSuppression: true }); // (off for everyone else here, for the test signal's sake: see user())
    if (SYNTH) assert.deepEqual(await mic(), { rnnoise: true, asked: false }, 'RNNoise, and not the browser\'s filter as well');
    else assert.equal((await mic()).rnnoise, true);
    await reopen({ music: true });
    assert.equal((await mic()).rnnoise, false, 'music mode: off');
    if (SYNTH) assert.equal((await mic()).asked, false);
    await reopen({ music: false, noiseSuppression: false });
    assert.equal((await mic()).rnnoise, false, 'switched off: off');
    await reopen({ noiseSuppression: true });
    assert.equal((await mic()).rnnoise, true);
    // Two changes in a row (on, with RNNoise to set up, then off at once): what ends up open is the last one asked for.
    const last = await a.evaluate(async () => {
      yap.settings.noiseSuppression = false;
      const first = yap.voice.restartMic();
      yap.settings.noiseSuppression = true;
      const second = yap.voice.restartMic();
      yap.settings.noiseSuppression = false;
      await Promise.all([first, second, yap.voice.restartMic()]);
      return { rnnoise: yap.voice.rnnoise, open: yap.voice.rawStream?.getAudioTracks()[0]?.readyState };
    });
    assert.deepEqual(last, { rnnoise: false, open: 'live' }, 'the last change wins, with one live mic');
  } finally {
    await reopen({ music: false, noiseSuppression: false }); // (the plain test signal again for the tests after this one)
  }
  await a.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'));
  // Settings from before: "Stronger noise removal" on (with the old switch off) means noise removal stays on.
  const ctx = await newContext({ viewport: { width: 800, height: 600 } }, 'old settings');
  await ctx.addInitScript(() => location.origin.startsWith('http') && localStorage.setItem('yap.settings', JSON.stringify({ noiseSuppression: false, rnnoise: true })));
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`);
  await page.waitForFunction(() => globalThis.yap?.settings);
  assert.deepEqual(await page.evaluate(() => [yap.settings.noiseSuppression, 'rnnoise' in yap.settings]), [true, false]);
  await ctx.close();
});

await test('a dropped connection keeps the call: same peer connection, audio keeps flowing, no chimes', async () => {
  const countChimes = () => {
    globalThis.__chimes = 0;
    if (globalThis.__chimeHook) return;
    globalThis.__chimeHook = true;
    const start = OscillatorNode.prototype.start;
    OscillatorNode.prototype.start = function (...args) {
      globalThis.__chimes++;
      return start.apply(this, args);
    };
  };
  await a.evaluate(countChimes);
  await b.evaluate(countChimes);
  await b.evaluate(() => (globalThis.__pc = [...yap.voice.peers.values()][0].pc));
  const before = await a.evaluate(() => yap.state.sid);
  assert.ok(await a.evaluate(() => Boolean(yap.state.resumeKey)), 'the server handed out a resume key');
  await a.evaluate(() => yap.ui.dropSocket());
  await a.waitForFunction((sid) => yap.state.sid && yap.state.sid !== sid && yap.state.voiceChannel === 'lounge' && !yap.state.resume, before, { timeout: 15000 });
  const after = await a.evaluate(() => yap.state.sid);
  await b.waitForFunction((sid) => yap.voice.peers.has(sid), after, { timeout: 5000 });
  assert.ok(await b.evaluate((sid) => yap.voice.peers.get(sid).pc === globalThis.__pc, after), 'the same RTCPeerConnection, moved to the new session');
  assert.equal(await b.evaluate(() => yap.voice.peers.size), 1);
  assert.equal(await b.evaluate(() => yap.state.ghosts.size), 0);
  await b.waitForTimeout(1000);
  assert.ok((await inbound(b)).level > 0.001, 'still hearing Ada');
  assert.equal(await a.evaluate(() => globalThis.__chimes), 0, 'no chimes for Ada');
  assert.equal(await b.evaluate(() => globalThis.__chimes), 0, 'no chimes for Ben');
  assert.equal(await b.locator('.voice-member').count(), 2, 'Ada listed once');
});

await test('a half-dead old socket: the server still lists our old session, and it is us, not someone to call', async () => {
  // Drop the connection the way a network change does: the page gives up on its socket, but the
  // server doesn't hear about it yet, so the old session is still in the call when we're back.
  const before = await a.evaluate(() => {
    const close = WebSocket.prototype.close;
    globalThis.__halfDead = [];
    WebSocket.prototype.close = function () { globalThis.__halfDead.push(this); }; // keep it open to the server
    const sid = yap.state.sid;
    yap.ui.dropSocket();
    WebSocket.prototype.close = close;
    return sid;
  });
  await a.waitForFunction((sid) => yap.state.sid && yap.state.sid !== sid && yap.state.voiceChannel === 'lounge' && !yap.state.resume, before, { timeout: 15000 });
  assert.equal(await a.evaluate((sid) => yap.voice.peers.has(sid), before), false, 'no call to our own old session');
  assert.equal(await a.evaluate((sid) => yap.state.sessions.has(sid), before), false, 'not listed as someone else');
  assert.equal(await a.evaluate(() => yap.voice.peers.size), 1, 'just Ben');
  // The old socket finally goes: no "reconnecting…" for ourselves, Ada listed once, the call with Ben intact.
  await a.evaluate(() => globalThis.__halfDead.forEach((ws) => ws.close()));
  await a.waitForTimeout(1500);
  assert.equal(await a.locator('.voice-member.reconnecting').count(), 0);
  assert.equal(await a.evaluate(() => yap.state.ghosts.size), 0);
  assert.equal(await b.locator('.voice-member').count(), 2, 'Ada listed once for Ben');
  assert.equal(await a.locator('.voice-member').count(), 2, 'and for herself');
  await b.waitForFunction(() => [...yap.voice.peers.values()].every((p) => p.pc.connectionState === 'connected'), null, { timeout: 5000 });
});

/** What a page put on the clipboard (the clipboard itself isn't readable headless). */
const catchCopies = (page) => page.evaluate(() => {
  globalThis.__copied = null;
  navigator.clipboard.writeText = async (text) => { globalThis.__copied = text; };
});
const IPV4 = /\b\d{1,3}(?:\.\d{1,3}){3}\b/;

await test("call report: Copy call report (the call's ⋯): header, Ada with red+opus, a timeline with the join, the kept reconnects and a line per person", async () => {
  await catchCopies(b);
  await b.click('#voice-panel [data-action="voice-more"]');
  await b.waitForSelector('#popover [data-action="copy-call-report"]');
  await b.locator('#popover').screenshot({ path: path.join(OUT, 'call-report-menu.png') });
  await b.click('#popover [data-action="copy-call-report"]');
  await b.waitForSelector('#toast:not([hidden]):text-is("Call report copied.")', { timeout: 3000 });
  const text = await b.evaluate(() => globalThis.__copied);
  fs.writeFileSync(path.join(OUT, 'call-report-example.txt'), text);
  const lines = text.split('\n');
  assert.equal(lines[0], 'Yap call report');
  assert.match(text, /^App: Yap web \d+\.\d+\.\d+ \(build [0-9a-f]{12}\)$/m);
  assert.match(text, /^Device: (Headless )?Chrome \d+ on \w+ \(computer\)$/m);
  assert.match(text, /^Channel: Lounge$/m);
  assert.match(text, /^Started: \d{4}-\d\d-\d\d \d\d:\d\d:\d\d \(local time\)$/m);
  assert.match(text, /^Lasted: \d+:\d\d so far \(still in the call\)$/m);
  assert.match(text, new RegExp(`^  Quality: ${BITRATE / 1000} kbps$`, 'm'));
  assert.match(text, /^ {2}Channel quality: each person's own$/m);
  assert.ok(lines.includes('+00:00 You joined Lounge; Ada was there') && text.includes(' Settings: channel quality 256 kbps for everyone\n'), 'a setting changed mid-call (the channel quality test)');
  assert.match(text, /^ {2}Input mode: always on$/m);
  assert.match(text, /^ {2}Echo cancellation: on$/m);
  assert.match(text, /^ {2}Microphone: .+$/m);
  const at = (l) => lines.indexOf(l);
  assert.ok(at('Settings:') > 0 && at('Settings:') < at('Audio devices:') && at('Audio devices:') < at('People (1)') && at('People (1)') < at('Timeline'), 'header, people, timeline');
  const ada = lines.slice(at('People (1)') + 1, at('Timeline')).join('\n');
  assert.match(ada, /^Ada$/m);
  assert.match(ada, /^ {2}Connection: direct \((host|srflx|prflx) ↔ (host|srflx|prflx)\)/m);
  assert.match(ada, /^ {2}Codec: red\+opus out, red\+opus in$/m);
  assert.match(ada, new RegExp(`^  Opus fmtp sent: (?=.*useinbandfec=1)(?=.*maxaveragebitrate=${BITRATE}(;|$))`, 'm'), '(Chrome sorts the parameters)');
  assert.match(ada, /^ {2}Bitrate \(median\): \d+ kbps out, \d+ kbps in$/m);
  assert.match(ada, /^ {2}Round trip: \d+ ms median, \d+ ms worst$/m);
  assert.match(ada, /^ {2}Lost packets: \d+\.\d % \([\d,]+ of [\d,]+\)$/m);
  assert.match(ada, /^ {2}Audio patched over: \d+\.\d %$/m);
  assert.match(ada, /^ {2}ICE restarts: \d+, connection rebuilds: \d+, reconnects kept: [1-9]\d*$/m, 'Ada dropped her socket twice and the call was kept');
  assert.match(ada, /^ {2}From you: they lost \d+\.\d % of yours \(their round trip \d+ ms median\)$/m, 'what Ada reports getting from Ben (remote-inbound-rtp)');
  const timeline = lines.slice(at('Timeline') + 1);
  assert.equal(timeline[0], '+00:00 You joined Lounge; Ada was there');
  assert.ok(timeline.some((l) => /^\+\d\d:\d\d Ada: connected$/.test(l)), 'connection states');
  assert.ok(timeline.some((l) => /^\+\d\d:\d\d Ada: red\+opus out, red\+opus in$/.test(l)), 'the codec, once negotiated');
  assert.ok(timeline.some((l) => /^\+\d\d:\d\d Ada lost their connection to Yap; keeping the call/.test(l)), 'her socket dropping');
  assert.ok(timeline.some((l) => /^\+\d\d:\d\d Ada is back: the call was kept$/.test(l)), 'and the call kept');
  const per = timeline.filter((l) => /^\+\d\d:\d\d Ada: rtt \d+ ms, lost \d+\.\d %, patched \d+\.\d %, \d+ kbps in, \d+ out$/.test(l));
  assert.ok(per.length >= 2, `a line for Ada every 10 s (${per.length})`);
  assert.ok(per[0].startsWith('+00:1'), `the first at +00:10 (${per[0]})`);
  assert.doesNotMatch(text, IPV4, 'no IP addresses');
  assert.doesNotMatch(text, /candidate|typ host|\.local\b/);
  // The same report is kept on this device while the call goes on.
  assert.match(await b.evaluate(() => JSON.parse(localStorage.getItem('yap.callReport')).text), /^Yap call report\n/);
});

await test('your own connection: when the others lose > 3 % of your audio (their reports), "Your connection is dropping some of your audio"; gone when they stop; in the call report', async () => {
  try {
    // Ben's receiver reports say he loses 8 % of Ada's voice (a stand-in for a bad uplink: localhost loses nothing).
    await a.evaluate(() => {
      const real = RTCPeerConnection.prototype.getStats;
      globalThis.__realGetStats = real;
      RTCPeerConnection.prototype.getStats = async function (...args) {
        const report = await real.apply(this, args);
        const out = new Map();
        report.forEach((s, id) => out.set(id, s.type === 'remote-inbound-rtp' && s.kind === 'audio' && !globalThis.__lossDone ? { ...s, fractionLost: 0.08 } : s));
        return out;
      };
    });
    assert.equal(await a.locator('#voice-panel .own-weak').count(), 0, 'not before');
    await a.waitForSelector('#voice-panel .own-weak:text-is("Your connection is dropping some of your audio")', { timeout: 10000 });
    assert.match(await a.getAttribute('#voice-panel .own-weak', 'title'), /^The others say about 8% of what you send never reaches them/);
    assert.equal(await b.locator('#voice-panel .own-weak').count(), 0, 'Ben, who loses nothing of Ada\'s… is fine');
    await a.locator('#voice-panel').screenshot({ path: path.join(OUT, 'TW4-own-connection.png') });
    await a.evaluate(() => (globalThis.__lossDone = true));
    await a.waitForSelector('#voice-panel .own-weak', { state: 'detached', timeout: 10000 });
    await a.evaluate(() => (RTCPeerConnection.prototype.getStats = globalThis.__realGetStats));
    await catchCopies(a);
    await a.click('#voice-panel [data-action="voice-more"]');
    await a.click('#popover [data-action="copy-call-report"]');
    const text = await a.evaluate(() => globalThis.__copied);
    assert.match(text, /^\+\d\d:\d\d Your connection is dropping some of your audio \(they lose about 8 % of yours\)$/m);
    assert.match(text, /^\+\d\d:\d\d Your connection is fine again$/m);
  } finally {
    await a.evaluate(() => {
      if (globalThis.__realGetStats) RTCPeerConnection.prototype.getStats = globalThis.__realGetStats;
      document.querySelector('#popover').hidden = true;
    }).catch(() => {});
  }
});

await test('soundboard: add a clip (decoded + trimmed in the browser), play it, the other person gets it', async () => {
  // A 7-second WAV: the panel trims it to 5 s before uploading.
  const rate = 24000;
  const n = rate * 7;
  const wav = Buffer.alloc(44 + n * 2);
  wav.write('RIFF', 0); wav.writeUInt32LE(36 + n * 2, 4); wav.write('WAVEfmt ', 8); wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(rate, 24); wav.writeUInt32LE(rate * 2, 28); wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write('data', 36); wav.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) wav.writeInt16LE(Math.round(Math.sin((i * 2 * Math.PI * 440) / rate) * 0.9 * 32767), 44 + i * 2);
  await a.click('[data-action="soundboard"]');
  await a.waitForSelector('#soundboard:not([hidden])');
  await a.setInputFiles('#soundboard input[name=sb-file]', { name: 'horn.wav', mimeType: 'audio/wav', buffer: wav });
  await a.waitForSelector('#soundboard input[name=sb-name]');
  await a.fill('#soundboard input[name=sb-name]', `Horn ${RUN}`);
  await a.click('[data-sb="save"]');
  await a.waitForSelector(`#soundboard .sb-name:text-is("Horn ${RUN}")`, { timeout: 8000 });
  const horn = await a.evaluate((name) => [...yap.state.sounds.values()].find((s) => s.name === name), `Horn ${RUN}`);
  assert.ok(horn.ms <= 5000 && horn.ms > 4900, `trimmed to 5 s (${horn.ms} ms)`);
  assert.ok(horn.gain < 0.5, `a loud clip is turned down (gain ${horn.gain})`);
  await a.screenshot({ path: path.join(OUT, 'desktop-soundboard.png') });
  const before = await b.evaluate(() => yap.state.soundsHeard ?? 0);
  await a.click(`#soundboard .sb-play:has(.sb-name:text-is("Horn ${RUN}"))`);
  await b.waitForFunction((n) => (yap.state.soundsHeard ?? 0) > n, before, { timeout: 5000 });
  await a.click('[data-sb="close"]');
});

// ── Screen sharing (PROTOCOL.md › Screen sharing) ──────────────────────────
/** Ada starts sharing (Smooth, with sound) unless she already is; Ben isn't watching (an earlier test may have failed midway). */
/** Settings › Voice › Lower shared media while friends talk, on or off. */
async function setDuckMedia(page, on) {
  await page.click('[data-action="open-settings"]');
  await page.click('#settings-tabs [role=tab]:text-is("Voice")');
  await page.setChecked('#settings-form [name=duckMedia]', on);
  await page.click('#settings-form .settings-actions .btn');
}

async function startSharing(page) {
  await b.evaluate(() => yap.state.screenSid && document.querySelector('#screen-view [data-screen="stop"]').click());
  if (await page.evaluate(() => Boolean(yap.voice.screenStream))) return;
  await page.click('.vp-buttons [data-action="share-screen"]');
  await page.waitForSelector('#share-dialog[open]');
  await page.click('#share-form label:has(input[value="smooth"])');
  await page.click('#share-form button[value="share"]');
  await page.waitForSelector('#share-panel:not([hidden])', { timeout: 5000 });
}
/** What the viewer says: connecting | live | paused | stopped, and its words. */
const viewer = (page) => page.evaluate(() => ({ state: document.querySelector('#screen-view').dataset.state, text: document.querySelector('.screen-state')?.textContent ?? '' }));
const sharePanel = (page) => page.evaluate(() => document.querySelector('#share-panel').textContent.replace(/\s+/g, ' ').trim());
/** The codec the sharer's screen goes out in to `sid`, from its stats. */
const sentCodec = (page, sid) => page.evaluate(async (s) => {
  const { pc, screenSenders } = yap.voice.peers.get(s);
  const sender = screenSenders?.find((x) => x.track?.kind === 'video');
  const mid = pc.getTransceivers().find((t) => t.sender === sender)?.mid;
  const report = await pc.getStats();
  let codec = null;
  report.forEach((x) => {
    if (x.type === 'outbound-rtp' && x.kind === 'video' && x.mid === mid && x.codecId) codec = report.get(x.codecId)?.mimeType ?? codec;
  });
  return codec;
}, sid);

await test('screen sharing: share in voice, a LIVE badge, the other person watches (a picture within ~1 s); video only goes to viewers', async () => {
  await b.evaluate(() => yap.state.screenSid && document.querySelector('#screen-view [data-screen="stop"]').click());
  await a.evaluate(() => yap.voice.stopScreen());
  // The page's own Permissions-Policy must allow it (it once didn't, and a stub hid that).
  assert.deepEqual(await a.evaluate(() => ['microphone', 'display-capture'].map((f) => document.featurePolicy.allowsFeature(f))), [true, true]);
  // The real getDisplayMedia (Chromium picks its fake screen); we only note what was asked for.
  await a.evaluate(() => {
    const real = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getDisplayMedia = (constraints) => {
      window.lastDisplayConstraints = constraints;
      return real(constraints).then((stream) => {
        // Stereo "tab audio" in place of whatever audio the fake screen has:
        // 440 Hz on the left only, 660 Hz on the right only.
        for (const t of stream.getAudioTracks()) { stream.removeTrack(t); t.stop(); }
        const ac = new AudioContext({ sampleRate: 48000 });
        const merge = ac.createChannelMerger(2);
        for (const [ch, f] of [[0, 440], [1, 660]]) {
          const o = ac.createOscillator();
          o.frequency.value = f;
          const g = ac.createGain();
          g.gain.value = 0.2;
          o.connect(g).connect(merge, 0, ch);
          o.start();
        }
        const dest = ac.createMediaStreamDestination();
        dest.channelCount = 2;
        merge.connect(dest);
        stream.addTrack(dest.stream.getAudioTracks()[0]);
        return stream;
      });
    };
  });
  const benSid = await b.evaluate(() => yap.state.sid);
  await a.click('.vp-buttons [data-action="share-screen"]');
  // What to share and how: a window, Smooth (1080p60), with its sound, by default.
  await a.waitForSelector('#share-dialog[open]');
  assert.equal(await a.isChecked('#share-form input[name=sound]'), true, 'Share sound is on by default');
  await a.screenshot({ path: path.join(OUT, 'desktop-share-dialog.png') });
  await a.click('#share-form button[value="share"]');
  await b.waitForSelector('.voice-member .live-badge', { timeout: 5000 });
  const asked = await a.evaluate(() => window.lastDisplayConstraints);
  assert.deepEqual([asked.video.displaySurface, asked.video.frameRate.max, asked.video.width.max], ['window', 60, 1920]);
  assert.equal(await a.evaluate((sid) => yap.voice.peers.get(sid).screenSenders ?? null, benSid), null, 'nothing sent until someone watches');
  assert.equal(await a.evaluate(() => yap.voice.screenStream?.getVideoTracks()[0]?.readyState), 'live', 'a real screen track');
  assert.equal(await sharePanel(a), "Sharing your screen No one's watching yet Smooth Sharp Max Share sound Share something else Stop sharing");
  assert.equal(await a.isVisible('.vp-buttons [data-action="share-screen"]'), false, 'Stop sharing is in the sharing panel now');
  const clicked = Date.now();
  // (Clicked from inside the page, so the first words are read before any frame can come.)
  assert.equal(await b.evaluate(() => {
    document.querySelector('.voice-member .live-badge').click();
    return document.querySelector('.screen-state').textContent;
  }), "Connecting to Ada's screen…");
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
  const firstFrameMs = Date.now() - clicked;
  console.log(`  (first frame ${firstFrameMs} ms after clicking LIVE)`);
  assert.ok(firstFrameMs < 2500, `a picture within ~1 s, not ${firstFrameMs} ms`);
  await a.waitForFunction(() => /Watching: Ben\b/.test(document.querySelector('#share-panel').textContent), null, { timeout: 5000 });
  await b.waitForTimeout(400);
  await b.screenshot({ path: path.join(OUT, 'desktop-screen-share.png') });
  await a.screenshot({ path: path.join(OUT, 'desktop-screen-share-sharer.png') });
  // Fitted to the window: the picture element fills the stage (bigger or smaller than the share), scaled with object-fit: contain.
  const fit = await b.evaluate(() => {
    const v = document.querySelector('.screen-video');
    const r = v.getBoundingClientRect();
    const s = document.querySelector('.screen-stage').getBoundingClientRect();
    return { fills: Math.abs(r.width - s.width) < 2 && Math.abs(r.height - s.height) < 2, fit: getComputedStyle(v).objectFit };
  });
  assert.deepEqual(fit, { fills: true, fit: 'contain' }, 'letterboxed to the window');
  assert.ok(await a.evaluate((sid) => yap.voice.peers.get(sid).screenSenders?.length > 0, benSid), 'sent to the viewer');
  const enc = await a.evaluate((sid) => {
    const sender = yap.voice.peers.get(sid).screenSenders.find((x) => x.track.kind === 'video');
    const p = sender.getParameters();
    return { fps: p.encodings[0].maxFramerate, bitrate: p.encodings[0].maxBitrate, hint: sender.track.contentHint };
  }, benSid);
  assert.deepEqual(enc, { fps: 60, bitrate: 8_000_000, hint: 'motion' }, 'Smooth: 60 fps, 8 Mbps, tuned for motion');
  // H.264 constrained baseline (packetization-mode=1) is always offered: the native apps only do H.264.
  const video = await a.evaluate((sid) => yap.voice.peers.get(sid).pc.localDescription.sdp.split(/\r\nm=/).find((m) => m.startsWith('video')), benSid);
  assert.match(video, /a=fmtp:\d+ .*(?=.*profile-level-id=42e01f)(?=.*packetization-mode=1)/, 'H.264 42e01f, packetization-mode=1 in the offer');
  assert.equal(await sentCodec(a, benSid), 'video/H264', 'Smooth sends H.264 first (hardware encoders)');
  // The tab's sound arrives in stereo (it used to be negotiated mono: only the first Opus line was tuned).
  const sharerSid = await a.evaluate(() => yap.state.sid);
  const fmtp = await b.evaluate((sid) => yap.voice.peers.get(sid).pc.localDescription.sdp.split(/\r\nm=/).filter((m) => m.startsWith('audio'))[1]?.match(/a=fmtp:\d+ ([^\r\n]+)/)?.[1], sharerSid);
  assert.match(fmtp ?? '', /stereo=1/);
  assert.match(fmtp ?? '', /maxaveragebitrate=160000/);
  // The share's sound stays plain Opus (RED is for the voice only): Opus first and no RED, both sides.
  for (const [page, sid] of [[a, benSid], [b, sharerSid]]) {
    const section = await page.evaluate((s) => yap.voice.peers.get(s).pc.localDescription.sdp.split(/\r\nm=/).filter((m) => m.startsWith('audio'))[1], sid);
    assert.match(section, new RegExp(`a=rtpmap:${section.split('\r\n')[0].split(' ')[3]} opus/48000`), "the share's sound lists Opus first");
    assert.doesNotMatch(section, /red\/48000/, "and no RED");
  }
  const split = await b.evaluate(async (sid) => {
    const track = yap.voice.screens.get(sid).getAudioTracks()[0];
    const ctx = new AudioContext();
    const splitter = ctx.createChannelSplitter(2);
    ctx.createMediaStreamSource(new MediaStream([track])).connect(splitter);
    const [l, r] = [ctx.createAnalyser(), ctx.createAnalyser()];
    splitter.connect(l, 0);
    splitter.connect(r, 1);
    await new Promise((res) => setTimeout(res, 1500));
    const bl = new Float32Array(l.fftSize);
    const br = new Float32Array(r.fftSize);
    l.getFloatTimeDomainData(bl);
    r.getFloatTimeDomainData(br);
    let diff = 0;
    let sum = 0;
    for (let i = 0; i < bl.length; i++) {
      diff += (bl[i] - br[i]) ** 2;
      sum += bl[i] ** 2 + br[i] ** 2;
    }
    ctx.close();
    return { diff, sum };
  }, sharerSid);
  assert.ok(split.sum > 0.01, 'tab audio arrives');
  assert.ok(split.diff / split.sum > 0.5, `left and right differ (${(split.diff / split.sum).toFixed(2)}; mono would be 0)`);
  // Stop watching: nothing goes to Ben any more, and Ada's list says so.
  const mlines = await a.evaluate((sid) => yap.voice.peers.get(sid).pc.getTransceivers().length, benSid);
  await b.click('[data-screen="stop"]');
  await a.waitForFunction((sid) => !yap.voice.peers.get(sid).screenSenders, benSid, { timeout: 5000 });
  await a.waitForFunction(() => /No one's watching yet/.test(document.querySelector('#share-panel').textContent), null, { timeout: 5000 });
  // Watching again reuses the same transceivers: no m-lines pile up.
  await b.click('.voice-member .live-badge');
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
  assert.equal(await a.evaluate((sid) => yap.voice.peers.get(sid).pc.getTransceivers().length, benSid), mlines, 'same m-lines the second time');
  await b.click('[data-screen="stop"]');
  await a.waitForFunction((sid) => !yap.voice.peers.get(sid).screenSenders, benSid, { timeout: 5000 });
  // Voice is still up both ways.
  assert.equal(await a.evaluate((sid) => yap.voice.peers.get(sid).pc.connectionState, benSid), 'connected');
});

await test('screen sharing: Watch screen from the sharer\'s name; change quality and what you share without stopping; Share sound off and on', async () => {
  await startSharing(a);
  const [adaSid, benSid] = await Promise.all([a.evaluate(() => yap.state.sid), b.evaluate(() => yap.state.sid)]);
  // Ben clicks Ada's name under the voice channel: Watch screen (and his volume for her, as before).
  await b.click(`.voice-member[data-voice-sid="${adaSid}"] .name`);
  await b.waitForSelector('#popover:not([hidden]) [data-action="watch-screen"]');
  assert.equal(await b.locator('#popover input[data-volume-uid]').count(), 1);
  await b.screenshot({ path: path.join(OUT, 'desktop-screen-popover.png') });
  await b.click('#popover [data-action="watch-screen"]');
  assert.equal(await b.isHidden('#popover'), true, 'the popover closes');
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
  const stream = await b.evaluateHandle((sid) => yap.voice.screens.get(sid), adaSid);
  // While watching, her popover says Stop watching instead.
  await b.click(`.voice-member[data-voice-sid="${adaSid}"] .name`);
  await b.waitForSelector('#popover:not([hidden]) [data-action="stop-watching"]');
  await b.keyboard.press('Escape');
  await b.click('.screen-bar .screen-title'); // (closes the popover)
  // Sharp while sharing: 30 fps, 6 Mbps, tuned for detail, VP9 first (H.264 still offered).
  await a.click('#share-panel label:has(input[value="sharp"])');
  await until(() => a.evaluate((sid) => {
    const sender = yap.voice.peers.get(sid).screenSenders?.find((x) => x.track?.kind === 'video');
    const p = sender?.getParameters();
    return p?.encodings[0].maxFramerate === 30 && p.encodings[0].maxBitrate === 6_000_000 && sender.track.contentHint === 'detail' && sender.track.getConstraints().height?.max === 1440;
  }, benSid), 5000, 'Sharp: 30 fps, 6 Mbps, detail, up to 1440p');
  await until(async () => (await sentCodec(a, benSid)) === 'video/VP9', 8000, 'Sharp sends VP9 (after a renegotiation)');
  assert.match(await a.evaluate((sid) => yap.voice.peers.get(sid).pc.localDescription.sdp, benSid), /H264\/90000/, 'H.264 stays in the list');
  assert.equal(await a.evaluate(() => yap.settings.shareQuality), 'sharp', 'remembered for next time');
  // Back to Smooth: H.264 again.
  await a.click('#share-panel label:has(input[value="smooth"])');
  await until(async () => (await sentCodec(a, benSid)) === 'video/H264', 8000, 'Smooth sends H.264 again');
  // Share sound off: the sound's track goes quiet, no renegotiation, Ben's stream stays the same.
  await a.click('#share-panel input[name="share-sound"]');
  await until(() => a.evaluate((sid) => yap.voice.peers.get(sid).screenSenders.length === 1, benSid), 3000, 'only the video goes out');
  assert.equal(await a.evaluate((sid) => yap.voice.peers.get(sid).pc.signalingState, benSid), 'stable');
  await a.click('#share-panel input[name="share-sound"]');
  await until(() => a.evaluate((sid) => yap.voice.peers.get(sid).screenSenders.length === 2, benSid), 3000, 'the sound goes out again');
  // Share something else: the picker again; the new window takes the old one's place on the same connection.
  const before = await a.evaluateHandle(() => yap.voice.screenStream.getVideoTracks()[0]);
  await a.click('#share-panel [data-action="share-change"]');
  await until(() => a.evaluate((old) => yap.voice.screenStream.getVideoTracks()[0] !== old && old.readyState === 'ended', before), 5000, 'a new screen track, the old one stopped');
  await until(() => a.evaluate((sid) => {
    const sender = yap.voice.peers.get(sid).screenSenders?.find((x) => x.track?.kind === 'video');
    return sender?.track === yap.voice.screenStream.getVideoTracks()[0];
  }, benSid), 3000, 'the new track is what Ben gets');
  await b.waitForTimeout(1500);
  assert.equal((await viewer(b)).state, 'live', 'Ben kept watching');
  assert.ok(await b.evaluate(([s, sid]) => yap.voice.screens.get(sid) === s, [stream, adaSid]), 'the same stream on Ben\'s side throughout');
  await b.click('[data-screen="stop"]');
});

await test('screen sharing: the viewer has its own volume for the share\'s sound; deafen silences it; paused, then stopped, then it closes itself', async () => {
  await startSharing(a);
  const [adaSid, adaUid] = await a.evaluate(() => [yap.state.sid, yap.settings.uid]);
  await b.click('.voice-member .live-badge');
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
  // The share's sound: 0–100 %, remembered per person, apart from their voice volume.
  const voiceVolume = await b.evaluate((uid) => yap.settings.volumes[uid] ?? null, adaUid);
  await b.evaluate(() => {
    const slider = document.querySelector('.screen-volume input');
    slider.value = '40';
    slider.dispatchEvent(new Event('input', { bubbles: true }));
  });
  assert.deepEqual(await b.evaluate((uid) => [document.querySelector('.screen-video').volume, yap.settings.screenVolumes[uid], document.querySelector('.screen-volume output').textContent], adaUid), [0.4, 0.4, '40%']);
  assert.equal(await b.evaluate((uid) => yap.settings.volumes[uid] ?? null, adaUid), voiceVolume, 'her voice volume is untouched');
  await b.click('[data-action="toggle-deafen"]');
  assert.equal(await b.evaluate(() => document.querySelector('.screen-video').muted), true, 'deafened: the share is silent too');
  await b.click('[data-action="toggle-deafen"]');
  assert.equal(await b.evaluate(() => document.querySelector('.screen-video').muted), false);
  // Lower shared media while friends talk (Ada never stops talking): half of the 40 %, and the slider still says 40 %.
  try {
    await setDuckMedia(b, true);
    await until(() => b.evaluate(() => Math.abs(document.querySelector('.screen-video').volume - 0.2) < 0.001), 3000, 'the share at half while Ada talks');
    assert.deepEqual(await b.evaluate((uid) => [document.querySelector('.screen-volume output').textContent, yap.settings.screenVolumes[uid]], adaUid), ['40%', 0.4], 'your volume stays yours');
    await a.click('[data-action="toggle-mute"]');
    await until(() => b.evaluate(() => Math.abs(document.querySelector('.screen-video').volume - 0.4) < 0.001), 4000, 'back to 40 % once she stops');
  } finally {
    await voiceReset(a);
    await b.evaluate(() => (yap.settings.duckMedia = false));
  }
  // Double-click: full screen (and back).
  await b.dblclick('.screen-stage');
  await until(() => b.evaluate(() => document.fullscreenElement?.classList.contains('screen-stage')), 3000, 'full screen');
  await b.evaluate(() => document.exitFullscreen());
  // No frame for 3 s: paused. (Ada's video sender sends nothing for a moment.)
  await a.evaluate((sid) => yap.voice.peers.get(sid).screenSenders.find((s) => s.track.kind === 'video').replaceTrack(null), await b.evaluate(() => yap.state.sid));
  await until(async () => (await viewer(b)).state === 'paused', 6000, 'paused after 3 s without a frame');
  assert.equal((await viewer(b)).text, "Ada's screen is paused.");
  await b.screenshot({ path: path.join(OUT, 'desktop-screen-paused.png') });
  await a.evaluate(() => {
    for (const peer of yap.voice.peers.values()) peer.syncScreen();
  });
  await until(async () => (await viewer(b)).state === 'live', 6000, 'live again with the next frame');
  // Ada stops: Ben's viewer says so, then closes itself 2 s later.
  await a.click('#share-panel [data-action="share-stop"]');
  await until(async () => (await viewer(b)).state === 'stopped', 5000, 'stopped');
  assert.equal((await viewer(b)).text, 'Ada stopped sharing.');
  await b.screenshot({ path: path.join(OUT, 'desktop-screen-stopped.png') });
  const shownAt = Date.now();
  await b.waitForFunction(() => yap.state.view === 'chat' && !yap.state.screenSid, null, { timeout: 5000 });
  const shown = Date.now() - shownAt;
  assert.ok(shown > 1200 && shown < 3500, `closed about 2 s later (${shown} ms)`);
  // Opening the viewer again shows her screen volume as she left it.
  await startSharing(a);
  await b.click(`.voice-member[data-voice-sid="${adaSid}"] .live-badge`);
  await b.waitForFunction(() => document.querySelector('.screen-volume output').textContent === '40%', null, { timeout: 5000 });
  await b.click('[data-screen="stop"]');
  await a.click('#share-panel [data-action="share-stop"]');
});

await test('screen sharing: the sharer sees who\'s watching; someone whose connection goes drops off the list', async () => {
  await startSharing(a);
  const benSid = await b.evaluate(() => yap.state.sid);
  await b.click('.voice-member .live-badge');
  await a.waitForFunction(() => /Watching: Ben\b/.test(document.querySelector('#share-panel').textContent), null, { timeout: 8000 });
  // Every list Ada's panel shows from here on.
  await a.evaluate(() => {
    window.panelLog = [];
    yap.voice.addEventListener('viewers', () => queueMicrotask(() => window.panelLog?.push(document.querySelector('.sp-viewers')?.textContent)));
  });
  await a.evaluate((sid) => yap.voice.restart(sid), benSid); // the connection to Ben goes (then comes back on its own)
  await until(() => a.evaluate((sid) => yap.voice.peers.get(sid)?.pc.connectionState === 'connected', benSid), 15000, 'reconnected');
  // Ben is still watching, so he asks again once connected: back on the list.
  await a.waitForFunction(() => /Watching: Ben\b/.test(document.querySelector('#share-panel').textContent), null, { timeout: 8000 });
  const log = await a.evaluate(() => window.panelLog);
  await a.evaluate(() => delete window.panelLog);
  assert.ok(log.includes("No one's watching yet") && log.at(-1) === 'Watching: Ben', `dropped, then back: ${JSON.stringify(log)}`);
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
  await b.click('[data-screen="stop"]');
  await a.click('#share-panel [data-action="share-stop"]');
  await b.waitForFunction(() => !document.querySelector('.voice-member .live-badge'), null, { timeout: 5000 });
});

await test('screen sharing: changing voice channels — a viewer who moves away stops watching; a sharer who moves takes the share along, LIVE there', async () => {
  const adaSid = await a.evaluate(() => yap.state.sid);
  const inLounge = (page) => page.waitForFunction(() => yap.state.voiceChannel === 'lounge' && [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
  try {
    await startSharing(a);
    await b.click(`.voice-member[data-voice-sid="${adaSid}"] .live-badge`);
    await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live', null, { timeout: 15000 });
    // Ben moves to Gaming: Ada's picture can't follow him there, so his viewer closes (it used to say "Connecting…" for good).
    await b.click('button.channel.voice:has-text("Gaming")');
    await b.waitForFunction(() => yap.state.voiceChannel === 'gaming' && !yap.state.screenSid && yap.state.view !== 'screen', null, { timeout: 8000 });
    await a.waitForFunction(() => /No one's watching yet/.test(document.querySelector('#share-panel').textContent), null, { timeout: 8000 });
    // Ada follows him, still sharing: the share is LIVE in Gaming too (it used to go on unseen there).
    await a.click('button.channel.voice:has-text("Gaming")');
    await b.waitForFunction((sid) => { const v = yap.state.sessions.get(sid)?.voice; return v?.channelId === 'gaming' && v.screen; }, adaSid, { timeout: 8000 });
    assert.equal(await b.locator(`.voice-member[data-voice-sid="${adaSid}"] .live-badge`).count(), 1, 'LIVE by her name');
    assert.match(await sharePanel(a), /^Sharing your screen /, 'still sharing, panel and all');
  } finally {
    await a.evaluate(() => yap.voice.stopScreen()).catch(() => {});
    await a.click('button.channel.voice:has-text("Lounge")').catch(() => {});
    await b.click('button.channel.voice:has-text("Lounge")').catch(() => {});
    await Promise.all([inLounge(a), inLounge(b)]);
  }
});

await test('screen sharing: watching a native app\'s share — H.264 constrained baseline only, no stream id on the video, its sound in a stream of its own', async () => {
  await b.evaluate(() => yap.state.screenSid && document.querySelector('#screen-view [data-screen="stop"]').click());
  await a.evaluate(() => yap.voice.stopScreen());
  const [adaSid, benSid] = await Promise.all([a.evaluate(() => yap.state.sid), b.evaluate(() => yap.state.sid)]);
  // Ada's page plays a native app: it adds the tracks the way a non-browser stack might, and only offers H.264 42e01f.
  await a.evaluate(async (sid) => {
    const peer = yap.voice.peers.get(sid);
    const screen = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
    const ac = new AudioContext({ sampleRate: 48000 });
    const osc = ac.createOscillator();
    const dest = ac.createMediaStreamDestination();
    osc.connect(dest);
    osc.start();
    const v = peer.pc.addTransceiver(screen.getVideoTracks()[0], { direction: 'sendonly' }); // no stream: "a=msid:- …"
    const h264 = RTCRtpReceiver.getCapabilities('video').codecs.filter((c) => c.mimeType === 'video/H264' && /profile-level-id=42e01f/.test(c.sdpFmtpLine) && /packetization-mode=1/.test(c.sdpFmtpLine));
    v.setCodecPreferences(h264);
    const s = peer.pc.addTransceiver(dest.stream.getAudioTracks()[0], { direction: 'sendonly', streams: [new MediaStream()] });
    window.native = { v, s, screen, ac };
    yap.ui.send({ t: 'voice.state', muted: false, deafened: false, screen: true });
  }, benSid);
  // (Its own m-line: earlier shares left theirs in the SDP, unused.)
  const [offer, mid] = await until(() => a.evaluate((sid) => {
    const { pc } = yap.voice.peers.get(sid);
    const mid = window.native.v.mid;
    const video = (pc.currentLocalDescription?.sdp ?? '').split(/\r\nm=/).find((m) => m.includes(`\r\na=mid:${mid}\r\n`));
    return mid && video && pc.signalingState === 'stable' ? [video, mid] : null;
  }, benSid), 5000, 'the native-style offer');
  assert.doesNotMatch(offer, /VP8|VP9|AV1/, 'H.264 only');
  assert.match(offer, /a=msid:- /, 'no stream id on the video');
  await b.waitForSelector(`.voice-member[data-voice-sid="${adaSid}"] .live-badge`, { timeout: 5000 });
  await b.click(`.voice-member[data-voice-sid="${adaSid}"] .live-badge`);
  await b.waitForFunction(() => document.querySelector('#screen-view').dataset.state === 'live' && document.querySelector('.screen-video').videoWidth > 0, null, { timeout: 15000 });
  const got = await b.evaluate(async ([sid, mid]) => {
    const report = await yap.voice.peers.get(sid).pc.getStats();
    let codec = null;
    report.forEach((x) => {
      if (x.type === 'inbound-rtp' && x.kind === 'video' && x.mid === mid && x.codecId) codec = report.get(x.codecId);
    });
    const stream = yap.voice.screens.get(sid);
    return { mime: codec?.mimeType, fmtp: codec?.sdpFmtpLine, video: stream.getVideoTracks().length, audio: stream.getAudioTracks().length };
  }, [adaSid, mid]);
  assert.equal(got.mime, 'video/H264');
  assert.match(got.fmtp, /profile-level-id=42e01f/);
  assert.match(got.fmtp, /packetization-mode=1/);
  assert.deepEqual([got.video, got.audio], [1, 1], 'its video and its sound, as one screen');
  await b.screenshot({ path: path.join(OUT, 'desktop-screen-native.png') });
  // The "native app" stops: it takes its tracks off and says so.
  await a.evaluate(() => {
    for (const tr of [window.native.v, window.native.s]) {
      tr.sender.replaceTrack(null);
      tr.direction = 'inactive';
    }
    for (const t of window.native.screen.getTracks()) t.stop();
    window.native.ac.close();
    yap.ui.send({ t: 'voice.state', muted: false, deafened: false, screen: false });
  });
  await until(async () => (await viewer(b)).state === 'stopped', 5000, 'stopped');
  await b.waitForFunction(() => !yap.state.screenSid, null, { timeout: 5000 });
  await until(() => b.evaluate((sid) => !yap.voice.screens.has(sid), adaSid), 5000, 'gone on Ben\'s side');
  assert.equal(await a.evaluate((sid) => yap.voice.peers.get(sid).pc.connectionState, benSid), 'connected', 'voice still up');
});

await test('voice popover: someone\'s volume (0–200 %, step 5) changes how loud they are and shows on their row ("50 %", a muted speaker and a dimmed ring at 0), Ctrl/⌘ + scroll too; your own row has none', async () => {
  try {
    const [adaSid, adaUid] = await a.evaluate(() => [yap.state.sid, yap.settings.uid]);
    const benSid = await b.evaluate(() => yap.state.sid);
    await b.click(`.voice-member[data-voice-sid="${adaSid}"] .name`);
    const slider = '#popover:not([hidden]) input[type=range][data-volume-uid]';
    await b.waitForSelector(slider);
    assert.deepEqual(await b.$eval(slider, (el) => [el.min, el.max, el.step, el.title, el.closest('label').textContent.trim().startsWith('Volume')]), ['0', '200', '5', 'Past 100% boosts quiet people', true]);
    const set = (v) => b.$eval(slider, (el, value) => {
      el.value = String(value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }, v);
    await set(150);
    assert.deepEqual(await b.evaluate(([sid, uid]) => {
      const peer = yap.voice.peers.get(sid);
      return [yap.settings.volumes[uid], peer.boost?.gain.gain.value ?? null, peer.audio.muted];
    }, [adaSid, adaUid]), [1.5, 1.5, true], '150 %: boosted through WebAudio (the element plays muted)');
    assert.equal(await b.textContent('#popover output'), '150%');
    const row = `.voice-member[data-voice-sid="${adaSid}"]`;
    assert.equal(await b.textContent(`${row} .vol-tag`), '150\u00a0%', 'her row says what you set');
    await set(50);
    assert.deepEqual(await b.evaluate(([sid, uid]) => {
      const peer = yap.voice.peers.get(sid);
      return [yap.settings.volumes[uid], peer.boost ?? null, peer.audio.volume];
    }, [adaSid, adaUid]), [0.5, null, 0.5], '50 %: the element plays it quieter');
    assert.equal(await b.textContent(`${row} .vol-tag`), '50\u00a0%');
    // All the way down while she talks: a muted speaker on her row and her ring, dimmed.
    await set(0);
    await b.waitForSelector(`${row}.silenced.speaking .vol-tag.off svg`, { timeout: 3000 });
    const ring = (sel) => b.$eval(`${sel} .avatar`, (el) => getComputedStyle(el).boxShadow);
    assert.match(await ring(row), /rgb\(90, 143, 90\) 0px 0px 0px 3px/, 'a dimmed ring');
    // Ben's synthetic voice has gaps between words: wait for him to be speaking before reading his ring.
    await b.waitForSelector(`.voice-member[data-voice-sid="${benSid}"].speaking .avatar`, { timeout: 5000 });
    assert.match(await ring(`.voice-member[data-voice-sid="${benSid}"].speaking`), /rgb\(0, 224, 0\)/, 'Ben\'s own ring is the bright one');
    await b.locator('#voice-channels').screenshot({ path: path.join(OUT, 'TW6-row-muted.png') });
    await set(100);
    assert.equal(await b.locator(`${row} .vol-tag`).count(), 0, '100 %: nothing on the row');
    assert.equal(await b.locator(`${row}.silenced`).count(), 0);
    await b.keyboard.press('Escape');
    await b.click('.chat-header');
    // Ctrl/⌘ + scroll over her row: 5 % a notch, up = louder (and not the page's zoom).
    await b.hover(`${row} .name`);
    await b.keyboard.down('Control');
    await b.mouse.wheel(0, -100);
    await b.keyboard.up('Control');
    await until(() => b.evaluate((uid) => yap.settings.volumes[uid] === 1.05, adaUid), 2000, '105 %');
    assert.equal(await b.textContent(`${row} .vol-tag`), '105\u00a0%');
    await b.keyboard.down('Meta');
    for (let i = 0; i < 3; i++) await b.mouse.wheel(0, 100);
    await b.keyboard.up('Meta');
    await until(() => b.evaluate((uid) => yap.settings.volumes[uid] === 0.9, adaUid), 2000, '90 % (⌘ too)');
    assert.equal(await b.evaluate(([sid]) => yap.voice.peers.get(sid).audio.volume, [adaSid]), 0.9, 'and she plays at that');
    await b.locator('#voice-channels').screenshot({ path: path.join(OUT, 'TW6-row-volume.png') });
    await b.mouse.wheel(0, 100); // no modifier: the sidebar just scrolls
    assert.equal(await b.evaluate((uid) => yap.settings.volumes[uid], adaUid), 0.9);
    await b.evaluate((uid) => {
      const input = Object.assign(document.createElement('input'), { value: '100' });
      input.dataset.volumeUid = uid;
      document.body.append(input);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.remove();
    }, adaUid);
    // Your own row isn't a button.
    assert.equal(await b.locator(`.voice-member[data-voice-sid="${benSid}"][data-action]`).count(), 0);
  } finally {
    await b.evaluate((uid) => {
      const input = Object.assign(document.createElement('input'), { value: '100' });
      input.dataset.volumeUid = uid;
      document.body.append(input);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.remove();
      document.querySelector('#popover').hidden = true;
    }, await a.evaluate(() => yap.settings.uid)).catch(() => {});
    await b.keyboard.up('Control');
    await b.keyboard.up('Meta');
  }
});

await test('Settings › Voice › Hear yourself: 4 s through the gate and an Opus loopback, played back', async () => {
  await a.click('[data-action="open-settings"]');
  const voiceTab = await a.evaluate(() => [...document.querySelectorAll('#settings-tabs [data-tab]')].find((b) => b.textContent === 'Voice')?.dataset.tab);
  await a.click(`#settings-tabs [data-tab="${voiceTab}"]`);
  await a.click('[data-action="hear-yourself"]');
  await a.waitForFunction(() => /Recording/.test(document.querySelector('#hear-status').textContent), null, { timeout: 8000 });
  await a.waitForFunction(() => /how you sound/.test(document.querySelector('#hear-status').textContent), null, { timeout: 15000 });
  const t = await a.evaluate(() => {
    const { samples, sampleRate, codec } = yap.voice.lastTest;
    const rms = Math.sqrt(samples.reduce((s, x) => s + x * x, 0) / samples.length);
    return { secs: samples.length / sampleRate, codec, rms };
  });
  assert.equal(t.codec, true, 'went through a real Opus connection');
  assert.ok(t.secs > 3.8, `${t.secs} s recorded`);
  assert.ok(t.rms > 0.01, `it heard the mic (rms ${t.rms.toFixed(3)})`);
  await a.keyboard.press('Escape');
});

/**
 * In a page: `__tap(track)` times every click on `track` (an analyser, polled every 10 ms; a click =
 * a sample over 0.1 after 300 ms without one) → { onsets: [wall-clock ms], stop() }.
 */
const TAP = () => {
  globalThis.__tap ??= async (track) => {
    const ctx = new AudioContext({ sampleRate: 48000 });
    await ctx.resume();
    const source = ctx.createMediaStreamSource(new MediaStream([track]));
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 4096; // 85 ms, so nothing slips by between two looks
    const hush = ctx.createGain();
    hush.gain.value = 0;
    source.connect(analyser).connect(hush).connect(ctx.destination);
    const buf = new Float32Array(analyser.fftSize);
    const onsets = [];
    let last = -1;
    const timer = setInterval(() => {
      const t = ctx.currentTime;
      const now = performance.timeOrigin + performance.now();
      analyser.getFloatTimeDomainData(buf);
      if (ctx.currentTime !== t) return; // the audio thread moved on meanwhile: look again next time
      const k = buf.findIndex((x) => Math.abs(x) > 0.1);
      if (k < 0) return;
      const at = t - (buf.length - k) / ctx.sampleRate;
      if (at > last + 0.3) {
        last = at;
        onsets.push(now - (t - at) * 1000);
      }
    }, 10);
    return { onsets, stop: () => (clearInterval(timer), ctx.close()) };
  };
};

/**
 * Ada's voice → her OBS page, in ms (the median of `clicks`): just for this, Ada sends 40 ms 1 kHz
 * clicks instead of her mic on her connection to the OBS page, and both pages time each click the
 * same way (TAP: where it leaves Ada, and the OBS page's receiver track, after the jitter buffer).
 * So it's the encoder, the network and the OBS page's jitter buffer; not Ada's own audio graph.
 */
async function obsClickDelay(obs, obsSid, adaSid, clicks = 8) {
  await a.evaluate(TAP);
  await obs.evaluate(TAP);
  await a.evaluate(async (sid) => {
    const ctx = new AudioContext({ sampleRate: 48000 });
    await ctx.resume();
    const tone = ctx.createOscillator();
    tone.frequency.value = 1000;
    const gain = ctx.createGain();
    gain.gain.value = 0;
    const out = ctx.createMediaStreamDestination();
    out.channelCount = 1;
    tone.connect(gain).connect(out);
    tone.start();
    const track = out.stream.getAudioTracks()[0];
    const peer = yap.voice.peers.get(sid);
    await peer.pc.getTransceivers().find((t) => t.receiver.track.kind === 'audio').sender.replaceTrack(track);
    globalThis.__clicker = { ctx, gain, peer, tap: await __tap(track) };
  }, obsSid);
  await obs.evaluate(async (sid) => {
    const receiver = yapObs.voice.peers.get(sid).pc.getReceivers().find((r) => r.track.kind === 'audio');
    globalThis.__played = await __tap(receiver.track);
  }, adaSid);
  try {
    await a.waitForTimeout(1500); // the clicker's silence settles in
    for (let i = 0; i < clicks; i++) {
      await a.evaluate(() => {
        const { ctx, gain } = __clicker;
        const t = ctx.currentTime + 0.005;
        gain.gain.setValueAtTime(0.5, t);
        gain.gain.setValueAtTime(0, t + 0.04);
      });
      await a.waitForTimeout(550 + Math.random() * 100);
    }
    await a.waitForTimeout(400);
    const sent = await a.evaluate(() => __clicker.tap.onsets);
    const played = await obs.evaluate(() => __played.onsets);
    const delays = sent.map((s) => played.find((p) => p > s && p - s < 1000) - s).filter((d) => d > 0).sort((x, y) => x - y);
    return { median: delays.length ? delays[Math.floor(delays.length / 2)] : null, heard: delays.length, sent: sent.length };
  } finally {
    await a.evaluate(() => {
      const { ctx, peer, tap } = __clicker;
      peer.setTrack(yap.voice.outTrack); // her mic again
      tap.stop();
      ctx.close();
    });
    await obs.evaluate(() => __played.stop());
  }
}
/**
 * How cleanly a 1 kHz tone (-12 dBFS) from Ada comes out of her OBS page: Ada sends it instead of
 * her mic on her connection to the OBS page for ~2.5 s; the OBS page fits a sine to 0.68 s of its
 * receiver track and reports what's left (THD+N, dB), plus what its jitter buffer had to patch over.
 */
async function obsToneClean(obs, obsSid, adaSid) {
  await a.evaluate(async (sid) => {
    const ctx = new AudioContext({ sampleRate: 48000 });
    await ctx.resume();
    const tone = ctx.createOscillator();
    tone.frequency.value = 1000;
    const gain = ctx.createGain();
    gain.gain.value = 0.25;
    const out = ctx.createMediaStreamDestination();
    out.channelCount = 1;
    tone.connect(gain).connect(out);
    tone.start();
    const peer = yap.voice.peers.get(sid);
    await peer.pc.getTransceivers().find((t) => t.receiver.track.kind === 'audio').sender.replaceTrack(out.stream.getAudioTracks()[0]);
    globalThis.__tone = { ctx, peer };
  }, obsSid);
  try {
    await a.waitForTimeout(1200); // the tone settles in (and Ada's own mic is out of the jitter buffer)
    const inbound = () => obs.evaluate(async (sid) => {
      let r = null;
      (await yapObs.voice.peers.get(sid).pc.getStats()).forEach((s) => { if (s.type === 'inbound-rtp' && s.kind === 'audio') r = s; });
      return { samples: r.totalSamplesReceived, concealed: r.concealedSamples - r.silentConcealedSamples, lost: r.packetsLost };
    }, adaSid);
    const before = await inbound();
    const fit = await obs.evaluate(async (sid) => {
      const receiver = yapObs.voice.peers.get(sid).pc.getReceivers().find((r) => r.track.kind === 'audio');
      const ctx = new AudioContext({ sampleRate: 48000 });
      await ctx.resume();
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 32768;
      const hush = ctx.createGain();
      hush.gain.value = 0;
      ctx.createMediaStreamSource(new MediaStream([receiver.track])).connect(analyser).connect(hush).connect(ctx.destination);
      await new Promise((r) => setTimeout(r, 900));
      const y = new Float32Array(analyser.fftSize);
      analyser.getFloatTimeDomainData(y);
      ctx.close();
      // least squares: a·sin + b·cos + c at 1 kHz; what's left is distortion + noise
      let ss = 0, cc = 0, sc = 0, ys = 0, yc = 0, sum = 0;
      for (let i = 0; i < y.length; i++) {
        const w = (2 * Math.PI * 1000 * i) / 48000;
        const sn = Math.sin(w), cs = Math.cos(w);
        ss += sn * sn; cc += cs * cs; sc += sn * cs; ys += y[i] * sn; yc += y[i] * cs; sum += y[i];
      }
      const det = ss * cc - sc * sc;
      const A = (ys * cc - yc * sc) / det, B = (yc * ss - ys * sc) / det, C = sum / y.length;
      let res = 0, fitE = 0;
      for (let i = 0; i < y.length; i++) {
        const w = (2 * Math.PI * 1000 * i) / 48000;
        const m = A * Math.sin(w) + B * Math.cos(w);
        fitE += m * m;
        res += (y[i] - m - C) ** 2;
      }
      return { thdn: 10 * Math.log10(res / fitE), levelDb: 10 * Math.log10(fitE / y.length) };
    }, adaSid);
    const after = await inbound();
    return { ...fit, concealed: (after.concealed - before.concealed) / Math.max(1, after.samples - before.samples), lost: after.lost - before.lost };
  } finally {
    await a.evaluate(() => {
      const { ctx, peer } = __tone;
      peer.setTrack(yap.voice.outTrack); // her mic again
      ctx.close();
    });
  }
}
let obsMeasured;

await test('OBS link: a listen-only page joins the call and receives audio', async () => {
  await a.click('[data-action="open-settings"]');
  const voiceTab = await a.evaluate(() => [...document.querySelectorAll('#settings-tabs [data-tab]')].find((b) => b.textContent === 'Voice')?.dataset.tab);
  await a.click(`#settings-tabs [data-tab="${voiceTab}"]`);
  await a.click('#voice-advanced > summary'); // OBS lives under Advanced
  await a.selectOption('#obs-channel', 'lounge');
  await a.click('[data-action="create-obs"]');
  await a.waitForSelector('#obs-made:not([hidden])', { timeout: 5000 });
  // "Only one person" and "Sound only" are plain options that shape the link.
  await a.selectOption('#obs-only', 'Ben');
  await a.check('#obs-sound-only');
  assert.match(await a.inputValue('#obs-link'), /\/obs#listen=[^&]+&channel=lounge&only=Ben&overlay=0$/, 'no invite key in an OBS link');
  await a.selectOption('#obs-only', ' others');
  assert.match(await a.inputValue('#obs-link'), /\/obs#listen=[^&]+&channel=lounge&others=1&overlay=0$/, 'everyone but you');
  await a.selectOption('#obs-only', '');
  await a.uncheck('#obs-sound-only');
  const link = await a.inputValue('#obs-link');
  assert.match(link, /\/obs#listen=[^&]+&channel=lounge$/, 'everyone, you too: the default');
  await a.keyboard.press('Escape');
  const obs = await a.context().newPage();
  await obs.addInitScript(() => {
    // Count the page's own audio graphs (it should make none: straight through).
    globalThis.__audioContexts = 0;
    const Real = globalThis.AudioContext;
    globalThis.AudioContext = class extends Real {
      constructor(...args) {
        super(...args);
        globalThis.__audioContexts++;
      }
    };
  });
  await obs.goto(link);
  await obs.click('#start');
  await obs.waitForFunction(() => yapObs.state.inCall && [...yapObs.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
  // It listens without showing up in the channel (and without a chime).
  const obsSid = await obs.evaluate(() => yapObs.state.sid);
  await b.waitForTimeout(500);
  assert.equal(await b.locator(`#voice-channels [data-voice-sid="${obsSid}"], #voice-channels .obs-listening`).count(), 0, 'the OBS link is not listed');
  try {
    // Audio packets arrive (give RTP a moment to start). (Polled from here: waitForFunction takes
    // an async function's promise as a truthy answer at once, so it never waited for the bytes.)
    const received = () => obs.evaluate(async () => {
      let total = 0;
      for (const p of yapObs.voice.peers.values()) {
        const stats = await p.pc.getStats();
        stats.forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'audio') total += r.bytesReceived; });
      }
      return total;
    });
    await until(async () => (await received()) > 1000, 10000, 'audio at the OBS page');
    assert.ok((await received()) > 1000, 'audio bytes arrived');
    await obs.screenshot({ path: path.join(OUT, 'obs-overlay.png') });
    // Talking and stopping changes that person's card in place (so its lift and glow ease on stream), not the whole overlay.
    const card = await obs.evaluate((sid) => {
      const before = [...document.querySelectorAll('#stage .who')];
      const mine = before.find((el) => el.dataset.sid === sid);
      for (const speaking of [true, false]) yapObs.voice.dispatchEvent(new CustomEvent('speaking', { detail: { sid, speaking } }));
      return { found: Boolean(mine), same: before.every((el) => el.isConnected), quiet: mine?.classList.contains('quiet') && !mine.classList.contains('speaking') };
    }, await a.evaluate(() => yap.state.sid));
    assert.deepEqual(card, { found: true, same: true, quiet: true }, 'the card, in place');

    // The way VDO.Ninja's viewer plays a call (web/js/obs.js): each person from an element of their
    // own at full volume (OBS sets the volume), no audio graph of its own, the browser's own jitter
    // buffer and packet size (v0.45.0's shortest buffer and 10 ms packets are gone).
    const adaSid = await a.evaluate(() => yap.state.sid);
    const how = await obs.evaluate((sid) => {
      const peer = yapObs.voice.peers.get(sid);
      const receivers = [...yapObs.voice.peers.values()].flatMap((p) => p.pc.getReceivers()).filter((r) => r.track.kind === 'audio');
      const voiceSection = peer.pc.localDescription.sdp.split(/\r?\nm=/).find((s) => s.startsWith('audio'));
      return {
        contexts: globalThis.__audioContexts,
        played: peer.audio.srcObject?.getAudioTracks()[0] === peer.pc.getReceivers().find((r) => r.track.kind === 'audio').track && !peer.audio.muted && peer.audio.volume === 1 && !peer.audio.paused,
        targets: receivers.map((r) => ('jitterBufferTarget' in r ? r.jitterBufferTarget : r.playoutDelayHint ?? null)),
        ptime: /^a=ptime:/m.test(voiceSection),
      };
    }, adaSid);
    const pps = async () => {
      const read = () => obs.evaluate(async (sid) => {
        let packets = 0;
        (await yapObs.voice.peers.get(sid).pc.getStats()).forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'audio') packets = r.packetsReceived; });
        return { packets, at: performance.now() };
      }, adaSid);
      const s1 = await read();
      await obs.waitForTimeout(1000);
      const s2 = await read();
      return ((s2.packets - s1.packets) * 1000) / (s2.at - s1.at);
    };
    obsMeasured = { pps: Math.round(await pps()), ...(await obsClickDelay(obs, obsSid, adaSid)) };
    console.log(`    OBS page: Ada's click heard ${obsMeasured.median?.toFixed(1)} ms later (${obsMeasured.heard}/${obsMeasured.sent} clicks), ${obsMeasured.pps} packets/s from her`);
    assert.equal(how.contexts, 0, 'no audio graph on the OBS page');
    assert.ok(how.played, 'Ada plays straight from her own element, at full volume');
    assert.ok(how.targets.every((t) => t == null), `the browser's own jitter buffer (${how.targets})`);
    assert.ok(!how.ptime, 'no packet size asked for');
    assert.ok(obsMeasured.pps > 30, `packets keep coming: ${obsMeasured.pps} a second`);
    assert.ok(obsMeasured.heard >= 6, `Ada's clicks reached the OBS page (${obsMeasured.heard} of ${obsMeasured.sent})`);
    assert.ok(obsMeasured.median < 150, `the OBS page plays Ada ${obsMeasured.median} ms after she sends`);

    // Clean: a tone from Ada comes out with distortion + noise far below it, nothing patched over.
    const clean = await obsToneClean(obs, obsSid, adaSid);
    obsMeasured.clean = clean;
    console.log(`    OBS page: Ada's 1 kHz tone THD+N ${clean.thdn.toFixed(1)} dB at ${clean.levelDb.toFixed(1)} dBFS, ${(clean.concealed * 100).toFixed(2)} % patched over, ${clean.lost} lost`);
    assert.ok(clean.levelDb > -18 && clean.levelDb < -12, `the tone arrives at its level (${clean.levelDb.toFixed(1)} dBFS, sent at -15)`);
    assert.ok(clean.thdn < -35, `THD+N ${clean.thdn.toFixed(1)} dB`);
    assert.ok(clean.concealed < 0.01, `${(clean.concealed * 100).toFixed(2)} % patched over`);
    assert.equal(clean.lost, 0, 'no packets lost');

    // An element that pauses plays again by itself (like VDO.Ninja's viewer).
    await obs.evaluate((sid) => yapObs.voice.peers.get(sid).audio.pause(), adaSid);
    await obs.waitForFunction((sid) => !yapObs.voice.peers.get(sid).audio.paused, adaSid, { timeout: 4000 });

    // It keeps OBS capturing through pauses (web/js/obs.js › keepAlive), and only then: while anyone
    // is heard (Ada and Ben's mics are on), nothing else plays; once both are muted, after a second,
    // an inaudible 20 Hz tone at -60 dBFS (above Chromium's -72.25 dBFS "silent" line); it goes as
    // soon as someone talks again.
    const keepAlive = () => obs.evaluate(() => ({ on: yapObs.keepAlive.on, playing: !yapObs.keepAlive.element.paused }));
    assert.deepEqual(await keepAlive(), { on: false, playing: false }, 'nothing mixed into their voices');
    await a.click('[data-action="toggle-mute"]');
    await b.click('[data-action="toggle-mute"]');
    try {
      await obs.waitForFunction(() => yapObs.keepAlive.on && !yapObs.keepAlive.element.paused && yapObs.keepAlive.element.volume === 1, null, { timeout: 5000 });
      const keepDb = await obs.evaluate(async () => {
        const ctx = new AudioContext();
        const analyser = ctx.createAnalyser();
        analyser.fftSize = 32768;
        ctx.createMediaStreamSource(yapObs.keepAlive.element.captureStream()).connect(analyser);
        await new Promise((r) => setTimeout(r, 1200));
        const buf = new Float32Array(analyser.fftSize);
        analyser.getFloatTimeDomainData(buf);
        ctx.close();
        return 10 * Math.log10(buf.reduce((sum, v) => sum + v * v, 0) / buf.length);
      });
      obsMeasured.keepDb = keepDb;
      assert.equal(await obs.evaluate(() => yapObs.keepAlive.element.loop), true, 'looping');
      assert.ok(keepDb > -63 && keepDb < -57, `the keep-alive tone is at ${keepDb.toFixed(1)} dBFS`);
    } finally {
      await a.click('[data-action="toggle-mute"]');
      await b.click('[data-action="toggle-mute"]');
    }
    await obs.waitForFunction(() => !yapObs.keepAlive.on && yapObs.keepAlive.element.paused, null, { timeout: 3000 });

    // It carries everyone, its own account included: Ada's link plays Ada (her voice from another
    // device of hers, on a stream) and Ben, and connects to nobody else.
    const carried = await obs.evaluate(() => [...yapObs.voice.peers.values()].map((p) => yapObs.state.users.get(p.uid)?.name).sort());
    assert.deepEqual(carried, ['Ada', 'Ben'], 'its own account too');

    // &only=Ben: one source per person. Ada's element is muted (not turned down by a gain), Ben plays,
    // and the overlay shows just Ben, speaking (from his packets' audio level: no audio graph here either).
    const benOnly = await a.context().newPage();
    try {
      await benOnly.goto(`${link}&only=Ben`);
      await benOnly.click('#start');
      await benOnly.waitForFunction(() => yapObs.state.inCall && yapObs.voice.peers.size === 2 && [...yapObs.voice.peers.values()].every((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
      await benOnly.waitForSelector('#stage .who.speaking .name:text-is("Ben")', { timeout: 8000 });
      const only = await benOnly.evaluate(() => ({
        playing: [...yapObs.voice.peers.values()].map((p) => [yapObs.state.users.get(p.uid)?.name, !p.audio.muted && p.audio.volume === 1]).sort(),
        names: [...document.querySelectorAll('#stage .who .name')].map((e) => e.textContent),
        elements: document.querySelectorAll('#audio-sink audio').length,
      }));
      assert.deepEqual(only.playing, [['Ada', false], ['Ben', true]]);
      assert.deepEqual(only.names, ['Ben']);
      assert.equal(only.elements, 2, 'an <audio> element per person');
      const benOnlySid = await benOnly.evaluate(() => yapObs.state.sid);
      assert.equal(await obs.evaluate((sid) => yapObs.voice.peers.has(sid), benOnlySid), false, 'two OBS links never connect to each other');
      await benOnly.locator('#stage').screenshot({ path: path.join(OUT, 'obs-only-ben.png') });
    } finally {
      await benOnly.close();
    }

    // Ada leaves (PROTOCOL.md › Voice › OBS links in a call): her OBS link listens on for a while,
    // and Lounge shows her greyed, "Ada's OBS is still listening" (not a call member: no menu).
    await a.click('[data-action="leave-voice"]');
    const row = '#voice-channels .voice-member.obs-listening';
    await b.waitForSelector(`${row}:has-text("Ada's OBS is still listening")`, { timeout: 5000 });
    assert.equal(await b.locator(`${row}[data-action]`).count(), 0, 'no volume menu');
    assert.equal(await obs.evaluate(() => yapObs.state.inCall), true, 'still listening');
    await b.locator('#voice-channels').screenshot({ path: path.join(OUT, 'MW2-after.png') });
    // Then (the test server's TEST_OBS_GRACE, not 5 minutes) the server takes it out: Paused, and the row goes.
    await obs.waitForFunction(() => !yapObs.state.inCall && document.querySelector('#note').textContent === 'Paused: waiting for Ada to join Lounge.', null, { timeout: 20000 });
    assert.deepEqual(await keepAlive(), { on: false, playing: false }, 'Paused: no keep-alive (OBS may stop capturing)');
    await obs.screenshot({ path: path.join(OUT, 'MW2-obs-paused.png') });
    await b.waitForSelector(row, { state: 'detached', timeout: 5000 });
    // Ada comes back: the OBS page joins again by itself.
    await a.click('button.channel.voice:has-text("Lounge")');
    await obs.waitForFunction(() => yapObs.state.inCall && [...yapObs.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
    assert.equal(await obs.isHidden('#note'), true, 'not paused any more');
  } finally {
    await obs.close();
  }
  // Ada is back in the call with Ben for the tests after this one.
  await a.waitForFunction(() => yap.state.voiceChannel === 'lounge' && [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
});

await test('board opens inside Yap during a call; cursors show for others', async () => {
  await a.click('[data-action="open-view"][data-view="board"]');
  await b.click('[data-action="open-view"][data-view="board"]');
  const cardA = a.frameLocator('#board-frame').locator('[data-lane] [data-id]').first();
  await cardA.waitFor({ timeout: 10000 });
  await b.frameLocator('#board-frame').locator('[data-lane] [data-id]').first().waitFor({ timeout: 10000 });
  assert.equal(await a.isVisible('#messages'), false, 'chat is swapped out for the board');
  await cardA.hover({ position: { x: 30, y: 12 } });
  await a.mouse.move(210, 300, { steps: 4 });
  await cardA.hover({ position: { x: 40, y: 14 } });
  const cursor = b.frameLocator('#board-frame').locator('.yap-cursor', { hasText: 'Ada' });
  await cursor.waitFor({ timeout: 5000 });
  await b.waitForSelector('#links .viewers .voice-member:has-text("Ada")', { timeout: 5000 });
  // Still in the call the whole time.
  assert.ok(await a.evaluate(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected')), 'voice survives the view switch');
  await b.screenshot({ path: path.join(OUT, 'desktop-board-cursors.png') });
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.isVisible('#messages'), true);
  await cursor.waitFor({ state: 'detached', timeout: 5000 });
});

await test('links to places and channels: the address says what is open, the header copies it, a link in chat opens here, a fresh tab lands there', async () => {
  await catchCopies(a);
  for (const place of ['space', 'board', 'when']) {
    await a.click(`[data-action="open-view"][data-view="${place}"]`);
    assert.equal(await a.evaluate(() => location.hash), `#place=${place}`, place);
  }
  await a.click('#copy-link');
  await a.waitForFunction(() => globalThis.__copied);
  assert.equal(await a.evaluate(() => globalThis.__copied), `${BASE}/#place=when`);
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.evaluate(() => location.hash), '#channel=general');
  // Your Personal space has no link (it's yours alone): nothing in the address, no button.
  const personal = await a.evaluate(() => [...yap.state.channels.values()].find((c) => c.kind === 'personal').id);
  await a.click(`[data-action="open-channel"][data-id="${personal}"]`);
  assert.deepEqual([await a.evaluate(() => location.hash), await a.isHidden('#copy-link')], ['', true]);
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.screenshot({ path: path.join(OUT, 'desktop-copy-link.png') });
  // Ben shares a link in chat; Ada clicks it: When's good opens in this tab, not a new one.
  const tabs = a.context().pages().length;
  await b.click('[data-action="open-channel"][data-id="general"]');
  await b.fill('#input', `pick a time ${BASE}/#place=when ${RUN}`);
  await b.press('#input', 'Enter');
  await a.click(`.msg a[href="${BASE}/#place=when"] >> nth=-1`);
  await a.waitForFunction(() => yap.state.view === 'when' && location.hash === '#place=when');
  assert.equal(a.context().pages().length, tabs, 'no new tab');
  // A fresh tab opened on a link lands there; one pasted into its address bar goes there too.
  const tab = await a.context().newPage();
  await tab.goto(`${BASE}/#place=draw`);
  await tab.waitForFunction(() => yap.state.status === 'online' && yap.state.view === 'draw', null, { timeout: 10000 });
  await tab.evaluate(() => { location.hash = 'channel=general'; });
  await tab.waitForFunction(() => yap.state.view === 'chat' && yap.settings.channel === 'general');
  await tab.evaluate(() => { location.hash = 'place=nowhere'; });
  await tab.waitForSelector('#toast:not([hidden]):has-text("doesn’t have")', { timeout: 3000 });
  await tab.close();
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('draw together: strokes show up live for the other person', async () => {
  await a.click('[data-action="open-view"][data-view="draw"]');
  await b.click('[data-action="open-view"][data-view="draw"]');
  await a.waitForFunction(() => yap.ui.draw && document.querySelector('.draw-paper').clientWidth > 100);
  await b.waitForFunction(() => yap.ui.draw);
  const box = await a.locator('.draw-paper').boundingBox();
  await a.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.3);
  await a.mouse.down();
  for (let i = 1; i <= 12; i++) await a.mouse.move(box.x + box.width * (0.2 + i * 0.04), box.y + box.height * (0.3 + Math.sin(i / 2) * 0.15));
  await a.mouse.up();
  await b.waitForFunction(() => yap.ui.draw.strokes.length >= 1, null, { timeout: 5000 });
  await a.click('.draw-colors [data-color="#0000ff"]');
  await a.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.7);
  await a.mouse.down();
  for (let i = 1; i <= 10; i++) await a.mouse.move(box.x + box.width * (0.5 + i * 0.03), box.y + box.height * (0.7 - i * 0.03));
  await a.mouse.up();
  await b.waitForFunction(() => yap.ui.draw.strokes.length >= 2, null, { timeout: 5000 });
  // A box and some words.
  const before = await b.evaluate(() => yap.ui.draw.strokes.length);
  await a.click('[data-shape="rect"]');
  await a.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.15);
  await a.mouse.down();
  await a.mouse.move(box.x + box.width * 0.85, box.y + box.height * 0.45, { steps: 5 });
  await a.mouse.up();
  await b.waitForFunction((n) => yap.ui.draw.strokes.some((s) => s.k === 'poly') && yap.ui.draw.strokes.length > n, before, { timeout: 5000 });
  await a.click('[data-shape="text"]');
  await a.mouse.click(box.x + box.width * 0.1, box.y + box.height * 0.75);
  await a.keyboard.type('hi from Ada');
  await a.keyboard.press('Enter');
  await b.waitForFunction(() => yap.ui.draw.strokes.some((s) => s.k === 'text' && s.text === 'hi from Ada'), null, { timeout: 5000 });
  await a.click('[data-shape="pen"]');
  await b.waitForTimeout(300);
  await b.screenshot({ path: path.join(OUT, 'desktop-draw.png') });
  // Pixel-art page: drawn at 640×400 and blown up with hard edges.
  assert.deepEqual(await a.evaluate(() => [document.querySelector('.draw-layer').width, document.querySelector('.draw-layer').height]), [640, 400]);
  // GIF: the wiggle as a looping animation, downloaded and kept in the gallery.
  const [download] = await Promise.all([a.waitForEvent('download'), a.click('[data-tool="gif"]')]);
  assert.match(download.suggestedFilename(), /\.gif$/);
  const gif = fs.readFileSync(await download.path());
  assert.equal(gif.subarray(0, 6).toString(), 'GIF89a');
  const decoded = await a.evaluate(async (b64) => {
    const img = new Image();
    img.src = `data:image/gif;base64,${b64}`;
    await img.decode();
    return [img.naturalWidth, img.naturalHeight];
  }, gif.toString('base64'));
  assert.deepEqual(decoded, [640, 400], 'Chromium decodes it');
  await until(() => b.evaluate(async () => {
    const first = document.querySelector('.gallery-item');
    return Boolean(first) && (await fetch(first.href)).headers.get('content-type') === 'image/gif';
  }), 8000, 'the GIF in the gallery');
  // Share puts the drawing in the chat.
  await a.click('[data-tool="share"]');
  await b.waitForFunction(() => [...yap.state.history.get('general')?.messages ?? []].some((m) => m.image?.width === 640), null, { timeout: 8000 });
});

await test('draw layers: draw on the top layer, erase it without touching the bottom, hide one, clear one for everyone', async () => {
  // Both are still on Draw from the test before; Ada's page has her strokes on the bottom layer.
  const box = await a.locator('.draw-paper').boundingBox();
  const at = (x, y) => [box.x + box.width * x, box.y + box.height * y];
  const ink = (page, layer) => page.evaluate((l) => {
    const c = document.querySelector(`[data-layer-canvas="${l}"]`);
    const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 3; i < d.length; i += 4) if (d[i]) n++;
    return n;
  }, layer);
  await a.evaluate(() => {
    yap.ui.draw.wiggle && document.querySelector('[data-tool="wiggle"]').click(); // straight lines: pixel counts stay put
  });
  const bottomBefore = await ink(a, 0);
  assert.ok(bottomBefore > 0, 'the bottom layer has ink');
  // Layer 3 (top), a thick red bar across the page, right over the bottom ink.
  await a.keyboard.press('3');
  assert.equal(await a.getAttribute('[data-layer="2"]', 'aria-pressed'), 'true');
  await a.click('.draw-colors [data-color="#ff0000"]');
  await a.click('[data-size="70"]');
  await a.mouse.move(...at(0.1, 0.5));
  await a.mouse.down();
  await a.mouse.move(...at(0.9, 0.5), { steps: 12 });
  await a.mouse.up();
  await b.waitForFunction(() => yap.ui.draw.strokes.some((s) => s.l === 2), null, { timeout: 5000 });
  assert.ok((await ink(a, 2)) > 0, 'ink on the top layer');
  assert.equal(await ink(a, 0), bottomBefore, 'the bottom layer is untouched');
  // The eraser on the top layer takes the red away but leaves the bottom alone.
  await a.click('[data-tool="eraser"]');
  await a.click('[data-size="160"]');
  await a.mouse.move(...at(0.05, 0.5));
  await a.mouse.down();
  await a.mouse.move(...at(0.95, 0.5), { steps: 12 });
  await a.mouse.up();
  await a.click('[data-tool="eraser"]');
  await a.waitForFunction(() => yap.ui.draw.strokes.some((s) => s.l === 2 && s.e));
  assert.equal(await ink(a, 0), bottomBefore, 'erasing the top layer kept the bottom');
  // Draw something on the middle layer too; Ben hides it — just for him.
  await a.keyboard.press('2');
  await a.click('[data-size="32"]');
  await a.click('[data-shape="ellipse"]');
  await a.mouse.move(...at(0.3, 0.2));
  await a.mouse.down();
  await a.mouse.move(...at(0.7, 0.8), { steps: 6 });
  await a.mouse.up();
  await a.click('[data-shape="pen"]');
  await b.waitForFunction(() => yap.ui.draw.strokes.some((s) => s.l === 1), null, { timeout: 5000 });
  await b.click('[data-eye="1"]');
  assert.equal(await b.isVisible('[data-layer-canvas="1"]'), false);
  assert.equal(await a.isVisible('[data-layer-canvas="1"]'), true, 'hiding is just for you');
  await b.waitForTimeout(150);
  await b.screenshot({ path: path.join(OUT, 'desktop-draw-layers.png') });
  await b.click('[data-eye="1"]');
  // Clear just layer 2 for everyone (the page goes to the gallery first).
  const gallery = await b.evaluate(() => yap.ui.draw.gallery.length);
  await a.click('[data-tool="clear"]');
  await a.waitForSelector('.draw-clear-pop:not([hidden])');
  await a.screenshot({ path: path.join(OUT, 'desktop-draw-clear.png') });
  await a.click('[data-clear="layer"]');
  await b.waitForFunction(() => !yap.ui.draw.strokes.some((s) => s.l === 1), null, { timeout: 8000 });
  await b.waitForFunction((n) => yap.ui.draw.gallery.length > n, gallery, { timeout: 8000 });
  assert.ok(await b.evaluate(() => yap.ui.draw.strokes.some((s) => !s.l)), 'the bottom layer is still there for Ben');
  assert.equal(await ink(b, 1), 0);
  await a.keyboard.press('1');
  await a.click('[data-tool="wiggle"]');
});

await test('draw on a touch screen: a second finger never takes over a stroke; Clear keeps a page the gallery couldn\'t take', async () => {
  // Both are still on Draw. Two fingers, the way a palm or a pinch lands on a phone or tablet.
  const box = await a.locator('.draw-paper').boundingBox();
  const cdp = await a.context().newCDPSession(a);
  const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
  const one = (i) => ({ x: box.x + box.width * (0.2 + i * 0.02), y: box.y + box.height * 0.25, id: 1 });
  const two = (i) => ({ x: box.x + box.width * 0.7, y: box.y + box.height * (0.5 + i * 0.02), id: 2 });
  const before = await a.evaluate(() => yap.ui.draw.strokes.length);
  await touch('touchStart', [one(0)]);
  for (let i = 1; i < 5; i++) await touch('touchMove', [one(i)]);
  await touch('touchStart', [one(5), two(0)]); // a second finger comes down…
  for (let i = 6; i < 12; i++) await touch('touchMove', [one(i), two(i)]);
  await touch('touchEnd', [two(12)]); // …and lifts first
  for (let i = 12; i < 16; i++) await touch('touchMove', [one(i)]);
  await touch('touchEnd', [one(16)]);
  await cdp.detach();
  await b.waitForFunction((n) => yap.ui.draw.strokes.length > n, before, { timeout: 5000 });
  await a.waitForTimeout(300);
  const ys = await a.evaluate((n) => yap.ui.draw.strokes.slice(n).map((s) => s.p.filter((_, i) => i % 2)), before);
  assert.equal(ys.length, 1, 'one stroke, from the first finger');
  assert.ok(Math.max(...ys[0]) - Math.min(...ys[0]) < 50, 'it stays on the first finger\'s line (no zigzag to the second)');
  assert.ok(ys[0].length >= 14, `it kept drawing after the second finger lifted (${ys[0].length} points)`);
  // The gallery says no: Clear keeps the page, and says so (it used to clear it anyway).
  await a.route('**/api/drawing?**', (route) => route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: false, message: 'Saving too fast.' }) }));
  const n = await b.evaluate(() => yap.ui.draw.strokes.length);
  await a.click('[data-tool="clear"]');
  await a.click('[data-clear="page"]');
  await a.waitForSelector('#toast:not([hidden]):has-text("nothing was cleared")', { timeout: 8000 });
  await b.waitForTimeout(500);
  assert.equal(await b.evaluate(() => yap.ui.draw.strokes.length), n, 'the page is still there for Ben');
  // And Save doesn't claim it saved.
  await a.click('[data-tool="save"]');
  await a.waitForSelector('#toast:not([hidden]):has-text("Couldn\'t save to the gallery: Saving too fast.")', { timeout: 5000 });
  await a.waitForTimeout(300);
  assert.doesNotMatch(await a.textContent('#toast'), /Saved/);
  await a.unroute('**/api/drawing?**');
});

await test('pictures: attach one, everyone sees it in the chat', async () => {
  for (const page of [a, b]) await page.click('[data-action="open-channel"][data-id="general"]');
  // A 64×48 PNG made on the spot.
  const pngB64 = await a.evaluate(async () => {
    const c = Object.assign(document.createElement('canvas'), { width: 64, height: 48 });
    const g = c.getContext('2d');
    g.fillStyle = '#e5484d';
    g.fillRect(0, 0, 64, 48);
    g.fillStyle = '#fff';
    g.fillRect(16, 12, 32, 24);
    return c.toDataURL('image/png').split(',')[1];
  });
  await a.setInputFiles('#image-file', { name: 'red.png', mimeType: 'image/png', buffer: Buffer.from(pngB64, 'base64') });
  await a.waitForSelector('#staged:not([hidden]) img');
  // Not connected at that moment: the picture and its caption wait in the box (they used to vanish).
  await a.fill('#input', `caption ${RUN}`);
  await a.evaluate(() => (yap.state.status = 'offline'));
  await a.click('.send-btn');
  await a.waitForSelector('#toast:not([hidden]):has-text("Not connected")');
  await a.evaluate(() => (yap.state.status = 'online'));
  assert.equal(await a.isVisible('#staged img'), true, 'the picture is still there');
  assert.equal(await a.inputValue('#input'), `caption ${RUN}`, 'and so is the caption');
  // The upload fails (the network, the server): it says why, and the picture goes back in the tray with its caption.
  await a.route(/\/api\/image\?/, (route) => route.fulfill({ json: { ok: false, message: 'Storage hiccup.' } }));
  await a.click('.send-btn');
  await a.waitForSelector('#toast:not([hidden]):has-text("Storage hiccup")');
  await a.unroute(/\/api\/image\?/);
  assert.equal(await a.isVisible('#staged img'), true, 'the picture is back in the tray');
  assert.equal(await a.inputValue('#input'), `caption ${RUN}`, 'with its caption');
  await a.fill('#input', '');
  // Changed your mind: × drops it, nothing is sent.
  await a.click('[data-action="unstage"]');
  assert.equal(await a.isHidden('#staged'), true);
  await a.setInputFiles('#image-file', { name: 'red.png', mimeType: 'image/png', buffer: Buffer.from(pngB64, 'base64') });
  await a.waitForSelector('#staged:not([hidden]) img');
  await a.click('.send-btn');
  await b.waitForSelector('.msg .msg-image[width="64"]', { timeout: 8000 });
  // A click shows it big, in Yap, with a way to every picture in the channel.
  await b.click('.msg .msg-image[width="64"]');
  await b.waitForSelector('#picture-dialog[open] #picture-img[src^="/media/"]');
  await b.screenshot({ path: path.join(OUT, 'desktop-picture.png') });
  await b.click('#picture-shelf');
  await b.waitForSelector('#shelf-dialog[open] #shelf-grid .shelf-item', { timeout: 5000 });
  assert.equal(await b.evaluate(() => document.querySelector('#picture-dialog').open), false);
  await b.click('[data-action="close-shelf"]');
  // A big photo is compressed by default (≤ 1600 px); "Full res" sends the original.
  const bigB64 = await a.evaluate(() => {
    const c = Object.assign(document.createElement('canvas'), { width: 3000, height: 2000 });
    const g = c.getContext('2d');
    for (let i = 0; i < 300; i++) {
      g.fillStyle = `hsl(${(i * 37) % 360} 70% 50%)`;
      g.fillRect((i * 97) % 3000, (i * 53) % 2000, 180, 120);
    }
    return c.toDataURL('image/jpeg', 0.9).split(',')[1];
  });
  const big = { name: 'big.jpg', mimeType: 'image/jpeg', buffer: Buffer.from(bigB64, 'base64') };
  const sent = (w) => b.waitForFunction((w) => yap.state.history.get('general')?.messages.some((m) => m.image?.width === w), w, { timeout: 10000 });
  await a.setInputFiles('#image-file', big);
  await a.waitForSelector('#staged:has-text("compressed")', { timeout: 10000 });
  await a.screenshot({ path: path.join(OUT, 'desktop-picture-compressed.png') });
  await a.click('.send-btn');
  await sent(1600);
  await a.setInputFiles('#image-file', big);
  await a.click('[data-action="toggle-full-res"]');
  await a.waitForFunction(() => /MB|KB/.test(document.querySelector('#staged .staged-text small')?.textContent ?? '') && !/compressed/.test(document.querySelector('#staged').textContent), null, { timeout: 10000 });
  await a.click('.send-btn');
  await sent(3000);
});

await test('pictures & GIFs shelf: every picture in the channel, newest first; click to jump', async () => {
  await b.click('[data-action="shelf"]');
  await b.waitForSelector('#shelf-grid .shelf-item', { timeout: 5000 });
  assert.ok(await b.locator('#shelf-grid .shelf-item').count() >= 3);
  await b.screenshot({ path: path.join(OUT, 'desktop-shelf.png') });
  await b.locator('#shelf-grid .shelf-item').last().click();
  await b.waitForSelector('.msg.flash', { timeout: 8000 });
});

await test('deleting a message takes it out of Pins, search results and the shelf', async () => {
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
  /** Ada posts a 1×1 picture with a caption; returns the message id once Ben has it. */
  const post = async (caption) => {
    await a.evaluate(async ([png, caption]) => {
      const bytes = Uint8Array.from(atob(png), (c) => c.charCodeAt(0));
      const res = await fetch(`/api/image?sid=${yap.state.sid}&ut=${yap.state.uploadToken}`, { method: 'POST', body: bytes, headers: { 'X-Yap-Key': yap.settings.key, 'Content-Type': 'image/png' } });
      const { id } = await res.json();
      yap.ui.send({ t: 'send', channelId: 'general', body: caption, image: { id, width: 1, height: 1 }, nonce: crypto.randomUUID() });
    }, [png, caption]);
    await b.waitForFunction((c) => yap.state.history.get('general')?.messages.some((m) => m.body === c), caption, { timeout: 5000 });
    return b.evaluate((c) => yap.state.history.get('general').messages.find((m) => m.body === c).id, caption);
  };
  const word = `quince${RUN}`;
  const first = await post(`a ${word} tart`);
  await a.evaluate((id) => yap.ui.send({ t: 'pin', id, on: true }), first);
  await a.waitForFunction((id) => yap.state.history.get('general').messages.find((m) => m.id === id)?.pinnedAt, first, { timeout: 5000 });
  // Ada has Pins open, Ben a search that found it.
  await a.click('[data-action="pins"]');
  await a.waitForSelector(`#pins-list [data-id="${first}"]`, { timeout: 5000 });
  await b.click('[data-action="search"]');
  await b.fill('#search-input', word);
  await b.waitForSelector(`#search-results [data-id="${first}"]`, { timeout: 5000 });
  await a.evaluate((id) => yap.ui.send({ t: 'delete', id }), first);
  await a.waitForSelector(`#pins-list [data-id="${first}"]`, { state: 'detached', timeout: 5000 });
  await b.waitForSelector(`#search-results [data-id="${first}"]`, { state: 'detached', timeout: 5000 });
  await a.click('[data-action="close-pins"]');
  await b.click('[data-action="close-search"]');
  // Ben has the shelf open when the next one goes.
  const second = await post(`another ${word} tart`);
  await b.click('[data-action="shelf"]');
  await b.waitForSelector(`#shelf-grid .shelf-item[data-id="${second}"]`, { timeout: 5000 });
  await a.evaluate((id) => yap.ui.send({ t: 'delete', id }), second);
  await b.waitForSelector(`#shelf-grid .shelf-item[data-id="${second}"]`, { state: 'detached', timeout: 5000 });
  await b.click('[data-action="close-shelf"]');
});

await test('space: walk around and the other person sees you move (radar too)', async () => {
  await a.click('[data-action="open-view"][data-view="space"]');
  await b.click('[data-action="open-view"][data-view="space"]');
  const aSid = await a.evaluate(() => yap.state.sid);
  await b.waitForFunction((sid) => yap.ui.space?.players.has(sid), aSid, { timeout: 5000 });
  const x0 = await b.evaluate((sid) => yap.ui.space.players.get(sid).tx, aSid);
  // Positions persist between runs, so walk away from whichever wall is nearer.
  const key = x0 > 1200 ? 'a' : 'd';
  await a.focus('.space-canvas');
  await a.keyboard.down(key);
  await a.waitForTimeout(700);
  await a.keyboard.up(key);
  await b.waitForFunction(([sid, x]) => Math.abs(yap.ui.space.players.get(sid).tx - x) > 60, [aSid, x0], { timeout: 5000 });
  // Emote (key 1) shows over Ada's head for Ben.
  await a.keyboard.press('1');
  await b.waitForFunction((sid) => yap.ui.space.emotes.get(sid)?.e === '👋', aSid, { timeout: 5000 });
  // Sit on the Lounge sofa: stand next to it, press E.
  await a.evaluate(() => Object.assign(yap.ui.space.me, { x: 230, y: 330 }));
  await a.keyboard.press('e');
  await b.waitForFunction((sid) => { const p = yap.ui.space.players.get(sid); return p.tx === 215 && p.ty === 300; }, aSid, { timeout: 5000 });
  await a.keyboard.press('2');
  await a.waitForTimeout(250);
  await a.screenshot({ path: path.join(OUT, 'desktop-space-sitting.png') });
  await b.waitForTimeout(300);
  await b.screenshot({ path: path.join(OUT, 'desktop-space.png') });
  // A reconnect (every deploy drops every socket) makes a new session: it's back on the map for
  // everyone straight away, not only once it moves. (Ben's second tab, so Ada's call isn't touched.)
  const tab = await b.context().newPage();
  await tab.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await tab.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  await tab.click('[data-action="open-view"][data-view="space"]');
  const first = await tab.evaluate(() => yap.state.sid);
  await a.waitForFunction((sid) => yap.ui.space.players.has(sid), first, { timeout: 5000 });
  await tab.evaluate(() => yap.ui.dropSocket());
  await tab.waitForFunction((old) => yap.state.status === 'online' && yap.state.sid !== old, first, { timeout: 15000 });
  const second = await tab.evaluate(() => yap.state.sid);
  await a.waitForFunction(([gone, sid]) => !yap.ui.space.players.has(gone) && yap.ui.space.players.has(sid), [first, second], { timeout: 5000 });
  await tab.close();
  await a.click('[data-action="open-channel"][data-id="general"]');
  await b.click('[data-action="open-channel"][data-id="general"]');
});

// Space rooms and games. spaceAt() puts someone somewhere and tells the server (walking there takes too long).
const spaceAt = (page, x, y) => page.evaluate(([x, y]) => {
  Object.assign(yap.ui.space.me, { x, y });
  yap.ui.send({ t: 'space.move', x, y, d: 0 });
}, [x, y]);
const sidA = await a.evaluate(() => yap.state.sid);
const sidB = await b.evaluate(() => yap.state.sid);

await test('space voice: rooms are voice zones (another room is silent, your room is full volume)', async () => {
  for (const page of [a, b]) await page.click('[data-action="open-view"][data-view="space"]');
  await b.waitForFunction((sid) => yap.ui.space?.players.has(sid), sidA, { timeout: 5000 });
  assert.equal(await b.evaluate(() => yap.ui.space.voiceMode), 'room', '"This room only" is the default');
  await spaceAt(a, 430, 420); // Lounge
  await spaceAt(b, 430, 800); // Plaza
  // (In your room, voices are panned, so they play through WebAudio: what you hear is the graph's gain then.)
  await b.waitForFunction((sid) => { const p = yap.voice.peers.get(sid); return p?.proximity === 0 && (p.boost ? p.boost.gain.gain.value : p.audio.volume) === 0; }, sidA, { timeout: 5000 });
  await b.waitForFunction(() => /1 here · 1 elsewhere/.test(document.querySelector('.space-room').textContent), null, { timeout: 2000 });
  assert.match(await b.locator('.space-room').innerText(), /Plaza[\s\S]*nobody in the rooms/);
  await spaceAt(b, 480, 430); // into the Lounge
  await b.waitForFunction((sid) => { const p = yap.voice.peers.get(sid); return p?.proximity === 1 && (p.boost ? p.boost.gain.gain.value : p.audio.volume) === 1; }, sidA, { timeout: 5000 });
  await a.waitForFunction((sid) => yap.voice.peers.get(sid)?.proximity === 1, sidB, { timeout: 5000 });
  await b.waitForFunction(() => document.querySelector('.space-room-name').textContent === 'Lounge' && document.querySelectorAll('.space-room-people li').length === 2, null, { timeout: 2000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-space-room.png') });
});

await test('space voice: left and right — someone 400 to your left plays in your left ear (the right at 1/4), close by in both; "Nearby is louder" too; "Everyone" not at all', async () => {
  try {
    // What Ben's speakers get from Ada, per ear (RMS over ~0.6 s), tapped from the graph that plays her.
    const ears = () => b.evaluate(async (sid) => {
      const peer = yap.voice.peers.get(sid);
      if (!peer?.boost) return null;
      const ctx = peer.boost.merge.context;
      const split = ctx.createChannelSplitter(2);
      const an = [ctx.createAnalyser(), ctx.createAnalyser()];
      peer.boost.merge.connect(split);
      an.forEach((x, i) => split.connect(x, i));
      const sum = [0, 0];
      const buf = new Float32Array(2048);
      for (let k = 0; k < 12; k++) {
        await new Promise((r) => setTimeout(r, 50));
        an.forEach((x, i) => {
          x.getFloatTimeDomainData(buf);
          sum[i] += buf.reduce((s, v) => s + v * v, 0);
        });
      }
      peer.boost.merge.disconnect(split);
      return { left: Math.sqrt(sum[0]), right: Math.sqrt(sum[1]), pan: peer.pan };
    }, sidA);
    await spaceAt(a, 150, 420); // both in the Lounge: Ada at the left wall…
    await spaceAt(b, 550, 420); // …Ben 400 to her right
    // (Space eases where people are drawn, so the pan gets within a hair of ±1.)
    await b.waitForFunction((sid) => Math.abs(yap.voice.peers.get(sid)?.pan + 1) < 0.01, sidA, { timeout: 5000 });
    await a.waitForFunction((sid) => Math.abs(yap.voice.peers.get(sid)?.pan - 1) < 0.01, sidB, { timeout: 5000 });
    let e = await ears();
    assert.ok(e.left > 0.01, `Ada is heard (${e.left.toFixed(3)})`);
    assert.ok(Math.abs(e.right / e.left - 0.25) < 0.04, `from the left: the right ear at ${(e.right / e.left).toFixed(2)} of the left (1/4)`);
    const [l, r] = await b.evaluate((sid) => [yap.voice.peers.get(sid).boost.left.gain.value, yap.voice.peers.get(sid).boost.right.gain.value], sidA);
    assert.ok(l === 1 && Math.abs(r - 0.25) < 0.01, `ear gains ${l}, ${r}`);
    await spaceAt(b, 170, 440); // right next to her: the middle
    await b.waitForFunction((sid) => yap.voice.peers.get(sid)?.pan === 0, sidA, { timeout: 5000 });
    e = await ears();
    assert.ok(Math.abs(e.right / e.left - 1) < 0.05, `close by: both ears full (${(e.right / e.left).toFixed(2)})`);
    // "Nearby is louder": panned too (and a bit quieter at 400).
    await spaceAt(b, 550, 420);
    await b.selectOption('.space-voice select', 'near');
    await b.waitForFunction((sid) => { const p = yap.voice.peers.get(sid); return Math.abs(p?.pan + 1) < 0.01 && p.proximity < 1; }, sidA, { timeout: 5000 });
    // "Everyone": no panning; she plays straight from her element again.
    await b.selectOption('.space-voice select', 'everyone');
    await b.waitForFunction((sid) => { const p = yap.voice.peers.get(sid); return p?.pan === null && !p.boost && p.audio.volume === 1; }, sidA, { timeout: 5000 });
    await b.selectOption('.space-voice select', 'room');
    // Leaving Space: back to normal.
    await b.click('[data-action="open-channel"][data-id="general"]');
    await b.waitForFunction((sid) => { const p = yap.voice.peers.get(sid); return p?.pan === null && !p.boost; }, sidA, { timeout: 3000 });
    await b.click('[data-action="open-view"][data-view="space"]');
  } finally {
    await b.evaluate(() => {
      const select = document.querySelector('.space-voice select');
      if (select && select.value !== 'room') {
        select.value = 'room';
        select.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }).catch(() => {});
  }
});

await test('space ball: one kicks it, the other sees it roll; a goal counts for both', async () => {
  const before = await b.evaluate(() => ({ ...yap.ui.space.ball }));
  await spaceAt(b, 1120, 900); // out of the way
  await spaceAt(a, before.x, before.y - 30); // right on top of it: a kick
  await b.waitForFunction(([sid, k, x, y]) => {
    const ball = yap.ui.space.ball;
    return ball.owner === sid && ball.k > k && Math.hypot(ball.x - x, ball.y - y) > 40;
  }, [sidA, before.k, before.x, before.y], { timeout: 5000 });
  // A shot into Red's net (right): a goal for Blue, on both screens.
  await a.evaluate(() => Object.assign(yap.ui.space.ball, { x: 1600, y: 1240, vx: 0, vy: 0, dirty: true }));
  await spaceAt(a, 1545, 1240);
  await a.focus('.space-canvas');
  await a.keyboard.down('d');
  await a.waitForTimeout(300);
  await a.keyboard.up('d');
  await b.waitForFunction((blue) => yap.ui.space.ball.s[0] === blue + 1, before.s[0], { timeout: 5000 });
  await b.waitForSelector('.space-game:has-text("GOAL for Blue")', { timeout: 2000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-space-goal.png') });
});

await test('space tag: T makes you "it", touching passes it on', async () => {
  await spaceAt(a, 1200, 800);
  await spaceAt(b, 1500, 800);
  await a.focus('.space-canvas');
  await a.keyboard.press('t');
  await b.waitForFunction((id) => !yap.ui.space.tag.over && yap.ui.space.tag.it === id, sidA.slice(0, 8), { timeout: 5000 });
  await b.waitForSelector('.space-game:has-text("Ada is it")', { timeout: 2000 });
  await a.waitForTimeout(700); // "it" can't tag in the first moment
  await spaceAt(b, 1230, 800);
  await a.waitForFunction((id) => yap.ui.space.tag.it === id, sidB.slice(0, 8), { timeout: 5000 });
  await b.waitForSelector(".space-game:has-text(\"You're it\")", { timeout: 5000 });
  assert.ok((await a.evaluate((id) => yap.ui.space.tag.t[id], sidA.slice(0, 8))) > 0, 'Ada was it for a while');
  await b.screenshot({ path: path.join(OUT, 'desktop-space-tag.png') });
  await a.click('[data-action="open-channel"][data-id="general"]');
  await b.click('[data-action="open-channel"][data-id="general"]');
});

await test('space fire and water: walk into the campfire and everyone sees you burn; the fountain puts it out', async () => {
  for (const page of [a, b]) await page.click('[data-action="open-view"][data-view="space"]');
  await b.waitForFunction((sid) => yap.ui.space?.players.has(sid), sidA, { timeout: 5000 });
  await spaceAt(b, 2100, 1150);
  await spaceAt(a, 2100, 1250); // the fire
  await b.waitForFunction((sid) => (yap.ui.space.fx.get(sid)?.fire ?? 0) > performance.now(), sidA, { timeout: 5000 });
  await spaceAt(a, 2165, 1310);
  await b.waitForTimeout(300);
  await b.screenshot({ path: path.join(OUT, 'desktop-space-fire.png') });
  await spaceAt(a, 340, 1240); // into the fountain
  await b.waitForFunction((sid) => {
    const f = yap.ui.space.fx.get(sid);
    return f?.wet > performance.now() && !(f.fire > performance.now());
  }, sidA, { timeout: 5000 });
});

await test('space water balloons: F throws one; a hit soaks the other person and counts for the thrower', async () => {
  await spaceAt(a, 1150, 800); // facing right, at Ben
  await spaceAt(b, 1400, 800);
  await a.focus('.space-canvas');
  await a.keyboard.press('f');
  await b.waitForFunction(() => yap.ui.space.myFx.wet > performance.now(), null, { timeout: 5000 });
  await a.waitForFunction(() => yap.ui.space.mySplashes >= 1, null, { timeout: 5000 });
  await a.waitForSelector('.space-game:has-text("You splashed Ben")', { timeout: 3000 });
});

await test('space race: a lap of the track is timed and goes on everyone\'s board', async () => {
  // Start line on the left (crossed going up), then the four corners clockwise, then the line again.
  for (const [x, y] of [[56, 860], [56, 760], [56, 60], [2340, 60], [2340, 1540], [60, 1540], [56, 860], [56, 760]]) {
    await spaceAt(a, x, y);
    await a.waitForTimeout(120);
  }
  await b.waitForFunction((sid) => yap.ui.space.laps.get(sid)?.best > 0, sidA, { timeout: 5000 });
  await b.waitForSelector('.space-game:has-text("Ada ran a lap")', { timeout: 3000 });
});

await test('space music: stepping on the floor piano and drums plays them for everyone in the Music room', async () => {
  await spaceAt(b, 1520, 360);
  const heard = await b.evaluate(() => yap.ui.space.heard);
  await spaceAt(a, 1344, 250); // C, the first white key
  await b.waitForFunction((sid) => yap.ui.space.inst.get(sid) === 'w0', sidA, { timeout: 5000 });
  await spaceAt(a, 1400, 430); // the kick drum
  await b.waitForFunction((sid) => yap.ui.space.inst.get(sid) === 'p0', sidA, { timeout: 5000 });
  assert.ok((await b.evaluate(() => yap.ui.space.heard)) >= heard + 2, 'Ben heard both');
  await b.screenshot({ path: path.join(OUT, 'desktop-space-music.png') });
  await a.click('[data-action="open-channel"][data-id="general"]');
  await b.click('[data-action="open-channel"][data-id="general"]');
});

await test('space gallery: a drawing saved in Draw goes up live; walk up to it and it opens big (GIFs too)', async () => {
  await b.click('[data-action="open-view"][data-view="space"]');
  await spaceAt(b, 1200, 800); // the Gallery's aisle
  await b.waitForFunction(() => yap.ui.space.gallery.length >= 2, null, { timeout: 5000 }); // the GIF and the shared page from the Draw test
  const newest = await b.evaluate(() => yap.ui.space.gallery[0].id);
  // Ada saves her page in Draw: it's hung first on the first wall, for Ben, right away.
  await a.click('[data-action="open-view"][data-view="draw"]');
  await a.waitForFunction(() => yap.ui.draw && document.querySelector('.draw-paper').clientWidth > 100);
  await Promise.all([a.waitForEvent('download'), a.click('[data-tool="save"]')]);
  await b.waitForFunction((id) => yap.ui.space.gallery[0].id !== id && yap.ui.space.galleryPage === 0, newest, { timeout: 8000 });
  await b.waitForSelector('.space-game:has-text("Ada hung a new drawing")', { timeout: 3000 });
  await b.waitForSelector('.space-wall:not([hidden]):has-text("Wall 1 of")');
  await b.screenshot({ path: path.join(OUT, 'desktop-space-gallery.png') });
  // Walk up to the first frame and stop: the viewer opens with the real image.
  await spaceAt(b, 835, 790);
  await b.waitForSelector('.space-viewer:not([hidden])', { timeout: 3000 });
  await b.waitForFunction(() => {
    const img = document.querySelector('.space-viewer-img');
    return img.getAttribute('src') === `/media/${yap.ui.space.gallery[0].media}` && img.complete && img.naturalWidth === 640;
  }, null, { timeout: 5000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-space-gallery-viewer.png') });
  // Older: the shared page, then the GIF, which the viewer shows as a real (wiggling) GIF.
  await b.keyboard.press('ArrowRight');
  await b.click('[data-viewer="1"]');
  await b.waitForFunction(() => yap.ui.space.viewing === 2);
  assert.equal(await b.evaluate(async () => (await fetch(document.querySelector('.space-viewer-img').src)).headers.get('content-type')), 'image/gif');
  await b.keyboard.press('Escape');
  await b.waitForSelector('.space-viewer', { state: 'hidden' });
  await b.click('[data-action="open-channel"][data-id="general"]');
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('📣 ping reaches the other person', async () => {
  await a.click('[data-action="ping"]');
  await a.waitForSelector('#confirm-dialog[open] #confirm-input');
  await a.fill('#confirm-input', 'hop on!');
  await a.press('#confirm-input', 'Enter');
  await b.waitForSelector(`#toast:has-text("pinged everyone: hop on!")`, { timeout: 5000 });
});

// ── Watch together ──────────────────────────────────────────────────────
// YouTube is stubbed with a tiny fake player page that speaks the embed's
// postMessage protocol, the way the real one does (checked against
// youtube-nocookie.com): "listening" → initialDelivery + onReady, then
// infoDelivery {currentTime, playerState, duration, videoData} ~4×/s and
// onStateChange; commands playVideo/pauseVideo/seekTo/loadVideoById/… Its
// own ⏯ and +30s buttons stand in for YouTube's controls. ytBlock makes a
// person's next player refuse to start N times, like a browser that blocks
// autoplay with sound. A video id starting with "broken" never plays: the
// player answers error 150, like one whose owner turned off embedding.
const ytBlock = new Map();
const fakePlayerHtml = (block) => `<!doctype html><meta charset="utf-8">
<style>html,body{margin:0;height:100%;background:#24243a;color:#fff;font:600 15px system-ui,sans-serif}body{display:grid;place-items:center;text-align:center}button{font:inherit;margin:4px 3px}</style>
<div><div id="title"></div><div id="clock"></div><button id="pp">⏯</button><button id="fwd">+30s</button></div>
<script>
const q = new URLSearchParams(location.search);
const parentOrigin = q.get('origin');
const durationOf = (id) => (id.startsWith('short') ? 12 : 600);
const P = (window.fakePlayer = { id: location.pathname.split('/').pop(), t: Number(q.get('start')) || 0, at: performance.now(), state: -1, volume: 100, muted: false, block: ${block}, bootId: Math.random().toString(36).slice(2), listening: false });
P.duration = durationOf(P.id);
P.now = () => Math.min(P.duration, P.t + (P.state === 1 ? (performance.now() - P.at) / 1000 : 0));
const post = (m) => P.listening && parent.postMessage(JSON.stringify({ ...m, id: 1, channel: 'widget' }), parentOrigin);
const info = () => ({ currentTime: P.now(), playerState: P.state, duration: P.duration, videoData: { video_id: P.id } });
const setTime = (t) => { P.t = Math.max(0, Math.min(P.duration, t)); P.at = performance.now(); };
function setState(s) {
  setTime(P.now());
  if (P.state === s) return;
  P.state = s;
  post({ event: 'onStateChange', info: s });
  post({ event: 'infoDelivery', info: info() });
}
const start = () => (P.id.startsWith('broken') ? post({ event: 'onError', info: 150 }) : P.block > 0 ? P.block-- : setState(1));
setInterval(() => {
  if (P.state === 1 && P.now() >= P.duration) setState(0);
  post({ event: 'infoDelivery', info: info() });
  document.getElementById('title').textContent = 'fake YouTube · ' + P.id;
  document.getElementById('clock').textContent = P.now().toFixed(1) + ' s · state ' + P.state;
}, 250);
addEventListener('message', (e) => {
  if (e.origin !== parentOrigin) return;
  let m;
  try { m = JSON.parse(e.data); } catch { return; }
  if (m.event === 'listening') {
    if (!P.listening) {
      P.listening = true;
      post({ event: 'initialDelivery', info: { ...info(), volume: P.volume, muted: P.muted } });
      post({ event: 'onReady', info: null });
    }
    return;
  }
  if (m.event !== 'command') return;
  const a = m.args || [];
  switch (m.func) {
    case 'playVideo': return start();
    case 'pauseVideo': return setState(2);
    case 'stopVideo': setTime(0); return setState(5);
    case 'seekTo':
      setTime(Number(a[0]) || 0);
      if (P.state === -1 || P.state === 5) start();
      return post({ event: 'infoDelivery', info: info() });
    case 'loadVideoById': case 'cueVideoById':
      P.id = String(a[0]); P.duration = durationOf(P.id); P.state = -1; setTime(Number(a[1]) || 0);
      return m.func === 'loadVideoById' ? start() : setState(5);
    case 'setVolume': P.volume = Number(a[0]); return post({ event: 'infoDelivery', info: { volume: P.volume, muted: P.muted } });
    case 'mute': case 'unMute': P.muted = m.func === 'mute'; return post({ event: 'infoDelivery', info: { volume: P.volume, muted: P.muted } });
  }
});
document.getElementById('pp').onclick = () => setState(P.state === 1 ? 2 : 1);
document.getElementById('fwd').onclick = () => { setTime(P.now() + 30); post({ event: 'infoDelivery', info: info() }); };
</script>`;
async function fakeYouTube(page, name) {
  const context = page.context();
  await context.route('https://www.youtube-nocookie.com/**', (route) => {
    const block = ytBlock.get(name) ?? 0;
    ytBlock.delete(name);
    return route.fulfill({ contentType: 'text/html', body: fakePlayerHtml(block) });
  });
  await context.route('https://i.ytimg.com/**', (route) => {
    const id = route.request().url().split('/vi/')[1]?.split('/')[0] ?? '';
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="${PALETTE[id.length % PALETTE.length]}"/><text x="160" y="100" font-size="28" font-family="sans-serif" fill="#fff" text-anchor="middle">${id}</text></svg>`;
    return route.fulfill({ contentType: 'image/svg+xml', body: svg });
  });
}
const ytFrame = (page) => page.frames().find((f) => f.url().startsWith('https://www.youtube-nocookie.com/embed/'));
const ytPlayer = async (page) => {
  const frame = ytFrame(page);
  return frame ? frame.evaluate(() => ({ id: fakePlayer.id, state: fakePlayer.state, time: fakePlayer.now(), bootId: fakePlayer.bootId, volume: fakePlayer.volume })) : null;
};
/** Polls fn (async) until it returns something truthy. */
async function until(fn, timeout, what) {
  const end = Date.now() + timeout;
  let last = null;
  while (Date.now() < end) {
    try {
      const value = await fn();
      if (value) return value;
    } catch (err) {
      last = err;
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}${last ? ` (${last.message})` : ''}`);
}
const watchInSync = async () => {
  const pa = await ytPlayer(a);
  const pb = await ytPlayer(b);
  return pa && pb && pa.id === pb.id && Math.abs(pa.time - pb.time) < 1 ? [pa, pb] : false;
};
const voiceUp = (page) => page.evaluate(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'));
const seatNames = (page) => page.evaluate(() => [...document.querySelectorAll('.watch-seat:not(.empty) .watch-seat-name')].map((e) => e.textContent).sort().join());
const watchAudience = (page) => page.evaluate(() => {
  const row = document.querySelector('#links [data-view="watch"]')?.closest('li');
  return [...(row?.nextElementSibling?.querySelectorAll('.voice-member .name') ?? [])].map((e) => e.textContent).sort().join();
});

await test('watch together: both see the same video at the same spot, heads in the seats, voice stays up', async () => {
  await fakeYouTube(a, 'Ada');
  await fakeYouTube(b, 'Ben');
  await a.click('[data-action="open-view"][data-view="watch"]');
  if (!(await a.evaluate(() => yap.ui.watch.state?.videoId))) {
    await a.waitForSelector('.watch-curtain:has-text("Nothing\'s on")', { timeout: 5000 });
    // One calm line and the link field: no dead transport, no empty queue.
    assert.ok(await a.isHidden('.watch-transport') && await a.isHidden('.watch-queue'), 'nothing on: no transport or queue');
    assert.equal(await a.textContent('.watch-title'), '');
  }
  await a.waitForFunction(() => document.querySelector('.watch-seat:not(.empty) .watch-seat-name')?.textContent === 'Ada', null, { timeout: 5000 });
  await a.fill('.watch-add input', 'https://www.youtube.com/watch?v=vidAAAAAAA1&list=PLx&index=2');
  await a.click('.watch-add button[value="play"]'); // "Play" (or "Play now" if something was on)
  await until(async () => (await ytPlayer(a))?.state === 1, 8000, "Ada's player to start");
  assert.equal(await a.evaluate(() => document.querySelector('#watch-dock').dataset.mode), 'view');
  await a.waitForTimeout(3000); // the show has been on a while when Ben walks in
  ytBlock.set('Ben', 2); // and Ben's browser blocks autoplay with sound
  await b.click('[data-action="open-view"][data-view="watch"]');
  await b.waitForSelector('#watch-dock .watch-gate:not([hidden])', { timeout: 10000 });
  await b.screenshot({ path: path.join(OUT, 'desktop-watch-click-to-join.png') });
  await b.click('#watch-dock .watch-gate button');
  const [pa, pb] = await until(watchInSync, 10000, 'both players at the same spot');
  assert.equal(pa.id, 'vidAAAAAAA1');
  assert.ok(pb.time > 4, `Ben came in mid-show at ${pb.time.toFixed(1)} s`);
  assert.equal(pb.volume, 70, 'your own volume (default 70)');
  for (const page of [a, b]) {
    await until(async () => (await seatNames(page)) === 'Ada,Ben', 5000, 'two heads in the seats');
    await until(async () => (await watchAudience(page)) === 'Ada,Ben', 5000, 'the audience under Watch in the sidebar');
    await page.waitForSelector('#links .watch-now:has-text("▶")', { timeout: 3000 });
  }
  assert.ok((await voiceUp(a)) && (await voiceUp(b)), 'voice stays connected');
  await b.waitForTimeout(500);
  await b.screenshot({ path: path.join(OUT, 'desktop-watch.png') });
});

await test('watch together: pause, play and seek reach everyone (our controls and the player\'s own)', async () => {
  await a.click('.watch-play'); // Ada pauses with the controls under the video
  await until(async () => (await ytPlayer(b))?.state === 2, 5000, "Ben's player to pause");
  await b.waitForSelector('.watch-note:has-text("Ada paused")', { timeout: 3000 });
  await until(async () => {
    const [pa, pb] = [await ytPlayer(a), await ytPlayer(b)];
    return pa.state === 2 && Math.abs(pa.time - pb.time) < 1.6;
  }, 6000, 'both paused at the same spot');
  await b.waitForTimeout(2000);
  await ytFrame(b).click('#pp'); // Ben presses play inside his own player
  await until(async () => (await ytPlayer(a))?.state === 1, 5000, "Ada's player to resume");
  await a.waitForSelector('.watch-note:has-text("Ben pressed play")', { timeout: 3000 });
  await a.$eval('.watch-seek', (el) => {
    el.value = '120';
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await until(async () => {
    const p = await ytPlayer(b);
    return p.time > 119 && p.time < 126;
  }, 6000, "Ben's player at 2:00");
  await b.waitForSelector('.watch-note:has-text("Ada skipped to 2:0")', { timeout: 3000 });
  await b.waitForTimeout(3000);
  await ytFrame(b).click('#fwd'); // Ben skips 30 s in his own player
  await until(async () => {
    const p = await ytPlayer(a);
    return p.time > 150 && p.time < 160;
  }, 6000, "Ada's player 30 s further on");
  await until(watchInSync, 6000, 'everyone back in sync');
});

await test('watch together: "Lower shared media while friends talk" halves your volume while someone talks, never saved as yours, then brings it back', async () => {
  try {
    assert.equal(await b.evaluate(() => yap.settings.duckMedia), false, 'off by default');
    await setDuckMedia(b, true);
    // Ada's test mic never stops: Ben's player goes to half of his 70.
    await until(async () => (await ytPlayer(b))?.volume === 35, 4000, "Ben's player at half while Ada talks");
    assert.equal(await b.inputValue('.watch-vol'), '70', 'his slider still shows his volume');
    // The player reports that volume back long after our last command, with Ben's focus in it: still not his volume.
    await b.waitForTimeout(2000);
    await b.evaluate(() => (yap.ui.watch.touchedAt = performance.now()));
    await ytFrame(b).evaluate(() => post({ event: 'infoDelivery', info: { volume: fakePlayer.volume, muted: fakePlayer.muted } }));
    await b.waitForTimeout(300);
    assert.deepEqual(await b.evaluate(() => [yap.ui.watch.prefs, JSON.parse(localStorage.getItem('yap.watch'))?.volume ?? 70]), [{ volume: 70, muted: false }, 70], 'the dip was never saved as his volume');
    await b.screenshot({ path: path.join(OUT, 'TW5-watch-lowered.png') });
    await a.click('[data-action="toggle-mute"]');
    await until(async () => (await ytPlayer(b))?.volume === 70, 4000, 'back to 70 once Ada stops');
    await a.click('[data-action="toggle-mute"]');
    await setDuckMedia(b, false);
    await b.waitForTimeout(1500);
    assert.equal((await ytPlayer(b)).volume, 70, 'off: Ada talking changes nothing');
  } finally {
    await b.evaluate(() => (yap.settings.duckMedia = false)).catch(() => {});
    await voiceReset(a);
  }
});

await test('watch together: browsing keeps the same player going in a mini player (no reload), voice too', async () => {
  const before = await ytPlayer(a);
  await a.evaluate(() => (window.watchFrame = document.querySelector('#watch-dock iframe')));
  await a.click('[data-action="open-channel"][data-id="general"]');
  await a.waitForSelector('#watch-dock[data-mode="mini"]', { timeout: 3000 });
  assert.ok(await a.evaluate(() => document.querySelector('#watch-dock iframe') === window.watchFrame), 'the very same iframe element');
  await a.fill('#input', 'this part is so good');
  await a.press('#input', 'Enter');
  await a.waitForTimeout(1200);
  const after = await ytPlayer(a);
  assert.equal(after.bootId, before.bootId, 'the player page never reloaded');
  assert.equal(after.state, 1);
  assert.ok(after.time > before.time + 1, `still playing: ${before.time.toFixed(1)} → ${after.time.toFixed(1)}`);
  const box = await a.evaluate(() => ({
    dock: document.querySelector('#watch-dock').getBoundingClientRect().toJSON(),
    composer: document.querySelector('.composer-wrap').getBoundingClientRect().toJSON(),
  }));
  assert.ok(box.dock.bottom <= box.composer.top && box.dock.width > 200, `mini player above the composer: ${JSON.stringify(box)}`);
  assert.ok(await voiceUp(a), 'voice stays connected');
  await a.screenshot({ path: path.join(OUT, 'desktop-watch-mini.png') });
  // Another place: still the same player.
  await a.click('[data-action="open-view"][data-view="draw"]');
  await a.waitForTimeout(300);
  assert.equal(await a.evaluate(() => document.querySelector('#watch-dock').dataset.mode), 'mini');
  assert.ok(await a.evaluate(() => document.querySelector('#watch-dock iframe') === window.watchFrame), 'same iframe on Draw');
  // ⤢ goes back to the Watch view, still the same player.
  await a.click('#watch-dock [data-watch="expand"]');
  await a.waitForSelector('#watch-dock[data-mode="view"]', { timeout: 3000 });
  assert.ok(await a.evaluate(() => document.querySelector('#watch-dock iframe') === window.watchFrame), 'same iframe back on Watch');
  assert.equal((await ytPlayer(a)).bootId, before.bootId);
});

await test('watch together: queue with thumbnails, remove, and the next video starts when one ends', async () => {
  await a.fill('.watch-add input', 'https://youtu.be/shortvid001?si=abc');
  await a.click('.watch-add button[value="play"]'); // Play now
  await until(async () => (await ytPlayer(b))?.id === 'shortvid001', 5000, 'Ben on the short video');
  await a.fill('.watch-add input', 'https://youtube.com/shorts/vidBBBBBBB2');
  await a.press('.watch-add input', 'Enter'); // Enter = Add to queue
  // One at a time: two adds racing each other's title lookup on the server can lose one (reported).
  await b.waitForSelector('.watch-q-item img[src="https://i.ytimg.com/vi/vidBBBBBBB2/mqdefault.jpg"]', { timeout: 5000 });
  await a.fill('.watch-add input', 'https://music.youtube.com/watch?v=vidCCCCCCC3&feature=share');
  await a.click('.watch-add button[value="queue"]');
  await b.waitForSelector('.watch-q-item img[src="https://i.ytimg.com/vi/vidCCCCCCC3/mqdefault.jpg"]', { timeout: 5000 });
  await b.waitForSelector('.watch-q-item img[src="https://i.ytimg.com/vi/vidBBBBBBB2/mqdefault.jpg"]');
  await b.waitForSelector('.watch-note:has-text("Ada queued")');
  await b.screenshot({ path: path.join(OUT, 'desktop-watch-queue.png') });
  await b.click('.watch-q-item:nth-child(2) [data-watch="remove"]');
  await a.waitForFunction(() => yap.ui.watch.state.queue.length === 1 && yap.ui.watch.state.queue[0].id === 'vidBBBBBBB2', null, { timeout: 5000 });
  await until(async () => {
    const [pa, pb] = [await ytPlayer(a), await ytPlayer(b)];
    return pa.id === 'vidBBBBBBB2' && pb.id === 'vidBBBBBBB2' && pa.state === 1 && pb.state === 1;
  }, 20000, 'the queued video on both once the short one ended');
  assert.equal(await a.evaluate(() => yap.ui.watch.state.queue.length), 0);
  assert.ok(await a.evaluate(() => document.querySelector('#watch-dock iframe') === window.watchFrame), 'new videos load in the same player');
});

await test('watch together: leaving takes your seat away; voice keeps going', async () => {
  await b.click('[data-action="open-channel"][data-id="general"]');
  await b.waitForSelector('#watch-dock[data-mode="mini"]', { timeout: 3000 });
  await b.click('#watch-dock [data-watch="close"]');
  await b.waitForSelector('#watch-dock', { state: 'hidden', timeout: 3000 });
  await until(async () => !ytFrame(b), 3000, "Ben's player to go away");
  await until(async () => (await seatNames(a)) === 'Ada', 5000, "Ben's head to leave Ada's seats");
  await until(async () => (await watchAudience(a)) === 'Ada', 5000, 'Ben gone from the sidebar audience');
  assert.ok(await voiceUp(b), 'still in voice');
  // Ada leaves from the Watch view: the slot offers her seat back.
  await a.click('.watch-head [data-watch="leave"]');
  await a.waitForSelector('.watch-curtain [data-watch="join"]', { timeout: 3000 });
  await until(async () => !ytFrame(a), 3000, "Ada's player to go away");
  await a.click('[data-action="open-channel"][data-id="general"]');
  assert.equal(await a.evaluate(() => document.querySelector('#watch-dock').hidden), true);
});

await test('rejoin after a reload: "You were in Lounge." with Rejoin puts you back in the call; × means no bar next time', async () => {
  const online = async () => {
    await b.reload();
    await settle(b, 'Ben');
  };
  await online();
  await b.waitForSelector('#rejoin-bar:visible', { timeout: 10000 });
  assert.equal((await b.textContent('#rejoin-bar span')).trim(), 'You were in Lounge.');
  assert.equal(await b.evaluate(() => yap.state.voiceWanted), null, "it doesn't rejoin by itself (a click starts the audio)");
  await b.locator('#rejoin-bar').screenshot({ path: path.join(OUT, 'rejoin-bar.png') });
  // ×: gone, and not back after another reload.
  await b.click('#rejoin-bar [data-action="rejoin-dismiss"]');
  assert.equal(await b.isHidden('#rejoin-bar'), true);
  await online();
  await b.waitForTimeout(1000);
  assert.equal(await b.isHidden('#rejoin-bar'), true, 'dismissed: nothing to rejoin');
  // In the call again (the usual way), then a reload: Rejoin.
  await b.click('button.channel.voice:has-text("Lounge")');
  await b.waitForFunction(() => yap.state.voiceChannel === 'lounge', null, { timeout: 10000 });
  await online();
  await b.click('#rejoin-bar [data-action="rejoin-voice"]');
  await b.waitForSelector('#rejoin-bar', { state: 'hidden', timeout: 3000 });
  await b.waitForFunction(() => yap.state.voiceChannel === 'lounge', null, { timeout: 10000 });
  for (const page of [a, b]) {
    await page.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 15000 });
  }
  // A call worth a report (the call report test after the next one reads this one): a few rounds of stats.
  await b.waitForFunction(() => yap.state.voiceStats?.peers?.length > 0, null, { timeout: 10000 });
  await b.waitForTimeout(4500);
});

await test('leaving hangs up on both sides', async () => {
  await b.click('[data-action="leave-voice"]');
  await a.waitForFunction(() => yap.voice.peers.size === 0, null, { timeout: 5000 });
  assert.equal(await b.evaluate(() => yap.voice.peers.size), 0);
  assert.equal(await b.evaluate(() => sessionStorage.getItem('yap.call')), null, 'left on purpose: no "You were in …" after a reload');
});

await test('call report after the call: Settings › Voice › Copy last call report copies the one that ended, kept on this device', async () => {
  await catchCopies(b);
  await b.click('[data-action="open-settings"]');
  await b.click('#settings-tabs [role=tab]:text-is("Voice")');
  await b.waitForSelector('#call-report-copy:enabled', { timeout: 3000 });
  assert.equal(await b.textContent('#call-report-copy'), 'Copy last call report');
  assert.equal(await b.isVisible('#call-report-none'), false);
  await b.locator('#call-report-field').screenshot({ path: path.join(OUT, 'call-report-settings.png') });
  await b.click('#call-report-copy');
  await b.waitForSelector('#toast:not([hidden]):text-is("Call report copied.")', { timeout: 3000 });
  const text = await b.evaluate(() => globalThis.__copied);
  assert.match(text, /^Lasted: \d+:\d\d$/m, 'over, so no "so far"');
  assert.match(text, /\n\+\d+:\d\d(:\d\d)? Call ended: you left\n$/);
  assert.match(text, /^ {2}Codec: red\+opus out, red\+opus in$/m);
  assert.equal(await b.evaluate(() => JSON.parse(localStorage.getItem('yap.callReport')).text), text, 'kept in localStorage');
  await b.click('#settings-form .settings-actions .btn');
});

// Phone-sized screenshots.
const phone = await user('Cleo', '#d6409f', { width: 390, height: 844 }, 'dark');
await phone.waitForTimeout(800);
await phone.screenshot({ path: path.join(OUT, 'phone-chat.png') });
await phone.click('[data-action="toggle-sidebar"]');
await phone.waitForTimeout(400);
await phone.screenshot({ path: path.join(OUT, 'phone-sidebar.png') });
await a.click('[data-action="open-settings"]');
await a.waitForTimeout(400);
await a.screenshot({ path: path.join(OUT, 'desktop-settings.png') });
// The light scheme ("Windows Standard") too.
for (const page of [a, phone]) await page.evaluate(() => (document.documentElement.dataset.theme = 'light'));
await a.click('#settings-tabs [data-tab="2"]');
await a.waitForTimeout(200);
await a.screenshot({ path: path.join(OUT, 'desktop-settings-light.png') });
await a.keyboard.press('Escape');
await a.waitForTimeout(200);
await a.screenshot({ path: path.join(OUT, 'desktop-light.png') });
await phone.screenshot({ path: path.join(OUT, 'phone-sidebar-light.png') });


// Space games aren't stored: whoever owns one answers a newcomer's sync. Then "it" walks off.
await test('space: a newcomer catches up on the ball and tag; when "it" leaves, someone else is it', async () => {
  for (const page of [a, phone]) await page.evaluate(() => (document.documentElement.dataset.theme = 'dark'));
  for (const page of [a, b]) await page.click('[data-action="open-view"][data-view="space"]');
  const sidA = await a.evaluate(() => yap.state.sid);
  await b.waitForFunction((sid) => yap.ui.space.players.has(sid), sidA, { timeout: 5000 });
  const ball = await a.evaluate(() => ({ ...yap.ui.space.ball }));
  await spaceAt(a, ball.x - 30, ball.y); // kick it: Ada owns the ball now
  await b.waitForFunction((sid) => yap.ui.space.ball.owner === sid, sidA, { timeout: 5000 });
  await spaceAt(a, 1150, 800);
  await spaceAt(b, 1400, 760);
  await a.focus('.space-canvas');
  await a.keyboard.press('t');
  await b.waitForFunction((id) => !yap.ui.space.tag.over && yap.ui.space.tag.it === id, sidA.slice(0, 8), { timeout: 5000 });
  // Ada catches fire on the way back to the Plaza.
  await spaceAt(a, 2100, 1250);
  await a.waitForFunction(() => yap.ui.space.myFx.fire > performance.now(), null, { timeout: 3000 });
  await spaceAt(a, 1150, 800);
  // Cleo opens Space on her phone and gets all three.
  if (!(await phone.evaluate(() => document.querySelector('.app').classList.contains('sidebar-open')))) await phone.click('[data-action="toggle-sidebar"]');
  await phone.click('[data-action="open-view"][data-view="space"]');
  await phone.waitForFunction(([sid, id]) => {
    const s = yap.ui.space;
    return s?.ball.owner === sid && s.ball.k > 0 && !s.tag.over && s.tag.it === id && s.fx.get(sid)?.fire > performance.now();
  }, [sidA, sidA.slice(0, 8)], { timeout: 5000 });
  await spaceAt(phone, 1260, 860);
  await phone.waitForTimeout(500);
  await phone.screenshot({ path: path.join(OUT, 'phone-space.png') });
  await a.screenshot({ path: path.join(OUT, 'desktop-space-three.png') });
  // Ada ("it", and the ball's owner) leaves: Ben and Cleo agree on who's it now, and one of them takes the ball.
  await a.click('[data-action="open-channel"][data-id="general"]');
  await b.waitForFunction((id) => !yap.ui.space.tag.over && yap.ui.space.tag.it !== id, sidA.slice(0, 8), { timeout: 5000 });
  await phone.waitForFunction((id) => !yap.ui.space.tag.over && yap.ui.space.tag.it !== id, sidA.slice(0, 8), { timeout: 5000 });
  assert.equal(await b.evaluate(() => yap.ui.space.tag.it), await phone.evaluate(() => yap.ui.space.tag.it));
  await b.waitForFunction((sid) => yap.ui.space.ball.owner && yap.ui.space.ball.owner !== sid, sidA, { timeout: 5000 });
});

await test('watch together: a video YouTube refuses says why, and never asks you to "Click to join the show"', async () => {
  const before = await a.evaluate(() => yap.ui.watch.state?.videoId);
  await a.click('[data-action="open-view"][data-view="watch"]'); // a seat again
  await a.fill('.watch-add input', 'https://youtu.be/brokenvid01');
  await a.click('.watch-add button[value="play"]');
  await a.waitForSelector('.watch-note:has-text("YouTube won\'t play this one here: its owner turned off embedding")', { timeout: 8000 });
  await a.waitForTimeout(6500); // past the two tries to start it, after which a blocked player gets the gate
  assert.ok(await a.isHidden('#watch-dock .watch-gate'), 'no "Click to join the show" for a video that can\'t play');
  assert.ok(await a.isHidden('#watch-dock .watch-hint'), 'no "press ▶ on the video" either');
  // The show as it was, for the next test.
  await a.evaluate((id) => yap.ui.send({ t: 'watch.set', videoId: id }), before ?? 'vidBBBBBBB2');
  await until(async () => (await ytPlayer(a))?.state === 1, 8000, 'the show back on');
  await a.click('.watch-head [data-watch="leave"]');
  await a.click('[data-action="open-channel"][data-id="general"]');
});

await test('watch together on a phone: the view (heads scroll sideways) and the mini player above the composer', async () => {
  await fakeYouTube(phone, 'Cleo');
  await a.click('[data-action="open-view"][data-view="watch"]'); // a second head in the seats
  if (!(await phone.evaluate(() => document.querySelector('.app').classList.contains('sidebar-open')))) await phone.click('[data-action="toggle-sidebar"]');
  await phone.click('[data-action="open-view"][data-view="watch"]');
  await until(async () => (await ytPlayer(phone))?.state === 1, 10000, "Cleo's player to start");
  await until(async () => (await seatNames(phone)) === 'Ada,Cleo', 5000, 'both heads on the phone');
  // Someone who connects while the show is on sees the audience straight from `ready`.
  const late = await b.context().newPage();
  await late.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await late.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  await until(async () => (await watchAudience(late)) === 'Ada,Cleo', 3000, 'the audience in a newcomer\'s sidebar');
  await late.click('[data-action="open-view"][data-view="watch"]');
  await until(async () => (await seatNames(late)) === 'Ada,Ben,Cleo', 5000, 'everyone in a newcomer\'s seats');
  await late.close();
  await until(async () => (await seatNames(phone)) === 'Ada,Cleo', 5000, 'the closed tab leaves its seat');
  const fits = await phone.evaluate(() => {
    const row = document.querySelector('.watch-seats');
    const dock = document.querySelector('#watch-dock').getBoundingClientRect();
    return { scrolls: row.scrollWidth > row.clientWidth, overflowX: getComputedStyle(row).overflowX, dockWidth: dock.width, page: document.documentElement.scrollWidth <= innerWidth };
  });
  assert.equal(fits.overflowX, 'auto', 'the heads row scrolls sideways');
  assert.ok(fits.page && fits.dockWidth >= 380, `video full width, no sideways page scroll: ${JSON.stringify(fits)}`);
  await phone.waitForTimeout(600);
  await phone.screenshot({ path: path.join(OUT, 'phone-watch.png') });
  await phone.click('[data-action="toggle-sidebar"]');
  await phone.click('[data-action="open-channel"][data-id="general"]');
  await phone.waitForSelector('#watch-dock[data-mode="mini"]', { timeout: 3000 });
  await phone.waitForTimeout(700);
  const box = await phone.evaluate(() => ({
    dock: document.querySelector('#watch-dock').getBoundingClientRect().toJSON(),
    composer: document.querySelector('.composer-wrap').getBoundingClientRect().toJSON(),
  }));
  assert.ok(box.dock.bottom <= box.composer.top && box.dock.right <= 390, `mini player above the composer: ${JSON.stringify(box)}`);
  await phone.screenshot({ path: path.join(OUT, 'phone-watch-mini.png') });
  assert.equal((await ytPlayer(phone)).state, 1, 'still playing');
});

await test('on a touch phone: Draw has one row of tools, a palette and the actions under the page; Space says to tap', async () => {
  const ctx = await newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, colorScheme: 'dark' }, 'Cleo, touch phone');
  await ctx.addInitScript(() => {
    if (window === window.top && location.origin.startsWith('http') && !localStorage.getItem('yap.settings')) localStorage.setItem('yap.settings', JSON.stringify({ mode: 'open' }));
  });
  const t = await ctx.newPage();
  await t.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await settle(t, 'Cleo');
  const open = (view) => t.evaluate((v) => document.querySelector(`[data-action="open-view"][data-view="${v}"]`).click(), view);
  await open('draw');
  await t.waitForFunction(() => document.querySelector('.draw-paper')?.clientWidth > 300, null, { timeout: 5000 });
  await t.waitForTimeout(400);
  const l = await t.evaluate(() => {
    const box = (sel) => document.querySelector(sel).getBoundingClientRect();
    const tools = document.querySelector('.draw-tools');
    return {
      toolsH: box('.draw-tools').height, toolsFit: tools.scrollWidth <= tools.clientWidth + 1,
      paletteHolds: Boolean(document.querySelector('.draw-palette .draw-colors') && document.querySelector('.draw-palette .draw-layers')),
      actionsUnder: document.querySelector('.draw-stage').nextElementSibling?.classList.contains('draw-actions'),
      paperGap: box('.draw-paper').top - box('.draw-stage').top, paperW: box('.draw-paper').width,
      page: document.documentElement.scrollWidth <= innerWidth,
    };
  });
  assert.ok(l.toolsH < 64 && l.toolsFit, `one row of tools that fits: ${JSON.stringify(l)}`);
  assert.ok(l.paletteHolds && l.actionsUnder && l.page, `palette + bar under the page: ${JSON.stringify(l)}`);
  assert.ok(l.paperGap < 20 && l.paperW > 340, `the page at the top, full width: ${JSON.stringify(l)}`);
  await t.screenshot({ path: path.join(OUT, 'phone-draw.png') });
  await t.tap('[data-tool="palette"]');
  await t.waitForSelector('.draw-palette:not([hidden])');
  await t.screenshot({ path: path.join(OUT, 'phone-draw-palette.png') });
  await t.tap('.draw-palette [data-color="#ff0000"]');
  await t.waitForSelector('.draw-palette', { state: 'hidden' });
  assert.equal(await t.evaluate(() => yap.ui.draw.color), '#ff0000');
  // Space on a touch screen: no keys to learn, just a tap.
  await t.evaluate(() => { try { localStorage.removeItem('yap.spaceTapped'); } catch {} });
  await open('space');
  await t.waitForSelector('.space-hud .space-tap-hint:has-text("Tap where you want to walk")', { timeout: 5000 });
  await t.screenshot({ path: path.join(OUT, 'phone-space.png') });
  const c = await t.locator('.space-canvas').boundingBox();
  await t.touchscreen.tap(c.x + c.width / 2 + 40, c.y + c.height / 2);
  await t.waitForSelector('.space-tap-hint', { state: 'detached', timeout: 3000 });
  await ctx.close();
});

// ── "Match system" ────────────────────────────────────────────────────────
// The tests above set light or dark directly. "Match system" (theme 'system')
// must look exactly the same as picking the OS's scheme yourself.

/** Every element's colours (pseudo-elements too), keyed by where it sits in the page. */
const colorsOf = (page) => page.evaluate(() => {
  const PROPS = ['color', 'background-color', 'background-image', 'border-top-color', 'border-right-color', 'border-bottom-color', 'border-left-color',
    'outline-color', 'box-shadow', 'text-shadow', 'fill', 'stroke', 'caret-color', 'accent-color', 'opacity', 'filter', 'backdrop-filter', 'color-scheme'];
  for (const anim of document.getAnimations()) anim.cancel(); // where things settle, not mid-fade
  const where = (el) => {
    const parts = [];
    for (let e = el; e && e !== document.body; e = e.parentElement) {
      if (e.id) {
        parts.unshift(`#${e.id}`);
        break;
      }
      parts.unshift(`${e.tagName.toLowerCase()}${e.classList.length ? `.${[...e.classList].join('.')}` : ''}:${[...e.parentElement.children].indexOf(e)}`);
    }
    return parts.join(' > ');
  };
  const out = {};
  for (const el of document.querySelectorAll('body *')) {
    for (const pseudo of [null, '::before', '::after']) {
      const cs = getComputedStyle(el, pseudo);
      if (pseudo && cs.content === 'none') continue;
      out[`${where(el)}${pseudo ?? ''}`] = Object.fromEntries(PROPS.map((p) => [p, cs.getPropertyValue(p)]));
    }
  }
  return out;
});
/** One line per element that differs: which properties, and the first one's values. */
const colorDiffs = (want, got) => Object.entries(want).flatMap(([el, props]) => {
  const changed = got[el] ? Object.keys(props).filter((p) => props[p] !== got[el][p]) : [];
  return changed.length ? [`${el} { ${changed.join(', ')} } ${props[changed[0]]} → ${got[el][changed[0]]}`] : [];
});

await test('"Match system" looks exactly like picking light or dark yourself', async () => {
  const page = await a.context().newPage();
  await page.goto(`${BASE}/`);
  await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
  // Some of everything with colours of its own: a Place (Draw, and its header note) and Settings (checkboxes).
  await page.click('[data-action="open-view"][data-view="draw"]');
  await page.waitForFunction(() => yap.ui.draw && document.querySelector('.draw-stage'));
  await page.click('[data-action="open-settings"]');
  const voiceTab = await page.evaluate(() => [...document.querySelectorAll('#settings-tabs [data-tab]')].find((b) => b.textContent === 'Voice')?.dataset.tab);
  await page.click(`#settings-tabs [data-tab="${voiceTab}"]`);
  // Open Advanced: its controls are compared too (and Chromium keeps stale computed styles inside a closed <details>).
  await page.click('#voice-advanced > summary');
  const settle = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  const problems = [];
  try {
    for (const scheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.evaluate((t) => yap.ui.setTheme(t), scheme);
      await settle();
      const picked = await colorsOf(page);
      await page.evaluate(() => yap.ui.setTheme('system'));
      await settle();
      const system = await colorsOf(page);
      await page.screenshot({ path: path.join(OUT, `system-${scheme}-settings.png`) });
      problems.push(...colorDiffs(picked, system).map((d) => `on a ${scheme} system: ${d}`));
    }
  } finally {
    await page.evaluate(() => yap.ui.setTheme('dark'));
    await page.close();
  }
  assert.deepEqual(problems, [], `"Match system" differs from the scheme picked by hand:\n${problems.join('\n')}`);
});

await test('no page errors or console errors anywhere in the run', async () => {
  assert.deepEqual(pageErrors, []);
});

await browser.close();
fs.rmSync(path.dirname(wav), { recursive: true, force: true });

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
if (measured) console.log(`\nOpus: ${measured.kbps} kbps received, ${measured.packets} packets, ${measured.lost} lost\nfmtp: ${measured.fmtp}`);
if (obsMeasured?.median) console.log(`OBS page: a click plays ${obsMeasured.median.toFixed(1)} ms after it leaves Ada (median of ${obsMeasured.heard}), ${obsMeasured.pps} packets/s${obsMeasured.clean ? `; a 1 kHz tone comes out at THD+N ${obsMeasured.clean.thdn.toFixed(1)} dB` : ''}${obsMeasured.keepDb ? `; keep-alive ${obsMeasured.keepDb.toFixed(1)} dBFS in pauses` : ''}`);
console.log(`screenshots: ${OUT}`);
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
// Exiting at once cuts off whatever is still on its way down a pipe (test:all reads this through
// one, and lost the list's end and the "passed" line): let stdout drain first.
process.exitCode = failed ? 1 : 0;
process.stdout.write('', () => process.exit());
