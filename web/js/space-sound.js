// Space's instruments, synthesized with WebAudio (no files): a floor piano and
// four drum pads. Quiet on purpose, and off when Settings › Sounds is off.
import { settings } from './settings.js';

let ctx = null;
let noise = null;
let out = null;

function audio() {
  if (!settings.sounds) return null;
  try {
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') ctx.resume();
    if (!out) {
      out = ctx.createGain();
      out.gain.value = 0.22;
      out.connect(ctx.destination);
    }
    return ctx;
  } catch {
    return null;
  }
}

function noiseBuffer(ac) {
  if (noise) return noise;
  noise = ac.createBuffer(1, ac.sampleRate * 0.4, ac.sampleRate);
  const data = noise.getChannelData(0);
  for (let i = 0; i < data.length; i++) data[i] = Math.random() * 2 - 1;
  return noise;
}

/** A soft, piano-ish note: a triangle plus a quieter octave, quick attack, long fade. */
export function playNote(midi) {
  const ac = audio();
  if (!ac) return;
  const t = ac.currentTime + 0.005;
  const f = 440 * 2 ** ((midi - 69) / 12);
  const env = ac.createGain();
  env.gain.setValueAtTime(0, t);
  env.gain.linearRampToValueAtTime(0.9, t + 0.008);
  env.gain.exponentialRampToValueAtTime(0.25, t + 0.25);
  env.gain.exponentialRampToValueAtTime(0.0001, t + 1.4);
  env.connect(out);
  for (const [type, mult, level] of [['triangle', 1, 1], ['sine', 2, 0.3]]) {
    const osc = ac.createOscillator();
    const g = ac.createGain();
    osc.type = type;
    osc.frequency.value = f * mult;
    g.gain.value = level;
    osc.connect(g).connect(env);
    osc.start(t);
    osc.stop(t + 1.45);
  }
}

/** kick, snare, tom or hat. */
export function playDrum(kind) {
  const ac = audio();
  if (!ac) return;
  const t = ac.currentTime + 0.005;
  const env = ac.createGain();
  env.connect(out);
  if (kind === 'kick' || kind === 'tom') {
    const [from, to, len] = kind === 'kick' ? [150, 42, 0.38] : [230, 110, 0.4];
    const osc = ac.createOscillator();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(from, t);
    osc.frequency.exponentialRampToValueAtTime(to, t + len * 0.7);
    env.gain.setValueAtTime(1.2, t);
    env.gain.exponentialRampToValueAtTime(0.0001, t + len);
    osc.connect(env);
    osc.start(t);
    osc.stop(t + len + 0.02);
    return;
  }
  const src = ac.createBufferSource();
  src.buffer = noiseBuffer(ac);
  const filter = ac.createBiquadFilter();
  filter.type = 'highpass';
  filter.frequency.value = kind === 'hat' ? 7000 : 1400;
  const len = kind === 'hat' ? 0.07 : 0.2;
  env.gain.setValueAtTime(kind === 'hat' ? 0.5 : 0.8, t);
  env.gain.exponentialRampToValueAtTime(0.0001, t + len);
  src.connect(filter).connect(env);
  src.start(t);
  src.stop(t + len + 0.02);
  if (kind === 'snare') {
    // the drum's body under the rattle
    const osc = ac.createOscillator();
    const g = ac.createGain();
    osc.type = 'triangle';
    osc.frequency.value = 190;
    g.gain.setValueAtTime(0.5, t);
    g.gain.exponentialRampToValueAtTime(0.0001, t + 0.12);
    osc.connect(g).connect(out);
    osc.start(t);
    osc.stop(t + 0.14);
  }
}
