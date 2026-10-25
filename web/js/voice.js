import { settings } from './settings.js';
import { negotiatedVoice, readCallStats } from './call-report.js';
import { ClipWatch, MutedReminder, PttLatch } from './gate-worklet.js';

/**
 * RNNoise (the noise remover the desktop app uses too), as an AudioWorklet
 * with WebAssembly: Stronger noise removal, instead of the browser's own
 * noise suppression. Where it can't run (no WebAssembly, e.g. Safari's
 * Lockdown Mode, or it didn't load), the browser's steps in. Loaded the
 * first time the mic opens with it on. Assumes 48 kHz, which is why the
 * audio graph runs at 48 kHz.
 */
const NOISE = new URL('../vendor/noise/', import.meta.url);
let rnnoiseBinary = null;
async function rnnoiseNode(ctx) {
  const { RnnoiseWorkletNode, loadRnnoise } = await import(new URL('noise-suppressor.js', NOISE).href);
  rnnoiseBinary ??= await loadRnnoise({ url: new URL('rnnoise.wasm', NOISE).href, simdUrl: new URL('rnnoise_simd.wasm', NOISE).href });
  if (!ctx.__rnnoiseLoaded) {
    await ctx.audioWorklet.addModule(new URL('rnnoise-worklet.js', NOISE).href);
    ctx.__rnnoiseLoaded = true;
  }
  return new RnnoiseWorkletNode(ctx, { maxChannels: 1, wasmBinary: rnnoiseBinary });
}

/*
 * Voice = WebRTC mesh. Every pair of people in a channel has a direct
 * peer-to-peer connection carrying Opus audio; the server only relays the
 * setup messages ("signals"). Audio never touches the server, which is why
 * bitrate is free.
 *
 * The voice goes as RED (RFC 2198) around Opus when the other app offers it:
 * each packet also carries the frame before it, so a lost packet is rebuilt
 * word for word from the next one (PROTOCOL.md › Voice › Redundancy). Browsers
 * only send RED when it's first in the list, so `preferAudio` puts it there.
 *
 * Who offers: the newcomer offers to everyone already in the channel.
 * Glare (both sides renegotiating at once) is resolved with the standard
 * "perfect negotiation" pattern: the session with the larger sid is polite.
 *
 * Calls outlive the WebSocket (PROTOCOL.md › Voice › Keeping calls across
 * reconnects): while it's down, connections stay up and their signals wait
 * in a queue; afterwards each connection is moved to the far side's new
 * session id (`rekey`) instead of being rebuilt. js/app/voice-ui.js decides which.
 *
 * Events: 'level' (detail: local dBFS), 'speaking' ({sid, speaking}),
 *         'peer' ({sid, state}), 'mic' ({ok, error}), 'health' (silent /
 *         stalled changed), 'screen' ({sid, stream}: a screen we watch came
 *         or went), 'screen-local' (our share started, stopped or changed),
 *         'viewers' (who watches our share changed), 'muted-talk' (you're
 *         talking, or pressed push-to-talk, while muted: remind them; at most
 *         every 5 s; {blip}: the first since they muted), 'push-mute' ({down}: the push-to-mute key),
 *         'own-quality' ({weak}: most people lose > 3 % of your audio, or
 *         not any more), 'ptt-latch' ({latched}: double-tapped to stay live, or not any
 *         more), 'ptt-click' ({open}: push-to-talk opened, or closed after
 *         its tail; for your ears only).
 *
 * The mic's path: getUserMedia (the browser's echo cancellation, gain and
 * noise suppression) → with Stronger noise removal, RNNoise in place of the
 * browser's noise suppression (loud, lasting sounds like a laugh get past it:
 * the gate worklet also hears the mic as RNNoise got it) → the transmit gate (gate-worklet.js,
 * on the audio thread: voice activity / push-to-talk with fades, a release
 * tail and a short pre-roll) → the track every peer sends. Mute and deafen
 * disable that track from here, so they hold even if the audio thread stalls.
 *
 * Never silently lose the mic or the speakers: a mic that ends (unplugged,
 * revoked) is reopened; a device change reopens the mic when the one in use
 * is gone, the one you picked came back, or the system default moved, and
 * re-points the speakers; a suspended/interrupted AudioContext is resumed
 * (on the next tap if the browser insists). If the gate stops reporting for
 * 1.5 s, peers get the unprocessed mic instead (fail open) until it's back.
 */

const REMOTE_SPEAKING_LEVEL = 0.02; // RTP audio level, linear 0..1 (~-34 dBov)
const REMOTE_HOLD_MS = 250;
/** A connection reads as weak when this much of their audio is patched over (concealed) or the round trip is this slow. */
const WEAK_CONCEALED = 0.03;
const WEAK_RTT_MS = 300;

/**
 * Your own connection (PROTOCOL.md › Connection hints): each 2 s, what the
 * people you're sending to report losing of your audio (`fractionLost` in
 * their receiver reports). When more than 3 % is lost for most of them (or
 * the only one) three rounds in a row, it's your side: "Your connection is
 * dropping some of your audio"; gone after three good rounds.
 */
export class OwnConnection {
  /** null, or { lost: 0..1 (the middle of what they report), rtt: ms } */
  weak = null;
  #run = 0; // consecutive bad (+) or good (−) rounds

  /** One round: [{ fraction, rtt }] for each person we're sending to who reported. True when `weak` changed. */
  round(reports) {
    const judged = reports.filter((r) => typeof r?.fraction === 'number');
    const was = Boolean(this.weak);
    if (!judged.length) {
      // Nobody to judge by (alone, or no reports yet): no hint.
      this.#run = 0;
      this.weak = null;
      return was;
    }
    const bad = judged.filter((r) => r.fraction > WEAK_CONCEALED).length * 2 > judged.length;
    this.#run = bad ? Math.max(1, this.#run + 1) : Math.min(-1, this.#run - 1);
    const middle = (list) => (list.length ? [...list].sort((a, b) => a - b)[list.length >> 1] : null);
    const info = { lost: middle(judged.map((r) => r.fraction)), rtt: middle(judged.map((r) => r.rtt).filter((v) => v != null)) };
    if (this.#run >= 3) this.weak = info;
    else if (this.#run <= -3) this.weak = null;
    else if (this.weak && bad) this.weak = info;
    return was !== Boolean(this.weak);
  }
}

/** The voice channel's quality, if it sets one; else your own setting. */
let channelBitrate = null;
const callBitrate = () => channelBitrate ?? settings.bitrate;
/** Settings › Voice › Mic volume as the gate takes it: 0–2, exactly 1 unless you moved it (so 100 % leaves the samples alone). */
export const micVolume = () => {
  const v = Number(settings.micVolume);
  return Number.isFinite(v) ? Math.min(2, Math.max(0, Math.round(v * 100) / 100)) : 1;
};

/**
 * Screen-share presets. Each viewer gets their own stream from the sharer
 * (peer to peer), so the bitrate is per viewer: 1080p60 ≈ 8 Mbps each.
 * Games want frames (hardware H.264 first), text wants pixels (VP9 first).
 * Whatever comes first, every codec the browser has stays in the list, so
 * H.264 is always there: the native apps send and receive only H.264
 * (constrained baseline, packetization-mode=1).
 */
export const SCREEN_QUALITY = {
  smooth: { label: 'Smooth', w: 1920, h: 1080, fps: 60, bitrate: 8_000_000, hint: 'motion', degradation: 'maintain-framerate', codecs: ['H264', 'VP9', 'VP8'] },
  sharp: { label: 'Sharp', w: 2560, h: 1440, fps: 30, bitrate: 6_000_000, hint: 'detail', degradation: 'maintain-resolution', codecs: ['VP9', 'H264', 'VP8'] },
  max: { label: 'Max', w: 3840, h: 2160, fps: 60, bitrate: 25_000_000, hint: 'motion', degradation: 'balanced', codecs: ['H264', 'VP9', 'VP8'] },
};

/** `codecs` with these names first, the rest in their order. Nothing is dropped (so H.264 always stays). */
export function orderCodecs(codecs, names) {
  const rank = (c) => {
    const i = names.findIndex((n) => c.mimeType.toLowerCase() === `video/${n.toLowerCase()}`);
    return i < 0 ? names.length : i;
  };
  return [...codecs].sort((a, b) => rank(a) - rank(b));
}

/** Put these video codecs first on a transceiver (if the browser has them). */
function preferCodecs(transceiver, names) {
  try {
    const caps = RTCRtpReceiver.getCapabilities?.('video')?.codecs;
    if (!caps || !transceiver.setCodecPreferences) return;
    transceiver.setCodecPreferences(orderCodecs(caps, names));
  } catch (err) {
    console.warn('codec preference not applied', err);
  }
}

/**
 * Audio codecs in the order we want them. The voice (`red`): RED first, so
 * every packet also carries the 20 ms before it, then Opus (an app without RED
 * just picks Opus), then the rest. Anything else (a screen share's sound, or
 * the voice with RED switched off): Opus first and no RED, so it's plain Opus
 * both ways. Nothing but RED is ever dropped.
 */
export function orderAudioCodecs(codecs, { red }) {
  const kind = (c) => c.mimeType.toLowerCase();
  const rank = (c) => (kind(c) === 'audio/red' ? 0 : kind(c) === 'audio/opus' ? 1 : 2);
  return codecs.filter((c) => red || kind(c) !== 'audio/red').sort((a, b) => rank(a) - rank(b));
}

/** Set an audio transceiver's codec order (see orderAudioCodecs). Without setCodecPreferences, the browser's own order stays. */
function preferAudio(transceiver, { red }) {
  try {
    const caps = RTCRtpReceiver.getCapabilities?.('audio')?.codecs;
    if (!caps || !transceiver?.setCodecPreferences || transceiver.currentDirection === 'stopped') return;
    transceiver.setCodecPreferences(orderAudioCodecs(caps, { red }));
  } catch (err) {
    console.warn('audio codec preference not applied', err);
  }
}

/** `?red=0` in the page's address: the voice goes as plain Opus (for measuring what RED does; not a setting). */
const RED_DEFAULT = !/(?:^\?|&)red=0(?:&|$)/.test(globalThis.location?.search ?? '');

export class Voice extends EventTarget {
  mySid = null;
  iceServers = [];
  /** Only through the TURN relay (hides your IP from peers). Set before prepare(). */
  relayOnly = false;
  /** Offer RED on the voice (on unless the page was opened with ?red=0). Applies to connections made after it changes. */
  red = RED_DEFAULT;
  /** @type {Map<string, Peer>} */
  peers = new Map();
  rawStream = null; // the mic, as captured
  sendTrack = null; // what's actually sent: the mic after processing and the gate
  micOk = false;
  muted = false;
  deafened = false;
  /** Push-to-talk is on: its key is held, or it's latched (double-tap). */
  pttDown = false;
  /** Push-to-talk is latched on by a double-tap ("Live — tap to stop"). */
  get pttLatched() {
    return this.#latch.latched;
  }
  /** The push-to-mute key is held: muted for that long (a "cough button"), whatever the mode. */
  pushMuted = false;
  /** The gate is letting audio through (voice activity incl. its hold, push-to-talk incl. its tail, or always-on). */
  gateOpen = false;
  /** Where voice activity opens right now (your sensitivity, or higher in a noisy room). */
  openAt = -50;
  localDb = -100;
  /** The mic's peaks keep hitting the top ("Too loud (clipping)" on the meter). */
  clipping = false;
  #clip = new ClipWatch();
  /** Mic volume (past 100 %) keeps pushing your loudest moments into the limiter: "turn Mic volume down". */
  limiting = false;
  #limit = new ClipWatch();
  /** A mic test is running: push-to-talk acts like always-on. */
  micTest = false;
  /** The mic has sent nothing but digital silence for 5 s (a hardware mute switch, a dead device). */
  silent = false;
  /** The audio graph stopped reporting: peers get the plain mic meanwhile (fail open). */
  stalled = false;
  /** Most people lose > 3 % of your audio ("Your connection is dropping some of your audio"): null or { lost, rtt }. */
  ownWeak = null;
  #own = new OwnConnection();
  #fallback = null; // that plain mic track, while stalled
  #lastLevelAt = 0;
  #watchdog = null;
  #deviceTimer = null;
  #resumeOnGesture = null;
  #ctx = null;
  #ctxReady = null; // the gate's worklet module, loading into #ctx
  #micOpening = Promise.resolve(); // mic (re)opens run one at a time (#queueMic)
  #out = null; // playback graph for volume past 100%
  #outSink = null; // the output #out was last pointed at (a missing one isn't tried again until devices change)
  #denoise = null; // RNNoise node, when on
  #rnnoiseBroken = false; // it failed to load here: the browser's noise suppression instead
  #source = null;
  #gate = null; // the transmit gate (AudioWorkletNode 'yap-gate')
  #gateSent = {}; // what the gate was last told
  #dest = null;
  #hush = null;
  #pollTimer = null;
  #handoverCheck = null;
  #iceAt = 0;
  #selfSpeaking = false;
  #reminder = new MutedReminder();
  #latch = new PttLatch();
  #clicked = 0; // when push-to-talk's opening click played (0: it isn't open)…
  #sawOpen = false; // …and the gate has said it's open since: its closing click comes when it closes

  /** Your screen share (getDisplayMedia), or null. */
  screenStream = null;
  /** The preset it goes out at (a SCREEN_QUALITY entry), while sharing. */
  screenQuality = null;
  /** Send the share's sound (when the browser gave it any): the Share sound switch. */
  screenSound = true;
  /** sid → that person's screen (video + any tab audio), while you watch it. */
  screens = new Map();
  /** Is the socket up? While it isn't, peers queue their signals (a call outlives the socket). */
  online = true;
  /** Sharers whose screen you asked to see. */
  #watching = new Set();

  /**
   * `listenOnly` (an OBS link): the way VDO.Ninja's viewer plays a call. Never a mic or an audio
   * graph; each person's track plays as it arrives from an element of its own at full volume (no
   * per-person gain or limiter: OBS sets the volume), with the browser's own jitter buffer, and an
   * element that pauses is started again (web/js/obs.js has the why and the measurements).
   */
  constructor({ signal, uidOf, resumable = () => false, fetchIce = null, listenOnly = false }) {
    super();
    this.signal = signal; // (toSid, data) => void
    this.uidOf = uidOf; // (sid) => uid
    this.resumable = resumable; // (sid) => does that session keep calls across reconnects?
    this.fetchIce = fetchIce; // async () => iceServers (fresh relay credentials for ICE restarts)
    this.listenOnly = listenOnly;
    navigator.mediaDevices?.addEventListener?.('devicechange', () => {
      clearTimeout(this.#deviceTimer);
      this.#deviceTimer = setTimeout(() => this.#devicesChanged(), 500); // they arrive in bursts
    });
    document.addEventListener?.('visibilitychange', () => document.visibilityState === 'visible' && this.#kickContext());
  }

  /** The track peers should send right now: the processed one, or the plain mic while that's stalled. */
  get outTrack() {
    return this.#fallback ?? this.sendTrack;
  }

  // ── Screen sharing (PROTOCOL.md › Screen sharing) ─────────────────────────
  // Video goes over the same peer connections as voice, but only to people who
  // ask for it ({screen: 'want'}), so a share costs upload per actual viewer.

  /**
   * @param {{ surface?: 'window' | 'monitor' | 'browser', quality?: keyof typeof SCREEN_QUALITY, sound?: boolean }} opts
   * Call from a click (browsers need a user gesture). Nothing is encoded
   * until someone asks to watch, and only for them.
   */
  async startScreen({ surface = 'window', quality = 'smooth', sound = true } = {}) {
    const q = SCREEN_QUALITY[quality] ?? SCREEN_QUALITY.smooth;
    const stream = await this.#pickScreen(surface, q);
    this.screenQuality = q;
    this.screenSound = sound;
    this.#useScreen(stream);
    return stream;
  }

  /** The browser's picker. Its sound is always asked for, so Share sound can go on mid-share. */
  async #pickScreen(surface, q) {
    const stream = await navigator.mediaDevices.getDisplayMedia({
      video: {
        displaySurface: surface, // which tab of the browser's picker opens first
        width: { ideal: q.w, max: q.w },
        height: { ideal: q.h, max: q.h },
        frameRate: { ideal: q.fps, max: q.fps },
      },
      audio: true, // tab audio, or system audio when sharing a whole screen (Chrome on Windows)
      systemAudio: 'include',
      surfaceSwitching: 'include',
      selfBrowserSurface: 'exclude',
    });
    const video = stream.getVideoTracks()[0];
    if (video) video.contentHint = q.hint;
    // Tab/system audio is music or a game, not a voice: no speech tuning, keep it stereo.
    for (const track of stream.getAudioTracks()) track.contentHint = 'music';
    return stream;
  }

  /** Share `stream` from now on (to whoever watches); a stream it replaces is stopped. */
  #useScreen(stream) {
    const old = this.screenStream;
    this.screenStream = stream;
    // The browser's own "Stop sharing" (or the window closing).
    stream.getVideoTracks()[0]?.addEventListener('ended', () => this.screenStream === stream && this.stopScreen());
    for (const peer of this.peers.values()) peer.syncScreen();
    for (const track of old?.getTracks() ?? []) track.stop();
    this.#emit('screen-local', { on: true });
  }

  /**
   * Share another window or screen without stopping: the browser's picker
   * again, then the new tracks take the old ones' place on every connection
   * (no renegotiation, viewers keep watching). Call from a click.
   */
  async changeScreen({ surface = 'window' } = {}) {
    if (!this.screenStream) return;
    const stream = await this.#pickScreen(surface, this.screenQuality ?? SCREEN_QUALITY.smooth);
    if (!this.screenStream) {
      for (const track of stream.getTracks()) track.stop(); // stopped while the picker was up
      return;
    }
    this.#useScreen(stream);
  }

  /**
   * Smooth / Sharp / Max while sharing, without stopping: the capture size and
   * frame rate, the encoder's bitrate, and (when the preset prefers another
   * codec) a renegotiation so the new codec takes over.
   */
  async setScreenQuality(quality) {
    const q = SCREEN_QUALITY[quality];
    if (!q || !this.screenStream || q === this.screenQuality) return;
    const codecs = this.screenQuality?.codecs[0] !== q.codecs[0];
    this.screenQuality = q;
    const video = this.screenStream.getVideoTracks()[0];
    if (video) {
      video.contentHint = q.hint;
      await video.applyConstraints({ width: { ideal: q.w, max: q.w }, height: { ideal: q.h, max: q.h }, frameRate: { ideal: q.fps, max: q.fps } })
        .catch((err) => console.warn('screen quality: constraints not applied', err));
    }
    for (const peer of this.peers.values()) peer.syncScreen({ codecs });
    this.#emit('screen-local', { on: true });
  }

  /** The Share sound switch: the share's sound goes out, or doesn't (no renegotiation when there's a track for it already). */
  setScreenSound(on) {
    this.screenSound = Boolean(on);
    for (const peer of this.peers.values()) peer.syncScreen();
    this.#emit('screen-local', { on: Boolean(this.screenStream) });
  }

  /** Did the browser give the share any sound (a tab's, or the system's)? */
  get screenHasSound() {
    return Boolean(this.screenStream?.getAudioTracks().some((t) => t.readyState === 'live'));
  }

  stopScreen() {
    if (!this.screenStream) return;
    for (const track of this.screenStream.getTracks()) track.stop();
    this.screenStream = null;
    for (const peer of this.peers.values()) {
      peer.wantsScreen = false; // a new share is asked for again (LIVE / Watch screen)
      peer.syncScreen();
    }
    this.#emit('screen-local', { on: false });
    this.#emit('viewers');
  }

  /** Who is watching your share, by session, in the order they asked (from their want/stop signals). */
  get viewers() {
    return [...this.peers.values()].filter((p) => p.wantsScreen).sort((a, b) => a.wantedAt - b.wantedAt).map((p) => p.sid);
  }

  /** Ask a sharer (someone in your voice channel) to send you their screen, or stop. */
  watchScreen(sid, on) {
    if (on) this.#watching.add(sid);
    else {
      this.#watching.delete(sid);
      this.#screenGone(sid);
    }
    this.peers.get(sid)?.send({ screen: on ? 'want' : 'stop' });
  }

  isWatching(sid) {
    return this.#watching.has(sid);
  }

  /** A peer's screen arrived, changed (its sound came or went) or left (null). */
  onRemoteScreen(sid, stream) {
    if (!stream) return this.#screenGone(sid);
    this.screens.set(sid, stream);
    this.#emit('screen', { sid, stream });
  }

  #screenGone(sid) {
    if (!this.screens.delete(sid)) return;
    this.#emit('screen', { sid, stream: null });
  }

  get active() {
    return this.rawStream !== null || this.peers.size > 0 || this.#ctx !== null;
  }

  /** The mic's audio graph (for tests and debugging). */
  get context() {
    return this.#ctx;
  }

  /** Get the mic and audio graph ready. Call from a user gesture. (Listen-only: just the ICE servers.) */
  async prepare(iceServers) {
    if (iceServers) this.iceServers = iceServers;
    if (this.listenOnly) return;
    if (!this.#ctx) {
      const ctx = new AudioContext({ latencyHint: 'interactive', sampleRate: 48000 });
      this.#ctx = ctx;
      ctx.onstatechange = () => {
        this.#emit('context', { state: this.#ctx?.state }); // (the call report notes interruptions)
        this.#kickContext();
      };
      this.#ctxReady = ctx.audioWorklet.addModule(new URL('./gate-worklet.js', import.meta.url)).catch((err) => {
        // A context without the gate can't carry a mic: the next try starts with a new one.
        if (this.#ctx === ctx) {
          this.#ctx = null;
          ctx.close().catch(() => {});
        }
        throw err;
      });
    }
    const ctx = this.#ctx;
    // (A second prepare() while the first is still loading the gate waits for it, rather than build a mic without it.)
    await this.#ctxReady;
    await ctx.resume();
    if (!this.rawStream) await this.#queueMic({ ifClosed: true });
  }

  /** Re-open the mic after a device or processing change. */
  async restartMic() {
    if (!this.#ctx) return;
    await this.#queueMic();
  }

  /**
   * Open the mic (again), one open at a time: each waits for the one before,
   * so the settings asked for last are the ones that end up open (a slow
   * RNNoise load can't finish after a later, quicker open and undo it).
   * `ifClosed`: only if it isn't open by then.
   */
  #queueMic({ ifClosed = false } = {}) {
    const run = this.#micOpening.then(() => (ifClosed && this.rawStream ? undefined : this.#openMic()));
    this.#micOpening = run.catch(() => {});
    return run;
  }

  async #openMic() {
    const ctx = this.#ctx;
    if (!ctx) return;
    const music = settings.music;
    // Noise suppression (off in music mode) = the browser's own; Stronger noise removal = RNNoise
    // instead, wherever it runs. RNNoise is loaded before the mic opens, so the browser's filter is
    // asked for only when RNNoise isn't running (never both).
    let denoise = null;
    if (!music && settings.noiseSuppression && settings.strongerNoise && !this.#rnnoiseBroken) {
      try {
        denoise = await rnnoiseNode(ctx);
      } catch (err) {
        if (this.#ctx !== ctx) return; // stopped meanwhile (that's not RNNoise's fault)
        console.warn("RNNoise can't run here; using the browser's noise suppression", err);
        this.#rnnoiseBroken = true;
      }
    }
    const audio = {
      echoCancellation: music ? false : settings.echoCancellation,
      noiseSuppression: !music && settings.noiseSuppression && !denoise,
      autoGainControl: music ? false : settings.autoGain,
      channelCount: music ? 2 : 1,
      sampleRate: 48000,
      sampleSize: 16,
    };
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: settings.inputId ? { ...audio, deviceId: { exact: settings.inputId } } : audio,
      });
    } catch (err) {
      if (settings.inputId && (err.name === 'OverconstrainedError' || err.name === 'NotFoundError')) {
        // The saved mic is gone (unplugged); fall back to the default one.
        try {
          stream = await navigator.mediaDevices.getUserMedia({ audio });
        } catch (err2) {
          denoise?.destroy();
          return this.#micFailed(err2);
        }
      } else {
        denoise?.destroy();
        return this.#micFailed(err);
      }
    }
    if (this.#ctx !== ctx) {
      // Stopped (Settings closed, the call left) while the browser was asking: let the mic go again.
      for (const track of stream.getTracks()) track.stop();
      denoise?.destroy();
      return;
    }
    this.#releaseMic();
    this.rawStream = stream;
    this.#denoise = denoise;
    const channels = music ? 2 : 1;
    try {
      this.#source = ctx.createMediaStreamSource(stream);
      this.#gate = new AudioWorkletNode(ctx, 'yap-gate', {
        numberOfInputs: 2,
        numberOfOutputs: 1,
        channelCount: channels,
        channelCountMode: 'explicit',
        outputChannelCount: [channels],
        processorOptions: this.#gateSettings(),
      });
      this.#gateSent = this.#gateSettings();
      this.#gate.port.onmessage = (e) => this.#onLevel(e.data);
      this.#dest = ctx.createMediaStreamDestination();
      this.#dest.channelCount = channels;
      // Also into the speakers at zero volume, so the graph is always pulled.
      const hush = ctx.createGain();
      hush.gain.value = 0;
      this.#gate.connect(this.#dest);
      this.#gate.connect(hush).connect(ctx.destination);
      this.#hush = hush;
      // mic → RNNoise → gate; voice activity hears the cleaned audio. The gate's second input is
      // the mic as RNNoise got it: loud, lasting sounds (a laugh) come through (gate-worklet.js › LoudPass).
      if (denoise) {
        this.#source.connect(denoise).connect(this.#gate, 0, 0);
        this.#source.connect(this.#gate, 0, 1);
      } else this.#source.connect(this.#gate);
    } catch (err) {
      // (Firefox, for one, won't connect a mic at another sample rate.) A half-built mic would sit
      // there open and silent, and everything that checks `rawStream` would think it's fine.
      console.warn("couldn't connect the microphone to the audio graph", err);
      this.#releaseMic();
      return this.#micFailed(err);
    }
    this.rnnoise = Boolean(denoise);
    this.sendTrack = this.#dest.stream.getAudioTracks()[0];
    this.sendTrack.contentHint = music ? 'music' : 'speech';
    const raw = stream.getAudioTracks()[0];
    raw.addEventListener('ended', () => this.#micEnded(raw));
    this.micOk = true;
    this.#lastLevelAt = performance.now();
    this.#setStalled(false);
    for (const peer of this.peers.values()) peer.setTrack(this.outTrack);
    this.#applyGate();
    this.#startWatchdog();
    this.#emit('mic', { ok: true });
  }

  /** The mic stopped by itself (unplugged, permission revoked, another app took it): reopen it. */
  #micEnded(track) {
    if (this.rawStream?.getAudioTracks()[0] !== track) return; // already replaced
    console.warn('the microphone stopped; reopening it');
    setTimeout(() => {
      if (this.rawStream?.getAudioTracks()[0] === track) this.#queueMic();
    }, 300);
  }

  /** Devices came or went: follow the mic (and the speakers) like a native app would. */
  async #devicesChanged() {
    let devices;
    try {
      devices = await navigator.mediaDevices.enumerateDevices();
    } catch {
      return; // (no list: no way to tell what changed, so leave the mic and speakers where they are)
    }
    const has = (kind, id) => devices.some((d) => d.kind === kind && d.deviceId === id);
    const track = this.rawStream?.getAudioTracks()[0];
    if (track && this.#ctx) {
      const using = track.getSettings?.() ?? {};
      const def = devices.find((d) => d.kind === 'audioinput' && d.deviceId === 'default');
      const reopen = track.readyState === 'ended'
        || (using.deviceId && !has('audioinput', using.deviceId)) // the mic in use is gone
        || (settings.inputId && using.deviceId !== settings.inputId && has('audioinput', settings.inputId)) // the one you picked is back
        || (!settings.inputId && def?.groupId && using.groupId && def.groupId !== using.groupId); // the system default moved (Chrome)
      if (reopen) await this.#queueMic();
    }
    // Speakers: the picked one if it's there, else the system default.
    for (const peer of this.peers.values()) peer.applyOutput({ force: true });
    if (this.#out?.setSinkId) {
      this.#outSink = settings.outputId && has('audiooutput', settings.outputId) ? settings.outputId : '';
      this.#out.setSinkId(this.#outSink).catch(() => {});
    }
  }

  /**
   * Keep the audio graphs running (the mic's, and the one playing people
   * turned up past 100 % or panned in Space): a suspended or interrupted
   * context (Safari: a phone call, Siri; any browser: the OS took the device)
   * gets resumed — on the next tap or key press if the browser wants a gesture.
   */
  #kickContext() {
    const stuck = [this.#ctx, this.#out].filter((ctx) => ctx && ctx.state !== 'running' && ctx.state !== 'closed');
    if (!stuck.length) return this.#stopKicking();
    for (const ctx of stuck) ctx.resume().catch(() => {});
    if (!this.#resumeOnGesture) {
      this.#resumeOnGesture = () => {
        this.#ctx?.resume().catch(() => {});
        this.#out?.resume().catch(() => {});
      };
      addEventListener('pointerdown', this.#resumeOnGesture, true);
      addEventListener('keydown', this.#resumeOnGesture, true);
    }
  }

  #stopKicking() {
    if (!this.#resumeOnGesture) return;
    removeEventListener('pointerdown', this.#resumeOnGesture, true);
    removeEventListener('keydown', this.#resumeOnGesture, true);
    this.#resumeOnGesture = null;
  }

  /** The gate reports every 20 ms; 1.5 s without one = the graph is stuck: fail open. */
  #startWatchdog() {
    if (this.#watchdog) return;
    this.#watchdog = setInterval(() => {
      if (!this.rawStream) return;
      const stuck = performance.now() - this.#lastLevelAt > 1500;
      if (stuck) this.#kickContext();
      this.#setStalled(stuck);
    }, 1000);
  }

  #setStalled(on) {
    if (on === this.stalled) return;
    this.stalled = on;
    if (on) {
      console.warn('the audio graph stopped; sending the plain mic until it is back');
      this.#fallback = this.rawStream?.getAudioTracks()[0]?.clone() ?? null;
      if (this.#fallback) this.#fallback.contentHint = this.sendTrack?.contentHint ?? 'speech';
    } else {
      this.#fallback?.stop();
      this.#fallback = null;
    }
    for (const peer of this.peers.values()) peer.setTrack(this.outTrack);
    this.#applyGate();
    this.#emit('health', { stalled: on, silent: this.silent });
  }

  #micFailed(error) {
    this.micOk = false;
    this.#emit('mic', { ok: false, error });
  }

  #releaseMic() {
    // (A warning about the mic that goes — "sending silence", "plain mic" — goes with it.)
    const unwell = this.stalled || this.silent;
    this.#source?.disconnect();
    this.#source = null;
    this.#denoise?.disconnect();
    this.#denoise?.destroy();
    this.#denoise = null;
    if (this.#gate) this.#gate.port.onmessage = null;
    this.#gate?.disconnect();
    this.#gate = null;
    this.#hush?.disconnect();
    this.#hush = null;
    this.#dest = null;
    this.gateOpen = false;
    this.#fallback?.stop();
    this.#fallback = null;
    this.stalled = false;
    this.silent = false;
    this.clipping = false;
    this.#clip = new ClipWatch();
    this.limiting = false;
    this.#limit = new ClipWatch();
    this.rawStream?.getTracks().forEach((t) => t.stop());
    this.sendTrack?.stop();
    this.rawStream = null;
    this.sendTrack = null;
    if (unwell) this.#emit('health', { stalled: false, silent: false });
  }

  // ── Transmit gate: mute / deafen / voice activity / push-to-talk ─────────

  /** `joined`: a call starts (its first talking-while-muted reminder blips again). */
  setMuted(muted, { joined = false } = {}) {
    if (muted && (joined || !this.muted)) this.#reminder.muted(performance.now());
    this.muted = muted;
    this.#applyGate();
  }

  /**
   * You're talking (the gate would open), or pressed push-to-talk, while
   * muted: 'muted-talk', at most every 5 s, with the blip only the first time
   * since you muted (gate-worklet.js › MutedReminder). Not while deafened:
   * that's a louder "I'm away" than mute.
   */
  #talkingWhileMuted() {
    if (!this.muted || this.deafened || !this.micOk) return;
    const due = this.#reminder.due(performance.now());
    if (due) this.#emit('muted-talk', { blip: due === 'blip' });
  }

  setDeafened(deafened) {
    this.deafened = deafened;
    for (const peer of this.peers.values()) peer.applyOutput();
    this.#applyGate();
  }

  /**
   * Playback graph for people turned up past 100% (a media element tops out
   * at 1.0). Separate from the mic graph so stopping the mic doesn't cut it.
   */
  outputContext() {
    if (!this.#out || this.#out.state === 'closed') {
      this.#out = new AudioContext({ latencyHint: 'interactive' });
      this.#out.onstatechange = () => this.#kickContext();
      this.#outSink = null;
    }
    // The picked speakers, or the system's again when that's what's picked now.
    const sink = settings.outputId || '';
    if (this.#out.setSinkId && this.#outSink !== sink) {
      this.#outSink = sink;
      this.#out.setSinkId(sink).catch(() => {});
    }
    this.#out.resume().catch(() => {});
    return this.#out;
  }

  /** Push-to-mute: muted while the key is held, like mute (a hard switch here), but nothing is saved or broadcast. */
  setPushToMute(down) {
    if (this.pushMuted === down) return;
    this.pushMuted = down;
    this.#applyGate();
    this.#emit('push-mute', { down });
  }

  /**
   * The push-to-talk key went down or up. Pressed twice within 400 ms it
   * stays open until the next press (PttLatch). Opening clicks at once (or
   * reminds you that you're muted); closing clicks when the gate closes,
   * after the tail (#onLevel).
   */
  setPtt(down) {
    if (down === this.#latch.held) return;
    const latched = this.#latch.latched;
    if (down) this.#latch.down(performance.now());
    else this.#latch.up();
    if (latched !== this.#latch.latched) this.#emit('ptt-latch', { latched: this.#latch.latched });
    const open = this.#latch.open;
    if (open === this.pttDown) return;
    this.pttDown = open;
    if (open && settings.mode === 'ptt') {
      if (this.muted || this.deafened) this.#talkingWhileMuted();
      else if (!this.pushMuted && !this.micTest && !this.#clicked) {
        // (Pressed again within the tail: it never closed, so no second click.)
        this.#clicked = performance.now();
        this.#sawOpen = false;
        this.#emit('ptt-click', { open: true });
      }
    }
    this.#applyGate();
  }

  /** Push-to-talk off for good, latch and all (the input mode changed). */
  releasePtt() {
    const latched = this.#latch.latched;
    this.#latch.reset();
    this.#clicked = 0;
    if (latched) this.#emit('ptt-latch', { latched: false });
    if (!this.pttDown) return;
    this.pttDown = false;
    this.#applyGate();
  }

  /** A report from the gate, every 20 ms (gate-worklet.js). */
  #onLevel({ db, peak, boost, open, voice, pttOpen, at, silent }) {
    this.localDb = db;
    this.clipping = this.#clip.push(peak ?? -120, performance.now());
    this.limiting = this.#limit.push(boost ?? -120, performance.now());
    this.gateOpen = open;
    this.openAt = at;
    // Push-to-talk's closing click: once the gate has closed after the tail.
    if (this.#clicked && pttOpen) this.#sawOpen = true;
    else if (this.#clicked && !pttOpen && !this.pttDown) {
      if (this.#sawOpen) this.#emit('ptt-click', { open: false });
      if (this.#sawOpen || performance.now() - this.#clicked > 1000) this.#clicked = 0; // (or it never opened: the gate stalled)
    }
    // Talking while muted: the level is measured before the mute (push-to-talk reminds on the key instead).
    if (voice && settings.mode !== 'ptt') this.#talkingWhileMuted();
    this.#lastLevelAt = performance.now();
    if (this.stalled) this.#setStalled(false);
    if (silent !== this.silent) {
      this.silent = silent;
      this.#emit('health', { stalled: this.stalled, silent });
    }
    this.#applyGate();
    this.#emit('level', db);
  }

  /**
   * What the gate should do. Music mode + voice activity = always on: a gate
   * would cut the quiet passages out of music.
   */
  #gateSettings() {
    const mode = settings.mode === 'vad' && settings.music ? 'open' : settings.mode;
    return { mode, threshold: settings.vadThreshold, ptt: this.pttDown, test: this.micTest, volume: micVolume() };
  }

  /** Mic volume changed (Settings › Voice): straight to the gate, no reopening the mic. */
  setMicVolume() {
    this.#syncGate();
  }

  /** Tell the gate whatever changed (mode, sensitivity, push-to-talk key, mic volume). */
  #syncGate() {
    if (!this.#gate) return;
    const now = this.#gateSettings();
    const changed = Object.fromEntries(Object.entries(now).filter(([k, v]) => this.#gateSent[k] !== v));
    if (!Object.keys(changed).length) return;
    this.#gateSent = now;
    this.#gate.port.postMessage(changed);
  }

  get transmitting() {
    return Boolean(this.sendTrack) && !this.muted && !this.pushMuted && !this.deafened && this.gateOpen;
  }

  #applyGate() {
    this.#syncGate();
    // Mute/deafen (and push-to-mute): certain, from here. Everything else is the gate's job.
    const enabled = !this.muted && !this.pushMuted && !this.deafened;
    if (this.sendTrack && this.sendTrack.enabled !== enabled) this.sendTrack.enabled = enabled;
    // The fail-open fallback has no gate: push-to-talk still works (from here), voice activity = always on.
    if (this.#fallback) this.#fallback.enabled = enabled && (settings.mode !== 'ptt' || this.pttDown);
    const speaking = this.transmitting && (settings.mode === 'vad' || this.localDb > settings.vadThreshold);
    if (speaking !== this.#selfSpeaking && this.mySid) {
      this.#selfSpeaking = speaking;
      this.#emit('speaking', { sid: this.mySid, speaking });
    }
  }

  // ── "Hear yourself" (Settings › Voice) ──────────────────────────────────

  /**
   * Record a few seconds of what peers would get — the browser's processing,
   * RNNoise, the gate (so voice activity cuts in and out as it would) — sent
   * through a local Opus connection at the call's bitrate, then play it back
   * on your speakers. `phase('recording' | 'playing' | 'done', info)`.
   * Outside a call, push-to-talk acts like always-on meanwhile; in a call it
   * doesn't (others would hear you), so hold the key.
   */
  async hearYourself({ seconds = 4, phase = () => {} } = {}) {
    if (!this.sendTrack || !this.#ctx) throw new Error('The microphone is not open.');
    const ctx = this.#ctx;
    const alone = this.peers.size === 0;
    if (alone) {
      this.micTest = true;
      this.#applyGate();
    }
    const track = this.sendTrack.clone();
    track.enabled = true; // even if you're muted: this goes nowhere but your own speakers
    const cleanup = [() => track.stop()];
    try {
      const heard = await this.#loopback(track, cleanup).catch((err) => {
        console.warn('hear yourself: no local connection, recording without the codec', err);
        return null;
      });
      const source = ctx.createMediaStreamSource(new MediaStream([heard ?? track]));
      const recorder = new AudioWorkletNode(ctx, 'yap-record');
      const hush = ctx.createGain();
      hush.gain.value = 0;
      source.connect(recorder).connect(hush).connect(ctx.destination);
      cleanup.push(() => {
        recorder.port.onmessage = null;
        source.disconnect();
        recorder.disconnect();
        hush.disconnect();
      });
      const chunks = [];
      recorder.port.onmessage = (e) => chunks.push(e.data);
      phase('recording', { codec: Boolean(heard), bitrate: callBitrate() });
      await new Promise((r) => setTimeout(r, seconds * 1000));
      recorder.port.onmessage = null;
      const samples = new Float32Array(chunks.reduce((n, c) => n + c.length, 0));
      let at = 0;
      for (const c of chunks) {
        samples.set(c, at);
        at += c.length;
      }
      this.lastTest = { samples, sampleRate: ctx.sampleRate, codec: Boolean(heard) }; // (tests read this)
      const out = this.outputContext(); // your picked speakers
      const buffer = out.createBuffer(1, Math.max(1, samples.length), ctx.sampleRate);
      buffer.copyToChannel(samples, 0);
      const play = out.createBufferSource();
      play.buffer = buffer;
      play.connect(out.destination);
      phase('playing');
      await new Promise((resolve) => {
        play.onended = resolve;
        play.start();
      });
      phase('done');
    } finally {
      for (const undo of cleanup.reverse()) {
        try {
          undo();
        } catch {}
      }
      if (alone) {
        this.micTest = false;
        this.#applyGate();
      }
    }
  }

  /** Send `track` through a local Opus connection tuned like a call; resolves to what comes out. */
  async #loopback(track, cleanup) {
    const a = new RTCPeerConnection();
    const b = new RTCPeerConnection();
    cleanup.push(() => {
      a.close();
      b.close();
    });
    a.onicecandidate = (e) => e.candidate && b.addIceCandidate(e.candidate).catch(() => {});
    b.onicecandidate = (e) => e.candidate && a.addIceCandidate(e.candidate).catch(() => {});
    const sender = a.addTrack(track, new MediaStream([track]));
    const arrived = new Promise((resolve) => (b.ontrack = (e) => resolve(e.track)));
    const connected = new Promise((resolve) => {
      a.onconnectionstatechange = () => a.connectionState === 'connected' && resolve(true);
    });
    const offer = await a.createOffer();
    await a.setLocalDescription({ type: 'offer', sdp: tuneOpus(offer.sdp) });
    await b.setRemoteDescription(a.localDescription);
    const answer = await b.createAnswer();
    await b.setLocalDescription({ type: 'answer', sdp: tuneOpus(answer.sdp) });
    await a.setRemoteDescription(b.localDescription);
    const params = sender.getParameters();
    if (params.encodings?.length) {
      params.encodings[0].maxBitrate = callBitrate();
      await sender.setParameters(params).catch(() => {});
    }
    const timeout = new Promise((resolve) => setTimeout(() => resolve(null), 3000));
    const remote = await Promise.race([arrived, timeout]);
    if (!remote || !(await Promise.race([connected, timeout]))) throw new Error('local connection did not come up');
    // Chrome only feeds WebRTC audio to WebAudio while a media element plays it.
    const element = new Audio();
    element.muted = true;
    element.srcObject = new MediaStream([remote]);
    element.play().catch(() => {});
    cleanup.push(() => (element.srcObject = null));
    return remote;
  }

  // ── Peers ────────────────────────────────────────────────────────────────

  /** We just joined: offer to everyone already in the channel. */
  enter(sids) {
    for (const sid of sids) if (sid !== this.mySid && !this.peers.has(sid)) this.#createPeer(sid, true);
    this.#startPolling();
  }

  handleSignal(from, data) {
    let peer = this.peers.get(from);
    if (peer && data?.sdp?.type === 'offer' && peer.pc.remoteDescription) {
      // A different DTLS fingerprint = the far side started a brand-new
      // connection (it couldn't keep ours across a reconnect): start over too.
      const print = (sdp) => sdp?.match(/^a=fingerprint:\S+ (\S+)/m)?.[1];
      const had = print(peer.pc.remoteDescription.sdp);
      const now = print(data.sdp.sdp);
      if (had && now && had !== now) {
        this.drop(from);
        peer = null;
        this.#emit('rebuild', { sid: from, by: 'them' });
      }
    }
    if (data?.screen) {
      // A viewer asking for (or done with) our screen. (A late "want" for a share that already ended is ignored.)
      if (!peer) return;
      const want = data.screen === 'want' && Boolean(this.screenStream);
      if (want !== peer.wantsScreen) {
        peer.wantsScreen = want;
        peer.wantedAt = Date.now();
        this.#emit('viewers');
      }
      peer.syncScreen();
      return;
    }
    if (!peer) {
      // Only an offer may open a new connection.
      if (data?.sdp?.type !== 'offer') return;
      peer = this.#createPeer(from, false);
      this.#startPolling();
    }
    peer.handle(data);
  }

  drop(sid) {
    const peer = this.peers.get(sid);
    if (!peer) return;
    peer.close();
    this.peers.delete(sid);
    this.#screenGone(sid);
    this.#emit('speaking', { sid, speaking: false });
    this.#emit('peer', { sid, state: 'closed' });
    if (peer.wantsScreen) this.#emit('viewers'); // they were watching us: not any more
  }

  /** Leave the call but keep the mic (e.g. socket reconnecting). */
  hangupAll() {
    for (const sid of [...this.peers.keys()]) this.drop(sid);
    this.#own = new OwnConnection();
    this.ownWeak = null;
    clearInterval(this.#pollTimer);
    this.#pollTimer = null;
    clearTimeout(this.#handoverCheck);
    this.online = true;
  }

  // ── Keeping a call across a reconnect ────────────────────────────────────

  /** Our socket dropped mid-call: keep every connection; signals wait until we're back. */
  suspend() {
    this.online = false;
  }

  /**
   * Move a connection to the far side's new session id (they reconnected and
   * continue the same call). Same RTCPeerConnection: no new ICE, no new DTLS,
   * no gap. Returns false if there was nothing to move.
   */
  rekey(from, to) {
    const peer = this.peers.get(from);
    if (!peer || from === to) return false;
    if (this.peers.has(to)) {
      // Already connected to the new session (it offered first): the old one is spare.
      this.drop(from);
      return false;
    }
    this.peers.delete(from);
    peer.sid = to;
    this.peers.set(to, peer);
    const screen = this.screens.get(from);
    if (screen) {
      this.screens.delete(from);
      this.screens.set(to, screen);
    }
    if (this.#watching.delete(from)) this.#watching.add(to);
    if (peer.speaking) {
      peer.speaking = false;
      this.#emit('speaking', { sid: from, speaking: false });
    }
    this.#emit('peer', { sid: from, state: 'closed' });
    this.#emit('peer', { sid: to, state: peer.pc.connectionState });
    if (peer.wantsScreen) this.#emit('viewers');
    if (this.online) peer.resync();
    return true;
  }

  /**
   * Our own call was handed over to our new session (our `voice` echo came
   * back with `prev`). `here` = the other sessions in the channel now.
   * Connections to people who keep calls stay; older clients hung up on us
   * when our socket dropped, so they (and anyone new) get a fresh offer, as
   * from a newcomer. Connections to people not back yet are left alone:
   * they may be reconnecting too (a deploy drops everyone at once).
   */
  handover(here) {
    this.online = true;
    for (const peer of [...this.peers.values()]) {
      if (!here.includes(peer.sid)) continue;
      if (!this.resumable(peer.sid)) this.drop(peer.sid);
      else peer.resync();
    }
    for (const sid of here) if (sid !== this.mySid && !this.peers.has(sid)) this.#createPeer(sid, true);
    this.#startPolling();
    // A kept connection that hasn't come back in 4 s is rebuilt (the far side may have given up on it).
    clearTimeout(this.#handoverCheck);
    this.#handoverCheck = setTimeout(() => {
      for (const sid of here) {
        const peer = this.peers.get(sid);
        if (peer && peer.pc.connectionState !== 'connected' && this.resumable(sid)) this.restart(sid);
      }
    }, 4000);
  }

  /** ICE servers for a restart: fetched again (relay credentials can expire), at most every 30 s. */
  async freshIceServers() {
    if (this.fetchIce && Date.now() - this.#iceAt > 30_000) {
      this.#iceAt = Date.now();
      try {
        const servers = await this.fetchIce();
        if (Array.isArray(servers) && servers.length) this.iceServers = servers;
      } catch {}
    }
    return this.iceServers;
  }

  /** Throw a connection away and offer a brand-new one (the far side sees a new fingerprint and follows). */
  restart(sid) {
    if (!this.peers.has(sid)) return;
    this.drop(sid);
    this.#createPeer(sid, true);
    this.#emit('rebuild', { sid, by: 'us' });
  }

  /** Fully off: no peers, no screen share, mic released, audio graph closed. */
  stop() {
    this.stopScreen(); // (out of the call = not sharing: PROTOCOL.md › Screen sharing)
    this.hangupAll();
    this.#watching.clear();
    this.#releaseMic();
    clearInterval(this.#watchdog);
    this.#watchdog = null;
    this.micOk = false;
    this.#ctx?.close();
    this.#ctx = null;
    this.#ctxReady = null;
    this.#out?.close();
    this.#out = null;
    this.#stopKicking();
    if (this.#selfSpeaking && this.mySid) this.#emit('speaking', { sid: this.mySid, speaking: false });
    this.#selfSpeaking = false;
  }

  /**
   * Space view: scale one person's volume by how far away they are (1 =
   * full), and pan them left or right (−1…1; null = not panned, played
   * straight from their element as usual).
   */
  setProximity(sid, factor, pan = null) {
    const peer = this.peers.get(sid);
    if (!peer) return;
    // (Small steps of an easing wait for a bigger one, but a resting value — 0, 1, ±1 — always lands.)
    const near = (a, b) => a === b || (a !== null && b !== null && Math.abs(a - b) < 0.02 && !Number.isInteger(b));
    if (near(peer.proximity ?? 1, factor) && near(peer.pan ?? null, pan)) return;
    peer.proximity = factor;
    peer.pan = pan;
    peer.applyOutput();
  }

  /**
   * A voice channel can set one quality for everyone in it; null = each
   * person's own setting. Renegotiates live calls when it changes.
   */
  setChannelBitrate(bitrate) {
    if ((channelBitrate ?? null) === (bitrate ?? null)) return;
    channelBitrate = bitrate ?? null;
    this.applySettings({ renegotiate: true });
  }

  /** Apply output device, per-person volume and bitrate to live calls. */
  applySettings({ renegotiate = false } = {}) {
    for (const peer of this.peers.values()) {
      peer.applyOutput();
      peer.applySenderParams();
      if (renegotiate) peer.renegotiate();
    }
  }

  #createPeer(sid, initiator) {
    const peer = new Peer(this, sid, initiator);
    this.peers.set(sid, peer);
    return peer;
  }

  // ── Remote speaking indicators + stats ───────────────────────────────────

  #startPolling() {
    if (this.#pollTimer) return;
    this.#pollTimer = setInterval(() => {
      const now = performance.now();
      for (const peer of this.peers.values()) {
        const level = peer.audioLevel();
        if (level > REMOTE_SPEAKING_LEVEL) peer.speakingUntil = now + REMOTE_HOLD_MS;
        const speaking = now < peer.speakingUntil && !this.deafened;
        if (speaking !== peer.speaking) {
          peer.speaking = speaking;
          this.#emit('speaking', { sid: peer.sid, speaking });
        }
      }
    }, 100);
  }

  /**
   * Average round-trip time (ms) and what we're actually sending (kbps), plus
   * each connection's reading for the call report (`peers`: { sid, uid, peer,
   * reading, codecs }; see call-report.js › readCallStats, negotiatedVoice).
   * Also judges your own connection (OwnConnection): 'own-quality' when the
   * hint comes or goes.
   */
  async stats() {
    let rtt = 0;
    let rttCount = 0;
    let sendKbps = 0;
    const peers = [];
    const yours = []; // what each person we're sending to reports losing of our voice
    for (const peer of [...this.peers.values()]) {
      // Someone who left during this round: their closed connection has no stats (getStats throws).
      if (this.peers.get(peer.sid) !== peer) continue;
      let s;
      try {
        s = await peer.stats();
      } catch {
        continue;
      }
      if (s.rtt != null) {
        rtt += s.rtt;
        rttCount++;
      }
      sendKbps = Math.max(sendKbps, s.sendKbps ?? 0);
      if (s.weakChanged) this.#emit('quality', { sid: peer.sid, weak: peer.weak });
      if (s.sending && s.reading.theirs) yours.push(s.reading.theirs);
      peers.push({ sid: peer.sid, uid: peer.uid, peer, reading: s.reading, codecs: negotiatedVoice(peer.pc.localDescription?.sdp, peer.pc.remoteDescription?.sdp) });
    }
    if (this.#own.round(yours)) {
      this.ownWeak = this.#own.weak;
      this.#emit('own-quality', { weak: this.ownWeak });
    } else if (this.ownWeak) {
      this.ownWeak = this.#own.weak; // (still on: the latest numbers)
    }
    return { rtt: rttCount ? Math.round(rtt / rttCount) : null, sendKbps: Math.round(sendKbps), peers };
  }

  #emit(type, detail) {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}

class Peer {
  speaking = false;
  speakingUntil = 0;
  #makingOffer = false;
  #ignoreOffer = false;
  #lastStats = null;
  #lastLevelTs = 0;
  #lastLevelSeenAt = 0;
  #queue = Promise.resolve();

  /** Signals waiting for the socket to come back (a call outlives it). */
  #outbox = [];
  #recoverTimer = null;
  /** ICE restarts since the last time this connection was up. */
  restarts = 0;
  /** Their audio reaches us badly (see stats()): null, or { concealed: 0..1, rtt: ms }. */
  weak = null;
  #weakRun = 0; // consecutive bad (+) or good (−) readings
  #connectedAt = 0;
  /** ICE failed since it was last connected (no route between you: may need the relay). */
  failed = false;
  /** They asked to watch our screen (signal {screen: 'want'}), and when. */
  wantsScreen = false;
  wantedAt = 0;
  /**
   * The transceivers our screen goes out on to this peer (video, its sound).
   * Kept after they stop watching and used again next time, so watching on
   * and off doesn't pile up m-lines in the SDP.
   */
  #screenOut = { video: null, audio: null };
  /** Their screen as we hand it to Voice (video + its sound), or null. */
  #screenIn = null;

  constructor(voice, sid, initiator) {
    this.voice = voice;
    this.sid = sid; // changes if the far side resumes the call under a new session (Voice.rekey)
    this.uid = voice.uidOf(sid);
    this.pc = new RTCPeerConnection({ iceServers: voice.iceServers, bundlePolicy: 'max-bundle', iceTransportPolicy: voice.relayOnly ? 'relay' : 'all' });
    this.audio = document.createElement('audio');
    this.audio.autoplay = true;
    this.audio.muted = voice.deafened;
    // An OBS link keeps playing whatever happens (like VDO.Ninja's viewer: its element's onpause plays it again).
    if (voice.listenOnly) this.audio.addEventListener('pause', () => this.resume());
    document.getElementById('audio-sink')?.append(this.audio);
    this.applyOutput();

    this.pc.ontrack = ({ track, transceiver }) => {
      // The voice is always the first audio transceiver (some clients send it
      // with a stream id, so that can't tell them apart); anything else — the
      // video, a tab's audio — is a screen share.
      const mic = this.#micTransceiver();
      if (track.kind === 'video' || (mic && transceiver !== mic)) {
        this.syncRemoteScreen();
        return;
      }
      this.audio.srcObject = new MediaStream([track]);
      this.audio.play().catch(() => {});
      this.#unboost();
      this.applyOutput();
    };
    this.pc.onicecandidate = ({ candidate }) => {
      if (candidate) this.send({ candidate: candidate.toJSON() });
    };
    this.pc.onnegotiationneeded = () => this.renegotiate();
    // Each finished negotiation says what they send us now: a screen came, changed or went.
    this.pc.onsignalingstatechange = () => this.pc.signalingState === 'stable' && this.syncRemoteScreen();
    this.pc.onconnectionstatechange = () => {
      const state = this.pc.connectionState;
      voice.dispatchEvent(new CustomEvent('peer', { detail: { sid: this.sid, state } }));
      if (state === 'connected') {
        this.applySenderParams();
        // (Re)connected to someone whose screen we're watching: ask again.
        if (voice.isWatching(this.sid)) this.send({ screen: 'want' });
      }
      this.#watchPath(state);
    };

    if (initiator) {
      // Adding the transceiver fires negotiationneeded → we send the offer (RED first, then Opus).
      const voiceTransceiver = this.pc.addTransceiver(voice.outTrack ?? 'audio', { direction: 'sendrecv' });
      preferAudio(voiceTransceiver, { red: voice.red });
    }
  }

  /** Their voice's element stopped (a browser can pause it): play it again. */
  resume() {
    if (this.audio.srcObject && this.audio.paused && this.voice.peers.get(this.sid) === this) this.audio.play().catch(() => {});
  }

  /**
   * The path died (Wi-Fi → cellular, a NAT rebinding, a sleeping laptop): the
   * impolite side restarts ICE after 3.5 s of `disconnected` (at once on
   * `failed`) with freshly fetched ICE servers; the polite side does it after
   * 10 s in case the other app only restarts on `failed`. It keeps trying
   * every 10 s; after the third try with someone whose app can start over,
   * it rebuilds the connection from scratch.
   */
  #watchPath(state) {
    clearTimeout(this.#recoverTimer);
    this.#recoverTimer = null;
    if (state === 'connected') {
      this.restarts = 0;
      this.failed = false;
      this.#connectedAt = performance.now();
      return;
    }
    if (state === 'failed') this.failed = true;
    if (state !== 'disconnected' && state !== 'failed') return;
    const wait = this.polite ? 10_000 : state === 'failed' ? 0 : 3500;
    this.#recoverTimer = setTimeout(() => this.#recover(), wait);
  }

  async #recover() {
    const state = this.pc.connectionState;
    if (state !== 'disconnected' && state !== 'failed') return;
    if (!this.voice.peers.has(this.sid) || this.voice.peers.get(this.sid) !== this) return; // closed meanwhile
    this.restarts++;
    if (this.restarts > 3 && !this.polite && this.voice.resumable(this.sid)) return this.voice.restart(this.sid);
    this.voice.dispatchEvent(new CustomEvent('restart', { detail: { sid: this.sid, attempt: this.restarts } }));
    const iceServers = await this.voice.freshIceServers();
    try {
      this.pc.setConfiguration({ ...this.pc.getConfiguration(), iceServers });
    } catch {}
    if (this.pc.signalingState === 'closed') return;
    this.pc.restartIce(); // → negotiationneeded → an offer with new ICE credentials
    this.#voiceEvent();
    this.#recoverTimer = setTimeout(() => this.#recover(), 10_000);
  }

  #voiceEvent() {
    this.voice.dispatchEvent(new CustomEvent('peer', { detail: { sid: this.sid, state: this.pc.connectionState } }));
  }

  /** The session with the larger sid is polite (recomputed: either side's sid can change on a resume). */
  get polite() {
    return this.voice.mySid > this.sid;
  }

  /** A signal to the far side, or into the queue while our socket is down. */
  send(data) {
    if (this.voice.online) this.voice.signal(this.sid, data);
    else this.#outbox.push(data);
  }

  /**
   * After a reconnect/resume: send what waited, and if an offer of ours is
   * still unanswered, send it again (the answer may have gone to our old
   * session and been lost).
   */
  resync() {
    const queued = this.#outbox.splice(0);
    for (const data of queued) this.voice.signal(this.sid, data);
    const local = this.pc.localDescription;
    if (this.pc.signalingState === 'have-local-offer' && local && !queued.some((d) => d.sdp?.sdp === local.sdp)) {
      this.voice.signal(this.sid, { sdp: local.toJSON() });
    }
  }

  async renegotiate() {
    try {
      this.#makingOffer = true;
      const offer = await this.pc.createOffer();
      if (this.pc.signalingState !== 'stable') return;
      await this.pc.setLocalDescription({ type: 'offer', sdp: tuneOpus(offer.sdp) });
      this.send({ sdp: this.pc.localDescription.toJSON() });
    } catch (err) {
      console.warn('offer failed', err);
    } finally {
      this.#makingOffer = false;
    }
  }

  /** Signals are applied strictly in order (a candidate can't beat its offer). */
  handle(data) {
    this.#queue = this.#queue.then(() => this.#handle(data));
  }

  async #handle({ sdp, candidate }) {
    try {
      if (sdp) {
        const collision = sdp.type === 'offer' && (this.#makingOffer || this.pc.signalingState !== 'stable');
        this.#ignoreOffer = !this.polite && collision;
        if (this.#ignoreOffer) return;
        await this.pc.setRemoteDescription(sdp);
        if (sdp.type === 'offer') {
          // Answer with our mic on the transceiver they offered.
          const tr = this.pc.getTransceivers().find((t) => t.receiver.track.kind === 'audio');
          if (tr && !tr.sender.track && this.voice.outTrack) await tr.sender.replaceTrack(this.voice.outTrack);
          if (tr) tr.direction = 'sendrecv';
          // The answer lists RED first on the voice (if they offered it), plain Opus on any other audio:
          // what they send us follows our answer's order, what we send follows their offer's.
          for (const t of this.pc.getTransceivers()) {
            if (t.receiver.track.kind === 'audio') preferAudio(t, { red: t === tr && this.voice.red });
          }
          const answer = await this.pc.createAnswer();
          await this.pc.setLocalDescription({ type: 'answer', sdp: tuneOpus(answer.sdp) });
          this.send({ sdp: this.pc.localDescription.toJSON() });
        }
        this.applySenderParams();
      } else if (candidate) {
        try {
          await this.pc.addIceCandidate(candidate);
        } catch (err) {
          if (!this.#ignoreOffer) throw err;
        }
      }
    } catch (err) {
      console.warn('signal handling failed', err);
    }
  }

  /** The mic is always the first audio transceiver (screen audio comes later). */
  #micTransceiver() {
    return this.pc.getTransceivers().find((t) => t.receiver.track.kind === 'audio');
  }

  setTrack(track) {
    this.#micTransceiver()?.sender.replaceTrack(track).catch((err) => console.warn('replaceTrack failed', err));
  }

  // ── Our screen, to this one peer ─────────────────────────────────────────

  /** The senders carrying our screen to this peer right now, or null. */
  get screenSenders() {
    const senders = [this.#screenOut.video, this.#screenOut.audio].filter((t) => t?.sender.track).map((t) => t.sender);
    return senders.length ? senders : null;
  }

  /**
   * Send what this peer should get: our screen if they asked to watch it
   * (video, plus its sound while Share sound is on), else nothing — so
   * nothing is encoded for someone who isn't watching. `codecs`: the preset's
   * codec order changed, renegotiate so it takes effect.
   */
  syncScreen({ codecs = false } = {}) {
    const stream = this.wantsScreen ? this.voice.screenStream : null;
    const video = stream?.getVideoTracks()[0] ?? null;
    const audio = video && this.voice.screenSound ? stream.getAudioTracks()[0] ?? null : null;
    const q = this.voice.screenQuality ?? SCREEN_QUALITY.smooth;
    this.#sendOn('video', video, stream, { codecs: q.codecs });
    // Share sound off mid-share: the sound's track just goes quiet (no renegotiation, no flicker for viewers).
    this.#sendOn('audio', audio, stream, { keep: Boolean(video) });
    if (codecs && video && this.#screenOut.video) {
      preferCodecs(this.#screenOut.video, q.codecs);
      this.renegotiate();
    }
    this.applySenderParams();
  }

  /**
   * Put `track` (or nothing) on our screen's `kind` transceiver, making or
   * reusing it. A new source (Share something else) is a replaceTrack: no
   * renegotiation. `keep`: with no track, stay negotiated as sending.
   */
  #sendOn(kind, track, stream, { codecs = null, keep = false } = {}) {
    let tr = this.#screenOut[kind];
    if (tr && tr.currentDirection === 'stopped') tr = this.#screenOut[kind] = null;
    const sending = Boolean(tr) && (tr.direction === 'sendrecv' || tr.direction === 'sendonly');
    if (!track) {
      if (!sending) return;
      if (tr.sender.track) tr.sender.replaceTrack(null).catch(() => {});
      if (!keep) tr.direction = tr.direction === 'sendrecv' ? 'recvonly' : 'inactive'; // → renegotiation: they see it go
      return;
    }
    if (!tr) {
      // addTrack (not addTransceiver): like every other client, so either side may reuse the other's m-line.
      const sender = this.pc.addTrack(track, stream);
      tr = this.#screenOut[kind] = this.pc.getTransceivers().find((t) => t.sender === sender) ?? null;
      if (tr && codecs) preferCodecs(tr, codecs);
      if (tr && kind === 'audio') preferAudio(tr, { red: false }); // a share's sound: plain Opus, never RED
      return;
    }
    if (tr.sender.track !== track) tr.sender.replaceTrack(track).catch((err) => console.warn('replaceTrack failed', err));
    if (!sending) tr.direction = tr.direction === 'recvonly' ? 'sendrecv' : 'sendonly';
  }

  // ── Their screen ────────────────────────────────────────────────────────

  /**
   * Their screen, from what is negotiated rather than from stream ids: every
   * transceiver but the mic that they send on. Works with any WebRTC stack —
   * a video track with no stream id, or its sound in a stream of its own.
   */
  syncRemoteScreen() {
    const mic = this.#micTransceiver();
    const tracks = this.pc.getTransceivers()
      .filter((t) => t !== mic && (t.currentDirection === 'recvonly' || t.currentDirection === 'sendrecv'))
      .map((t) => t.receiver.track)
      .filter((t) => t?.readyState === 'live');
    const video = tracks.find((t) => t.kind === 'video');
    if (!video) {
      if (this.#screenIn) {
        this.#screenIn = null;
        this.voice.onRemoteScreen(this.sid, null);
      }
      return;
    }
    const want = [video, ...tracks.filter((t) => t.kind === 'audio')];
    const have = this.#screenIn?.getTracks() ?? [];
    if (want.length === have.length && want.every((t) => have.includes(t))) return;
    this.#screenIn = new MediaStream(want);
    this.voice.onRemoteScreen(this.sid, this.#screenIn);
  }

  /**
   * Volume, deafen, Space's left/right and output device. Up to 100% the
   * <audio> element plays it; past that, or while Space pans them, a WebAudio
   * graph does (the element keeps playing, muted, because Chrome only feeds
   * WebRTC audio to WebAudio while an element plays it): a gain, a gain per
   * ear (panGains), then a limiter so a boosted shout can't clip or blast
   * your ears.
   */
  applyOutput({ force = false } = {}) {
    if (this.voice.listenOnly) {
      // Straight through: the element plays it as it comes, at full volume, to the default output
      // (OBS captures the page's sound and sets the volume itself). Off = `&only=` someone else.
      this.audio.volume = 1;
      this.audio.muted = this.voice.deafened || (this.proximity ?? 1) === 0;
      return;
    }
    const uid = this.voice.uidOf(this.sid);
    const level = Math.max(0, (settings.volumes[uid] ?? 1) * (this.proximity ?? 1));
    const pan = this.pan ?? null;
    if ((level > 1.001 || pan !== null) && this.audio.srcObject) {
      // (Every time, not just when the graph is made: it follows a newly picked output, and is resumed if it was interrupted.)
      const ctx = this.voice.outputContext();
      if (!this.boost) {
        const source = ctx.createMediaStreamSource(this.audio.srcObject);
        const gain = ctx.createGain();
        // Two channels from here on (a mono voice goes to both), one gain per ear.
        gain.channelCount = 2;
        gain.channelCountMode = 'explicit';
        gain.channelInterpretation = 'speakers';
        const split = ctx.createChannelSplitter(2);
        const left = ctx.createGain();
        const right = ctx.createGain();
        const merge = ctx.createChannelMerger(2);
        // A brick-wall-ish limiter just under full scale (the boost is for quiet talkers, not loud ones).
        const limiter = ctx.createDynamicsCompressor();
        limiter.threshold.value = -3;
        limiter.knee.value = 0;
        limiter.ratio.value = 20;
        limiter.attack.value = 0.002;
        limiter.release.value = 0.15;
        source.connect(gain).connect(split);
        split.connect(left, 0);
        split.connect(right, 1);
        left.connect(merge, 0, 0);
        right.connect(merge, 0, 1);
        merge.connect(limiter).connect(ctx.destination);
        this.boost = { source, gain, split, left, right, merge, limiter };
      }
      this.boost.gain.gain.value = this.voice.deafened ? 0 : level;
      const [l, r] = panGains(pan ?? 0);
      this.boost.left.gain.value = l;
      this.boost.right.gain.value = r;
      this.audio.muted = true;
    } else {
      this.#unboost();
      this.audio.muted = this.voice.deafened;
      this.audio.volume = Math.min(1, level);
    }
    // The picked speakers (after a device change, again: they may have just come back, or gone).
    if (this.audio.setSinkId && (force || this.audio.sinkId !== settings.outputId)) {
      this.audio.setSinkId(settings.outputId).catch(() => this.audio.setSinkId('').catch(() => {}));
    }
  }

  #unboost() {
    if (!this.boost) return;
    for (const node of Object.values(this.boost)) node.disconnect();
    this.boost = null;
  }

  applySenderParams() {
    for (const sender of this.pc.getSenders()) {
      const params = sender.getParameters();
      if (!params.encodings?.length) continue;
      if (sender === this.#screenOut.video?.sender || sender.track?.kind === 'video') {
        // A screen: the quality picked when sharing (games: frame rate first; text: sharpness first).
        const q = this.voice.screenQuality ?? SCREEN_QUALITY.smooth;
        params.encodings[0].maxBitrate = q.bitrate;
        params.encodings[0].maxFramerate = q.fps;
        params.encodings[0].scaleResolutionDownBy = 1;
        params.degradationPreference = q.degradation;
        sender.setParameters(params).catch(() => {});
        continue;
      }
      // The voice, or a screen share's audio (stereo music: its own, higher cap). The cap is Opus's
      // target: Chrome doesn't count RED's copy against it (measured: 128 kbps cap → Opus target
      // 128,000, ~258 kbps of payload), so the voice keeps the quality setting with RED on top.
      params.encodings[0].maxBitrate = sender === this.#screenOut.audio?.sender ? screenAudioBitrate() : callBitrate();
      params.encodings[0].priority = 'high';
      params.encodings[0].networkPriority = 'high';
      sender.setParameters(params).catch(() => {});
    }
  }

  /** Latest RTP audio level from the remote side, or 0 if packets stopped. */
  audioLevel() {
    const receiver = this.pc.getReceivers()[0];
    const source = receiver?.getSynchronizationSources?.()[0];
    if (!source) return 0;
    const now = performance.now();
    if (source.timestamp !== this.#lastLevelTs) {
      this.#lastLevelTs = source.timestamp;
      this.#lastLevelSeenAt = now;
    }
    return now - this.#lastLevelSeenAt > 400 ? 0 : source.audioLevel ?? 0;
  }

  /**
   * Round trip, what we send, and how well their voice reaches us. "Weak" =
   * more than 3 % of their audio concealed (patched over after loss or late
   * packets) or a round trip over 300 ms, judged only while they're actually
   * sending (an app that stops sending in silence isn't "weak") and not in
   * the first 10 s of a connection (the jitter buffer is still settling).
   * Shown after three bad readings in a row (~6 s), cleared after three good.
   */
  async stats() {
    const report = await this.pc.getStats();
    const voiceMid = this.#micTransceiver()?.mid;
    const reading = readCallStats(report, voiceMid);
    const { rtt, audioBytesSent: bytesSent, received: inbound } = reading;
    const now = performance.now();
    const last = this.#lastStats;
    let sendKbps = null;
    let weakChanged = false;
    let sending = false; // our voice goes out to them (≥ 30 packets a second): their reports on it count
    if (last) {
      const secs = (now - last.at) / 1000;
      if (secs > 0) sendKbps = ((bytesSent - last.bytesSent) * 8) / 1000 / secs;
      sending = secs > 0 && this.pc.connectionState === 'connected' && ((reading.sent?.packets ?? 0) - (last.sentPackets ?? 0)) / secs >= 30;
      const pps = inbound && last.inbound ? (inbound.packets - last.inbound.packets) / secs : 0;
      if (pps >= 30 && this.pc.connectionState === 'connected' && now - this.#connectedAt > 10_000) {
        const samples = inbound.samples - last.inbound.samples;
        const concealed = (inbound.concealed - last.inbound.concealed) - (inbound.silentConcealed - last.inbound.silentConcealed);
        const share = samples > 0 ? Math.max(0, concealed) / samples : 0;
        const bad = share > WEAK_CONCEALED || (rtt ?? 0) > WEAK_RTT_MS;
        this.#weakRun = bad ? Math.max(1, this.#weakRun + 1) : Math.min(-1, this.#weakRun - 1);
        const was = Boolean(this.weak);
        if (this.#weakRun >= 3) this.weak = { concealed: share, rtt };
        else if (this.#weakRun <= -3) this.weak = null;
        else if (this.weak && bad) this.weak = { concealed: share, rtt };
        weakChanged = was !== Boolean(this.weak);
      }
    }
    this.#lastStats = { at: now, bytesSent, inbound, sentPackets: reading.sent?.packets ?? 0 };
    return { rtt, sendKbps, weakChanged, reading, sending };
  }

  close() {
    clearTimeout(this.#recoverTimer);
    this.pc.ontrack = this.pc.onicecandidate = this.pc.onnegotiationneeded = this.pc.onconnectionstatechange = this.pc.onsignalingstatechange = null;
    this.#unboost();
    this.pc.close();
    this.audio.srcObject = null;
    this.audio.remove();
  }
}

/**
 * Space's left/right as a gain per ear (PROTOCOL.md › Space): the near ear
 * stays full, the far one goes down to 1/4 of it at the edge (pan ±1), and
 * the middle (0) is full in both. [left, right].
 */
export function panGains(pan) {
  const p = Math.max(-1, Math.min(1, Number(pan) || 0));
  const far = 1 - 0.75 * Math.abs(p);
  return p >= 0 ? [far, 1] : [1, far];
}

/** Screen-share audio (a tab, a game) is music: stereo and at least this rich. */
export const screenAudioBitrate = () => Math.max(callBitrate(), 160_000);

const TWCC = /^a=(?:rtcp-fb:\d+ transport-cc|extmap:\d+(?:\/\w+)? http:\/\/www\.ietf\.org\/id\/draft-holmer-rmcat-transport-wide-cc-extensions-01)$/;

/**
 * Tune every audio section of our SDP (offer or answer) before
 * setLocalDescription. The Opus parameters describe what *we* want to
 * receive, so each side tunes its own description:
 *  - maxaveragebitrate: our quality setting, what we ask to receive. It's a
 *    request, not a cap: browsers (libwebrtc) don't hold their encoder to the
 *    other side's value, so each person's voice goes out at their own setting
 *    (the sender caps itself via maxBitrate; measured 2026-10-11)
 *  - useinbandfec=1: forward error correction — survives packet loss
 *  - usedtx=0: keep sending during silence (no clipped first syllables)
 *  - stereo=1: we can play stereo (music mode, screen-share audio). It also
 *    makes a libwebrtc sender (Chrome, Firefox) encode with Opus's "audio"
 *    mode instead of its "voip" mode, which high-passes the voice and tunes
 *    for speech: measured on a synthetic voice at 510 kbps, 0.03 dB from the
 *    original spectrum against 0.8 dB in voip mode (2026-10-11, for the OBS link)
 *  - sprop-stereo: whether *we* send stereo there (music mode; our own
 *    screen's audio)
 *  - minptime=10, maxplaybackrate=48000: low latency, full band
 * The first audio section is the voice; any later one is a screen share's
 * audio (PROTOCOL.md › Screen sharing).
 *
 * RED doesn't need tuning here: the voice's codec order (RED, then Opus) comes
 * from setCodecPreferences (preferAudio), and the browser writes RED's own
 * `a=fmtp:<red> <opus>/<opus>` (one earlier frame). The Opus line is found by
 * its rtpmap, wherever it sits in the m= line, and is what RED carries, so
 * these parameters still set the bitrate, FEC and stereo of the voice.
 *
 * Audio sections also drop `transport-cc` (both the rtcp-fb and the header
 * extension): with it, Chrome's bandwidth probing pads the audio stream
 * (~18 kbps per direction per person when the network is clean, hundreds
 * when it's lossy) and gains nothing, since Opus runs at a fixed target.
 * Video (screen sharing) keeps it. An answer can't add back what the offer
 * left out, so this works whoever offers.
 */
export function tuneOpus(sdp) {
  const eol = sdp.includes('\r\n') ? '\r\n' : '\n';
  const [head, ...sections] = sdp.split(`${eol}m=`);
  let audio = 0;
  const tuned = sections.map((body) => {
    if (!body.startsWith('audio')) return body;
    const voice = audio++ === 0;
    const sending = /^a=(sendrecv|sendonly)$/m.test(body);
    return tuneAudio(body, eol, {
      'sprop-stereo': (voice ? settings.music : sending) ? '1' : '0',
      maxaveragebitrate: String(voice ? callBitrate() : screenAudioBitrate()),
    });
  });
  return [head, ...tuned].join(`${eol}m=`);
}

function tuneAudio(body, eol, own) {
  const pt = body.match(/^a=rtpmap:(\d+) opus\/48000/im)?.[1];
  const want = {
    minptime: '10',
    useinbandfec: '1',
    usedtx: '0',
    stereo: '1',
    'sprop-stereo': own['sprop-stereo'],
    maxaveragebitrate: own.maxaveragebitrate,
    maxplaybackrate: '48000',
    'sprop-maxcapturerate': '48000',
  };
  const join = (params) => Object.entries(params).map(([k, v]) => `${k}=${v}`).join(';');
  const lines = body.split(eol).filter((line) => !TWCC.test(line));
  if (!pt) return lines.join(eol);
  const fmtp = lines.findIndex((line) => line.startsWith(`a=fmtp:${pt} `));
  if (fmtp >= 0) {
    const params = Object.fromEntries(
      lines[fmtp].slice(`a=fmtp:${pt} `.length).split(';').map((kv) => kv.trim().split('=')).filter(([k]) => k),
    );
    lines[fmtp] = `a=fmtp:${pt} ${join({ ...params, ...want })}`;
  } else {
    const rtpmap = lines.findIndex((line) => line.toLowerCase().startsWith(`a=rtpmap:${pt} opus/48000`));
    lines.splice(rtpmap + 1, 0, `a=fmtp:${pt} ${join(want)}`);
  }
  return lines.join(eol);
}
