// Pure-function tests for the web client: the formatter, YouTube links, Opus tuning (no server needed).
//   node test/unit.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { renderBody, isJumbo, esc, plainText, plainHtml, clip } from '../web/js/format.js';
import { youtubeId, clock, WatchParty } from '../web/js/watch.js';
import { Ducker, DUCK } from '../web/js/duck.js';
import { tuneOpus } from '../web/js/voice.js';
import { findMentions } from '../web/js/mentions.js';
import { settings, keyProtocols, socketUrl } from '../web/js/settings.js';

const cases = [];
const t = (name, fn) => cases.push([name, fn]);

t('escapes HTML everywhere', () => {
  assert.equal(renderBody('<img src=x onerror=alert(1)>'), '&lt;img src=x onerror=alert(1)&gt;');
  assert.equal(esc(`"'<>&`), '&quot;&#39;&lt;&gt;&amp;');
});
t('links: only http(s), trailing punctuation left outside', () => {
  assert.match(renderBody('go to https://a.com/x?y=1&z=2.'), /<a href="https:\/\/a\.com\/x\?y=1&amp;z=2" [^>]*>https:\/\/a\.com\/x\?y=1&amp;z=2<\/a>\.$/);
  assert.doesNotMatch(renderBody('javascript:alert(1)'), /<a /);
});
t('an @name inside a URL stays part of the link (review finding)', () => {
  const html = renderBody('https://youtube.com/@Sam', ['Sam']);
  assert.equal(html, '<a href="https://youtube.com/@Sam" target="_blank" rel="noopener noreferrer">https://youtube.com/@Sam</a>');
});
t('plainText: snippets drop the formatting marks, keep the words', () => {
  assert.equal(plainText("who's around tonight? thinking **Overcooked** around 9 🍳"), "who's around tonight? thinking Overcooked around 9 🍳");
  assert.equal(plainText('__bold__ *it* _it_ ~~gone~~ `x_y` and ```js\nconst a_b = 1\n```'), 'bold it it gone x_y and const a_b = 1');
  assert.equal(plainText('> quoted\nreply'), 'quoted\nreply');
  assert.equal(plainText('> quoted\nreply', { line: true }), 'quoted reply');
  assert.equal(plainText('see https://a.com/x_y_z and _this_'), 'see https://a.com/x_y_z and this');
  assert.equal(plainText('the ending: ||the dog lives|| 🐶'), 'the ending: ▒▒▒ 🐶');
  assert.equal(plainText(`gets cold <:parrot:${'a'.repeat(32)}>`), 'gets cold :parrot:');
  assert.equal(plainText('2 * 3 * 4 = 24, file_name_here'), '2 * 3 * 4 = 24, file_name_here');
  assert.equal(plainText(null), '');
});
t('plainHtml: escaped, custom emojis drawn small, cut and marked safely', () => {
  const id = 'b'.repeat(32);
  assert.equal(plainHtml(`<b>hi</b> **there** <:parrot:${id}>`), `&lt;b&gt;hi&lt;/b&gt; there <img class="emoji" src="/emoji/${id}" alt=":parrot:" title=":parrot:" draggable="false" loading="lazy">`);
  // A cut can't split an emoji (it's one character while cutting).
  assert.equal(plainHtml(`ab <:parrot:${id}> cd`, { cut: (t) => t.slice(0, 4) }), `ab <img class="emoji" src="/emoji/${id}" alt=":parrot:" title=":parrot:" draggable="false" loading="lazy">`);
  assert.equal(plainHtml('one\ntwo **three**', { mark: (h) => h.replace('three', '<mark>three</mark>') }), 'one two <mark>three</mark>');
});
t('clip: cut at a word, one ellipsis, never "end.…"', () => {
  assert.equal(clip('short', 10), 'short');
  assert.equal(clip('the cafe at the far end. Then back', 24), 'the cafe at the far end…');
  assert.equal(clip('supercalifragilistic', 10), 'supercalif…');
});
t('mentions: longest name wins, not inside words or emails', () => {
  assert.match(renderBody('hi @Sam Lee!', ['Sam', 'Sam Lee']), /<span class="mention">@Sam Lee<\/span>!/);
  assert.equal(renderBody('mail a@bob.com', ['bob']), 'mail a@bob.com');
  assert.match(renderBody('@everyone @here'), /@everyone.*@here/);
});
t('names with HTML/regex characters are safe', () => {
  const html = renderBody('yo @<b>(x)', ['<b>(x)']);
  assert.match(html, /<span class="mention">@&lt;b&gt;\(x\)<\/span>/);
});
t('code is not formatted', () => {
  assert.equal(renderBody('`**not bold** @everyone`'), '<code>**not bold** @everyone</code>');
});
t('custom emoji tokens render; fake ones do not', () => {
  const id = 'a'.repeat(32);
  assert.match(renderBody(`<:party:${id}>`), /<img class="emoji" src="\/emoji\/a{32}"/);
  assert.doesNotMatch(renderBody('<:party:nothex>'), /<img/);
});
t('jumbo: 1–8 emojis (custom count too), not text', () => {
  assert.equal(isJumbo('🎉🎉'), true);
  assert.equal(isJumbo(`<:x1:${'b'.repeat(32)}>`), true);
  assert.equal(isJumbo('hi 🎉'), false);
  assert.equal(isJumbo('🎉'.repeat(9)), false);
});
t('Watch together: YouTube links of every shape give the 11-character id; others give null', () => {
  const id = 'dQw4w9WgXcQ';
  for (const link of [
    id, `https://www.youtube.com/watch?v=${id}&t=42s`, `youtube.com/watch?v=${id}`, `https://m.youtube.com/watch?v=${id}`,
    `https://youtu.be/${id}?si=abc`, `https://www.youtube.com/shorts/${id}`, `https://music.youtube.com/watch?v=${id}&list=RD`,
    `https://www.youtube-nocookie.com/embed/${id}`, `https://www.youtube.com/live/${id}?feature=share`, `  ${id}  `,
  ]) assert.equal(youtubeId(link), id, link);
  for (const bad of ['', 'hello', `https://evil.com/watch?v=${id}`, `https://youtube.com.evil.com/watch?v=${id}`, 'javascript:alert(1)', `https://www.youtube.com/watch?v=${id}x`, `ftp://youtu.be/${id}`]) {
    assert.equal(youtubeId(bad), null, bad);
  }
  assert.equal(clock(75), '1:15');
  assert.equal(clock(3725), '1:02:05');
});

const PEOPLE = [{ id: 'sam', name: 'Sam' }, { id: 'samlee', name: 'Sam Lee' }, { id: 'ben', name: 'Ben' }, { id: 'jo', name: 'Jo' }];
// [message, who it mentions]: what the server pushes and the page highlights (one rule: web/js/mentions.js).
const MENTIONS = [
  ['@Sam Lee hi', ['samlee']], ['@sam lee and @SAM', ['samlee', 'sam']], ['@Sam Lee, @Sam Lee!', ['samlee']], ['@Sam Leeds', ['sam']],
  ['hi @ben!', ['ben']], ['(@Ben)', ['ben']], ['@ben\n@jo', ['ben', 'jo']], ['@Ben@Jo', ['ben']],
  ['mail sam@ben.com', []], ['x@ben', []], ['@@ben', []], ['@bento', []], ['@ben_x', []], ['@ben2', []],
  ['`@ben` in code', []], ['```\n@ben\n```', []], ['https://youtube.com/@Ben', []], ['see https://x.com/a then @jo', ['jo']], ['no one', []],
  // Custom emojis are covered up like code: one right against the @ makes it no mention (on both sides).
  [`<:wave:${'a'.repeat(32)}>@ben`, []], [`@ben<:wave:${'a'.repeat(32)}>`, []], [`<:wave:${'a'.repeat(32)}> @ben`, ['ben']], [`@ben <:wave:${'a'.repeat(32)}>`, ['ben']],
  ['`x`@ben', []], ['@ben`x`', []], ['`x` @ben', ['ben']], ['☀️@ben', ['ben']],
];
t('mentions: who gets pinged is exactly who the message highlights', () => {
  const highlighted = (body) => [...new Set([...renderBody(body, PEOPLE.map((p) => p.name)).matchAll(/<span class="mention">@([^<]+)<\/span>/g)]
    .map((m) => PEOPLE.find((p) => p.name.toLowerCase() === m[1].toLowerCase())?.id))];
  for (const [body, want] of MENTIONS) {
    assert.deepEqual(findMentions(body, PEOPLE).uids.sort(), [...want].sort(), body);
    assert.deepEqual(highlighted(body).sort(), [...want].sort(), `highlight: ${body}`);
  }
  for (const [body, everyone] of [['@everyone game time', true], ['@here?', true], ['me@everyone.com', false], ['`@everyone`', false], ['@everyones', false]]) {
    assert.equal(findMentions(body, PEOPLE).everyone, everyone, body);
  }
});

t('the invite key rides in the socket’s subprotocol, or in its address only when it can’t', () => {
  globalThis.location ??= { protocol: 'https:', host: 'chat.example' };
  assert.deepEqual([keyProtocols('Ab-9_x'), socketUrl('Ab-9_x')], [['yap', 'key.Ab-9_x'], 'wss://chat.example/ws']);
  assert.deepEqual([keyProtocols('a+b/c='), socketUrl('a+b/c=')], [['yap'], 'wss://chat.example/ws?key=a%2Bb%2Fc%3D']);
});

/** Every file under a folder of the repo with this extension, as one text. */
const sources = (dir, ext) => fs.readdirSync(new URL(dir, import.meta.url), { recursive: true })
  .filter((f) => f.endsWith(ext)).sort().map((f) => fs.readFileSync(new URL(`${dir}${f}`, import.meta.url), 'utf8')).join('\n');

t('the mod log has words for every action the server records', () => {
  const server = sources('../worker/', '.ts');
  const logged = new Set([...server.matchAll(/\b(?:this|hub)\.log\([^\n]*/g)].flatMap(([call]) => [...call.matchAll(/'([a-z]+\.[A-Za-z]+)'/g)].map((m) => m[1])));
  const app = sources('../web/js/', '.js');
  const words = app.slice(app.indexOf('const LOG_WORDS = {'), app.indexOf('};', app.indexOf('const LOG_WORDS = {')));
  const worded = new Set([...words.matchAll(/^\s*'([a-z]+\.[A-Za-z]+)':/gm)].map((m) => m[1]));
  assert.ok(logged.size >= 20, `found the server's log calls (${logged.size})`);
  assert.deepEqual([...logged].filter((a) => !worded.has(a)), [], 'shown raw in Settings › Owner & mods');
});

t('every error code the server sends is in PROTOCOL.md, and nothing else is', () => {
  const server = sources('../worker/', '.ts');
  const sent = new Set([...server.matchAll(/new ClientError\('([a-z_]+)'/g), ...server.matchAll(/t: 'error', code: '([a-z_]+)'/g)].map((m) => m[1]));
  const doc = fs.readFileSync(new URL('../PROTOCOL.md', import.meta.url), 'utf8');
  const listed = new Set(/Codes: `([^`]+)`/.exec(doc)[1].split(/\s+/));
  assert.ok(sent.size >= 40, `found the server's codes (${sent.size})`);
  assert.deepEqual([...sent].filter((c) => !listed.has(c)).sort(), [], 'sent, but not in PROTOCOL.md › Server → client › error');
  assert.deepEqual([...listed].filter((c) => !sent.has(c)).sort(), [], 'in PROTOCOL.md, but never sent');
});

t('the stylesheet only uses colours and sizes it defines (or that the page sets on an element)', () => {
  const web = new URL('../web/', import.meta.url);
  const pages = [...fs.readdirSync(new URL('js/', web), { recursive: true }).filter((f) => f.endsWith('.js')).map((f) => new URL(`js/${f}`, web)), ...fs.readdirSync(web).filter((f) => f.endsWith('.html')).map((f) => new URL(f, web))];
  const setByPage = new Set(pages.flatMap((f) => [...fs.readFileSync(f, 'utf8').matchAll(/(--[a-z][\w-]*)\s*[:'"]/g)].map((m) => m[1])));
  const css = fs.readFileSync(new URL('style.css', web), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
  const missing = [...new Set([...css.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]))].filter((v) => !defined.has(v) && !setByPage.has(v));
  assert.deepEqual(missing, [], 'style.css uses tokens it never defines (their fallback always wins)');
});

t('Opus tuning: our fmtp asks for our bitrate, FEC, no DTX, full band; stereo send only in music mode; other codecs untouched', () => {
  const sdp = (opusFmtp) => [
    'v=0', 'm=audio 9 UDP/TLS/RTP/SAVPF 111 63 0', 'a=rtpmap:111 opus/48000/2', ...(opusFmtp ? [`a=fmtp:111 ${opusFmtp}`] : []),
    'a=rtpmap:63 red/48000/2', 'a=fmtp:63 111/111', 'a=rtpmap:0 PCMU/8000', '',
  ].join('\r\n');
  const fmtp = (text, pt) => Object.fromEntries(text.match(new RegExp(`^a=fmtp:${pt} (.*)$`, 'm'))[1].split(';').map((kv) => kv.split('=')));
  Object.assign(settings, { bitrate: 96000, music: false });
  const tuned = tuneOpus(sdp('minptime=10;useinbandfec=0;custom=7'));
  assert.deepEqual(fmtp(tuned, 111), {
    minptime: '10', useinbandfec: '1', custom: '7', usedtx: '0', stereo: '1', 'sprop-stereo': '0',
    maxaveragebitrate: '96000', maxplaybackrate: '48000', 'sprop-maxcapturerate': '48000',
  });
  assert.equal(fmtp(tuned, 63)['111/111'], undefined, 'RED keeps its own line');
  assert.match(tuned, /^a=fmtp:63 111\/111$/m);
  settings.music = true;
  assert.equal(fmtp(tuneOpus(sdp('')), 111)['sprop-stereo'], '1', 'no fmtp line yet: one is added');
  assert.match(tuneOpus(sdp('')), /a=rtpmap:111 opus\/48000\/2\r\na=fmtp:111 /);
  const noOpus = 'v=0\r\nm=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n';
  assert.equal(tuneOpus(noOpus), noOpus);
  Object.assign(settings, { bitrate: 128000, music: false });
});

t('lower shared media while friends talk: to 50 % over ~150 ms, back over ~600 ms after 1 s of quiet; off = untouched', () => {
  const d = new Ducker();
  const run = (from, to, talking, on = true, step = 10) => {
    let f;
    for (let now = from; now <= to; now += step) f = d.step(now, talking, on);
    return f;
  };
  assert.equal(run(0, 1000, false), 1, 'nobody talking: your volume');
  assert.ok(Math.abs(run(1010, 1080, true) - 0.75) < 0.05, 'halfway down after ~75 ms');
  assert.equal(run(1090, 1160, true), DUCK.level, 'at 50 % within ~150 ms');
  assert.equal(run(1170, 3000, true), 0.5, 'and stays there while they talk');
  assert.equal(run(3010, 3990, false), 0.5, 'still down in the first second of quiet');
  const half = run(4000, 4300, false);
  assert.ok(half > 0.7 && half < 0.8, `on its way back after ~300 ms (${half.toFixed(2)})`);
  assert.equal(run(4310, 4700, false), 1, 'back to your volume within ~600 ms');
  assert.equal(run(4710, 6000, true, false), 1, 'switched off: friends talking changes nothing');
  run(6010, 6500, true);
  assert.equal(d.factor, 0.5);
  assert.equal(run(6510, 7200, true, false), 1, 'switching it off (or leaving the call) brings it back');
});

t('Watch together: a dip while friends talk never becomes "your volume"; what you set in the player during one is your volume over the dip', () => {
  const sent = [];
  const saved = [];
  const party = Object.assign(Object.create(WatchParty.prototype), {
    prefs: { volume: 70, muted: false }, duck: 1, sentVolume: null, sentMuted: null, volAt: -Infinity, touchedAt: -Infinity,
    frame: {}, player: { ready: true },
    cmd: (func, args = []) => sent.push([func, ...args]), savePrefs() { saved.push({ ...this.prefs }); }, renderTransport() {},
  });
  party.applyVolume();
  assert.deepEqual(sent, [['setVolume', 70], ['unMute']]);
  party.setDuck(0.5);
  assert.deepEqual(sent.at(-1), ['setVolume', 35], 'the player goes to half');
  party.setDuck(0.5);
  assert.equal(sent.length, 3, 'nothing again for the same volume');
  // The player tells us its volume much later, with the person's focus in it: still our dip, not theirs.
  globalThis.document = { activeElement: party.frame };
  party.volAt = performance.now() - 10_000;
  party.onPlayerVolume({ volume: 35, muted: false });
  assert.deepEqual(party.prefs, { volume: 70, muted: false }, 'the dip was not saved as your volume');
  assert.equal(saved.length, 0);
  // They turn YouTube's own slider to 30 during the dip: that's 60 % of theirs once the dip ends.
  party.onPlayerVolume({ volume: 30, muted: false });
  assert.deepEqual(party.prefs, { volume: 60, muted: false });
  party.setDuck(1);
  assert.deepEqual(sent.at(-1), ['setVolume', 60], 'back up to what they set');
  party.volAt = performance.now() - 10_000;
  party.onPlayerVolume({ volume: 60, muted: true });
  assert.deepEqual(party.prefs, { volume: 60, muted: true }, 'muting in the player is theirs too');
  delete globalThis.document;
});

let failed = 0;
for (const [name, fn] of cases) {
  try {
    fn();
    console.log(`✓ ${name}`);
  } catch (err) {
    failed++;
    console.log(`✗ ${name}\n    ${err.message}`);
  }
}
console.log(`${cases.length - failed}/${cases.length} passed`);
process.exit(failed ? 1 : 0);
