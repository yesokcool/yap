// A synthetic mic and screen sound for the e2e tests, as a Playwright init
// script (test/e2e.test.mjs uses it when YAP_E2E_SYNTH_AUDIO=1, or by itself
// when headless Chromium can't open audio capture on this machine — on some
// macOS builds even its fake mic hangs forever).
//
// getUserMedia({audio}) → a WebAudio track playing the same 12 s of chords +
// noise the fake mic plays from the test WAV (so Opus has something to spend
// bits on). getDisplayMedia keeps the real (fake) screen video but never asks
// for audio capture; when audio was asked for, a stereo WebAudio track stands
// in for the tab's sound. Each track comes from its own AudioContext, separate
// from the page's, so suspending Yap's audio graph doesn't silence the "mic".
(() => {
  if (window !== window.top || !location.origin.startsWith('http') || !navigator.mediaDevices) return;
  const SR = 48000;
  let buffer = null;
  /** The test signal (test/e2e.test.mjs › writeTestWav), made once per page. */
  function signal(ctx) {
    if (buffer) return buffer;
    const n = SR * 12;
    buffer = ctx.createBuffer(1, n, SR);
    const data = buffer.getChannelData(0);
    let seed = 1;
    const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const chord = Math.floor(t) % 2 ? [196, 246.94, 293.66] : [220, 277.18, 329.63];
      let v = chord.reduce((s, f) => s + Math.sin(2 * Math.PI * f * t) * 0.18, 0) + rnd() * 0.12;
      v *= 0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t);
      data[i] = Math.max(-1, Math.min(1, v)) * (30000 / 32768);
    }
    return buffer;
  }
  function micTrack() {
    const ctx = new AudioContext({ sampleRate: SR });
    const source = ctx.createBufferSource();
    source.buffer = signal(ctx);
    source.loop = true;
    const dest = ctx.createMediaStreamDestination();
    dest.channelCount = 1;
    source.connect(dest);
    source.start();
    ctx.resume().catch(() => {});
    const track = dest.stream.getAudioTracks()[0];
    track.addEventListener('ended', () => ctx.close().catch(() => {}));
    return track;
  }
  /** Stereo "tab sound": 440 Hz on the left only, 660 Hz on the right only. */
  function screenSoundTrack() {
    const ctx = new AudioContext({ sampleRate: SR });
    const merge = ctx.createChannelMerger(2);
    for (const [ch, f] of [[0, 440], [1, 660]]) {
      const o = ctx.createOscillator();
      o.frequency.value = f;
      const g = ctx.createGain();
      g.gain.value = 0.2;
      o.connect(g).connect(merge, 0, ch);
      o.start();
    }
    const dest = ctx.createMediaStreamDestination();
    dest.channelCount = 2;
    merge.connect(dest);
    ctx.resume().catch(() => {});
    return dest.stream.getAudioTracks()[0];
  }
  const md = navigator.mediaDevices;
  const realUserMedia = md.getUserMedia.bind(md);
  md.getUserMedia = async (constraints = {}) => {
    if (!constraints.audio) return realUserMedia(constraints);
    window.__micAsked = constraints.audio; // (what the page asked for: the noise switch test reads it)
    const stream = constraints.video ? await realUserMedia({ video: constraints.video }) : new MediaStream();
    stream.addTrack(micTrack());
    return stream;
  };
  const realDisplay = md.getDisplayMedia?.bind(md);
  if (realDisplay) {
    md.getDisplayMedia = async (constraints = {}) => {
      const { audio, systemAudio, ...rest } = constraints;
      const stream = await realDisplay({ ...rest, audio: false });
      if (audio) stream.addTrack(screenSoundTrack());
      return stream;
    };
  }
  window.__yapSynthCapture = true;
})();
