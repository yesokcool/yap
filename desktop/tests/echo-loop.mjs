// Echo cancellation end to end: two `yap --headless` in `lounge`, the whole
// call path in between (Opus, jitter buffers, real threads). Ari talks (a
// speech-like WAV) and records what it hears. Bea says nothing, but her mic
// hears her own speakers 60 ms later at −6 dB (YAP_DEBUG_ECHO_LOOP), the way a
// laptop on speakers does. Bea once without echo cancellation, once with it:
// Ari should hear himself back the first time and (almost) nothing the second.
//
//   YAP_URL=http://localhost:8788 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
//     YAP_BIN=desktop/target/release/yap node desktop/tests/echo-loop.mjs
//
// Needs the local test server (desktop/README.md › Interop test). A doorman
// (--approve-all; the owner, or made a mod: only they let people in) lets
// the two in. Exits non-zero if a check fails; the
// numbers go to $YAP_OUT/echo-loop.json.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const BASE = process.env.YAP_URL ?? 'http://localhost:8788';
const KEY = process.env.YAP_KEY ?? '';
const ADMIN = process.env.YAP_ADMIN_KEY ?? 'testadmin';
const BIN = process.env.YAP_BIN ?? 'desktop/target/release/yap';
const OUT = process.env.YAP_OUT ?? fs.mkdtempSync(path.join(os.tmpdir(), 'yap-echo-'));
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

/** RMS per second (dBFS) of a 16-bit PCM WAV, all channels averaged. */
function levels(file) {
  const b = fs.readFileSync(file);
  const ch = b.readUInt16LE(22);
  const start = b.indexOf('data') + 8;
  const n = Math.floor((b.length - start) / 2 / ch);
  const out = [];
  for (let s = 0; (s + 1) * RATE <= n; s++) {
    let sum = 0;
    for (let i = s * RATE; i < (s + 1) * RATE; i++) {
      const v = b.readInt16LE(start + i * ch * 2) / 32768;
      sum += v * v;
    }
    out.push(sum > 0 ? 10 * Math.log10(sum / RATE) : -120);
  }
  return out;
}

// Ari: syllables of a pitched voice (harmonics up to 8 kHz), talking 2–8 s and 10–16 s of every 18 s.
let seed = 7;
const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
const talk = new Float32Array(18 * RATE);
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
    const sec = (i + k) / RATE;
    talk[i + k] = (sec >= 2 && sec < 8) || (sec >= 10 && sec < 16) ? 0.06 * env * s : 0;
  }
  i += syl + gap;
}
const ariWav = path.join(OUT, 'ari.wav');
const quietWav = path.join(OUT, 'quiet.wav');
writeWav(ariWav, talk);
writeWav(quietWav, new Float32Array(RATE));

function bot(name, extra, env = {}) {
  const args = ['--headless', '--server', BASE, '--name', name, '--password', PASSWORD, ...extra];
  if (KEY) args.push('--key', KEY);
  const child = spawn(BIN, args, { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, ...env } });
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
for (const mode of ['off', 'on']) {
  const bea = bot(`Bea${RUN}${mode}`, ['--input-wav', quietWav, '--join-voice', 'lounge', '--duration', '45', ...(mode === 'on' ? ['--echo-cancel'] : [])], { YAP_DEBUG_ECHO_LOOP: '60,-6' });
  await bea.waitFor(/ready, sid/);
  const heard = path.join(OUT, `heard-${mode}.wav`);
  const ari = bot(`Ari${RUN}${mode}`, ['--input-wav', ariWav, '--join-voice', 'lounge', '--record-wav', heard, '--duration', '32']);
  await ari.done;
  bea.child.kill();
  await bea.done;
  const lv = levels(heard);
  // After the call settles (from 6 s): the loudest 10 seconds.
  const loud = lv.slice(6).sort((a, b) => b - a).slice(0, 10);
  const aec = bea.lines.filter((l) => l.includes(' aec=')).pop()?.replace(/.*\| aec=/, 'aec=');
  result[mode] = { loudest10sDbfs: +(loud.reduce((a, b) => a + b, 0) / loud.length).toFixed(1), perSecond: lv.map((v) => Math.round(v)), canceller: aec ?? null };
  console.log(`echo cancellation ${mode}: Ari hears himself at ${result[mode].loudest10sDbfs} dBFS${aec ? ` (Bea: ${aec})` : ''}`);
}
doorman.child.kill();
await doorman.done;
fs.writeFileSync(path.join(OUT, 'echo-loop.json'), JSON.stringify(result, null, 2) + '\n');

assert.ok(result.off.loudest10sDbfs > -40, `without echo cancellation Ari should hear himself (${result.off.loudest10sDbfs} dBFS)`);
assert.ok(result.on.loudest10sDbfs < result.off.loudest10sDbfs - 30, `echo cancellation should take at least 30 dB off (${result.on.loudest10sDbfs} vs ${result.off.loudest10sDbfs} dBFS)`);
assert.match(result.on.canceller ?? '', /aec=\d+ms/, 'Bea reports the canceller and its delay estimate');
console.log(`✓ echo loop: ${(result.off.loudest10sDbfs - result.on.loudest10sDbfs).toFixed(1)} dB less of himself (${OUT})`);
