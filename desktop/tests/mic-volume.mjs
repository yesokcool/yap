// Mic volume end to end: two `yap --headless` in `lounge`, the whole call
// path in between (automatic gain, Mic volume, the gate, Opus, jitter
// buffers). Dan talks (a speech-like WAV) at Mic volume 100 %, 50 % and
// 200 %; Cleo records what she hears each time. 50 % should come out 6 dB
// quieter than 100 %, 200 % about 6 dB louder (its limiter only touches the
// loudest peaks), with nothing past full scale.
//
//   YAP_URL=http://localhost:8788 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
//     YAP_BIN=desktop/target/release/yap node desktop/tests/mic-volume.mjs
//
// Needs the local test server (desktop/README.md › Interop test). A doorman
// (--approve-all; the owner, or made a mod: only they let people in) lets
// the two in. Exits non-zero if a check fails; the numbers go to
// $YAP_OUT/mic-volume.json.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const BASE = process.env.YAP_URL ?? 'http://localhost:8788';
const KEY = process.env.YAP_KEY ?? '';
const ADMIN = process.env.YAP_ADMIN_KEY ?? 'testadmin';
const BIN = process.env.YAP_BIN ?? 'desktop/target/release/yap';
const OUT = process.env.YAP_OUT ?? fs.mkdtempSync(path.join(os.tmpdir(), 'yap-micvol-'));
const PASSWORD = 'correct-horse-582039';
const RATE = 48000;
const RUN = Date.now() % 100000;
fs.mkdirSync(OUT, { recursive: true });

/** 16-bit mono WAV. */
function writeWav(file, samples) {
  const data = Buffer.alloc(samples.length * 2);
  samples.forEach((s, i) => data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, s)) * 32767), i * 2));
  const h = Buffer.alloc(44);
  h.write('RIFF', 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write('WAVEfmt ', 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(RATE, 24);
  h.writeUInt32LE(RATE * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write('data', 36);
  h.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([h, data]));
}

/** RMS per second (dBFS) and the peak of a 16-bit PCM WAV, all channels. */
function levels(file) {
  const b = fs.readFileSync(file);
  const ch = b.readUInt16LE(22);
  const start = b.indexOf('data') + 8;
  const n = Math.floor((b.length - start) / 2 / ch);
  const out = [];
  let peak = 0;
  for (let s = 0; (s + 1) * RATE <= n; s++) {
    let sum = 0;
    for (let i = s * RATE; i < (s + 1) * RATE; i++) {
      for (let c = 0; c < ch; c++) {
        const v = b.readInt16LE(start + (i * ch + c) * 2) / 32768;
        sum += v * v;
        peak = Math.max(peak, Math.abs(v));
      }
    }
    out.push(sum > 0 ? 10 * Math.log10(sum / RATE / ch) : -120);
  }
  return { perSecond: out, peak };
}

// Dan: syllables of a pitched voice (harmonics up to 8 kHz), talking all the way through.
let seed = 11;
const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
const talk = new Float32Array(12 * RATE);
for (let i = 0, phase = 0; i < talk.length; ) {
  const syl = Math.floor((0.12 + 0.16 * rnd()) * RATE);
  const gap = Math.floor((0.03 + 0.05 * rnd()) * RATE);
  const f0 = 140 * (0.85 + 0.35 * rnd());
  const amps = Array.from({ length: 24 }, (_, h) => (0.3 + 0.7 * rnd()) / (h + 1) ** 0.9);
  for (let k = 0; k < syl + gap && i + k < talk.length; k++) {
    const t = k / syl;
    const env = k < syl ? Math.min(t * 8, 1) * (1 - t) ** 0.6 : 0;
    phase += f0 / RATE;
    let s = 0;
    for (let h = 1; h <= 24 && h * f0 < 8000; h++) s += amps[h - 1] * Math.sin(2 * Math.PI * h * phase);
    talk[i + k] = 0.06 * env * s;
  }
  i += syl + gap;
}
const danWav = path.join(OUT, 'dan.wav');
const quietWav = path.join(OUT, 'quiet.wav');
writeWav(danWav, talk);
writeWav(quietWav, new Float32Array(RATE));

function bot(name, extra) {
  const args = ['--headless', '--server', BASE, '--name', name, '--password', PASSWORD, ...extra];
  if (KEY) args.push('--key', KEY);
  const child = spawn(BIN, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  const lines = [];
  let buf = '';
  const onData = (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      lines.push(buf.slice(0, i));
      buf = buf.slice(i + 1);
    }
  };
  child.stdout.on('data', onData);
  child.stderr.on('data', onData);
  const done = new Promise((resolve) => child.on('exit', () => {
    fs.writeFileSync(path.join(OUT, `${name}.log`), lines.join('\n') + '\n');
    resolve();
  }));
  const waitFor = async (re, ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (lines.some((l) => re.test(l))) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`${name} never logged ${re}`);
  };
  return { child, lines, done, waitFor };
}

const doorman = bot(process.env.YAP_DOORMAN_NAME ?? 'Doorman', ['--approve-all', '--input-wav', quietWav, '--duration', '200']);
const makeMod = (uid) => fetch(`${BASE}/api/admin/role?uid=${uid}&role=mod`, { method: 'POST', headers: { Authorization: `Bearer ${ADMIN}`, 'X-Yap-Key': KEY } });
await doorman.waitFor(/ready, sid|PENDING uid/);
const pending = doorman.lines.map((l) => l.match(/PENDING uid (\S+)/)?.[1]).find(Boolean);
if (pending) await makeMod(pending);
await doorman.waitFor(/ACCOUNT uid=/);
// Left over as a plain member from an older run: only the owner and mods let people in.
const account = doorman.lines.map((l) => l.match(/ACCOUNT uid=(\S+) .* role=(\S+)/)).find(Boolean);
if (account && !['owner', 'mod'].includes(account[2])) await makeMod(account[1]);

const result = {};
for (const pct of [100, 50, 200]) {
  const heard = path.join(OUT, `heard-${pct}.wav`);
  const cleo = bot(`Cleo${RUN}v${pct}`, ['--input-wav', quietWav, '--join-voice', 'lounge', '--record-wav', heard, '--duration', '26']);
  await cleo.waitFor(/ready, sid/);
  const dan = bot(`Dan${RUN}v${pct}`, ['--input-wav', danWav, '--join-voice', 'lounge', '--mic-volume', String(pct), '--duration', '30']);
  await cleo.done;
  dan.child.kill();
  await dan.done;
  const { perSecond, peak } = levels(heard);
  // After the call (and the automatic gain) settle: the loudest 10 seconds.
  const loud = perSecond.slice(8).sort((a, b) => b - a).slice(0, 10);
  result[pct] = { loudest10sDbfs: +(loud.reduce((a, b) => a + b, 0) / loud.length).toFixed(2), peak: +peak.toFixed(3), perSecond: perSecond.map((v) => Math.round(v)) };
  console.log(`mic volume ${pct}%: Cleo hears Dan at ${result[pct].loudest10sDbfs} dBFS (peak ${result[pct].peak})`);
}
doorman.child.kill();
await doorman.done;
fs.writeFileSync(path.join(OUT, 'mic-volume.json'), JSON.stringify(result, null, 2) + '\n');

const half = result[50].loudest10sDbfs - result[100].loudest10sDbfs;
const double = result[200].loudest10sDbfs - result[100].loudest10sDbfs;
assert.ok(result[100].loudest10sDbfs > -40, `Cleo should hear Dan at 100 % (${result[100].loudest10sDbfs} dBFS)`);
assert.ok(Math.abs(half + 6) < 1, `50 % should be 6 dB quieter (${half.toFixed(2)} dB)`);
assert.ok(double > 4.5 && double < 6.5, `200 % should be about 6 dB louder (${double.toFixed(2)} dB)`);
// (The limiter holds −1 dBFS before Opus; the codec can overshoot that a little, never to the top.)
assert.ok(result[200].peak < 0.99, `200 % never reaches full scale (peak ${result[200].peak})`);
console.log(`✓ mic volume: 50 % → ${half.toFixed(2)} dB, 200 % → +${double.toFixed(2)} dB (${OUT})`);
