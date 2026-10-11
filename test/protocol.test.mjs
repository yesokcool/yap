// End-to-end protocol test against a running server. Easiest: npm run test:all
// (it starts a server with everything below). By hand:
//   YAP_URL=http://localhost:8787 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin node test/protocol.test.mjs
// with the server started as CLAUDE.md › Commands says; the fake ntfy and Web
// Push services listen on YAP_TEST_NTFY_PORT (8790) and YAP_TEST_PUSH_PORT (8791).
// Uses Node 22's built-in WebSocket. Each run uses fresh random user ids, so
// it is safe against a dev server with existing data.
import assert from 'node:assert/strict';
import http from 'node:http';
import crypto from 'node:crypto';

const BASE = process.env.YAP_URL ?? 'http://localhost:8787';
const KEY = process.env.YAP_KEY ?? '';
// Needed: new users wait in the waiting room until let in (the tests use the admin API).
const ADMIN = process.env.YAP_ADMIN_KEY ?? '';
const admin = (path, init = {}) => fetch(`${BASE}${path}`, { ...init, headers: { ...init.headers, Authorization: `Bearer ${ADMIN}` } });
const WS_BASE = BASE.replace(/^http/, 'ws');
const rid = () => Math.random().toString(36).slice(2, 10);
// Names are unique per server now, so every run uses fresh ones.
const RUN = rid().slice(0, 4);
const ALICE = `Alice ${RUN}`;
const BOB = `Bob ${RUN}`;

const TEST_PIN = '582039';

class Client {
  constructor(name) {
    this.name = name;
    this.uid = null; // known after hello
    this.token = null; // device token; set to reconnect as the same account
    this.inbox = [];
    this.waiters = [];
  }
  /** Key travels in the WebSocket subprotocol like the web client does (or ?key= with `query`). */
  async open(key = KEY, { query = false } = {}) {
    this.ws = query || !key
      ? new WebSocket(`${WS_BASE}/ws?key=${encodeURIComponent(key)}`)
      : new WebSocket(`${WS_BASE}/ws`, ['yap', `key.${key}`]);
    this.ws.addEventListener('message', (e) => {
      if (e.data === 'pong') return;
      const msg = JSON.parse(e.data);
      const i = this.waiters.findIndex((w) => w.pred(msg));
      if (i >= 0) this.waiters.splice(i, 1)[0].resolve(msg);
      else this.inbox.push(msg);
    });
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true });
      this.ws.addEventListener('error', () => reject(new Error(`${this.name}: socket failed`)), { once: true });
    });
    return this;
  }
  send(msg) {
    this.ws.send(typeof msg === 'string' ? msg : JSON.stringify(msg));
  }
  next(pred, ms = 3000) {
    const i = this.inbox.findIndex(pred);
    if (i >= 0) return Promise.resolve(this.inbox.splice(i, 1)[0]);
    return new Promise((resolve, reject) => {
      const w = { pred, resolve };
      this.waiters.push(w);
      setTimeout(() => {
        const j = this.waiters.indexOf(w);
        if (j >= 0) {
          this.waiters.splice(j, 1);
          reject(new Error(`${this.name}: timed out waiting (inbox: ${JSON.stringify(this.inbox.map((m) => m.t))})`));
        }
      }, ms);
    });
  }
  /** Reconnect with the device token, or sign up (name + PIN) and get let in via the admin API. */
  async hello() {
    if (this.token) this.send({ t: 'hello', v: 1, token: this.token, client: 'test' });
    else this.send({ t: 'hello', v: 1, signup: { name: this.name, pin: TEST_PIN, color: '#ff8800' }, client: 'test' });
    const first = await this.next((m) => m.t === 'ready' || m.t === 'pending' || (m.t === 'error' && m.re === 'hello'));
    if (first.t === 'error') throw new Error(`${this.name}: ${first.code} ${first.message}`);
    if (first.token) this.token = first.token;
    this.uid = first.uid;
    if (first.t === 'pending') {
      if (!ADMIN) throw new Error('new users wait to be let in: set YAP_ADMIN_KEY');
      await admin(`/api/admin/role?uid=${this.uid}&role=member`, { method: 'POST' });
      this.ready = await this.next((m) => m.t === 'ready');
    } else {
      this.ready = first;
    }
    return this.ready;
  }
  close() {
    this.ws.close();
  }
}

/** A test that can't run here says so (and why) instead of passing without checking anything. */
class Skip extends Error {}
const skip = (why) => {
  throw new Skip(why);
};
// npm run test:all sets this: everything a test needs is there, so a skip is a failure.
const STRICT = process.env.YAP_TEST_STRICT === '1';

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(err instanceof Skip ? ['○', `${name} (skipped: ${err.message})`] : ['✗', name, err]);
  }
}

// ── Auth ────────────────────────────────────────────────────────────────
await test('/api/check reports key state', async () => {
  const good = await (await fetch(`${BASE}/api/check?key=${encodeURIComponent(KEY)}`)).json();
  assert.equal(good.ok, true);
  assert.equal(good.protocol, 1);
  if (KEY) {
    const bad = await (await fetch(`${BASE}/api/check?key=nope`)).json();
    assert.equal(bad.ok, false);
    assert.equal(bad.keyRequired, true);
  }
});

if (KEY) {
  await test('wrong key cannot open a socket', async () => {
    await assert.rejects(new Client('Intruder').open('nope'));
  });
}

await test('/api/ice returns at least STUN', async () => {
  const body = await (await fetch(`${BASE}/api/ice?key=${encodeURIComponent(KEY)}`)).json();
  assert.ok(Array.isArray(body.iceServers) && body.iceServers.length > 0);
});

await test('web client is served at /', async () => {
  const res = await fetch(`${BASE}/`);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /<title>Yap<\/title>/);
});

// ── Session & text ──────────────────────────────────────────────────────
const a = await new Client(ALICE).open();
const b = await new Client(BOB).open();

await test('commands before hello are rejected', async () => {
  a.send({ t: 'typing', channelId: 'general' });
  const err = await a.next((m) => m.t === 'error');
  assert.equal(err.code, 'no_hello');
});

await test('heartbeat ping gets pong', async () => {
  const got = new Promise((resolve) => a.ws.addEventListener('message', (e) => e.data === 'pong' && resolve(), { once: false }));
  a.send('ping');
  await Promise.race([got, new Promise((_, r) => setTimeout(() => r(new Error('no pong')), 2000))]);
});

await test('a frame over the size limit is refused, saying what the limit is', async () => {
  a.send(JSON.stringify({ t: 'typing', pad: 'x'.repeat(400_000) }));
  const err = await a.next((m) => m.t === 'error');
  assert.equal(err.code, 'bad_frame');
  assert.match(err.message, /384 KB/);
});

await test('valid JSON that isn’t an object gets bad_json, and the socket keeps working', async () => {
  for (const frame of ['null', '42', '"hi"', '[1,2]']) {
    a.send(frame);
    assert.equal((await a.next((m) => m.t === 'error')).code, 'bad_json', frame);
  }
});

await test('hello → ready with seeded channels', async () => {
  const ready = await a.hello();
  assert.equal(ready.v, 1);
  assert.ok(ready.sid);
  const ids = ready.channels.map((c) => c.id);
  for (const id of ['general', 'lounge', 'gaming']) assert.ok(ids.includes(id), `missing ${id}`);
  assert.ok(ready.users.some((u) => u.id === a.uid && u.name === ALICE));
  assert.equal(ready.uid, a.uid, 'ready echoes the canonical uid');
  assert.ok(ready.sessions.some((s) => s.sid === ready.sid));
  // The live web build, so open pages can update themselves (PROTOCOL.md › Updates).
  assert.match(ready.build?.app ?? '', /^[0-9a-f]{12}$/);
  assert.match(ready.build?.style ?? '', /^[0-9a-f]{12}$/);
});

await test('no presence: connecting is never announced; joining voice is (session.join, then voice)', async () => {
  await b.hello();
  const user = await a.next((m) => m.t === 'user' && m.user.id === b.uid);
  assert.equal(user.user.name, BOB, 'a brand-new member is introduced');
  assert.equal(user.user.lastSeen, 0, 'no last-seen');
  await new Promise((r) => setTimeout(r, 300));
  assert.ok(!a.inbox.some((m) => m.t === 'session.join'), 'coming online tells nobody');
  assert.ok(!b.ready.sessions.some((s) => s.uid === a.uid), 'idle sessions are not listed');
  assert.ok(b.ready.sessions.some((s) => s.sid === b.ready.sid), 'but your own is');
  b.send({ t: 'voice.join', channelId: 'lounge' });
  const join = await a.next((m) => m.t === 'session.join' && m.session.sid === b.ready.sid);
  assert.equal(join.session.voice, null, 'announced as it was, then the voice change');
  assert.equal((await a.next((m) => m.t === 'voice' && m.sid === b.ready.sid)).channelId, 'lounge');
  b.send({ t: 'voice.leave' });
  assert.equal((await a.next((m) => m.t === 'voice' && m.sid === b.ready.sid)).channelId, null);
  await a.next((m) => m.t === 'session.leave' && m.sid === b.ready.sid);
  await b.next((m) => m.t === 'voice' && m.sid === b.ready.sid && m.channelId === null);
});

let sentId;
await test('send → echo with nonce to sender, broadcast to others', async () => {
  a.send({ t: 'send', channelId: 'general', body: 'hello **world**  ', nonce: 'n1' });
  const echo = await a.next((m) => m.t === 'message' && m.nonce === 'n1');
  assert.equal(echo.message.body, 'hello **world**', 'trailing whitespace trimmed');
  assert.equal(echo.message.uid, a.uid);
  const other = await b.next((m) => m.t === 'message' && m.message.id === echo.message.id);
  assert.equal(other.nonce, undefined);
  sentId = echo.message.id;
});

await test('history returns the message, oldest first', async () => {
  b.send({ t: 'history', channelId: 'general', limit: 5 });
  const h = await b.next((m) => m.t === 'history');
  assert.equal(h.messages.at(-1).id, sentId);
  for (let i = 1; i < h.messages.length; i++) assert.ok(h.messages[i - 1].ts <= h.messages[i].ts);
});

await test('history paging with before/beforeId', async () => {
  // A channel of its own, so nothing else posting can shift the pages.
  a.send({ t: 'channel.create', name: `paging-${RUN}`, kind: 'text' });
  const { channel } = await a.next((m) => m.t === 'channel' && m.channel.name === `paging-${RUN}`);
  await b.next((m) => m.t === 'channel' && m.channel.id === channel.id);
  for (let i = 0; i < 4; i++) {
    a.send({ t: 'send', channelId: channel.id, body: `page ${i}`, nonce: `p${i}` });
    await a.next((m) => m.t === 'message' && m.nonce === `p${i}`);
  }
  a.send({ t: 'history', channelId: channel.id, limit: 2 });
  const first = await a.next((m) => m.t === 'history' && m.channelId === channel.id);
  assert.deepEqual(first.messages.map((m) => m.body), ['page 2', 'page 3']);
  assert.equal(first.hasMore, true);
  const oldest = first.messages[0];
  a.send({ t: 'history', channelId: channel.id, limit: 2, before: oldest.ts, beforeId: oldest.id });
  const second = await a.next((m) => m.t === 'history' && m.channelId === channel.id);
  assert.deepEqual(second.messages.map((m) => m.body), ['page 0', 'page 1']);
  assert.equal(second.hasMore, false);
  a.send({ t: 'channel.archive', channelId: channel.id });
  await a.next((m) => m.t === 'channel.archived' && m.channelId === channel.id);
  await b.next((m) => m.t === 'channel.archived' && m.channelId === channel.id);
  for (const c of [a, b]) c.inbox = c.inbox.filter((m) => !(m.t === 'message' && m.message.channelId === channel.id));
});

await test('empty and oversized messages are rejected', async () => {
  a.send({ t: 'send', channelId: 'general', body: '   ' });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'empty');
  a.send({ t: 'send', channelId: 'general', body: 'x'.repeat(40_001) });
  const long = await a.next((m) => m.t === 'error');
  assert.equal(long.code, 'too_long');
  assert.match(long.message, /40,000 characters/, 'the real limit, readably');
  a.send({ t: 'send', channelId: 'lounge', body: 'voice channels have no chat' });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'no_channel');
});

const GIF = {
  provider: 'klipy', slug: 'hello-hi-662', title: 'Hello', width: 498, height: 280,
  mp4: 'https://static.klipy.com/ii/abc/14/af/MCCBoQlZ.mp4',
  webp: 'https://static.klipy.com/ii/abc/14/af/eUbp2uNc.webp',
  still: 'https://static1.klipy.com/ii/abc/14/af/LyWpim71.jpg',
};

await test('GIF-only message is accepted, stored and returned by history', async () => {
  a.send({ t: 'send', channelId: 'general', body: '', gif: GIF, nonce: 'g1' });
  const echo = await a.next((m) => m.t === 'message' && m.nonce === 'g1');
  assert.deepEqual(echo.message.gif, GIF);
  assert.equal(echo.message.body, '');
  const seen = await b.next((m) => m.t === 'message' && m.message.id === echo.message.id);
  assert.deepEqual(seen.message.gif, GIF);
  a.send({ t: 'history', channelId: 'general', limit: 1 });
  const h = await a.next((m) => m.t === 'history');
  assert.deepEqual(h.messages[0].gif, GIF);
});

await test('plain messages carry no gif field', async () => {
  a.send({ t: 'send', channelId: 'general', body: 'just text', nonce: 'g2' });
  const echo = await a.next((m) => m.t === 'message' && m.nonce === 'g2');
  assert.equal('gif' in echo.message, false);
});

await test('GIFs pointing anywhere but KLIPY are rejected', async () => {
  const bad = [
    { ...GIF, mp4: 'https://evil.example/pixel.mp4' },
    { ...GIF, webp: 'http://static.klipy.com/x.webp' },
    { ...GIF, still: 'https://static.klipy.com.evil.example/x.jpg' },
    { ...GIF, gif: 'https://static.klipy.com/x" onerror="alert(1).gif' },
    { ...GIF, provider: 'giphy' },
    { ...GIF, width: 0 },
    { ...GIF, slug: '../../etc' },
    { provider: 'klipy', slug: 'x', title: '', width: 10, height: 10 },
  ];
  for (const gif of bad) {
    a.send({ t: 'send', channelId: 'general', body: '', gif });
    const err = await a.next((m) => m.t === 'error');
    assert.equal(err.code, 'bad_gif', JSON.stringify(gif));
  }
});

await test('/api/config hands the GIF key only to invited clients', async () => {
  const ok = await fetch(`${BASE}/api/config?key=${encodeURIComponent(KEY)}`);
  assert.equal(ok.status, 200);
  const body = await ok.json();
  assert.ok('gifs' in body);
  if (KEY) assert.equal((await fetch(`${BASE}/api/config?key=nope`)).status, 401);
});

await test('typing reaches others, not the sender', async () => {
  a.send({ t: 'typing', channelId: 'general' });
  const t = await b.next((m) => m.t === 'typing');
  assert.equal(t.uid, a.uid);
});

await test('only the author can delete; delete is broadcast', async () => {
  b.send({ t: 'delete', id: sentId });
  assert.equal((await b.next((m) => m.t === 'error')).code, 'forbidden');
  a.send({ t: 'delete', id: sentId });
  const d = await b.next((m) => m.t === 'deleted');
  assert.equal(d.id, sentId);
  a.send({ t: 'history', channelId: 'general', limit: 10 });
  const h = await a.next((m) => m.t === 'history');
  assert.ok(!h.messages.some((m) => m.id === sentId), 'deleted message hidden from history');
});

await test('profile change is broadcast', async () => {
  a.send({ t: 'profile', name: `  Alice\u0000 Cooper ${RUN}  `, color: '#00AA00' });
  const u = await b.next((m) => m.t === 'user' && m.user.id === a.uid);
  assert.equal(u.user.name, `Alice Cooper ${RUN}`);
  assert.equal(u.user.color, '#00aa00');
});

let created;
await test('channel names are kept as typed; archive hides it', async () => {
  a.send({ t: 'channel.create', name: 'Movie Night!', kind: 'text' });
  created = (await b.next((m) => m.t === 'channel')).channel;
  assert.equal(created.name, 'Movie Night!');
  assert.equal(created.kind, 'text');
  b.send({ t: 'channel.archive', channelId: created.id });
  assert.equal((await a.next((m) => m.t === 'channel.archived')).channelId, created.id);
  // Its messages are kept for backups, but nobody can page them any more.
  a.send({ t: 'history', channelId: created.id });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'history')).code, 'no_channel');
});

await test('personal invites: key works, newcomer shows who invited them, revoke; ready hands members the main key', async () => {
  assert.equal(a.ready.key, KEY || undefined);
  a.send({ t: 'invite.create', label: 'for Cleo' });
  const made = await a.next((m) => m.t === 'invite.created');
  assert.match(made.key, /^yi_[A-Za-z0-9_-]{32}$/);
  assert.equal(made.invite.label, 'for Cleo');
  const check = await (await fetch(`${BASE}/api/check`, { headers: { 'X-Yap-Key': made.key } })).json();
  assert.equal(check.ok, true, 'invite key opens the door');
  const bogus = await (await fetch(`${BASE}/api/check`, { headers: { 'X-Yap-Key': `yi_${'x'.repeat(32)}` } })).json();
  assert.equal(bogus.ok, false);
  const cleo = new Client(`Cleo ${RUN}`);
  await cleo.open(made.key);
  cleo.send({ t: 'hello', v: 1, signup: { name: cleo.name, pin: TEST_PIN, color: '#00aaff' }, client: 'test' });
  const pending = await cleo.next((m) => m.t === 'pending');
  const req = await b.next((m) => m.t === 'join.request' && m.user.id === pending.uid);
  assert.match(req.user.invitedBy, /^Alice .+ \(for Cleo\)$/, 'says whose invite it was');
  b.send({ t: 'member.approve', uid: pending.uid });
  const cleoReady = await cleo.next((m) => m.t === 'ready');
  assert.equal(cleoReady.key, KEY || undefined, 'members get the main key, so revoking the invite never locks them out');
  b.send({ t: 'invite.revoke', id: made.invite.id });
  assert.equal((await b.next((m) => m.t === 'error' && m.re === 'invite.revoke')).code, 'forbidden');
  a.send({ t: 'invite.revoke', id: made.invite.id });
  const list = await a.next((m) => m.t === 'invites');
  const mine = list.invites.find((i) => i.id === made.invite.id);
  assert.equal(mine.revoked, true);
  assert.equal(mine.uses, 1);
  cleo.close();
});

await test('a personal link used twice: the second request says who used it before (links can be passed on)', async () => {
  a.send({ t: 'invite.create', label: 'for Dee' });
  const made = await a.next((m) => m.t === 'invite.created');
  const join = async (name) => {
    const c = new Client(name);
    await c.open(made.key);
    c.send({ t: 'hello', v: 1, signup: { name, pin: TEST_PIN, color: '#00aaff' }, client: 'test' });
    const pending = await c.next((m) => m.t === 'pending');
    const req = await b.next((m) => m.t === 'join.request' && m.user.id === pending.uid);
    return { c, uid: pending.uid, user: req.user };
  };
  const first = await join(`Dee ${RUN}`);
  assert.match(first.user.invite.by, /^Alice /, "the inviter's name");
  assert.deepEqual([first.user.invite.label, first.user.invite.usedBy], ['for Dee', []], 'a first use: nobody before');
  assert.match(first.user.invitedBy, /^Alice .+ \(for Dee\)$/, 'older clients still get invitedBy');
  const second = await join(`Eve ${RUN}`);
  assert.deepEqual(second.user.invite.usedBy, [`Dee ${RUN}`], 'the same link came in before, as Dee');
  // A member connecting now sees the same in ready.pending.
  const look = new Client(ALICE);
  look.token = a.token;
  await look.open();
  const ready = await look.hello();
  assert.deepEqual(ready.pending.find((u) => u.id === second.uid)?.invite?.usedBy, [`Dee ${RUN}`]);
  assert.deepEqual(ready.pending.find((u) => u.id === first.uid)?.invite?.usedBy, [`Eve ${RUN}`], 'and the other way round');
  look.close();
  for (const p of [first, second]) {
    b.send({ t: 'member.deny', uid: p.uid });
    await b.next((m) => m.t === 'join.resolved' && m.uid === p.uid);
    p.c.close();
  }
  a.send({ t: 'invite.revoke', id: made.invite.id });
  await a.next((m) => m.t === 'invites');
});

await test('hello.client: the iPad app is "Yap for iPad" in Your devices; a client value the server doesn\'t know still gets in', async () => {
  const name = `Tablet ${RUN}`;
  const owner = await new Client(name).open();
  await owner.hello();
  const signIn = async (client) => {
    const c = await new Client(`${name} (${client})`).open();
    c.send({ t: 'hello', v: 1, login: { name, pin: TEST_PIN }, client });
    const ready = await c.next((m) => m.t === 'ready' || (m.t === 'error' && m.re === 'hello'));
    assert.equal(ready.t, 'ready', `${client}: ${JSON.stringify(ready)}`);
    return c;
  };
  const ipad = await signIn('ipad');
  const odd = await signIn('some-new-thing');
  owner.send({ t: 'account.devices' });
  const { devices } = await owner.next((m) => m.t === 'devices');
  assert.equal(devices.find((d) => d.client === 'ipad')?.label, 'Yap for iPad');
  assert.ok(devices.find((d) => d.client === 'some-new-thing')?.label, 'labeled from its User-Agent');
  // An iPad that signed in with an older app (it said "ios", so "Yap for iPhone") is relabeled
  // when it comes back saying "ipad"; a device whose client didn't change keeps its label.
  const old = await new Client(`${name} (old app)`).open();
  old.send({ t: 'hello', v: 1, login: { name, pin: TEST_PIN }, client: 'ios' });
  const first = await old.next((m) => m.t === 'ready');
  old.close();
  const labels = async () => {
    owner.send({ t: 'account.devices' });
    return (await owner.next((m) => m.t === 'devices')).devices;
  };
  const before = (await labels()).find((d) => d.client === 'ios');
  assert.equal(before?.label, 'Yap for iPhone');
  const back = await new Client(`${name} (updated app)`).open();
  back.send({ t: 'hello', v: 1, token: first.token, client: 'ipad' });
  await back.next((m) => m.t === 'ready');
  assert.equal((await labels()).find((d) => d.id === before.id)?.label, 'Yap for iPad', 'the default label follows the new client value');
  back.close();
  const again = await new Client(`${name} (same app)`).open();
  again.send({ t: 'hello', v: 1, token: first.token, client: 'ipad' });
  await again.next((m) => m.t === 'ready');
  assert.equal((await labels()).find((d) => d.id === before.id)?.label, 'Yap for iPad');
  for (const c of [ipad, odd, again, owner]) c.close();
});

await test('sidebar links: in ready, add (https only), remove, everyone sees it', async () => {
  const boardLink = a.ready.links.find((l) => l.view === 'board');
  assert.equal(boardLink?.name, 'Test board', 'the board link is seeded and named by BOARD_NAME');
  assert.equal(boardLink.url, `${BASE}/board`, 'with no address of its own it opens this server’s /board');
  a.send({ t: 'link.add', name: 'Bad', url: 'javascript:alert(1)' });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'link.add')).code, 'bad_url');
  a.send({ t: 'link.add', name: 'Wiki', url: 'https://example.com/wiki' });
  const added = (await b.next((m) => m.t === 'links' && m.links.some((l) => l.name === 'Wiki'))).links.find((l) => l.name === 'Wiki');
  assert.equal(added.url, 'https://example.com/wiki');
  const config = await (await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': KEY } })).json();
  assert.ok(config.links.some((l) => l.id === added.id), '/api/config serves the same links');
  b.send({ t: 'link.remove', id: added.id });
  assert.ok(!(await a.next((m) => m.t === 'links' && !m.links.some((l) => l.id === added.id))).links.some((l) => l.id === added.id));
});

await test('OBS links: listen-only token can join voice (shown as obs) and nothing else; turning it off kicks it', async () => {
  a.send({ t: 'obs.create' });
  const { token } = await a.next((m) => m.t === 'obs.created');
  const obs = new Client('obs');
  await obs.open();
  obs.send({ t: 'hello', v: 1, client: 'web', token });
  const ready = await obs.next((m) => m.t === 'ready');
  assert.equal(ready.uid, a.uid, 'listens as its maker');
  assert.equal(ready.key, undefined, 'no main key for OBS links');
  obs.send({ t: 'send', channelId: 'general', body: 'hi from obs' });
  assert.equal((await obs.next((m) => m.t === 'error' && m.re === 'send')).code, 'forbidden');
  obs.send({ t: 'voice.join', channelId: 'lounge', muted: false });
  const joined = await b.next((m) => m.t === 'session.join' && m.session.sid === ready.sid);
  assert.equal(joined.session.client, 'obs', 'everyone can see it is an OBS link');
  const v = await b.next((m) => m.t === 'voice' && m.sid === ready.sid);
  assert.equal(v.channelId, 'lounge');
  assert.equal(v.muted, true, 'always muted');
  const closed = new Promise((r) => obs.ws.addEventListener('close', r, { once: true }));
  a.send({ t: 'obs.revoke' });
  assert.equal((await a.next((m) => m.t === 'obs.revoked')).count, 1);
  await closed;
  const again = new Client('obs2');
  await again.open();
  again.send({ t: 'hello', v: 1, client: 'web', token });
  assert.equal((await again.next((m) => m.t === 'error')).code, 'bad_token');
  again.close();
});

await test('home-screen sign-in pass: works once, gives the new app its own token', async () => {
  a.send({ t: 'device.handoff' });
  const { token: pass } = await a.next((m) => m.t === 'device.handoff');
  const app = new Client('homescreen');
  await app.open();
  app.send({ t: 'hello', v: 1, client: 'web', token: pass });
  const ready = await app.next((m) => m.t === 'ready');
  assert.equal(ready.uid, a.uid, 'signed in as the same person');
  assert.ok(ready.token && ready.token !== pass, 'gets its own lasting token');
  app.close();
  const again = new Client('homescreen2');
  await again.open();
  again.send({ t: 'hello', v: 1, client: 'web', token: pass });
  assert.equal((await again.next((m) => m.t === 'error')).code, 'bad_token', 'a pass works once');
  again.close();
  const back = new Client('homescreen3');
  await back.open();
  back.send({ t: 'hello', v: 1, client: 'web', token: ready.token });
  assert.equal((await back.next((m) => m.t === 'ready')).uid, a.uid, 'and the new token keeps working');
  back.close();
});

await test('Personal space: yours alone — your other devices get it, nobody else can read, post, react or delete', async () => {
  const mine = a.ready.channels.find((c) => c.kind === 'personal');
  assert.equal(mine.id, `p_${a.uid}`);
  assert.ok(!b.ready.channels.some((c) => c.id === mine.id), "others never see your Personal space");
  // Your other device (same account).
  const other = new Client('a-laptop');
  other.token = a.token;
  await other.open();
  await other.hello();
  a.send({ t: 'send', channelId: mine.id, body: 'link for later https://example.com/x', nonce: 'p1' });
  const sent = (await a.next((m) => m.t === 'message' && m.nonce === 'p1')).message;
  assert.equal((await other.next((m) => m.t === 'message' && m.message.id === sent.id)).message.channelId, mine.id);
  await new Promise((r) => setTimeout(r, 400));
  assert.ok(!b.inbox.some((m) => m.t === 'message' && m.message?.id === sent.id), 'not broadcast to anyone else');
  for (const [msg, re] of [
    [{ t: 'history', channelId: mine.id }, 'history'],
    [{ t: 'send', channelId: mine.id, body: 'sneaky' }, 'send'],
    [{ t: 'react', id: sent.id, emoji: '👀', on: true }, 'react'],
    [{ t: 'delete', id: sent.id }, 'delete'],
  ]) {
    b.send(msg);
    const err = await b.next((m) => m.t === 'error' && m.re === re);
    assert.ok(['no_channel', 'no_message'].includes(err.code), `${re} refused (${err.code})`);
  }
  other.send({ t: 'history', channelId: mine.id, limit: 5 });
  assert.ok((await other.next((m) => m.t === 'history')).messages.some((m) => m.id === sent.id), 'readable from your other device');
  other.close();
});

await test('voice channel quality is shared; bad values refused', async () => {
  a.send({ t: 'channel.bitrate', channelId: 'lounge', bitrate: 256000 });
  assert.equal((await b.next((m) => m.t === 'channel' && m.channel.id === 'lounge')).channel.bitrate, 256000);
  a.send({ t: 'channel.bitrate', channelId: 'lounge', bitrate: 123 });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'channel.bitrate')).code, 'bad_bitrate');
  a.send({ t: 'channel.bitrate', channelId: 'lounge', bitrate: null });
  assert.equal((await b.next((m) => m.t === 'channel' && m.channel.id === 'lounge')).channel.bitrate, null);
});

await test('moderation log records who did what (no message text); storage is for mods', async () => {
  a.send({ t: 'modlog' });
  const { entries } = await a.next((m) => m.t === 'modlog');
  for (const action of ['invite.create', 'invite.revoke', 'member.approve', 'link.add', 'link.remove', 'channel.bitrate']) {
    assert.ok(entries.some((e) => e.action === action), `logged ${action}`);
  }
  assert.ok(entries.every((e) => !String(e.detail ?? '').includes('react to me')), 'no message text in the log');
  const role = (c) => c.ready.users.find((u) => u.id === c.uid)?.role;
  const member = [a, b].find((c) => role(c) === 'member');
  if (member) {
    member.send({ t: 'stats' });
    assert.equal((await member.next((m) => m.t === 'error' && m.re === 'stats')).code, 'forbidden');
  }
});

await test('edit: your own messages only; everyone sees it; history keeps editedAt', async () => {
  a.send({ t: 'send', channelId: 'general', body: 'tpyo here', nonce: 'ed1' });
  const m = (await a.next((x) => x.t === 'message' && x.nonce === 'ed1')).message;
  a.send({ t: 'edit', id: m.id, body: 'typo fixed' });
  const e = await b.next((x) => x.t === 'edited' && x.id === m.id);
  assert.equal(e.body, 'typo fixed');
  assert.equal(e.channelId, 'general');
  assert.ok(e.editedAt >= m.ts);
  b.send({ t: 'edit', id: m.id, body: 'hijacked' });
  assert.equal((await b.next((x) => x.t === 'error' && x.re === 'edit')).code, 'forbidden');
  a.send({ t: 'edit', id: m.id, body: '   ' });
  assert.equal((await a.next((x) => x.t === 'error' && x.re === 'edit')).code, 'empty');
  a.send({ t: 'history', channelId: 'general', limit: 5 });
  const got = (await a.next((x) => x.t === 'history')).messages.find((x) => x.id === m.id);
  assert.equal(got.body, 'typo fixed');
  assert.ok(got.editedAt);
});

// ── Custom emojis ───────────────────────────────────────────────────────
const PNG_1PX = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64');
// Same image + unique trailing bytes = a distinct emoji (different content hash).
const png = () => Buffer.concat([PNG_1PX, Buffer.from(rid())]);
const upload = (body, name, sid = a.ready.sid, key = KEY, type = 'image/png', ut = a.ready.uploadToken) =>
  fetch(`${BASE}/api/emoji?${new URLSearchParams({ key, sid, name, ut })}`, { method: 'POST', body, headers: { 'Content-Type': type } });
const emojiName = `e_${rid()}`;
let emoji;

await test('emoji upload → stored, broadcast, served immutable', async () => {
  const img = png();
  const res = await upload(img, emojiName);
  assert.equal(res.status, 200, await res.clone().text());
  emoji = (await res.json()).emoji;
  assert.match(emoji.id, /^[a-f0-9]{32}$/);
  assert.equal(emoji.mime, 'image/png');
  assert.equal(emoji.uid, a.uid);
  const seen = await b.next((m) => m.t === 'emoji' && m.emoji.id === emoji.id);
  assert.equal(seen.emoji.name, emojiName);
  const get = await fetch(`${BASE}/emoji/${emoji.id}`);
  assert.equal(get.status, 200);
  assert.equal(get.headers.get('content-type'), 'image/png');
  assert.match(get.headers.get('cache-control'), /immutable/);
  assert.equal(get.headers.get('x-content-type-options'), 'nosniff');
  assert.deepEqual(Buffer.from(await get.arrayBuffer()), img);
  assert.equal((await fetch(`${BASE}/emoji/${'0'.repeat(32)}`)).status, 404);
  // HEAD: the same headers, no body (it used to be a 500).
  const head = await fetch(`${BASE}/emoji/${emoji.id}`, { method: 'HEAD' });
  assert.deepEqual([head.status, head.headers.get('content-type'), head.headers.get('content-length'), (await head.arrayBuffer()).byteLength], [200, 'image/png', String(img.length), 0]);
  assert.equal((await fetch(`${BASE}/media/${'0'.repeat(32)}`, { method: 'HEAD' })).status, 404);
  assert.equal((await fetch(`${BASE}/media/${emoji.id}`, { method: 'POST' })).status, 405);
});

await test('emoji upload rejects bad input', async () => {
  const cases = [
    [() => upload(png(), emojiName.toUpperCase()), 409, 'name_taken'],
    [() => upload(png(), 'x'), 400, 'bad_name'],
    [() => upload(png(), 'has space'), 400, 'bad_name'],
    [() => upload(Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'), `s_${rid()}`, a.ready.sid, KEY, 'image/svg+xml'), 415, 'bad_type'],
    [() => upload(Buffer.concat([PNG_1PX, Buffer.alloc(513 * 1024)]), `big_${rid()}`), 413, 'too_big'],
    [() => upload(png(), `nosid_${rid()}`, 'not-a-session'), 401, 'no_session'],
  ];
  for (const [run, status, code] of cases) {
    const res = await run();
    assert.equal(res.status, status, `${code}: ${await res.clone().text()}`);
    assert.equal((await res.json()).code, code);
  }
  if (KEY) assert.equal((await upload(png(), `k_${rid()}`, a.ready.sid, 'nope')).status, 401);
});

await test('an emoji upload without a live session stores nothing', async () => {
  const img = png();
  const id = crypto.createHash('sha256').update(img).digest('hex').slice(0, 32);
  assert.equal((await upload(img, `ghost_${rid()}`, 'not-a-session')).status, 401);
  assert.equal((await upload(img, `ghost_${rid()}`, a.ready.sid, KEY, 'image/png', 'f'.repeat(32))).status, 401, 'a real session id with the wrong upload token');
  if (ADMIN) assert.ok(!(await (await admin('/api/admin/media')).json()).some((b) => b.id === id), 'the bytes never reached the media store');
});

await test('same image twice is a duplicate; after delete it can come back renamed', async () => {
  const img = png();
  const first = await (await upload(img, `d_${rid()}`)).json();
  assert.equal(first.ok, true);
  const again = await upload(img, `d_${rid()}`);
  assert.equal(again.status, 409);
  assert.equal((await again.json()).code, 'duplicate');
  b.send({ t: 'emoji.delete', id: first.emoji.id });
  assert.equal((await a.next((m) => m.t === 'emoji.deleted' && m.id === first.emoji.id)).id, first.emoji.id);
  assert.equal((await fetch(`${BASE}/emoji/${first.emoji.id}`)).status, 200, 'image survives a delete (old messages)');
  const back = await (await upload(img, `back_${rid()}`)).json();
  assert.equal(back.ok, true);
  assert.equal(back.emoji.id, first.emoji.id);
});

await test('ready lists live emojis', async () => {
  const c = new Client(`Cara ${RUN}`);
  await c.open();
  const ready = await c.hello();
  assert.ok(ready.emojis.some((e) => e.id === emoji.id && e.name === emojiName));
  c.close();
});

if (ADMIN) {
  await test('admin: stats + export (incl. deleted) need the admin key', async () => {
    assert.equal((await fetch(`${BASE}/api/admin/stats`)).status, 401);
    assert.equal((await fetch(`${BASE}/api/admin/export`, { headers: { Authorization: 'Bearer nope' } })).status, 401);
    const auth = { headers: { Authorization: `Bearer ${ADMIN}` } };
    const stats = await (await fetch(`${BASE}/api/admin/stats`, auth)).json();
    assert.ok(stats.hub.messages > 0 && stats.hub.databaseBytes > 0 && stats.media.blobs > 0);
    let after = 0;
    const messages = [];
    let first;
    for (;;) {
      const page = await (await fetch(`${BASE}/api/admin/export?after=${after}`, auth)).json();
      first ??= page;
      messages.push(...page.messages);
      if (page.next === null) break;
      after = page.next;
    }
    assert.ok(first.channels.length && first.users.length && first.emojis.length);
    assert.ok(first.schema >= 3);
    assert.ok(messages.some((m) => m.id === sentId && m.deletedAt), 'deleted message is in the backup');
    const media = await (await fetch(`${BASE}/api/admin/media`, auth)).json();
    assert.ok(media.some((m) => m.id === emoji.id));
  });
}

// ── Names & emoji management ────────────────────────────────────────────
await test('a newcomer cannot take an existing name (any case)', async () => {
  const imposter = new Client(BOB.toUpperCase());
  await imposter.open();
  imposter.send({ t: 'hello', v: 1, signup: { name: BOB.toUpperCase(), pin: TEST_PIN } });
  const err = await imposter.next((m) => m.t === 'error');
  assert.equal(err.code, 'name_taken');
  assert.equal(err.re, 'hello');
  imposter.send({ t: 'hello', v: 1, signup: { name: `Not Bob ${RUN}`, pin: TEST_PIN } });
  assert.equal((await imposter.next((m) => m.t === 'pending')).t, 'pending', 'can retry with another name (then waits to be let in)');
  imposter.close();
});

await test('renaming to a taken name is refused', async () => {
  b.send({ t: 'profile', name: `alice cooper ${RUN}` });
  assert.equal((await b.next((m) => m.t === 'error')).code, 'name_taken');
});

await test('emoji rename: broadcast, unique, old tokens still resolve', async () => {
  const renamed = `r_${rid()}`;
  a.send({ t: 'emoji.rename', id: emoji.id, name: renamed });
  const ev = await b.next((m) => m.t === 'emoji' && m.emoji.id === emoji.id);
  assert.equal(ev.emoji.name, renamed);
  const other = await (await upload(png(), `o_${rid()}`)).json();
  a.send({ t: 'emoji.rename', id: other.emoji.id, name: renamed.toUpperCase() });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'name_taken');
  a.send({ t: 'emoji.rename', id: emoji.id, name: 'no spaces allowed' });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'bad_name');
});

if (ADMIN) {
  await test('admin emoji upload needs the admin key, not a session', async () => {
    const post = (auth) => fetch(`${BASE}/api/admin/emoji?name=adm_${rid()}`, { method: 'POST', body: png(), headers: auth ? { Authorization: `Bearer ${ADMIN}` } : {} });
    assert.equal((await post(false)).status, 401);
    const res = await post(true);
    assert.equal(res.status, 200);
    assert.equal((await res.json()).emoji.uid, 'admin');
  });
}

// ── Accounts: name + PIN, device tokens ─────────────────────────────────
const PIN = '482916';
const raw = async () => new Client(`raw-${rid()}`).open();
const helloWith = (c, extra) => c.send({ t: 'hello', v: 1, client: 'test', ...extra });
const reply = (c) => c.next((m) => m.t === 'ready' || m.t === 'pending' || (m.t === 'error' && m.re === 'hello'));
let acct; // { name, uid, token }

await test('sign up with name + PIN → waiting room with a device token; let in → ready', async () => {
  const name = `Pinny ${RUN}`;
  const c = await raw();
  helloWith(c, { signup: { name, pin: PIN, color: '#00ff00' } });
  const pending = await reply(c);
  assert.equal(pending.t, 'pending');
  assert.match(pending.token, /^[A-Za-z0-9_-]{40,}$/);
  const req = await a.next((m) => m.t === 'join.request' && m.user.name === name);
  a.send({ t: 'member.approve', uid: req.user.id });
  const ready = await c.next((m) => m.t === 'ready');
  assert.equal(ready.needsPin, false);
  acct = { name, uid: req.user.id, token: pending.token };
  c.close();
});

await test('sign in on a new device with name + PIN (any case) → same account, new token', async () => {
  const c = await raw();
  helloWith(c, { login: { name: acct.name.toUpperCase(), pin: PIN } });
  const ready = await reply(c);
  assert.equal(ready.t, 'ready', JSON.stringify(ready));
  assert.equal(ready.uid, acct.uid);
  assert.ok(ready.token && ready.token !== acct.token, 'a fresh token for this device');
  c.close();
});

await test('a saved device token reconnects without the PIN', async () => {
  const c = await raw();
  helloWith(c, { token: acct.token });
  const ready = await reply(c);
  assert.equal(ready.uid, acct.uid);
  assert.equal(ready.token, undefined, 'no new token needed');
  c.close();
});

await test('wrong PINs are refused, then the name locks', async () => {
  for (let i = 0; i < 5; i++) {
    const c = await raw();
    helloWith(c, { login: { name: acct.name, pin: '000000' } });
    assert.equal((await reply(c)).code, 'bad_login');
    c.close();
  }
  const c = await raw();
  helloWith(c, { login: { name: acct.name, pin: PIN } });
  assert.equal((await reply(c)).code, 'locked', 'even the right PIN waits out the lock');
  c.close();
});

await test('signing up with a taken name points you to signing in', async () => {
  const c = await raw();
  helloWith(c, { signup: { name: acct.name, pin: '111111' } });
  const err = await reply(c);
  assert.equal(err.code, 'name_taken');
  assert.match(err.message, /I have an account/);
  c.close();
});

await test('PINs change only with the current one; a bare uid never signs anyone in; sign-out kills the token', async () => {
  const c = await new Client(`Pinned ${RUN}`).open();
  const ready = await c.hello();
  assert.equal(ready.needsPin, false);
  c.send({ t: 'account.pin', pin: '246802' });
  assert.equal((await c.next((m) => m.t === 'error')).code, 'bad_login', 'changing needs the current PIN');
  c.send({ t: 'account.pin', pin: '246802', current: TEST_PIN });
  assert.equal((await c.next((m) => m.t === 'account')).token, undefined, 'device already has a token');
  const imposter = await raw();
  helloWith(imposter, { uid: c.uid, name: c.name, color: '#000000' });
  assert.equal((await reply(imposter)).code, 'pin_required', 'knowing the uid is not enough');
  const stranger = await raw();
  helloWith(stranger, { uid: `nobody-${rid()}`, name: `Nobody ${RUN}`, color: '#000000' });
  assert.equal((await reply(stranger)).code, 'pin_required', 'the uid-only sign-in is gone');
  const back = await raw();
  helloWith(back, { token: c.token });
  assert.equal((await reply(back)).uid, c.uid);
  back.send({ t: 'account.signout' });
  await new Promise((r) => setTimeout(r, 200));
  const after = await raw();
  helloWith(after, { token: c.token });
  assert.equal((await reply(after)).code, 'bad_token', 'signed-out token is dead');
  c.close();
});

if (ADMIN) {
  await test('owner resets a PIN (and the admin API can too)', async () => {
    a.send({ t: 'owner.claim', key: ADMIN });
    await a.next((m) => m.t === 'user' && m.user.id === a.uid && m.user.role === 'owner');
    a.send({ t: 'member.resetPin', uid: acct.uid });
    const reset = await a.next((m) => m.t === 'pin.reset');
    assert.match(reset.pin, /^\d{6}$/);
    const c = await raw();
    helloWith(c, { login: { name: acct.name, pin: reset.pin } });
    assert.equal((await reply(c)).uid, acct.uid, 'temporary PIN works (and clears the lock)');
    c.close();
    const viaApi = await (await admin(`/api/admin/reset-pin?uid=${acct.uid}`, { method: 'POST' })).json();
    assert.match(viaApi.pin, /^\d{6}$/);
  });
}

// ── Waiting room & moderation ───────────────────────────────────────────
/** Sign up without being let in yet; fills in uid + token from the waiting-room reply. */
const signUp = async (c) => {
  c.send({ t: 'hello', v: 1, signup: { name: c.name, pin: TEST_PIN, color: '#123456' }, client: 'test' });
  const pending = await c.next((m) => m.t === 'pending' || m.t === 'ready');
  Object.assign(c, { uid: pending.uid, token: pending.token });
  return pending;
};

await test('waiting room: a newcomer waits, can do nothing, any member lets them in', async () => {
  const n = await new Client(`New ${RUN}`).open();
  assert.equal((await signUp(n)).t, 'pending');
  const req = await a.next((m) => m.t === 'join.request' && m.user.id === n.uid);
  assert.equal(req.user.role, 'pending');
  n.send({ t: 'send', channelId: 'general', body: 'spam' });
  assert.equal((await n.next((m) => m.t === 'error')).code, 'pending');
  b.send({ t: 'member.approve', uid: n.uid });
  const ready = await n.next((m) => m.t === 'ready');
  assert.ok(ready.channels.length > 0);
  const resolved = await a.next((m) => m.t === 'join.resolved' && m.uid === n.uid);
  assert.deepEqual([resolved.approved, resolved.by], [true, b.uid]);
  n.close();
});

await test('turning someone away keeps them out for good', async () => {
  const x = await new Client(`Stranger ${RUN}`).open();
  await signUp(x);
  await a.next((m) => m.t === 'join.request' && m.user.id === x.uid);
  a.send({ t: 'member.deny', uid: x.uid });
  assert.equal((await x.next((m) => m.t === 'error')).code, 'denied');
  const again = new Client(`Stranger ${RUN}`);
  again.token = x.token;
  await again.open();
  again.send({ t: 'hello', v: 1, token: x.token });
  assert.equal((await again.next((m) => m.t === 'error')).code, 'banned');
});

if (ADMIN) {
  await test('owner claim needs the admin key', async () => {
    b.send({ t: 'owner.claim', key: 'wrong' });
    assert.equal((await b.next((m) => m.t === 'error')).code, 'bad_key');
    a.send({ t: 'owner.claim', key: ADMIN });
    assert.equal((await b.next((m) => m.t === 'user' && m.user.id === a.uid && m.user.role === 'owner')).user.role, 'owner');
  });

  await test('owner makes mods; mods delete anything and ban; nobody bans the owner', async () => {
    a.send({ t: 'member.role', uid: b.uid, role: 'mod' });
    await b.next((m) => m.t === 'user' && m.user.id === b.uid && m.user.role === 'mod');
    const t = await new Client(`Troll ${RUN}`).open();
    await t.hello();
    t.send({ t: 'send', channelId: 'general', body: 'something awful', nonce: 'troll' });
    const bad = (await t.next((m) => m.t === 'message' && m.nonce === 'troll')).message;
    b.send({ t: 'delete', id: bad.id });
    assert.equal((await a.next((m) => m.t === 'deleted' && m.id === bad.id)).id, bad.id);
    b.send({ t: 'member.ban', uid: t.uid });
    assert.equal((await t.next((m) => m.t === 'error')).code, 'banned');
    assert.equal((await a.next((m) => m.t === 'user' && m.user.id === t.uid && m.user.role === 'banned')).user.role, 'banned');
    b.send({ t: 'member.ban', uid: a.uid });
    assert.equal((await b.next((m) => m.t === 'error')).code, 'forbidden');
    a.send({ t: 'member.role', uid: b.uid, role: 'member' });
    await b.next((m) => m.t === 'user' && m.user.id === b.uid && m.user.role === 'member');
  });

  await test('banned through the admin API: told why, and out of voice for everyone at once', async () => {
    const t = await new Client(`Banned ${RUN}`).open();
    await t.hello();
    t.send({ t: 'voice.join', channelId: 'gaming' });
    await b.next((m) => m.t === 'voice' && m.sid === t.ready.sid && m.channelId === 'gaming');
    const closed = new Promise((r) => t.ws.addEventListener('close', (e) => r(e.code), { once: true }));
    await admin(`/api/admin/role?uid=${t.uid}&role=banned`, { method: 'POST' });
    assert.equal((await t.next((m) => m.t === 'error')).code, 'banned', 'told why');
    assert.equal(await closed, 4003);
    await b.next((m) => m.t === 'voice' && m.sid === t.ready.sid && m.channelId === null);
    await b.next((m) => m.t === 'session.leave' && m.sid === t.ready.sid);
  });

  await test('a burst of messages gets slowed down', async () => {
    const s = await new Client(`Spammer ${RUN}`).open();
    await s.hello();
    for (let i = 0; i < 16; i++) s.send({ t: 'send', channelId: 'general', body: `spam ${i}` });
    await new Promise((r) => setTimeout(r, 600));
    const slowed = s.inbox.filter((m) => m.t === 'error' && m.code === 'slow_down').length;
    const sent = s.inbox.filter((m) => m.t === 'message' && m.message.uid === s.uid).length;
    assert.ok(slowed >= 4 && sent <= 12, `slowed ${slowed}, sent ${sent}`);
    s.close();
  });
}

await test('board view presence + cursors go only to people on the board', async () => {
  a.send({ t: 'view', view: 'board' });
  assert.equal((await b.next((m) => m.t === 'view' && m.sid === a.ready.sid)).view, 'board');
  b.send({ t: 'cursor', view: 'board', a: { k: 'card', id: 'g0_0', x: 0.5, y: 0.25 } });
  const c1 = await a.next((m) => m.t === 'cursor');
  assert.deepEqual([c1.sid, c1.a], [b.ready.sid, { k: 'card', id: 'g0_0', x: 0.5, y: 0.25 }]);
  a.send({ t: 'cursor', view: 'board', a: { k: 'lane', id: 'l0', x: 0.1, y: 0.1 } });
  await assert.rejects(b.next((m) => m.t === 'cursor', 400), 'b is not on the board, so no cursors');
  b.send({ t: 'view', view: 'board' });
  await a.next((m) => m.t === 'view' && m.sid === b.ready.sid);
  a.send({ t: 'cursor', view: 'board', a: { k: 'lane', id: 'l0', x: 9, y: -9, extra: 'dropped' } });
  assert.deepEqual((await b.next((m) => m.t === 'cursor')).a, { k: 'lane', id: 'l0', x: 1.5, y: -0.5 });
  a.send({ t: 'cursor', view: 'board', a: { k: 'card', id: '<img onerror=x>', x: 0, y: 0 } });
  assert.equal((await b.next((m) => m.t === 'cursor')).a, null, 'bad ids are dropped');
  a.send({ t: 'view', view: 'not-a-view' });
  assert.equal((await b.next((m) => m.t === 'view' && m.sid === a.ready.sid)).view, null);
  b.send({ t: 'view', view: null });
});

await test('Minecraft status + config', async () => {
  // test/wrangler.test.jsonc: MINECRAFT_SERVER is a port nobody answers on, and there's a board.
  const cfg = await (await fetch(`${BASE}/api/config?key=${encodeURIComponent(KEY)}`)).json();
  assert.equal(cfg.minecraft.address, '127.0.0.1:9');
  assert.ok(cfg.links.some((l) => l.view === 'board'), 'the board link');
  const mc = await (await fetch(`${BASE}/api/minecraft?key=${encodeURIComponent(KEY)}`)).json();
  assert.equal(mc.address, '127.0.0.1:9');
  assert.equal(mc.online, false);
  if (KEY) assert.equal((await fetch(`${BASE}/api/minecraft?key=nope`)).status, 401);
});

await test('the board is served through Yap; only the paths in BOARD_PATHS reach it', async () => {
  assert.match(await (await fetch(`${BASE}/board`)).text(), /<title>Test board<\/title>/);
  const board = await (await fetch(`${BASE}/api/board`)).json();
  assert.ok(Array.isArray(board.board.lanes));
  const other = await fetch(`${BASE}/api/tidy`, { method: 'POST', body: '{}' });
  assert.notEqual(other.status, 200);
});

// ── Pings, pushes, avatars, drawing, space ──────────────────────────────
/** A fake service on a fixed port; a busy port ends the run with a clear message. */
const fakeService = (name, port, host, handler) => {
  const server = http.createServer(handler);
  server.on('error', (err) => {
    console.error(`The fake ${name} can't listen on port ${port} (${err.code ?? err.message}). Pick another with ${name === 'ntfy' ? 'YAP_TEST_NTFY_PORT (and the server\'s NTFY_SERVER)' : 'YAP_TEST_PUSH_PORT'}, or use npm run test:all.`);
    process.exit(1);
  });
  return server.listen(port, host);
};
// The server's NTFY_SERVER must point here: --var NTFY_SERVER:http://127.0.0.1:<this port>.
const NTFY_PORT = Number(process.env.YAP_TEST_NTFY_PORT ?? 8790);
const pushes = [];
const ntfy = fakeService('ntfy', NTFY_PORT, undefined, (req, res) => {
  let body = '';
  req.on('data', (c) => (body += c)).on('end', () => {
    try { pushes.push(JSON.parse(body)); } catch {}
    res.end('{}');
  });
});
const pushFor = (topic, pred = () => true, ms = 3000) => new Promise((resolve, reject) => {
  const started = Date.now();
  const tick = () => {
    const hit = pushes.find((p) => p.topic === topic && pred(p));
    if (hit) return resolve(hit);
    if (Date.now() - started > ms) return reject(new Error(`no push for ${topic} (got ${pushes.length})`));
    setTimeout(tick, 50);
  };
  tick();
});

let bTopic;
await test('phone pushes: pick a mode → private ntfy topic (kept when turned off); /api/config says which ntfy', async () => {
  const config = await (await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': KEY } })).json();
  assert.equal(config.ntfy.server, `http://127.0.0.1:${NTFY_PORT}`, 'the server’s NTFY_SERVER, so people subscribe where pushes go');
  b.send({ t: 'notify.settings', mode: 'pings' });
  const n = await b.next((m) => m.t === 'notify');
  assert.match(n.topic, /^yap-[a-z0-9]{24}$/);
  b.send({ t: 'notify.settings', mode: 'off' });
  assert.equal((await b.next((m) => m.t === 'notify')).topic, n.topic);
  b.send({ t: 'notify.settings', mode: 'pings' });
  bTopic = (await b.next((m) => m.t === 'notify')).topic;
});

await test('📣 ping: everyone hears it, pushes go out, and it is rate limited', async () => {
  a.send({ t: 'ping', text: 'game night!' });
  const heard = await b.next((m) => m.t === 'ping');
  assert.deepEqual([heard.from, heard.text], [a.uid, 'game night!']);
  const push = await pushFor(bTopic, (p) => /pinged everyone/.test(p.title));
  assert.equal(push.message, 'game night!');
  a.send({ t: 'ping' });
  await b.next((m) => m.t === 'ping');
  a.send({ t: 'ping' });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'slow_down');
});

await test('@mention pushes to the person mentioned', async () => {
  a.send({ t: 'send', channelId: 'general', body: `hey @${BOB} check this`, nonce: 'mention' });
  await a.next((m) => m.t === 'message' && m.nonce === 'mention');
  const push = await pushFor(bTopic, (p) => /mentioned you/.test(p.title));
  assert.match(push.message, /check this/);
});

await test('@mentions push to exactly who is named: "@Sam Lee" is never also "@Sam"; code mentions nobody', async () => {
  const sam = await new Client(`Sam${RUN}`).open();
  const lee = await new Client(`Sam${RUN} Lee`).open();
  const topics = [];
  for (const c of [sam, lee]) {
    await c.hello();
    c.send({ t: 'notify.settings', mode: 'pings' });
    topics.push((await c.next((m) => m.t === 'notify')).topic);
  }
  const [samTopic, leeTopic] = topics;
  const mark = pushes.length;
  a.send({ t: 'send', channelId: 'general', body: `hey @Sam${RUN} Lee! (and \`@Sam${RUN}\` in code)`, nonce: 'mention2' });
  await a.next((m) => m.t === 'message' && m.nonce === 'mention2');
  await pushFor(leeTopic, (p) => /mentioned you/.test(p.title));
  await new Promise((r) => setTimeout(r, 600));
  assert.ok(!pushes.slice(mark).some((p) => p.topic === samTopic), 'Sam was not mentioned');
  sam.close();
  lee.close();
});

// A fake Web Push service: records what the Worker POSTs to subscriptions.
const PUSH_PORT = Number(process.env.YAP_TEST_PUSH_PORT ?? 8791);
const webPushes = [];
const pushServer = fakeService('Web Push service', PUSH_PORT, '127.0.0.1', (req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    webPushes.push({ path: req.url, headers: req.headers, body: Buffer.concat(chunks) });
    res.writeHead(201).end();
  });
});
const b64u = (buf) => Buffer.from(buf).toString('base64url');
/** What a browser does with a Web Push body (RFC 8291). */
function openWebPush(body, ua, auth) {
  const salt = body.subarray(0, 16);
  const asPublic = body.subarray(21, 21 + body[20]);
  const ct = body.subarray(21 + body[20]);
  const ikm = Buffer.from(crypto.hkdfSync('sha256', ua.computeSecret(asPublic), auth, Buffer.concat([Buffer.from('WebPush: info\0'), ua.getPublicKey(), asPublic]), 32));
  const cek = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: aes128gcm\0'), 16));
  const nonce = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: nonce\0'), 12));
  const d = crypto.createDecipheriv('aes-128-gcm', cek, nonce);
  d.setAuthTag(ct.subarray(ct.length - 16));
  const plain = Buffer.concat([d.update(ct.subarray(0, ct.length - 16)), d.final()]);
  return JSON.parse(plain.subarray(0, plain.length - 1).toString());
}

await test('browser pushes: subscribe, an @mention arrives encrypted + VAPID-signed, unsubscribe stops it', async () => {
  const config = await (await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': KEY } })).json();
  if (!config.push) throw new Error('start the dev server with --var VAPID_PRIVATE_JWK:<jwk>');
  const ua = crypto.createECDH('prime256v1');
  ua.generateKeys();
  const auth = crypto.randomBytes(16);
  const endpoint = `http://127.0.0.1:${PUSH_PORT}/push/${rid()}`;
  b.send({ t: 'push.subscribe', endpoint: 'javascript:alert(1)', keys: { p256dh: b64u(ua.getPublicKey()), auth: b64u(auth) } });
  assert.equal((await b.next((m) => m.t === 'error' && m.re === 'push.subscribe')).code, 'bad_subscription');
  b.send({ t: 'push.subscribe', endpoint, keys: { p256dh: b64u(ua.getPublicKey()), auth: b64u(auth) } });
  assert.notEqual((await b.next((m) => m.t === 'notify')).mode, 'off', 'subscribing turns pushes on');
  a.send({ t: 'send', channelId: 'general', body: `@${BOB} browser push?`, nonce: 'wp' });
  let got;
  for (let i = 0; i < 40 && !got; i++) {
    got = webPushes.find((p) => endpoint.endsWith(p.path));
    if (!got) await new Promise((r) => setTimeout(r, 100));
  }
  assert.ok(got, 'the Worker POSTed to the subscription');
  assert.equal(got.headers['content-encoding'], 'aes128gcm');
  assert.match(got.headers.authorization, new RegExp(`^vapid t=[\\w-]+\\.[\\w-]+\\.[\\w-]+, k=${config.push.publicKey}$`));
  // The VAPID JWT verifies with the server's public key (ES256), for this push service, not expired.
  const [, jwt] = got.headers.authorization.match(/^vapid t=([^,]+),/);
  const [h, c, sig] = jwt.split('.');
  const raw = Buffer.from(config.push.publicKey, 'base64url');
  const vapidKey = crypto.createPublicKey({ key: { kty: 'EC', crv: 'P-256', x: b64u(raw.subarray(1, 33)), y: b64u(raw.subarray(33)) }, format: 'jwk' });
  assert.ok(crypto.verify('sha256', Buffer.from(`${h}.${c}`), { key: vapidKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(sig, 'base64url')), 'VAPID signature');
  const claims = JSON.parse(Buffer.from(c, 'base64url'));
  assert.equal(claims.aud, `http://127.0.0.1:${PUSH_PORT}`);
  assert.ok(claims.exp * 1000 > Date.now() && claims.exp * 1000 < Date.now() + 24 * 3600 * 1000, 'exp within 24 h (push services require it)');
  const msg = openWebPush(got.body, ua, auth);
  assert.match(msg.title, /mentioned you/);
  assert.match(msg.body, /browser push\?/);
  b.send({ t: 'push.unsubscribe', endpoint });
  await new Promise((r) => setTimeout(r, 200));
  const before = webPushes.length;
  a.send({ t: 'send', channelId: 'general', body: `@${BOB} still there?`, nonce: 'wp2' });
  await new Promise((r) => setTimeout(r, 1200));
  assert.equal(webPushes.filter((p) => endpoint.endsWith(p.path)).length, 1, 'no more after unsubscribing');
  assert.ok(webPushes.length >= before);
});

await test('pushes turned off stay off, though the browser sends its subscription again on every connect', async () => {
  const ua = crypto.createECDH('prime256v1');
  ua.generateKeys();
  const subscribe = { t: 'push.subscribe', endpoint: `http://127.0.0.1:${PUSH_PORT}/push/${rid()}`, keys: { p256dh: b64u(ua.getPublicKey()), auth: b64u(crypto.randomBytes(16)) } };
  b.send({ t: 'notify.settings', mode: 'off' });
  assert.equal((await b.next((m) => m.t === 'notify')).mode, 'off');
  b.send(subscribe);
  assert.equal((await b.next((m) => m.t === 'notify')).mode, 'pings', 'turning on browser pushes turns pushes on');
  b.send({ t: 'notify.settings', mode: 'off' });
  assert.equal((await b.next((m) => m.t === 'notify')).mode, 'off');
  b.send(subscribe); // what the page does after every reconnect
  assert.equal((await b.next((m) => m.t === 'notify')).mode, 'off', 'still off');
  b.send({ t: 'push.unsubscribe', endpoint: subscribe.endpoint });
  b.send({ t: 'notify.settings', mode: 'pings' }); // back as the later tests expect
  assert.equal((await b.next((m) => m.t === 'notify')).topic, bTopic);
});

await test('avatars: upload → everyone sees it; remove → gone', async () => {
  // Someone else's session id alone (ids are visible to everyone) isn't enough.
  const hijack = await fetch(`${BASE}/api/avatar?${new URLSearchParams({ key: KEY, sid: a.ready.sid, ut: b.ready.uploadToken })}`, { method: 'POST', body: png() });
  assert.equal(hijack.status, 403, 'another member cannot set your picture');
  const res = await fetch(`${BASE}/api/avatar?${new URLSearchParams({ key: KEY, sid: a.ready.sid, ut: a.ready.uploadToken })}`, { method: 'POST', body: png() });
  assert.equal(res.status, 200, await res.clone().text());
  const u = await b.next((m) => m.t === 'user' && m.user.id === a.uid && m.user.avatar);
  assert.equal((await fetch(`${BASE}/media/${u.user.avatar}`)).status, 200);
  a.send({ t: 'profile', avatar: null });
  assert.equal((await b.next((m) => m.t === 'user' && m.user.id === a.uid && m.user.avatar === null)).user.avatar, null);
});

await test('pictures in messages: upload needs a session; a message carries the picture', async () => {
  const noSession = await fetch(`${BASE}/api/image?sid=nope`, { method: 'POST', headers: { 'X-Yap-Key': KEY, 'Content-Type': 'image/png' }, body: png() });
  assert.equal(noSession.status, 403, 'no upload without a live session');
  const up = await (await fetch(`${BASE}/api/image?sid=${a.ready.sid}&ut=${a.ready.uploadToken}`, { method: 'POST', headers: { 'X-Yap-Key': KEY, 'Content-Type': 'image/png' }, body: png() })).json();
  assert.match(up.id, /^[a-f0-9]{32}$/);
  a.send({ t: 'send', channelId: 'general', body: 'look', image: { id: up.id, width: 640, height: 480 }, nonce: 'pic1' });
  const got = (await b.next((m) => m.t === 'message' && m.message.image?.id === up.id)).message;
  assert.deepEqual(got.image, { id: up.id, width: 640, height: 480 });
  a.send({ t: 'send', channelId: 'general', body: '', image: { id: 'not-an-id', width: 1, height: 1 } });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'send')).code, 'bad_image');
  a.send({ t: 'history', channelId: 'general', limit: 5 });
  assert.ok((await a.next((m) => m.t === 'history')).messages.some((m) => m.image?.id === up.id));
  // A picture can be erased (its post deleted), so nothing caches it for long, whichever path asks for it.
  for (const path of ['media', 'emoji']) {
    assert.equal((await fetch(`${BASE}/${path}/${up.id}?p=${rid()}`)).headers.get('cache-control'), 'public, max-age=86400', path);
  }
  if (ADMIN) {
    const backup = await admin(`/api/admin/media/${up.id}`);
    assert.deepEqual([backup.status, backup.headers.get('cache-control')], [200, 'no-store'], 'backups read it uncached');
    assert.equal((await fetch(`${BASE}/api/admin/media/${up.id}`)).status, 401);
  }
});

await test('long messages: 40,000 characters fit; history pages stay small', async () => {
  a.send({ t: 'channel.create', name: `long-${RUN}`, kind: 'text' });
  const { channel } = await a.next((m) => m.t === 'channel' && m.channel.name === `long-${RUN}`);
  const long = (i) => `${i} ${'words'.repeat(8_000)}`.slice(0, 40_000);
  await new Promise((r) => setTimeout(r, 4000)); // earlier tests used up the send burst
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 1100)); // 1 send/s
    a.send({ t: 'send', channelId: channel.id, body: long(i), nonce: `long${i}` });
    assert.equal((await a.next((m) => m.t === 'message' && m.nonce === `long${i}`, 6000)).message.body.length, 40_000);
  }
  a.send({ t: 'history', channelId: channel.id, limit: 50 });
  const h = await a.next((m) => m.t === 'history' && m.channelId === channel.id, 6000);
  assert.ok(h.messages.length < 8 && h.messages.length >= 1, `page cut short (${h.messages.length})`);
  assert.equal(h.hasMore, true);
  assert.ok(h.messages.at(-1).body.startsWith('7 '), 'newest first page');
  a.send({ t: 'channel.archive', channelId: channel.id });
  await a.next((m) => m.t === 'channel.archived' && m.channelId === channel.id);
  await new Promise((r) => setTimeout(r, 5000)); // let the send limit refill
});

let replyTarget;
await test('replies: carry who/what they answer, in history too; replying across channels is refused', async () => {
  a.send({ t: 'send', channelId: 'general', body: 'who is up for a game tonight?', nonce: 'q1' });
  replyTarget = (await a.next((m) => m.t === 'message' && m.nonce === 'q1')).message;
  b.send({ t: 'send', channelId: 'general', body: 'me!', replyTo: replyTarget.id, nonce: 'r1' });
  const echo = (await b.next((m) => m.t === 'message' && m.nonce === 'r1')).message;
  assert.deepEqual(echo.reply, { id: replyTarget.id, uid: a.uid, channelId: 'general', body: 'who is up for a game tonight?' });
  const seen = (await a.next((m) => m.t === 'message' && m.message.id === echo.id)).message;
  assert.equal(seen.reply.uid, a.uid);
  b.send({ t: 'history', channelId: 'general', limit: 3 });
  assert.equal((await b.next((m) => m.t === 'history')).messages.find((m) => m.id === echo.id)?.reply?.id, replyTarget.id);
  a.send({ t: 'channel.create', name: `other-${RUN}`, kind: 'text' });
  const { channel: other } = await b.next((m) => m.t === 'channel' && m.channel.name === `other-${RUN}`);
  b.send({ t: 'send', channelId: other.id, body: 'wrong room', replyTo: replyTarget.id });
  assert.equal((await b.next((m) => m.t === 'error' && m.re === 'send')).code, 'no_message');
  a.send({ t: 'channel.archive', channelId: other.id });
  // Save to Personal: a note in your Personal space may point at any message you can read.
  const personal = `p_${b.uid}`;
  b.send({ t: 'send', channelId: personal, body: '', replyTo: replyTarget.id, nonce: 'save1' });
  const saved = (await b.next((m) => m.t === 'message' && m.nonce === 'save1')).message;
  assert.deepEqual([saved.channelId, saved.body, saved.reply.channelId, saved.reply.id], [personal, '', 'general', replyTarget.id]);
  b.send({ t: 'send', channelId: personal, body: '' });
  assert.equal((await b.next((m) => m.t === 'error' && m.re === 'send')).code, 'empty', 'an empty note needs something to point at');
  b.send({ t: 'send', channelId: `p_${a.uid}`, body: 'x' });
  assert.equal((await b.next((m) => m.t === 'error' && m.re === 'send')).code, 'no_channel');
});

await test('deleting erases: text, edits, reactions and the picture are gone; replies say deleted', async () => {
  const up = await (await fetch(`${BASE}/api/image?sid=${a.ready.sid}&ut=${a.ready.uploadToken}`, { method: 'POST', headers: { 'X-Yap-Key': KEY, 'Content-Type': 'image/png' }, body: png() })).json();
  a.send({ t: 'send', channelId: 'general', body: 'secret plans v1', image: { id: up.id, width: 10, height: 10 }, nonce: 'del1' });
  const m = (await a.next((x) => x.t === 'message' && x.nonce === 'del1')).message;
  a.send({ t: 'edit', id: m.id, body: 'secret plans v2' });
  await a.next((x) => x.t === 'edited' && x.id === m.id);
  b.send({ t: 'react', id: m.id, emoji: '👍', on: true });
  await a.next((x) => x.t === 'reaction' && x.messageId === m.id);
  b.send({ t: 'send', channelId: 'general', body: 'nice', replyTo: m.id, nonce: 'del2' });
  const reply = (await b.next((x) => x.t === 'message' && x.nonce === 'del2')).message;
  assert.equal((await fetch(`${BASE}/media/${up.id}`)).status, 200);
  a.send({ t: 'delete', id: m.id });
  await b.next((x) => x.t === 'deleted' && x.id === m.id);
  if (ADMIN) {
    const dump = await (await admin('/api/admin/export?after=0')).json();
    const row = dump.messages.find((x) => x.id === m.id);
    assert.ok(row.deletedAt, 'tombstone stays');
    assert.equal(row.body, '');
    assert.equal(row.image, undefined);
    assert.ok(!JSON.stringify(dump).includes('secret plans'), 'no copy of the text anywhere (edits included)');
    assert.ok(!dump.reactions.some((r) => r.message_id === m.id), 'reactions gone');
  }
  // (A fresh URL skips the edge cache, which keeps a picture at most a day.)
  let status = 200;
  for (let i = 0; i < 20 && status === 200; i++) {
    await new Promise((r) => setTimeout(r, 100));
    status = (await fetch(`${BASE}/media/${up.id}?fresh=${rid()}`)).status;
  }
  assert.equal(status, 404, 'the picture bytes are erased too');
  b.send({ t: 'history', channelId: 'general', limit: 5 });
  assert.deepEqual((await b.next((x) => x.t === 'history')).messages.find((x) => x.id === reply.id).reply, { id: m.id, uid: a.uid, channelId: 'general', body: '', deleted: true });
});

await test("when's good: set your times, everyone hears; out-of-window slots dropped; empty clears", async () => {
  const now = Math.floor(Date.now() / 1_800_000);
  a.send({ t: 'when.set', slots: [now + 2, now + 3, now + 3, now - 24 * 2 * 30, 'x', now + 2 * 24 * 40] });
  const heard = await b.next((m) => m.t === 'when' && m.uid === a.uid);
  assert.deepEqual(heard.slots, [now + 2, now + 3], 'deduped, sorted, only this week-ish');
  b.send({ t: 'when.get' });
  const all = await b.next((m) => m.t === 'when.all');
  assert.deepEqual(all.people.find((p) => p.uid === a.uid)?.slots, [now + 2, now + 3]);
  a.send({ t: 'when.set', slots: [] });
  assert.deepEqual((await b.next((m) => m.t === 'when' && m.uid === a.uid)).slots, []);
  b.send({ t: 'when.get' });
  assert.ok(!(await b.next((m) => m.t === 'when.all')).people.some((p) => p.uid === a.uid));
});

await test('encrypted Personal space: wrap shape checked, own devices hear it, long sealed notes fit, others never see', async () => {
  const personal = `p_${a.uid}`;
  a.send({ t: 'vault.set', wrap: { v: 1, iter: 600000, salt: 'short', iv: 'x', ct: 'y' } });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'vault.set')).code, 'bad_vault');
  const b64 = (n) => Buffer.from(crypto.getRandomValues(new Uint8Array(Math.min(n, 60_000)))).toString('base64url').repeat(Math.ceil(n / 60_000));
  const wrap = { v: 1, iter: 600000, salt: b64(16), iv: b64(12), ct: b64(48) };
  a.send({ t: 'vault.set', wrap });
  assert.deepEqual((await a.next((m) => m.t === 'vault')).wrap, wrap);
  await assert.rejects(b.next((m) => m.t === 'vault', 300), 'nobody else hears about it');
  const sealed = `e1:${b64(90_000)}`;
  a.send({ t: 'send', channelId: personal, body: sealed, nonce: 'seal1' });
  assert.equal((await a.next((m) => m.t === 'message' && m.nonce === 'seal1')).message.body, sealed);
  a.send({ t: 'send', channelId: personal, body: 'x'.repeat(40_001) });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'send')).code, 'too_long', 'plain notes keep the normal limit');
  await assert.rejects(b.next((m) => m.t === 'message' && m.message.channelId === personal, 300));
  // Sealing notes written before encryption: the plain copy (and its picture, and old versions) is erased.
  const up = await (await fetch(`${BASE}/api/image?sid=${a.ready.sid}&ut=${a.ready.uploadToken}`, { method: 'POST', headers: { 'X-Yap-Key': KEY, 'Content-Type': 'image/png' }, body: png() })).json();
  a.send({ t: 'send', channelId: personal, body: 'plain old note', image: { id: up.id, width: 4, height: 4 }, nonce: 'plain1' });
  const plain = (await a.next((m) => m.t === 'message' && m.nonce === 'plain1')).message;
  a.send({ t: 'edit', id: plain.id, body: 'plain old note v2' });
  await a.next((m) => m.t === 'edited' && m.id === plain.id);
  a.send({ t: 'personal.plain' });
  const list = await a.next((m) => m.t === 'personal.plain');
  assert.deepEqual(list.notes.find((n) => n.id === plain.id), { id: plain.id, body: 'plain old note v2', image: { id: up.id, width: 4, height: 4 } });
  assert.ok(!list.notes.some((n) => n.body.startsWith('e1:')), 'sealed notes are not listed');
  a.send({ t: 'personal.seal', notes: [{ id: plain.id, body: 'not sealed' }] });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'personal.seal')).code, 'bad_seal');
  const sealedBody = `e1:${b64(300)}`;
  a.send({ t: 'personal.seal', notes: [{ id: plain.id, body: sealedBody }, { id: replyTarget.id, body: sealedBody }] });
  const swapped = await a.next((m) => m.t === 'edited' && m.id === plain.id);
  assert.deepEqual([swapped.body, swapped.sealed], [sealedBody, true]);
  if (ADMIN) {
    const dump = await (await admin('/api/admin/export?after=0')).json();
    const row = dump.messages.find((m) => m.id === plain.id);
    assert.equal(row.image, undefined, 'the picture moved inside the note');
    assert.ok(!JSON.stringify(dump).includes('plain old note'), 'no plain copy left, edits included');
    assert.notEqual(dump.messages.find((m) => m.id === replyTarget.id).body, sealedBody, 'only your own Personal space can be sealed');
  }
  let status = 200;
  for (let i = 0; i < 20 && status === 200; i++) {
    await new Promise((r) => setTimeout(r, 100));
    status = (await fetch(`${BASE}/media/${up.id}?fresh=${rid()}`)).status;
  }
  assert.equal(status, 404, 'the plain picture is erased');
  await assert.rejects(b.next((m) => m.t === 'edited' && m.id === plain.id, 300));
  a.send({ t: 'vault.set', wrap: null });
  assert.equal((await a.next((m) => m.t === 'vault')).wrap, null);
});

await test('search: words, from:, has:, one channel or everywhere; encrypted notes never match', async () => {
  const tag = `zebra${RUN}`;
  a.send({ t: 'send', channelId: 'general', body: `the ${tag} crossing at noon`, nonce: 's1' });
  const hit = (await a.next((m) => m.t === 'message' && m.nonce === 's1')).message;
  b.send({ t: 'send', channelId: 'general', body: `another ${tag} sighting`, nonce: 's2' });
  await b.next((m) => m.t === 'message' && m.nonce === 's2');
  a.send({ t: 'send', channelId: `p_${a.uid}`, body: `private ${tag} note`, nonce: 's3' });
  await a.next((m) => m.t === 'message' && m.nonce === 's3');
  await new Promise((r) => setTimeout(r, 2100)); // search limit: 1 per 2 s after a burst
  a.send({ t: 'search', q: tag });
  let res = await a.next((m) => m.t === 'search.results');
  assert.equal(res.messages.length, 3, 'everywhere includes your own Personal space');
  assert.ok(res.messages[0].ts >= res.messages[1].ts, 'newest first');
  b.send({ t: 'search', q: tag });
  assert.equal((await b.next((m) => m.t === 'search.results')).messages.length, 2, 'never someone else’s Personal space');
  a.send({ t: 'search', q: `${tag} crossing` });
  res = await a.next((m) => m.t === 'search.results');
  assert.deepEqual(res.messages.map((m) => m.id), [hit.id], 'every word must match');
  a.send({ t: 'search', q: `${tag} from:${BOB.slice(0, 5)}` });
  assert.ok((await a.next((m) => m.t === 'search.results')).messages.every((m) => m.uid === b.uid));
  a.send({ t: 'search', q: `${tag}`, channelId: 'general' });
  assert.equal((await a.next((m) => m.t === 'search.results')).messages.length, 2);
  a.send({ t: 'search', q: '100%_' });
  assert.ok(Array.isArray((await a.next((m) => m.t === 'search.results')).messages), 'LIKE wildcards are escaped');
  a.send({ t: 'search', q: tag, channelId: `p_${b.uid}` });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'search')).code, 'no_channel');
});

await test('pins: pin/unpin reaches everyone, the list shows them, history carries pinnedAt, deleting unpins', async () => {
  a.send({ t: 'send', channelId: 'general', body: `pin me ${RUN}`, nonce: 'p1' });
  const m = (await a.next((x) => x.t === 'message' && x.nonce === 'p1')).message;
  b.send({ t: 'pin', id: m.id, on: true });
  const pinned = await a.next((x) => x.t === 'pinned' && x.id === m.id);
  await b.next((x) => x.t === 'pinned' && x.id === m.id && x.pinnedAt);
  assert.ok(pinned.pinnedAt > 0);
  assert.equal(pinned.by, b.uid);
  a.send({ t: 'pins', channelId: 'general' });
  const list = await a.next((x) => x.t === 'pins');
  assert.equal(list.messages[0].id, m.id);
  assert.equal(list.messages[0].pinnedAt, pinned.pinnedAt);
  b.send({ t: 'history', channelId: 'general', limit: 5 });
  assert.ok((await b.next((x) => x.t === 'history')).messages.find((x) => x.id === m.id).pinnedAt);
  a.send({ t: 'pin', id: m.id, on: false });
  await b.next((x) => x.t === 'pinned' && x.id === m.id && x.pinnedAt === null);
  a.send({ t: 'pin', id: m.id, on: true });
  await b.next((x) => x.t === 'pinned' && x.id === m.id && x.pinnedAt);
  a.send({ t: 'delete', id: m.id });
  await b.next((x) => x.t === 'deleted' && x.id === m.id);
  a.send({ t: 'pins', channelId: 'general' });
  assert.ok(!(await a.next((x) => x.t === 'pins')).messages.some((x) => x.id === m.id), 'deleted messages are unpinned');
  a.send({ t: 'pins', channelId: `p_${b.uid}` });
  assert.equal((await a.next((x) => x.t === 'error' && x.re === 'pins')).code, 'no_channel');
});

await test('draw.gallery: just the saved drawings, no strokes', async () => {
  a.send({ t: 'draw.gallery' });
  const g = await a.next((m) => m.t === 'draw.gallery');
  assert.ok(Array.isArray(g.gallery));
  assert.equal(g.strokes, undefined);
});

await test('space.game: relayed to others on Space only, never stored, size-capped', async () => {
  a.send({ t: 'view', view: 'space' });
  b.send({ t: 'view', view: 'space' });
  await a.next((m) => m.t === 'view' && m.sid === b.ready.sid && m.view === 'space');
  a.send({ t: 'space.game', g: 'ball', d: { x: 10, y: 20, vx: 1 } });
  const got = await b.next((m) => m.t === 'space.game');
  assert.deepEqual([got.sid, got.g, got.d], [a.ready.sid, 'ball', { x: 10, y: 20, vx: 1 }]);
  await assert.rejects(a.next((m) => m.t === 'space.game', 300), 'not echoed to the sender');
  a.send({ t: 'space.game', g: 'Bad Name', d: 1 });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'space.game')).code, 'bad_game');
  a.send({ t: 'space.game', g: 'ball', d: 'x'.repeat(600) });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'space.game')).code, 'bad_game');
  b.send({ t: 'view', view: null });
  a.send({ t: 'view', view: null });
  await b.next((m) => m.t === 'view' && m.sid === a.ready.sid && m.view === null);
});

await test('screen share flag: rides on voice state, only inside voice, gone when you leave', async () => {
  a.send({ t: 'voice.join', channelId: 'lounge' });
  await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid && m.channelId === 'lounge');
  a.send({ t: 'voice.state', muted: false, deafened: false, screen: true });
  const on = await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid && m.screen);
  assert.equal(on.channelId, 'lounge');
  b.send({ t: 'voice.join', channelId: 'lounge' });
  const ready = await b.next((m) => m.t === 'voice' && m.sid === b.ready.sid);
  assert.equal(ready.channelId, 'lounge');
  a.send({ t: 'voice.leave' });
  const off = await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid && m.channelId === null);
  assert.equal(off.screen, undefined);
  b.send({ t: 'voice.leave' });
  await b.next((m) => m.t === 'voice' && m.sid === b.ready.sid && m.channelId === null);
  // Leave no stray voice/session events behind for later tests.
  await new Promise((r) => setTimeout(r, 300));
  const about = (m, sid) => (m.sid === sid || m.session?.sid === sid) && /^(voice|session\.)/.test(m.t);
  a.inbox = a.inbox.filter((m) => !about(m, b.ready.sid) && !about(m, a.ready.sid));
  b.inbox = b.inbox.filter((m) => !about(m, a.ready.sid) && !about(m, b.ready.sid));
});

await test('watch together: audience is visible, one shared player, queue, end → next (once)', async () => {
  a.send({ t: 'watch.join' });
  const start = (await a.next((m) => m.t === 'watch.state')).state;
  assert.equal(start.videoId !== undefined, true);
  // A server that kept its data may have videos queued from before: start from an empty queue.
  for (let i = 0; i < start.queue.length; i++) a.send({ t: 'watch.queue', remove: 0 });
  if (start.queue.length) await a.next((m) => m.t === 'watch.state' && m.state.queue.length === 0);
  for (const c of [a, b]) c.inbox = c.inbox.filter((m) => m.t !== 'watch.state');
  const joined = await b.next((m) => m.t === 'watching' && m.sid === a.ready.sid);
  assert.equal(joined.on, true);
  a.send({ t: 'watch.set', videoId: 'not-an-id' });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'watch.set')).code, 'bad_video');
  a.send({ t: 'watch.set', videoId: 'dQw4w9WgXcQ' });
  let st = await b.next((m) => m.t === 'watch.state' && m.state.videoId === 'dQw4w9WgXcQ', 10000);
  assert.equal(st.state.playing, true);
  assert.ok(Math.abs(st.now - st.state.at) < 5000, 'server clock in every update');
  b.send({ t: 'watch.set', playing: false, position: 42 });
  st = await a.next((m) => m.t === 'watch.state' && m.state.playing === false);
  assert.deepEqual([st.state.position, st.state.by], [42, b.uid]);
  b.send({ t: 'watch.queue', add: 'aaaaaaaaaaa' });
  b.send({ t: 'watch.queue', add: 'bbbbbbbbbbb' }); // right behind it: neither may be lost
  st = await a.next((m) => m.t === 'watch.state' && m.state.queue.length === 2, 15000);
  assert.deepEqual(st.state.queue.map((q) => q.id).sort(), ['aaaaaaaaaaa', 'bbbbbbbbbbb']);
  b.send({ t: 'watch.queue', remove: st.state.queue.findIndex((q) => q.id === 'bbbbbbbbbbb') });
  st = await a.next((m) => m.t === 'watch.state' && m.state.queue.length === 1);
  assert.equal(st.state.queue[0].id, 'aaaaaaaaaaa');
  a.send({ t: 'watch.next', expect: 'dQw4w9WgXcQ' });
  b.send({ t: 'watch.next', expect: 'dQw4w9WgXcQ' });
  st = await a.next((m) => m.t === 'watch.state' && m.state.videoId === 'aaaaaaaaaaa');
  assert.deepEqual([st.state.queue.length, st.state.position, st.state.playing], [0, 0, true]);
  await assert.rejects(a.next((m) => m.t === 'watch.state' && m.state.videoId === null, 400), 'the second "ended" report is ignored');
  a.send({ t: 'watch.next', expect: 'aaaaaaaaaaa' });
  assert.equal((await b.next((m) => m.t === 'watch.state' && m.state.videoId === null)).state.playing, false, 'queue empty: stops');
  a.send({ t: 'watch.leave' });
  assert.equal((await b.next((m) => m.t === 'watching' && m.sid === a.ready.sid)).on, false);
  await b.next((m) => m.t === 'session.leave' && m.sid === a.ready.sid);
});

await test('watch together: two quick adds with nothing on: one plays, the other waits in the queue', async () => {
  a.send({ t: 'watch.queue', add: 'ccccccccccc' });
  a.send({ t: 'watch.queue', add: 'ddddddddddd' }); // right behind it, both looking up titles at once
  const ours = ['ccccccccccc', 'ddddddddddd'];
  const st = await b.next((m) => m.t === 'watch.state' && ours.includes(m.state.videoId) && m.state.queue.length === 1, 15000);
  assert.deepEqual([st.state.videoId, st.state.queue[0].id].sort(), ['ccccccccccc', 'ddddddddddd'], 'neither is lost');
  a.send({ t: 'watch.next', expect: st.state.videoId });
  await b.next((m) => m.t === 'watch.state' && m.state.videoId === st.state.queue[0].id);
  a.send({ t: 'watch.next', expect: st.state.queue[0].id });
  await b.next((m) => m.t === 'watch.state' && m.state.videoId === null);
});

await test('a sign-up with a secret that is neither a PIN nor a password says what would work', async () => {
  const c = await new Client(`Shorty ${RUN}`).open();
  c.send({ t: 'hello', v: 1, signup: { name: `Shorty ${RUN}`, pin: '1234', color: '#ff8800' }, client: 'test' });
  const err = await c.next((m) => m.t === 'error');
  assert.equal(err.code, 'bad_pin');
  assert.match(err.message, /6 digits.*password/, 'passwords work too');
  c.close();
});

await test('weak PINs are refused (repeats, runs, patterns, dates); passwords work instead', async () => {
  for (const pin of ['424242', '123456', '111111', '123321', '251299', '159753']) {
    const c = await new Client(`Weak ${pin} ${RUN}`).open();
    c.send({ t: 'hello', v: 1, signup: { name: `Weak ${pin} ${RUN}`, pin, color: '#ff8800' }, client: 'test' });
    assert.equal((await c.next((m) => m.t === 'error')).code, 'weak_pin', pin);
    c.close();
  }
  const p = await new Client(`Pass ${RUN}`).open();
  await p.hello();
  p.send({ t: 'account.pin', pin: '111111', current: TEST_PIN });
  assert.equal((await p.next((m) => m.t === 'error' && m.re === 'account.pin')).code, 'weak_pin');
  p.send({ t: 'account.pin', pin: 'short pw', current: TEST_PIN });
  assert.equal((await p.next((m) => m.t === 'error' && m.re === 'account.pin')).code, 'bad_pin');
  p.send({ t: 'account.pin', pin: 'purple tiger river 7', current: TEST_PIN });
  assert.equal((await p.next((m) => m.t === 'account')).kind, 'password');
  const again = await new Client(`Pass ${RUN} again`).open();
  again.send({ t: 'hello', v: 1, login: { name: `Pass ${RUN}`, pin: TEST_PIN }, client: 'test' });
  assert.equal((await again.next((m) => m.t === 'error')).code, 'bad_login', 'the old PIN is gone');
  again.send({ t: 'hello', v: 1, login: { name: `Pass ${RUN}`, pin: 'purple tiger river 7' }, client: 'test' });
  const ready = await again.next((m) => m.t === 'ready');
  assert.equal(ready.secretKind, 'password');
  assert.equal(ready.weakPin, undefined);
  again.close();
  p.close();
});

await test('your devices: a new sign-in alerts your other devices; list, sign one out, sign out everywhere else', async () => {
  const name = `Devices ${RUN}`;
  const first = await new Client(name).open();
  await first.hello();
  const login = async () => {
    const c = await new Client(`${name} (2)`).open();
    c.send({ t: 'hello', v: 1, login: { name, pin: TEST_PIN }, client: 'web' });
    c.ready = await c.next((m) => m.t === 'ready');
    return c;
  };
  const second = await login();
  const alert = await first.next((m) => m.t === 'signin.alert');
  assert.equal(alert.device.client, 'web');
  assert.match(alert.device.id, /^[0-9a-f]{16}$/);
  await assert.rejects(second.next((m) => m.t === 'signin.alert', 300), 'the new device itself is not alerted');
  first.send({ t: 'account.devices' });
  const { devices } = await first.next((m) => m.t === 'devices');
  assert.equal(devices.length, 2);
  assert.equal(devices.filter((d) => d.current).length, 1);
  const other = devices.find((d) => !d.current);
  assert.equal(other.id, alert.device.id);
  const closed = new Promise((r) => second.ws.addEventListener('close', (e) => r(e.code), { once: true }));
  first.send({ t: 'account.revoke', id: other.id });
  assert.equal((await first.next((m) => m.t === 'devices')).devices.length, 1);
  assert.equal(await closed, 4001, 'its socket is closed');
  first.send({ t: 'account.revoke', id: devices.find((d) => d.current).id });
  assert.equal((await first.next((m) => m.t === 'error' && m.re === 'account.revoke')).code, 'no_device', 'not this device (that is Sign out)');
  const third = await login();
  await first.next((m) => m.t === 'signin.alert');
  const closed3 = new Promise((r) => third.ws.addEventListener('close', (e) => r(e.code), { once: true }));
  first.send({ t: 'account.signoutOthers' });
  assert.equal((await first.next((m) => m.t === 'devices')).devices.length, 1);
  assert.equal(await closed3, 4001);
  first.send({ t: 'history', channelId: 'general', limit: 1 });
  await first.next((m) => m.t === 'history');
  first.close();
});

if (ADMIN) {
  await test('admin: add channels by name (skips ones that exist)', async () => {
    const names = [`games-${RUN}`, `clips-${RUN}`];
    const first = await (await admin(`/api/admin/channels?names=${names.join(',')}`, { method: 'POST' })).json();
    assert.deepEqual(first.added, names);
    const made = [];
    for (const n of names) made.push((await b.next((m) => m.t === 'channel' && m.channel.name === n)).channel);
    assert.ok(made.every((c) => c.kind === 'text') && made[1].position > made[0].position, 'text channels, in order');
    const again = await (await admin(`/api/admin/channels?names=${names.join(',')}`, { method: 'POST' })).json();
    assert.deepEqual(again.added, []);
    const gone = await (await admin(`/api/admin/channels?archive=${names.join(',')},no-such-${RUN}`, { method: 'POST' })).json();
    assert.deepEqual(gone.archived, names, 'archived by name, unknown names skipped');
    for (const c of made) assert.equal((await b.next((m) => m.t === 'channel.archived' && m.channelId === c.id)).channelId, c.id);
    const back = await (await admin(`/api/admin/channels?unarchive=${names[0]}`, { method: 'POST' })).json();
    assert.deepEqual(back.restored, [names[0]]);
    assert.equal((await b.next((m) => m.t === 'channel' && m.channel.id === made[0].id)).channel.name, names[0], 'back for everyone, same id (history intact)');
    await admin(`/api/admin/channels?archive=${names[0]}`, { method: 'POST' });
  });
}

await test('drawing: shapes (straight segments) and text strokes are kept and relayed', async () => {
  a.send({ t: 'view', view: 'draw' });
  b.send({ t: 'view', view: 'draw' });
  await new Promise((r) => setTimeout(r, 200));
  a.send({ t: 'draw.stroke', sk: `box${rid()}`, c: '#ff0000', w: 32, e: false, p: [100, 100, 900, 100, 900, 600, 100, 600, 100, 100], k: 'poly' });
  const box = await b.next((m) => m.t === 'draw.stroke' && m.stroke.k === 'poly');
  assert.equal(box.stroke.p.length, 10);
  a.send({ t: 'draw.stroke', sk: `txt${rid()}`, c: '#0000ff', w: 32, e: false, p: [500, 800], k: 'text', text: 'hello <b>' });
  const txt = await b.next((m) => m.t === 'draw.stroke' && m.stroke.k === 'text');
  assert.equal(txt.stroke.text, 'hello <b>');
  a.send({ t: 'draw.stroke', sk: `bad${rid()}`, c: '#0000ff', w: 32, e: false, p: [1, 2, 3, 4], k: 'text', text: 'two points' });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'draw.stroke')).code, 'bad_stroke');
  b.send({ t: 'draw.state' });
  const st = await b.next((m) => m.t === 'draw.state');
  assert.ok(st.strokes.some((s) => s.k === 'text' && s.text === 'hello <b>'), 'text survives a reload');
  a.send({ t: 'view', view: null });
  b.send({ t: 'view', view: null });
});

await test('drawing: live pieces + strokes reach others; undo is yours; clear starts fresh; saves go to the gallery', async () => {
  a.send({ t: 'view', view: 'draw' });
  b.send({ t: 'view', view: 'draw' });
  await b.next((m) => m.t === 'view' && m.sid === a.ready.sid && m.view === 'draw');
  await a.next((m) => m.t === 'view' && m.sid === b.ready.sid && m.view === 'draw');
  a.send({ t: 'draw.seg', sk: `stroke_${RUN}`, c: '#E5484D', w: 30, e: false, p: [100, 100, 200, 200] });
  assert.deepEqual((await b.next((m) => m.t === 'draw.seg')).p, [100, 100, 200, 200]);
  a.send({ t: 'draw.stroke', sk: `stroke_${RUN}`, c: '#E5484D', w: 30, e: false, p: [100, 100, 200, 200, 300, 250] });
  const st = (await b.next((m) => m.t === 'draw.stroke')).stroke;
  assert.deepEqual([st.id, st.uid, st.c, st.p.length], [`stroke_${RUN}`, a.uid, '#e5484d', 6]);
  a.send({ t: 'draw.stroke', sk: 'bad', c: 'red', w: 30, e: false, p: [1] });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'bad_stroke');
  b.send({ t: 'draw.state' });
  const state = await b.next((m) => m.t === 'draw.state');
  assert.ok(state.strokes.some((x) => x.id === `stroke_${RUN}`));
  b.send({ t: 'draw.undo', id: `stroke_${RUN}` });
  assert.equal((await b.next((m) => m.t === 'error')).code, 'forbidden');
  a.send({ t: 'draw.undo', id: `stroke_${RUN}` });
  assert.equal((await b.next((m) => m.t === 'draw.undo')).id, `stroke_${RUN}`);
  const save = await fetch(`${BASE}/api/drawing?${new URLSearchParams({ key: KEY, sid: b.ready.sid, ut: b.ready.uploadToken })}`, { method: 'POST', body: png() });
  assert.equal(save.status, 200, await save.clone().text());
  assert.equal((await a.next((m) => m.t === 'drawing')).drawing.uid, b.uid);
  a.send({ t: 'draw.clear' });
  const cleared = await b.next((m) => m.t === 'draw.clear');
  assert.ok(cleared.canvas > state.canvas);
  b.send({ t: 'draw.state' });
  const fresh = await b.next((m) => m.t === 'draw.state');
  assert.equal(fresh.strokes.length, 0);
  assert.ok(fresh.gallery.length >= 1);
});

await test('drawing: layers travel with strokes; a wipe empties one layer for everyone', async () => {
  // Both are still on the draw view from the test before (view changes are rate-limited).
  const bottom = `bot${rid()}`;
  const top = `top${rid()}`;
  a.send({ t: 'draw.seg', sk: top, c: '#000000', w: 14, e: false, p: [10, 10, 20, 20], l: 2 });
  assert.equal((await b.next((m) => m.t === 'draw.seg' && m.sk === top)).l, 2);
  a.send({ t: 'draw.stroke', sk: top, c: '#000000', w: 14, e: false, p: [10, 10, 20, 20], l: 2 });
  assert.equal((await b.next((m) => m.t === 'draw.stroke' && m.stroke.id === top)).stroke.l, 2);
  // Layer 0 is the default: it isn't written out.
  a.send({ t: 'draw.stroke', sk: bottom, c: '#000000', w: 14, e: false, p: [30, 30, 40, 40], l: 0 });
  assert.equal('l' in (await b.next((m) => m.t === 'draw.stroke' && m.stroke.id === bottom)).stroke, false);
  a.send({ t: 'draw.stroke', sk: `bad${rid()}`, c: '#000000', w: 14, e: false, p: [1, 2], l: 3 });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'draw.stroke')).code, 'bad_stroke');
  a.send({ t: 'draw.wipe', l: 7 });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'draw.wipe')).code, 'bad_layer');
  b.send({ t: 'draw.wipe', l: 2 });
  assert.deepEqual(await a.next((m) => m.t === 'draw.wipe'), { t: 'draw.wipe', l: 2, by: b.uid });
  b.send({ t: 'draw.state' });
  const st = await b.next((m) => m.t === 'draw.state');
  assert.equal(st.strokes.some((s) => s.id === top), false, 'the top layer is empty');
  assert.ok(st.strokes.some((s) => s.id === bottom), 'the bottom layer is untouched');
});

await test('space: moves reach others on the space view; newcomers get everyone’s spot', async () => {
  a.send({ t: 'view', view: 'space' });
  b.send({ t: 'view', view: 'space' });
  await a.next((m) => m.t === 'view' && m.sid === b.ready.sid && m.view === 'space');
  a.send({ t: 'space.move', x: 640.4, y: 99999, d: 1.5 });
  const pos = await b.next((m) => m.t === 'space.pos');
  assert.deepEqual([pos.sid, pos.x, pos.y, pos.d], [a.ready.sid, 640, 1600, 1.5]);
  b.send({ t: 'space.state' });
  const st = await b.next((m) => m.t === 'space.state');
  assert.ok(st.players.some((p) => p.sid === a.ready.sid && p.x === 640));
  a.send({ t: 'view', view: null });
  b.send({ t: 'view', view: null });
});

// ── Reactions, Space memory, security ──────────────────────────────────
await test('reactions: add, see who, remove; only real emojis', async () => {
  a.send({ t: 'send', channelId: 'general', body: 'react to me', nonce: 'rx' });
  const m = (await a.next((x) => x.t === 'message' && x.nonce === 'rx')).message;
  b.send({ t: 'react', id: m.id, emoji: '🔥', on: true });
  const r = await a.next((x) => x.t === 'reaction' && x.messageId === m.id);
  assert.deepEqual([r.emoji, r.uid, r.on, r.channelId], ['🔥', b.uid, true, 'general']);
  a.send({ t: 'react', id: m.id, emoji: `<:${emojiName}:${emoji.id}>`, on: true });
  await b.next((x) => x.t === 'reaction' && x.uid === a.uid);
  a.send({ t: 'history', channelId: 'general', limit: 3 });
  const h = await a.next((x) => x.t === 'history');
  const got = h.messages.find((x) => x.id === m.id).reactions;
  assert.deepEqual(got.map((x) => [x.emoji, x.uids]), [['🔥', [b.uid]], [`<:${emojiName}:${emoji.id}>`, [a.uid]]]);
  b.send({ t: 'react', id: m.id, emoji: '🔥', on: false });
  assert.equal((await a.next((x) => x.t === 'reaction' && x.uid === b.uid)).on, false);
  for (const bad of ['lol', '<img src=x>', '🔥'.repeat(20)]) {
    b.send({ t: 'react', id: m.id, emoji: bad, on: true });
    assert.equal((await b.next((x) => x.t === 'error')).code, 'bad_emoji', bad);
  }
});

await test('space emotes: relayed to everyone on Space; only the set ones', async () => {
  a.send({ t: 'view', view: 'space' });
  b.send({ t: 'view', view: 'space' });
  await new Promise((r) => setTimeout(r, 200));
  a.send({ t: 'space.emote', e: '🎉' });
  assert.equal((await b.next((m) => m.t === 'space.emote')).e, '🎉');
  a.send({ t: 'space.emote', e: '💩' });
  assert.equal((await a.next((m) => m.t === 'error' && m.re === 'space.emote')).code, 'bad_emoji');
  a.send({ t: 'view', view: null });
  b.send({ t: 'view', view: null });
});

await test('space remembers where you were and what you look like', async () => {
  a.send({ t: 'view', view: 'space' });
  a.send({ t: 'space.move', x: 777, y: 555, d: 0.5 });
  a.send({ t: 'profile', look: '🐸' });
  assert.equal((await b.next((m) => m.t === 'user' && m.user.id === a.uid && m.user.look === '🐸')).user.look, '🐸');
  a.send({ t: 'profile', look: 'not an emoji' });
  assert.equal((await a.next((m) => m.t === 'error')).code, 'bad_emoji');
  a.send({ t: 'view', view: null });
  await new Promise((r) => setTimeout(r, 300));
  const a3 = new Client(ALICE);
  a3.token = a.token;
  await a3.open();
  await a3.hello();
  a3.send({ t: 'view', view: 'space' });
  a3.send({ t: 'space.state' });
  const st = await a3.next((m) => m.t === 'space.state');
  assert.deepEqual(st.me, { x: 777, y: 555, d: 0.5 });
  a3.close();
});

await test('security: key in header works, key in URL still works, headers lock the page down', async () => {
  const config = await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': KEY } });
  assert.equal(config.status, 200);
  assert.ok((await config.json()).links.some((l) => l.view === 'board'));
  if (KEY) assert.equal((await fetch(`${BASE}/api/config`, { headers: { 'X-Yap-Key': 'nope' } })).status, 401);
  const legacy = await new Client(`Query ${RUN}`).open(KEY, { query: true });
  legacy.close();
  const page = await fetch(`${BASE}/`);
  // Only Yap's own scripts; WebAssembly may compile (RNNoise), but no inline scripts and no eval.
  const pageCsp = page.headers.get('content-security-policy') ?? '';
  assert.match(pageCsp, /script-src 'self'( 'wasm-unsafe-eval')?(;|$)/);
  assert.doesNotMatch(pageCsp.match(/script-src[^;]*/)[0], /'unsafe-inline'|'unsafe-eval'/);
  assert.equal(page.headers.get('x-content-type-options'), 'nosniff');
  // Permissions-Policy: what Yap's features need (voice, screen sharing) and nothing more.
  const policy = Object.fromEntries((page.headers.get('permissions-policy') ?? '').split(',').map((p) => p.trim().split('=')));
  assert.deepEqual([policy.microphone, policy['display-capture']], ['(self)', '(self)'], 'voice and screen sharing allowed');
  for (const off of ['camera', 'geolocation', 'payment', 'usb']) assert.equal(policy[off], '()', `${off} off`);
  const board = await fetch(`${BASE}/board`);
  const csp = board.headers.get('content-security-policy') ?? '';
  assert.match(csp, /script-src 'self' 'sha256-/);
  assert.doesNotMatch(csp, /unsafe-inline'[^;]*script|script-src[^;]*unsafe-inline/);
});

// ── Voice signaling ─────────────────────────────────────────────────────
await test('voice join/state/leave are broadcast to everyone', async () => {
  a.send({ t: 'voice.join', channelId: 'lounge' });
  const self = await a.next((m) => m.t === 'voice' && m.sid === a.ready.sid);
  assert.equal(self.channelId, 'lounge');
  const seen = await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid);
  assert.equal(seen.channelId, 'lounge');
  a.send({ t: 'voice.state', muted: true, deafened: false });
  assert.equal((await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid)).muted, true);
  b.send({ t: 'voice.join', channelId: 'text-is-not-voice' });
  assert.equal((await b.next((m) => m.t === 'error')).code, 'no_channel');
});

await test('mute/deafen from someone not in voice is ignored (it would reveal their session)', async () => {
  const aboutB = (m) => (m.t === 'voice' && m.sid === b.ready.sid) || (m.t === 'session.join' && m.session.sid === b.ready.sid);
  a.inbox = a.inbox.filter((m) => !aboutB(m));
  b.send({ t: 'voice.state', muted: true, deafened: true });
  await assert.rejects(a.next(aboutB, 400), 'nothing is announced');
});

await test('signals are relayed to exactly the target session', async () => {
  b.send({ t: 'voice.join', channelId: 'lounge' });
  await a.next((m) => m.t === 'voice' && m.sid === b.ready.sid);
  b.send({ t: 'signal', to: a.ready.sid, data: { sdp: { type: 'offer', sdp: 'v=0' } } });
  const s = await a.next((m) => m.t === 'signal');
  assert.equal(s.from, b.ready.sid);
  assert.deepEqual(s.data, { sdp: { type: 'offer', sdp: 'v=0' } });
  b.send({ t: 'signal', to: 'nobody', data: {} });
  assert.equal((await b.next((m) => m.t === 'error')).code, 'no_peer');
});

await test('signals only reach someone in your voice channel', async () => {
  const c = await new Client(`Caller ${RUN}`).open();
  await c.hello();
  c.send({ t: 'signal', to: a.ready.sid, data: { sdp: { type: 'offer', sdp: 'v=0' } } });
  assert.equal((await c.next((m) => m.t === 'error' && m.re === 'signal')).code, 'no_peer', 'not in voice');
  c.send({ t: 'voice.join', channelId: 'gaming' });
  await c.next((m) => m.t === 'voice' && m.sid === c.ready.sid && m.channelId === 'gaming');
  c.send({ t: 'signal', to: a.ready.sid, data: { candidate: {} } });
  assert.equal((await c.next((m) => m.t === 'error' && m.re === 'signal')).code, 'no_peer', 'another channel');
  await assert.rejects(a.next((m) => m.t === 'signal' && m.from === c.ready.sid, 300), 'nothing got through');
  c.close();
});

await test('8 people joining a call at once: every offer, answer and ICE candidate gets through', async () => {
  const people = [];
  for (let i = 0; i < 8; i++) people.push(await new Client(`Eight ${i} ${RUN}`).open());
  await Promise.all(people.map((p) => p.hello()));
  for (const p of people) p.send({ t: 'voice.join', channelId: 'gaming' });
  await Promise.all(people.map((p) => p.next((m) => m.t === 'voice' && m.sid === p.ready.sid && m.channelId === 'gaming')));
  // Each to each of the other 7, as fast as it can: an SDP and 15 ICE candidates (a busy network's worth).
  for (const p of people) {
    for (const q of people) {
      if (q === p) continue;
      p.send({ t: 'signal', to: q.ready.sid, data: { sdp: { type: 'offer', sdp: `v=0 ${'x'.repeat(4000)}` } } });
      for (let k = 0; k < 15; k++) p.send({ t: 'signal', to: q.ready.sid, data: { candidate: { candidate: `candidate:${k} 1 udp 2122260223 192.168.1.${k} 5${k}000 typ host`, sdpMid: '0' } } });
    }
  }
  const per = 7 * 16;
  const until = Date.now() + 8000;
  const got = () => people.map((p) => p.inbox.filter((m) => m.t === 'signal').length);
  while (got().some((n) => n < per) && Date.now() < until) await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual(got(), people.map(() => per), 'everyone got all 112');
  assert.ok(!people.some((p) => p.inbox.some((m) => m.t === 'error')), 'no errors');
  for (const p of people) p.close();
});

await test('switching channels leaves the old one first', async () => {
  b.send({ t: 'voice.join', channelId: 'gaming' });
  const leave = await a.next((m) => m.t === 'voice' && m.sid === b.ready.sid);
  assert.equal(leave.channelId, null);
  const join = await a.next((m) => m.t === 'voice' && m.sid === b.ready.sid);
  assert.equal(join.channelId, 'gaming');
});

await test('disconnect clears voice and announces leave', async () => {
  a.close();
  const v = await b.next((m) => m.t === 'voice' && m.sid === a.ready.sid);
  assert.equal(v.channelId, null);
  const l = await b.next((m) => m.t === 'session.leave' && m.sid === a.ready.sid);
  assert.equal(l.sid, a.ready.sid);
});

await test('reconnecting user gets a fresh session and the same identity', async () => {
  const a2 = new Client(ALICE);
  a2.token = a.token;
  await a2.open();
  const ready = await a2.hello();
  assert.notEqual(ready.sid, a.ready.sid);
  assert.ok(!ready.sessions.some((s) => s.sid === a.ready.sid), 'old session is gone');
  a2.close();
});

b.close();

// ── Keeping calls across reconnects (PROTOCOL.md › Voice) ────────────────
{
  const X = `Xia ${RUN}`;
  const x1 = await new Client(X).open();
  await x1.hello();
  const y = await new Client(`Yan ${RUN}`).open();
  await y.hello();
  const voiceOf = (c, sid, pred = () => true) => c.next((m) => m.t === 'voice' && m.sid === sid && pred(m));
  let x2;

  await test('voice resume: ready carries a resume key; capable sessions say so; a dropped socket is "lost"', async () => {
    assert.match(x1.ready.resumeKey, /^[A-Za-z0-9_-]{43}$/);
    assert.notEqual(x1.ready.resumeKey, y.ready.resumeKey);
    x1.send({ t: 'voice.join', channelId: 'lounge', resume: true });
    const seen = await voiceOf(y, x1.ready.sid);
    assert.equal(seen.channelId, 'lounge');
    assert.equal(seen.resume, true);
    assert.equal(seen.prev, undefined);
    y.send({ t: 'voice.join', channelId: 'lounge' }); // an older client: no resume
    const legacy = await voiceOf(x1, y.ready.sid);
    assert.equal(legacy.resume, undefined);
    x1.close();
    const lost = await voiceOf(y, x1.ready.sid);
    assert.equal(lost.channelId, null);
    assert.equal(lost.lost, true, 'a socket that closed in voice is marked lost');
    await y.next((m) => m.t === 'session.leave' && m.sid === x1.ready.sid);
  });

  await test('voice resume: the same person continues the call with prev + proof; later joiners see prev too', async () => {
    x2 = new Client(X);
    x2.token = x1.token;
    await x2.open();
    await x2.hello();
    x2.send({ t: 'voice.join', channelId: 'lounge', resume: true, prev: x1.ready.sid, proof: x1.ready.resumeKey });
    const echo = await voiceOf(x2, x2.ready.sid);
    assert.equal(echo.prev, x1.ready.sid, 'your own echo says the handover was accepted');
    const seen = await voiceOf(y, x2.ready.sid);
    assert.equal(seen.channelId, 'lounge');
    assert.equal(seen.prev, x1.ready.sid);
    assert.equal(seen.resume, true);
    const late = await new Client(`Lou ${RUN}`).open();
    const ready = await late.hello();
    const s = ready.sessions.find((v) => v.sid === x2.ready.sid);
    assert.equal(s.voice.prev, x1.ready.sid);
    assert.equal(s.voice.resume, true);
    late.close();
  });

  await test("voice resume: another account's session is refused, even with that session's own key", async () => {
    const z = await new Client(`Zed ${RUN}`).open();
    await z.hello();
    // x1's key is only good for x1's own account: Zed can't take over Xia's call.
    z.send({ t: 'voice.join', channelId: 'lounge', resume: true, prev: x1.ready.sid, proof: x1.ready.resumeKey });
    const echo = await voiceOf(z, z.ready.sid);
    assert.equal(echo.channelId, 'lounge');
    assert.equal(echo.prev, undefined, 'joined, but not as a continuation');
    assert.equal((await voiceOf(y, z.ready.sid)).prev, undefined);
    z.send({ t: 'voice.leave' });
    await voiceOf(z, z.ready.sid, (m) => m.channelId === null);
    // A live session of someone else, with Zed's own key, or junk: refused, and nobody is kicked.
    for (const [prev, proof] of [[x2.ready.sid, z.ready.resumeKey], [x2.ready.sid, 'x'.repeat(43)], ['not-a-sid', z.ready.resumeKey], [x2.ready.sid, 42]]) {
      z.send({ t: 'voice.join', channelId: 'lounge', resume: true, prev, proof });
      assert.equal((await voiceOf(z, z.ready.sid)).prev, undefined);
      z.send({ t: 'voice.leave' });
      await voiceOf(z, z.ready.sid, (m) => m.channelId === null);
    }
    await new Promise((r) => setTimeout(r, 200));
    assert.ok(!y.inbox.some((m) => m.t === 'voice' && m.sid === x2.ready.sid && m.channelId === null), 'Xia is still in the call');
    assert.equal(x2.ws.readyState, WebSocket.OPEN);
    z.close();
  });

  await test('voice resume: a half-dead old session is closed after the handover is announced', async () => {
    const x3 = new Client(X);
    x3.token = x2.token;
    await x3.open();
    await x3.hello();
    const order = [];
    y.ws.addEventListener('message', (e) => {
      const m = e.data === 'pong' ? null : JSON.parse(e.data);
      if (m?.t === 'voice' || m?.t === 'session.leave') order.push(`${m.t}:${m.sid}:${m.channelId ?? ''}`);
    });
    const closed = new Promise((resolve) => x2.ws.addEventListener('close', (e) => resolve(e.code), { once: true }));
    x3.send({ t: 'voice.join', channelId: 'lounge', resume: true, prev: x2.ready.sid, proof: x2.ready.resumeKey });
    const resumed = await voiceOf(y, x3.ready.sid);
    assert.equal(resumed.prev, x2.ready.sid);
    const gone = await voiceOf(y, x2.ready.sid);
    assert.equal(gone.channelId, null);
    await y.next((m) => m.t === 'session.leave' && m.sid === x2.ready.sid);
    assert.deepEqual(order, [`voice:${x3.ready.sid}:lounge`, `voice:${x2.ready.sid}:`, `session.leave:${x2.ready.sid}:`], 'resume first, then the old one leaves');
    assert.equal(await closed, 4000);
    // A deliberate leave is not "lost", and clears resume/prev.
    x3.send({ t: 'voice.leave' });
    const left = await voiceOf(y, x3.ready.sid);
    assert.equal(left.channelId, null);
    assert.equal(left.lost, undefined);
    x3.send({ t: 'voice.join', channelId: 'lounge' });
    const plain = await voiceOf(y, x3.ready.sid);
    assert.equal(plain.prev, undefined);
    assert.equal(plain.resume, undefined);
    x3.close();
  });
  y.close();
}

if (ADMIN) {
  await test('admin merge: duplicate account reads as the real one; unmerge reverses', async () => {
    const auth = { headers: { Authorization: `Bearer ${ADMIN}` } };
    const real = await new Client(`Real ${RUN}`).open();
    await real.hello();
    real.send({ t: 'send', channelId: 'general', body: 'from the real account', nonce: 'r1' });
    await real.next((m) => m.t === 'message' && m.nonce === 'r1');
    const dup = await new Client(`Dup ${RUN}`).open();
    await dup.hello();
    dup.send({ t: 'send', channelId: 'general', body: 'from the duplicate', nonce: 'd1' });
    const dupMsg = (await dup.next((m) => m.t === 'message' && m.nonce === 'd1')).message;
    const dupEmoji = (await (await upload(png(), `dup_${rid()}`, dup.ready.sid, KEY, 'image/png', dup.ready.uploadToken)).json()).emoji;

    const merged = await (await fetch(`${BASE}/api/admin/merge-users?from=${dup.uid}&into=${real.uid}`, { method: 'POST', ...auth })).json();
    assert.equal(merged.ok, true, JSON.stringify(merged));
    await new Promise((r) => setTimeout(r, 300)); // everyone got bounced to reconnect

    const again = await new Client(`Dup ${RUN}`);
    again.token = dup.token;
    await again.open();
    const ready = await again.hello();
    assert.equal(ready.uid, real.uid, 'old device is told its canonical id');
    assert.ok(!ready.users.some((u) => u.id === dup.uid), 'duplicate gone from the member list');
    assert.equal(ready.emojis.find((e) => e.id === dupEmoji.id)?.uid, real.uid, "the duplicate's emoji reads as the real account's too");
    again.send({ t: 'history', channelId: 'general', limit: 10 });
    const h = await again.next((m) => m.t === 'history');
    assert.equal(h.messages.find((m) => m.id === dupMsg.id).uid, real.uid, "duplicate's messages now read as the real account");
    again.send({ t: 'delete', id: dupMsg.id });
    assert.equal((await again.next((m) => m.t === 'deleted')).id, dupMsg.id, 'and the real account can delete them');

    const un = await (await fetch(`${BASE}/api/admin/unmerge-user?alias=${dup.uid}`, { method: 'POST', ...auth })).json();
    assert.equal(un.ok, true);
    await new Promise((r) => setTimeout(r, 300));
    const back = new Client(`Dup ${RUN}`);
    back.token = dup.token;
    await back.open();
    assert.equal((await back.hello()).uid, dup.uid, 'unmerged: own id again');
    back.close();
  });
}

// ── Extras (v0.36): PROTOCOL.md › Extras ────────────────────────────────
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
const sendErr = async (who, re) => (await who.next((m) => m.t === 'error' && m.re === re)).code;
/** A mono 16-bit WAV: `ms` of a 440 Hz sine at amplitude `amp` (0–1). */
function wav(ms = 800, amp = 0.9, rate = 24000) {
  const n = Math.round((rate * ms) / 1000);
  const buf = Buffer.alloc(44 + n * 2);
  buf.write('RIFF', 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write('WAVEfmt ', 8);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(rate, 24);
  buf.writeUInt32LE(rate * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write('data', 36);
  buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) buf.writeInt16LE(Math.round(Math.sin((i * 2 * Math.PI * 440) / rate) * amp * 32767), 44 + i * 2);
  return buf;
}
const postTo = (path, who, body, params = {}) =>
  fetch(`${BASE}${path}?${new URLSearchParams({ sid: who.ready.sid, ut: who.ready.uploadToken, ...params })}`, { method: 'POST', headers: { 'X-Yap-Key': KEY }, body });
const erased = async (id) => {
  let status = 200;
  for (let i = 0; i < 20 && status === 200; i++) {
    await pause(100);
    status = (await fetch(`${BASE}/media/${id}?fresh=${rid()}`)).status;
  }
  return status === 404;
};

// a and b closed above: the same two accounts, reconnected.
const reconnect = async (old) => {
  const c = new Client(old.name);
  c.token = old.token;
  await c.open();
  await c.hello();
  return c;
};
const A = await reconnect(a);
const B = await reconnect(b);
// Names may have changed in the tests above.
const nameOf = (c) => c.ready.users.find((u) => u.id === c.uid).name;

await pause(1500);
await test('polls: 2–10 distinct options, one vote each (change / take back), everyone sees, history carries votes, delete erases them', async () => {
  A.send({ t: 'send', channelId: 'general', body: 'Pizza or tacos?', poll: { options: ['pizza'] } });
  assert.equal(await sendErr(A, 'send'), 'bad_poll');
  A.send({ t: 'send', channelId: 'general', body: 'Pizza or tacos?', poll: { options: ['Pizza', 'pizza'] } });
  assert.equal(await sendErr(A, 'send'), 'bad_poll', 'no duplicates');
  A.send({ t: 'send', channelId: `p_${A.uid}`, body: 'just me?', poll: { options: ['yes', 'no'] } });
  assert.equal(await sendErr(A, 'send'), 'bad_poll', 'not in a Personal space');
  A.send({ t: 'send', channelId: 'general', body: 'Pizza or tacos?', poll: { options: ['pizza', 'tacos', 'both'] }, nonce: 'poll1' });
  const poll = (await A.next((m) => m.t === 'message' && m.nonce === 'poll1')).message;
  assert.deepEqual(poll.poll, { options: ['pizza', 'tacos', 'both'], votes: [[], [], []] });
  await B.next((m) => m.t === 'message' && m.message.id === poll.id);
  const votes = async () => (await A.next((m) => m.t === 'poll' && m.id === poll.id)).votes;
  B.send({ t: 'poll.vote', id: poll.id, option: 1 });
  assert.deepEqual(await votes(), [[], [B.uid], []]);
  B.send({ t: 'poll.vote', id: poll.id, option: 2 });
  assert.deepEqual(await votes(), [[], [], [B.uid]], 'changing moves your vote');
  A.send({ t: 'poll.vote', id: poll.id, option: 2 });
  assert.deepEqual(await votes(), [[], [], [B.uid, A.uid]]);
  B.send({ t: 'poll.vote', id: poll.id, option: null });
  assert.deepEqual(await votes(), [[], [], [A.uid]], 'null takes it back');
  A.send({ t: 'poll.vote', id: poll.id, option: 7 });
  assert.equal(await sendErr(A, 'poll.vote'), 'bad_poll');
  B.send({ t: 'history', channelId: 'general', limit: 5 });
  assert.deepEqual((await B.next((m) => m.t === 'history')).messages.find((m) => m.id === poll.id).poll.votes, [[], [], [A.uid]]);
  A.send({ t: 'delete', id: poll.id });
  await B.next((m) => m.t === 'deleted' && m.id === poll.id);
  if (ADMIN) {
    const dump = await (await admin('/api/admin/export?after=0')).json();
    assert.ok(!dump.pollVotes.some((v) => v.message_id === poll.id), 'votes erased');
    assert.equal(dump.messages.find((m) => m.id === poll.id).poll, undefined, 'options erased');
  }
});

await pause(1500);
await test('/roll and /flip: rolled by the server; bad specs refused; a roll can’t be edited', async () => {
  const say = async (body, nonce) => {
    A.send({ t: 'send', channelId: 'general', body, nonce });
    return (await A.next((m) => m.t === 'message' && m.nonce === nonce)).message;
  };
  const hundred = await say('/roll', 'd1');
  assert.equal(hundred.roll.kind, 'roll');
  assert.ok(hundred.roll.total >= 1 && hundred.roll.total <= 100);
  assert.equal(hundred.body, `🎲 1–100: ${hundred.roll.total}`);
  const dice = await say('/roll 2d6+3', 'd2');
  assert.equal(dice.roll.rolls.length, 2);
  assert.ok(dice.roll.rolls.every((r) => r >= 1 && r <= 6));
  assert.equal(dice.roll.total, dice.roll.rolls[0] + dice.roll.rolls[1] + 3);
  assert.equal(dice.body, `🎲 2d6+3: ${dice.roll.rolls[0]} + ${dice.roll.rolls[1]} + 3 = ${dice.roll.total}`);
  const coin = await say('/flip', 'd3');
  assert.ok(['🪙 Heads', '🪙 Tails'].includes(coin.body));
  assert.deepEqual([coin.roll.kind, coin.roll.spec], ['flip', 'coin']);
  await pause(1200);
  const twenty = await say('/ROLL 20', 'd4');
  assert.ok(twenty.roll.total >= 1 && twenty.roll.total <= 20);
  const plain = await say('/roll the dice later', 'd5');
  assert.equal(plain.roll, undefined, 'a sentence is just a message');
  A.send({ t: 'send', channelId: 'general', body: '/roll 99d6' });
  assert.equal(await sendErr(A, 'send'), 'bad_roll');
  A.send({ t: 'send', channelId: 'general', body: '/roll banana' });
  assert.equal(await sendErr(A, 'send'), 'bad_roll');
  A.send({ t: 'edit', id: hundred.id, body: '🎲 1–100: 100' });
  assert.equal(await sendErr(A, 'edit'), 'forbidden', 'no faking a roll afterwards');
  B.send({ t: 'history', channelId: 'general', limit: 8 });
  assert.deepEqual((await B.next((m) => m.t === 'history')).messages.find((m) => m.id === dice.id).roll, dice.roll);
});

await pause(1500);
await test('away messages: everyone sees them; mentioning someone away tells only you, once; checked; cleared. Join chimes', async () => {
  A.send({ t: 'profile', away: { text: 'at work til 6', until: null } });
  const set = (await B.next((m) => m.t === 'user' && m.user.id === A.uid && m.user.away)).user.away;
  assert.deepEqual([set.text, set.until, typeof set.since], ['at work til 6', null, 'number']);
  B.send({ t: 'send', channelId: 'general', body: `@${nameOf(A)} you around?`, nonce: 'aw1' });
  await B.next((m) => m.t === 'message' && m.nonce === 'aw1');
  const note = await B.next((m) => m.t === 'away' && m.uid === A.uid);
  assert.deepEqual([note.text, note.until], ['at work til 6', null]);
  B.send({ t: 'send', channelId: 'general', body: `@${nameOf(A)} hello?`, nonce: 'aw2' });
  await B.next((m) => m.t === 'message' && m.nonce === 'aw2');
  await pause(300);
  assert.ok(!B.inbox.some((m) => m.t === 'away'), 'once per person per 10 minutes');
  assert.ok(!A.inbox.some((m) => m.t === 'away'), 'only the one who mentioned hears it');
  A.send({ t: 'profile', away: { text: 'nap', until: Date.now() - 1000 } });
  assert.equal(await sendErr(A, 'profile'), 'bad_away');
  A.send({ t: 'profile', away: { text: '   ', until: null } });
  assert.equal(await sendErr(A, 'profile'), 'bad_away');
  A.send({ t: 'profile', away: null });
  assert.equal((await B.next((m) => m.t === 'user' && m.user.id === A.uid && !m.user.away)).user.away, undefined);
  A.send({ t: 'profile', chime: 'tada' });
  assert.equal((await B.next((m) => m.t === 'user' && m.user.id === A.uid && m.user.chime)).user.chime, 'tada');
  A.send({ t: 'profile', chime: 'kazoo' });
  assert.equal(await sendErr(A, 'profile'), 'bad_chime');
  A.send({ t: 'profile', chime: null });
  assert.equal((await B.next((m) => m.t === 'user' && m.user.id === A.uid && !m.user.chime)).user.chime, undefined);
});

await test('a profile change with one bad field changes nothing (and tells nobody)', async () => {
  const before = B.ready.users.find((u) => u.id === A.uid) ?? { name: nameOf(A) };
  A.send({ t: 'profile', name: `Renamed ${RUN}`, color: '#123456', away: { text: '   ', until: null } });
  assert.equal(await sendErr(A, 'profile'), 'bad_away');
  A.send({ t: 'profile', name: `Renamed ${RUN}`, color: '#123456', chime: 'kazoo' });
  assert.equal(await sendErr(A, 'profile'), 'bad_chime');
  A.send({ t: 'profile', look: '🦊' });
  const now = (await B.next((m) => m.t === 'user' && m.user.id === A.uid && m.user.look === '🦊')).user;
  assert.deepEqual([now.name, now.color], [nameOf(A), before.color ?? now.color], 'neither the name nor the colour was saved');
  assert.notEqual(now.color, '#123456');
});

await test('soundboard: only checked WAVs, loudness capped; plays reach your voice channel only; rate limited; chimes; delete', async () => {
  assert.equal((await postTo('/api/sound', A, png(), { name: 'nope' })).status, 415, 'not a WAV');
  assert.equal((await postTo('/api/sound', A, wav(6000), { name: 'long' })).status, 415, 'over 5 s');
  assert.equal((await postTo('/api/sound', A, wav(500, 0), { name: 'shh' })).status, 415, 'silent');
  const res = await postTo('/api/sound', A, wav(800, 0.95), { name: 'Airhorn', emoji: '📯' });
  const body = await res.json();
  assert.equal(res.status, 200, JSON.stringify(body));
  const sound = body.sound;
  assert.deepEqual([sound.kind, sound.name, sound.emoji, sound.uid], ['board', 'Airhorn', '📯', A.uid]);
  assert.ok(sound.gain > 0.3 && sound.gain < 0.45, `a loud clip is turned down (gain ${sound.gain})`);
  assert.ok(Math.abs(sound.ms - 800) < 5);
  await B.next((m) => m.t === 'sound.added' && m.sound.id === sound.id);
  assert.equal((await fetch(`${BASE}/media/${sound.media}`)).headers.get('content-type'), 'audio/wav');
  A.send({ t: 'sound.play', id: sound.id });
  assert.equal(await sendErr(A, 'sound.play'), 'not_in_voice');
  A.send({ t: 'voice.join', channelId: 'lounge' });
  await A.next((m) => m.t === 'voice' && m.sid === A.ready.sid && m.channelId === 'lounge');
  A.send({ t: 'sound.play', id: sound.id });
  assert.equal((await A.next((m) => m.t === 'sound' && m.id === sound.id)).uid, A.uid);
  await pause(300);
  assert.ok(!B.inbox.some((m) => m.t === 'sound'), 'not in voice: hears nothing');
  B.send({ t: 'voice.join', channelId: 'lounge' });
  await B.next((m) => m.t === 'voice' && m.sid === B.ready.sid && m.channelId === 'lounge');
  A.send({ t: 'sound.play', id: sound.id });
  assert.equal((await B.next((m) => m.t === 'sound' && m.id === sound.id)).sid, A.ready.sid);
  A.send({ t: 'sound.play', id: sound.id });
  assert.equal(await sendErr(A, 'sound.play'), 'slow_down', '2 burst, then 1 per 3 s');
  // Your own join chime: ≤ 2 s, named after you, becomes your chime.
  assert.equal((await postTo('/api/sound', A, wav(2600, 0.5), { kind: 'chime' })).status, 415);
  const chime = (await (await postTo('/api/sound', A, wav(1500, 0.3), { kind: 'chime' })).json()).sound;
  assert.deepEqual([chime.kind, chime.name], ['chime', nameOf(A)]);
  assert.equal((await B.next((m) => m.t === 'user' && m.user.id === A.uid && m.user.chime)).user.chime, `s:${chime.id}`);
  B.send({ t: 'profile', chime: `s:${chime.id}` });
  assert.equal(await sendErr(B, 'profile'), 'bad_chime', "someone else's chime isn't yours to use");
  const again = await new Client(ALICE).open();
  again.token = A.token;
  const fresh = await again.hello();
  assert.ok(fresh.sounds.some((s) => s.id === sound.id) && fresh.sounds.some((s) => s.id === chime.id), 'ready.sounds');
  again.close();
  B.send({ t: 'sound.delete', id: sound.id });
  assert.equal(await sendErr(B, 'sound.delete'), 'forbidden');
  A.send({ t: 'sound.delete', id: sound.id });
  await B.next((m) => m.t === 'sound.deleted' && m.id === sound.id);
  A.send({ t: 'voice.leave' });
  B.send({ t: 'voice.leave' });
  await B.next((m) => m.t === 'voice' && m.sid === B.ready.sid && m.channelId === null);
  A.send({ t: 'profile', chime: null });
});

await pause(1500);
await test('doodle reactions: small PNG/GIF only; one each per message (a new one replaces yours); removing erases it', async () => {
  const big = Buffer.from(PNG_1PX);
  big.writeUInt32BE(300, 16);
  assert.equal((await postTo('/api/doodle', B, big)).status, 400, 'too many pixels');
  assert.equal((await postTo('/api/doodle', B, Buffer.from([0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]))).status, 415, 'JPEG is not a doodle');
  A.send({ t: 'send', channelId: 'general', body: 'draw me a reaction', nonce: 'dd1' });
  const m = (await A.next((x) => x.t === 'message' && x.nonce === 'dd1')).message;
  const first = (await (await postTo('/api/doodle', B, png())).json()).id;
  B.send({ t: 'react', id: m.id, emoji: `<d:${first}>`, on: true });
  await A.next((x) => x.t === 'reaction' && x.emoji === `<d:${first}>` && x.on);
  const second = (await (await postTo('/api/doodle', B, png())).json()).id;
  B.send({ t: 'react', id: m.id, emoji: `<d:${second}>`, on: true });
  await A.next((x) => x.t === 'reaction' && x.emoji === `<d:${first}>` && !x.on);
  await A.next((x) => x.t === 'reaction' && x.emoji === `<d:${second}>` && x.on);
  assert.ok(await erased(first), 'the replaced doodle is erased');
  B.send({ t: 'history', channelId: 'general', limit: 3 });
  assert.deepEqual((await B.next((x) => x.t === 'history')).messages.find((x) => x.id === m.id).reactions, [{ emoji: `<d:${second}>`, uids: [B.uid] }]);
  B.send({ t: 'react', id: m.id, emoji: `<d:${second}>`, on: false });
  await A.next((x) => x.t === 'reaction' && x.emoji === `<d:${second}>` && !x.on);
  assert.ok(await erased(second), 'a removed doodle is erased');
  B.send({ t: 'react', id: m.id, emoji: '<d:nothex>', on: true });
  assert.equal(await sendErr(B, 'react'), 'bad_emoji');
});

await pause(1500);
await test('media shelf: only pictures and GIFs, newest first; catch-up counts skip your own messages', async () => {
  A.send({ t: 'send', channelId: 'general', body: 'shelf: just words', nonce: 's0' });
  const words = (await A.next((x) => x.t === 'message' && x.nonce === 's0')).message;
  const up = await (await postTo('/api/image', A, png())).json();
  A.send({ t: 'send', channelId: 'general', body: 'shelf: a picture', image: { id: up.id, width: 1, height: 1 }, nonce: 's1' });
  const pic = (await A.next((x) => x.t === 'message' && x.nonce === 's1')).message;
  B.send({ t: 'shelf', channelId: 'general' });
  const shelf = await B.next((x) => x.t === 'shelf');
  assert.equal(shelf.items[0].id, pic.id);
  assert.ok(shelf.items.every((x) => x.image || x.gif), 'only pictures and GIFs');
  B.send({ t: 'shelf', channelId: `p_${A.uid}` });
  assert.equal(await sendErr(B, 'shelf'), 'no_channel');
  B.send({ t: 'unread.counts', since: { general: words.ts - 1, [`p_${A.uid}`]: 0, nope: 0 } });
  const { counts } = await B.next((x) => x.t === 'unread.counts');
  assert.deepEqual(Object.keys(counts), ['general']);
  assert.ok(counts.general.n >= 2);
  assert.deepEqual([counts.general.first.id, counts.general.first.uid, counts.general.first.body], [words.id, A.uid, 'shelf: just words']);
  A.send({ t: 'unread.counts', since: { general: words.ts - 1 } });
  assert.deepEqual((await A.next((x) => x.t === 'unread.counts')).counts.general, { n: 0, first: null, mentions: 0 }, 'your own messages are never unread');
});

await pause(1500);
await test('catch-up counts say how many of the new messages mention you: @you, @everyone, a reply to you (not code, not others)', async () => {
  A.send({ t: 'send', channelId: 'general', body: 'mention count: start', nonce: 'mc0' });
  const start = (await A.next((x) => x.t === 'message' && x.nonce === 'mc0')).message;
  const lines = [
    { body: `@${nameOf(A)} are you coming?` }, // mentions A
    { body: 'nothing for anyone here' },
    { body: 'heads up @everyone' }, // everyone
    { body: 'yes', replyTo: start.id }, // a reply to A
    { body: `\`@${nameOf(A)}\` in code doesn't count` },
    { body: `@${nameOf(B)} talking to myself` },
  ];
  for (const [i, line] of lines.entries()) {
    B.send({ t: 'send', channelId: 'general', nonce: `mc${i + 1}`, ...line });
    await B.next((x) => x.t === 'message' && x.nonce === `mc${i + 1}`);
  }
  A.send({ t: 'unread.counts', since: { general: start.ts } });
  const forA = (await A.next((x) => x.t === 'unread.counts')).counts.general;
  assert.equal(forA.n, lines.length);
  assert.equal(forA.mentions, 3, '@A, @everyone and the reply');
  B.send({ t: 'unread.counts', since: { general: start.ts - 1 } });
  const forB = (await B.next((x) => x.t === 'unread.counts')).counts.general;
  assert.deepEqual([forB.n, forB.mentions], [1, 0], 'only A\'s message is new for B, and it mentions nobody');
});

await pause(1500);
await test('quiet hours: checked, on every device; pushes held back (spoilers never shown); mentions let through when asked', async () => {
  B.send({ t: 'notify.quiet', quiet: { from: 0, to: 0, tz: 'Mars/Olympus', mentions: false } });
  assert.equal(await sendErr(B, 'notify.quiet'), 'bad_quiet');
  const until = Date.now() + 3_600_000;
  B.send({ t: 'notify.quiet', quiet: { from: 1380, to: 480, tz: 'Europe/Rome', mentions: false, until } });
  assert.deepEqual((await B.next((m) => m.t === 'notify' && m.quiet)).quiet, { from: 1380, to: 480, tz: 'Europe/Rome', mentions: false, until });
  const before = pushes.length;
  A.send({ t: 'send', channelId: 'general', body: `@${nameOf(B)} snoozed?`, nonce: 'qh1' });
  await A.next((m) => m.t === 'message' && m.nonce === 'qh1');
  await pause(700);
  assert.ok(!pushes.slice(before).some((p) => p.topic === bTopic), 'held back while snoozed');
  B.send({ t: 'notify.quiet', quiet: { from: 1380, to: 480, tz: 'Europe/Rome', mentions: true, until } });
  await B.next((m) => m.t === 'notify' && m.quiet?.mentions);
  A.send({ t: 'send', channelId: 'general', body: `@${nameOf(B)} the ending: ||he was a ghost||`, nonce: 'qh2' });
  const push = await pushFor(bTopic, (p) => p.message.includes('the ending'));
  assert.ok(push.message.includes('▒▒▒') && !push.message.includes('ghost'), `spoiler hidden: ${push.message}`);
  // A daily window around now (UTC), no snooze: held back again.
  const nowMin = new Date().getUTCHours() * 60 + new Date().getUTCMinutes();
  B.send({ t: 'notify.quiet', quiet: { from: (nowMin + 1435) % 1440, to: (nowMin + 5) % 1440, tz: 'UTC', mentions: false } });
  await B.next((m) => m.t === 'notify' && m.quiet?.tz === 'UTC');
  const mark = pushes.length;
  A.send({ t: 'send', channelId: 'general', body: `@${nameOf(B)} night owl?`, nonce: 'qh3' });
  await A.next((m) => m.t === 'message' && m.nonce === 'qh3');
  await pause(700);
  assert.ok(!pushes.slice(mark).some((p) => p.topic === bTopic), 'held back inside the daily window');
  B.send({ t: 'notify.quiet', quiet: null });
  assert.equal((await B.next((m) => m.t === 'notify' && m.quiet === null)).quiet, null);
});

await test('hangs: checked; everyone sees; only the maker cancels; at the time everyone hears and the free people get one push', async () => {
  A.send({ t: 'hang.set', at: Date.now() - 1000, title: 'Too late' });
  assert.equal(await sendErr(A, 'hang.set'), 'bad_hang');
  A.send({ t: 'hang.set', at: Date.now() + 600_000, title: 'Somewhere', channelId: 'general' });
  const where = await sendErr(A, 'hang.set');
  if (where === 'bad_hang') skip('start the server with --var TEST_HANGS:1 to test hangs end to end');
  assert.equal(where, 'no_channel', 'a voice channel or nothing');
  A.send({ t: 'hang.set', at: Date.now() + 600_000, title: 'Later', channelId: 'lounge' });
  const later = (await B.next((m) => m.t === 'hang' && m.hang.title === 'Later')).hang;
  assert.deepEqual([later.channelId, later.uid], ['lounge', A.uid]);
  B.send({ t: 'hang.cancel', id: later.id });
  assert.equal(await sendErr(B, 'hang.cancel'), 'forbidden');
  A.send({ t: 'hang.cancel', id: later.id });
  await B.next((m) => m.t === 'hang.cancelled' && m.id === later.id);
  const at = Date.now() + 3000;
  B.send({ t: 'when.set', slots: [Math.floor(at / 1_800_000)] });
  await B.next((m) => m.t === 'when' && m.uid === B.uid);
  A.send({ t: 'hang.set', at, title: `Game night ${RUN}`, channelId: 'lounge' });
  const hang = (await B.next((m) => m.t === 'hang' && m.hang.title === `Game night ${RUN}`)).hang;
  await B.next((m) => m.t === 'hang.now' && m.id === hang.id, 10_000);
  const push = await pushFor(bTopic, (p) => p.title.includes(`Game night ${RUN} is starting · Lounge`), 5000);
  assert.ok(push);
  const again = await new Client(BOB).open();
  again.token = B.token;
  assert.ok((await again.hello()).hangs.some((h) => h.id === hang.id), 'ready.hangs keeps one that just started');
  again.close();
  B.send({ t: 'when.set', slots: [] });
});
A.close();
B.close();

// Last: it can trip the per-IP limit for ~10 s.
await test('restore import refuses a server that already has people', async () => {
  const res = await admin('/api/admin/import', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ phase: 'check' }) });
  assert.equal(res.status, 409);
  const begin = await admin('/api/admin/import', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ phase: 'begin', data: { schema: 1 } }) });
  assert.equal(begin.status, 409);
  const page = await (await admin('/api/admin/export')).json();
  for (const k of ['reactions', 'edits', 'strokes', 'drawings', 'invites', 'links', 'modLog', 'pollVotes', 'sounds', 'hangs']) assert.ok(Array.isArray(page[k]), `export has ${k}`);
});

await test('per-IP limit covers the API but never images (review finding)', async () => {
  const imgs = await Promise.all(Array.from({ length: 150 }, () => fetch(`${BASE}/emoji/${emoji.id}`).then((r) => r.status)));
  assert.ok(imgs.every((s) => s === 200), `image statuses: ${[...new Set(imgs)]}`);
  const apis = await Promise.all(Array.from({ length: 150 }, () => fetch(`${BASE}/api/check`).then((r) => r.status)));
  // Leave the server as we found it: whatever runs next (e2e) shouldn't start out rate limited.
  for (let i = 0; i < 60 && (await fetch(`${BASE}/api/check`)).status === 429; i++) await new Promise((r) => setTimeout(r, 500));
  if (!apis.includes(429)) skip('no 429 from the API burst: the local rate limiter may not see a client IP');
});

ntfy.close();

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.stack ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
const skipped = results.filter((r) => r[0] === '○').length;
console.log(`\n${results.length - failed - skipped}/${results.length} passed${skipped ? `, ${skipped} skipped${STRICT ? ' (a failure here: YAP_TEST_STRICT)' : ''}` : ''}`);
pushServer.close();
process.exit(failed || (STRICT && skipped) ? 1 : 0);
