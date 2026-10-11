// The extras' pure rules (worker/extras.ts), no server needed.
//   node --experimental-strip-types --no-warnings test/extras.test.mjs
import assert from 'node:assert/strict';
import { checkWav, cleanAway, cleanPoll, cleanQuiet, doodleId, hideSpoilers, isQuietNow, rollDice } from '../worker/extras.ts';

const cases = [];
const t = (name, fn) => cases.push([name, fn]);
const code = (fn) => {
  try {
    fn();
  } catch (err) {
    return err.code;
  }
  return null;
};

t('dice: every spec, within bounds, and the body says what was rolled', () => {
  for (let i = 0; i < 200; i++) {
    const r = rollDice('/roll').roll;
    assert.ok(r.total >= 1 && r.total <= 100);
    const d = rollDice('/roll 3d6-2');
    assert.equal(d.roll.rolls.length, 3);
    assert.ok(d.roll.rolls.every((x) => x >= 1 && x <= 6));
    assert.equal(d.roll.total, d.roll.rolls.reduce((a, b) => a + b) - 2);
    assert.equal(d.body, `🎲 3d6-2: ${d.roll.rolls.join(' + ')} − 2 = ${d.roll.total}`);
  }
  assert.match(rollDice('/roll d20').body, /^🎲 1d20: \d+$/);
  assert.match(rollDice('/ROLL 6').body, /^🎲 1–6: [1-6]$/);
  assert.ok(['🪙 Heads', '🪙 Tails'].includes(rollDice('/flip').body));
  assert.equal(rollDice('/roll the dice'), null, 'a sentence is not a command');
  assert.equal(rollDice('hello /roll'), null);
  for (const bad of ['/roll 1', '/roll 21d6', '/roll 2d1', '/roll 2d6+5000', '/roll x', '/flip 3']) assert.equal(code(() => rollDice(bad)), 'bad_roll', bad);
});

t('dice are fair enough (each face of a d6 within 20 % of expected over 6,000 rolls)', () => {
  const counts = [0, 0, 0, 0, 0, 0];
  for (let i = 0; i < 6000; i++) counts[rollDice('/roll 6').roll.total - 1]++;
  assert.ok(counts.every((n) => n > 800 && n < 1200), counts.join(' '));
});

t('polls: 2–10 distinct trimmed options', () => {
  assert.deepEqual(cleanPoll({ options: [' a ', 'b'] }), { options: ['a', 'b'] });
  for (const bad of [{ options: ['a'] }, { options: ['a', 'A'] }, { options: ['a', ' '] }, { options: Array.from({ length: 11 }, (_, i) => `o${i}`) }, null, { options: 'a,b' }]) {
    assert.equal(code(() => cleanPoll(bad)), 'bad_poll', JSON.stringify(bad));
  }
});

t('quiet hours: windows (wrapping too), snooze, mentions are the caller’s call, bad zones refused', () => {
  const at = (h, m = 0) => Date.UTC(2026, 9, 6, h, m);
  const night = { from: 23 * 60, to: 8 * 60, tz: 'UTC', mentions: false };
  assert.equal(isQuietNow(night, at(23, 30)), true);
  assert.equal(isQuietNow(night, at(3)), true);
  assert.equal(isQuietNow(night, at(8)), false, 'the end is exclusive');
  assert.equal(isQuietNow(night, at(12)), false);
  const lunch = { from: 12 * 60, to: 13 * 60, tz: 'Europe/Rome', mentions: false }; // 10:00–11:00 UTC in October (CEST)
  assert.equal(isQuietNow(lunch, at(10, 30)), true);
  assert.equal(isQuietNow(lunch, at(12, 30)), false);
  assert.equal(isQuietNow({ from: 0, to: 0, tz: 'UTC', mentions: false, until: at(9) }, at(8)), true, 'snoozed');
  assert.equal(isQuietNow({ from: 0, to: 0, tz: 'UTC', mentions: false, until: at(9) }, at(10)), false, 'snooze over');
  assert.equal(code(() => cleanQuiet({ from: 0, to: 60, tz: 'Mars/Olympus' })), 'bad_quiet');
  assert.equal(code(() => cleanQuiet({ from: 1440, to: 60, tz: 'UTC' })), 'bad_quiet');
  assert.equal(code(() => cleanQuiet({ from: 0, to: 60, tz: 'UTC', until: Date.now() + 8 * 86_400_000 })), 'bad_quiet', 'a week at most');
  assert.deepEqual(cleanQuiet({ from: 60, to: 120, tz: 'Asia/Tokyo', mentions: 'yes' }), { from: 60, to: 120, tz: 'Asia/Tokyo', mentions: false, until: null });
});

t('away: 1–80 characters, ending within a week or "until I’m back"', () => {
  const a = cleanAway({ text: '  dinner   🍝 ', until: null });
  assert.deepEqual([a.text, a.until, typeof a.since], ['dinner 🍝', null, 'number']);
  assert.equal(cleanAway({ text: 'x'.repeat(200), until: null }).text.length, 80);
  for (const bad of [{ text: '', until: null }, { text: 'a', until: Date.now() - 1 }, { text: 'a', until: Date.now() + 8 * 86_400_000 }]) {
    assert.equal(code(() => cleanAway(bad)), 'bad_away', JSON.stringify(bad));
  }
});

t('spoilers and doodle tokens', () => {
  assert.equal(hideSpoilers('a ||b|| c ||d e|| f ||g'), 'a ▒▒▒ c ▒▒▒ f ||g');
  assert.equal(hideSpoilers('||x\ny||'), '||x\ny||', 'not across lines');
  assert.equal(doodleId(`<d:${'a'.repeat(32)}>`), 'a'.repeat(32));
  assert.equal(doodleId('<d:xyz>'), null);
  assert.equal(doodleId('<:name:' + 'a'.repeat(32) + '>'), null);
});

t('clips: only 16-bit mono PCM WAV, length checked, loud ones capped, quiet ones never boosted', () => {
  const wav = (ms, amp, { rate = 24000, channels = 1, bits = 16 } = {}) => {
    const n = Math.round((rate * ms) / 1000);
    const b = Buffer.alloc(44 + n * 2);
    b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(channels, 22);
    b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(bits, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40);
    for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.sin((i * 2 * Math.PI * 440) / rate) * amp * 32767), 44 + i * 2);
    return new Uint8Array(b);
  };
  const loud = checkWav(wav(1000, 1), 5000);
  assert.equal(loud.ms, 1000);
  assert.ok(loud.gain > 0.3 && loud.gain < 0.4, `full-scale sine: gain ${loud.gain}`);
  assert.equal(checkWav(wav(1000, 0.05), 5000).gain, 1, 'quiet clips are never made louder');
  assert.equal(typeof checkWav(wav(6000, 0.5), 5000), 'string', 'too long');
  assert.equal(typeof checkWav(wav(500, 0), 5000), 'string', 'silent');
  assert.equal(typeof checkWav(wav(500, 0.5, { channels: 2 }), 5000), 'string', 'stereo');
  assert.equal(typeof checkWav(new Uint8Array(100), 5000), 'string', 'not a WAV');
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
