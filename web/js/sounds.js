// Little synthesized blips, so there are no audio files to ship — plus join
// chimes and the soundboard (PROTOCOL.md › Extras). Everything goes through
// one limiter, so nothing can blast anyone's ears.
import { settings } from './settings.js';

let ctx;
let bus;

/** The shared output: a limiter in front of the speakers (your chosen output where the browser allows). */
function out() {
  ctx ??= new AudioContext();
  ctx.resume();
  if (!bus) {
    bus = ctx.createDynamicsCompressor();
    bus.threshold.value = -12;
    bus.knee.value = 0;
    bus.ratio.value = 20;
    bus.attack.value = 0.003;
    bus.release.value = 0.25;
    bus.connect(ctx.destination);
  }
  if (ctx.setSinkId && (ctx.sinkId || '') !== (settings.outputId || '')) ctx.setSinkId(settings.outputId || '').catch(() => {});
  return { ctx, bus };
}

function play(notes, { step = 0.075, length = 0.11, volume = 0.07, type = 'sine' } = {}, force = false) {
  if (!settings.sounds && !force) return;
  try {
    const { ctx, bus } = out();
    const start = ctx.currentTime + 0.01;
    notes.forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      const t = start + i * step;
      osc.type = type;
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(volume, t + 0.012);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + length);
      osc.connect(gain).connect(bus);
      osc.start(t);
      osc.stop(t + length + 0.02);
    });
  } catch {}
}

export const sounds = {
  join: () => play([523.25, 659.25, 783.99]),
  leave: () => play([783.99, 659.25, 523.25]),
  peerJoin: () => play([659.25, 880], { volume: 0.05 }),
  peerLeave: () => play([880, 659.25], { volume: 0.05 }),
  mute: () => play([392], { length: 0.08, volume: 0.05, type: 'triangle' }),
  ping: () => play([783.99, 1046.5, 1318.5, 1567.98], { step: 0.09, length: 0.22, volume: 0.13, type: 'square' }),
  mention: () => play([987.77, 1318.5], { step: 0.08, length: 0.16, volume: 0.09, type: 'triangle' }),
  unmute: () => play([587.33], { length: 0.08, volume: 0.05, type: 'triangle' }),
  // Push-to-talk opening and closing: a quiet tick each way, for your ears only (its own setting).
  pttOn: () => play([1318.51], { length: 0.03, volume: 0.035, type: 'triangle' }, true),
  pttOff: () => play([987.77], { length: 0.03, volume: 0.03, type: 'triangle' }, true),
  // Talking while muted: soft and low, two quick notes (its own setting, so it plays even with the others off).
  mutedTalk: () => play([329.63, 329.63], { step: 0.11, length: 0.07, volume: 0.035, type: 'sine' }, true),
};

/** Built-in join chimes: identical notes in every Yap client (PROTOCOL.md › Join chimes). */
export const CHIMES = {
  default: { label: 'Classic blip', notes: [659.25, 880], step: 0.075, length: 0.11, volume: 0.05, type: 'sine' },
  ding: { label: 'Ding', notes: [1318.51], step: 0, length: 0.5, volume: 0.08, type: 'sine' },
  chord: { label: 'Chord', notes: [523.25, 659.25, 783.99], step: 0, length: 0.6, volume: 0.05, type: 'triangle' },
  tada: { label: 'Ta-da', notes: [392, 523.25, 659.25, 783.99, 1046.5], step: 0.07, length: 0.25, volume: 0.06, type: 'triangle' },
  bubble: { label: 'Bubble', notes: [440, 660, 990], step: 0.04, length: 0.08, volume: 0.06, type: 'sine' },
  knock: { label: 'Knock knock', notes: [160, 160], step: 0.16, length: 0.09, volume: 0.14, type: 'sine' },
  arcade: { label: 'Arcade', notes: [523.25, 783.99, 1046.5, 1567.98], step: 0.05, length: 0.09, volume: 0.04, type: 'square' },
  none: { label: 'Silent', notes: [] },
};

/**
 * Someone's join chime: a built-in id, or `s:<id>` of a clip they uploaded
 * (`sound` = that clip, if we know it). `preview` plays even with sounds off.
 */
export function playChime(chime, sound, { preview = false } = {}) {
  if (!preview && (!settings.sounds || settings.deafened)) return;
  if (chime?.startsWith('s:')) {
    if (sound) playClip(sound, { force: true });
    else sounds.peerJoin();
    return;
  }
  const c = CHIMES[chime] ?? CHIMES.default;
  if (c.notes.length) play(c.notes, c, preview);
}

// ── Soundboard clips ──────────────────────────────────────────────────────
// The volume limit, in four layers: the server measured each clip and capped
// its `gain` (never louder than −12 dBFS RMS / −3 dBFS peak); you add your own
// soundboard volume on top (default 50 %, never above the clip's gain); the
// limiter catches anything that still peaks; and at most two clips overlap.

const decoded = new Map(); // media id → Promise<AudioBuffer>, the most recently played last
/** Decoded clips kept ready (each ~1 MB once decoded); the least recently played go first, and decode again if played again. */
const KEEP_DECODED = 32;
const playing = [];

function bufferFor(media) {
  let buffer = decoded.get(media);
  if (buffer) decoded.delete(media); // (re-added below: now the most recent)
  else {
    buffer = fetch(`/media/${media}`)
      .then((res) => (res.ok ? res.arrayBuffer() : Promise.reject(new Error(`HTTP ${res.status}`))))
      .then((bytes) => out().ctx.decodeAudioData(bytes))
      .catch((err) => {
        if (decoded.get(media) === buffer) decoded.delete(media);
        throw err;
      });
  }
  decoded.set(media, buffer);
  while (decoded.size > KEEP_DECODED) decoded.delete(decoded.keys().next().value);
  return buffer;
}

/** Play a clip (`{ media, gain }`). `force` ignores the mute switch (previews). */
export async function playClip(sound, { force = false, volume = settings.soundboardVolume } = {}) {
  if (!force && (settings.soundboardMuted || settings.deafened)) return;
  try {
    const buffer = await bufferFor(sound.media);
    const { ctx, bus } = out();
    while (playing.length >= 2) {
      try {
        playing.shift().stop();
      } catch {}
    }
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    const gain = ctx.createGain();
    gain.gain.value = Math.min(sound.gain ?? 1, (sound.gain ?? 1) * Math.max(0, Math.min(1, volume)));
    source.connect(gain).connect(bus);
    source.onended = () => {
      const i = playing.indexOf(source);
      if (i >= 0) playing.splice(i, 1);
    };
    playing.push(source);
    source.start();
  } catch {}
}

/** Stop every clip (deafening, leaving voice). */
export function stopClips() {
  for (const s of playing.splice(0)) {
    try {
      s.stop();
    } catch {}
  }
}

// ── Making a clip to upload ───────────────────────────────────────────────

/**
 * Any audio file the browser can decode → a WAV the server accepts: mono,
 * 24 kHz, 16-bit, leading silence trimmed, at most `maxMs` long.
 */
export async function makeClip(file, maxMs) {
  const { ctx } = out();
  const source = await ctx.decodeAudioData(await file.arrayBuffer());
  const rate = 24000;
  const offline = new OfflineAudioContext(1, Math.max(1, Math.ceil(source.duration * rate)), rate);
  const node = offline.createBufferSource();
  node.buffer = source; // the offline context mixes down to mono and resamples
  node.connect(offline.destination);
  node.start();
  const mono = (await offline.startRendering()).getChannelData(0);
  let start = 0;
  while (start < mono.length && Math.abs(mono[start]) < 0.01) start++;
  start = Math.max(0, start - Math.round(rate * 0.01));
  const end = Math.min(mono.length, start + Math.floor((rate * maxMs) / 1000));
  if (end - start < rate * 0.05) throw new Error('That clip is silent (or too short).');
  return { blob: wav(mono.subarray(start, end), rate), ms: Math.round(((end - start) / rate) * 1000), cut: mono.length - start > end - start + rate * 0.05 };
}

function wav(samples, rate) {
  const buf = new ArrayBuffer(44 + samples.length * 2);
  const v = new DataView(buf);
  const text = (at, s) => [...s].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  text(0, 'RIFF');
  v.setUint32(4, 36 + samples.length * 2, true);
  text(8, 'WAVEfmt ');
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  text(36, 'data');
  v.setUint32(40, samples.length * 2, true);
  samples.forEach((s, i) => v.setInt16(44 + i * 2, Math.round(Math.max(-1, Math.min(1, s)) * 32767), true));
  return new Blob([buf], { type: 'audio/wav' });
}
