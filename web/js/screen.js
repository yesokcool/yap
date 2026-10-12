/*
 * Watching someone's screen share (PROTOCOL.md › Screen sharing). The video
 * arrives on the voice peer connection once we ask the sharer for it; this
 * view shows it big, fitted to the window (letterboxed, never cropped or
 * stretched), with the share's own sound volume, Pop out (picture-in-picture,
 * so you can keep browsing), Full screen (or double-click) and Stop watching.
 *
 * It says what's going on: "Connecting to Ana's screen…" until the first
 * frame, "Ana's screen is paused." when no frame has come for 3 s, and
 * "Ana stopped sharing." (the page closes the view 2 s later).
 */
import { icon } from './icons.js';

/** No frame for this long = the sharer's screen is paused (a still screen still sends ~1 frame a second). */
export const PAUSED_AFTER_MS = 3000;

export class ScreenView {
  /**
   * @param {HTMLElement} el
   * @param {{
   *   who: (sid: string) => {name: string, color: string} | null,
   *   volume: (sid: string) => number, setVolume: (sid: string, v: number) => void,
   *   muted: () => boolean, sinkId: () => string,
   *   stop: () => void, leftPip?: () => void,
   * }} hooks
   */
  constructor(el, hooks) {
    this.el = el;
    this.hooks = hooks;
    this.sid = null;
    /** connecting | live | paused | stopped */
    this.state = 'connecting';
    this.lastFrame = 0;
    this.timer = null;
    this.frameCall = null;
    /** × the share's volume while friends talk (Settings › Voice › Lower shared media while friends talk). */
    this.duck = 1;
    el.innerHTML = `
      <div class="screen-bar">
        <span class="screen-title"></span>
        <span class="spacer"></span>
        <label class="screen-volume" title="The sound of their screen, just for you (their voice keeps its own volume)">
          ${icon('volume')}<input type="range" min="0" max="100" step="5" aria-label="Screen sound volume"><output></output>
        </label>
        <button class="btn small" type="button" data-screen="pip" title="Keep watching in a small window while you use the rest of Yap">Pop out</button>
        <button class="btn small" type="button" data-screen="full" title="Full screen (or double-click the picture)">Full screen</button>
        <button class="btn small" type="button" data-screen="stop">Stop watching</button>
      </div>
      <div class="screen-stage">
        <video class="screen-video" autoplay playsinline></video>
        <p class="screen-state" role="status"></p>
      </div>`;
    this.video = el.querySelector('video');
    this.stage = el.querySelector('.screen-stage');
    this.slider = el.querySelector('.screen-volume input');
    // Pop out only where the browser can (Firefox has its own button on the video instead).
    el.querySelector('[data-screen="pip"]').hidden = !document.pictureInPictureEnabled;
    el.addEventListener('click', (e) => {
      const what = e.target.closest('[data-screen]')?.dataset.screen;
      if (what === 'pip') this.video.requestPictureInPicture?.().catch(() => {});
      if (what === 'full') this.toggleFullScreen();
      if (what === 'stop') this.hooks.stop();
    });
    this.stage.addEventListener('dblclick', () => this.toggleFullScreen());
    this.slider.addEventListener('input', () => {
      if (!this.sid) return;
      this.hooks.setVolume(this.sid, Number(this.slider.value) / 100);
      this.applyAudio();
    });
    this.video.addEventListener('leavepictureinpicture', () => this.hooks.leftPip?.());
    // Back from another tab: frames that weren't drawn meanwhile don't count as a pause.
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') this.lastFrame = performance.now();
    });
  }

  get name() {
    return this.hooks.who(this.sid)?.name ?? 'Someone';
  }

  /** Start showing `sid`'s screen (the stream may follow a moment later). */
  open(sid) {
    this.sid = sid;
    this.el.querySelector('.screen-title').textContent = `${this.name}'s screen`;
    this.video.srcObject = null;
    this.#say('connecting');
    this.applyAudio();
    clearInterval(this.timer);
    this.timer = setInterval(() => this.#check(), 500);
  }

  /** The sharer resumed their call under a new session: same screen, new sid. */
  moveTo(sid) {
    if (this.sid) this.sid = sid;
  }

  setStream(sid, stream) {
    if (sid !== this.sid) return;
    if (this.video.srcObject !== stream) this.video.srcObject = stream;
    if (this.state === 'stopped') return;
    if (!stream) return this.#say('connecting');
    this.applyAudio();
    this.video.play().catch(() => {});
    this.#watchFrames();
  }

  /** They stopped sharing (the page closes the view in a moment). */
  ended() {
    this.#say('stopped');
  }

  /** The share's sound: this person's screen volume (lowered while friends talk, if asked), silent while you're deafened, on your speakers. */
  applyAudio() {
    if (!this.sid) return;
    const volume = Math.min(1, Math.max(0, this.hooks.volume(this.sid)));
    this.video.volume = volume * this.duck;
    this.video.muted = this.hooks.muted();
    this.slider.value = String(Math.round(volume * 100));
    this.el.querySelector('.screen-volume output').textContent = `${Math.round(volume * 100)}%`;
    const sink = this.hooks.sinkId();
    if (this.video.setSinkId && this.video.sinkId !== sink) this.video.setSinkId(sink).catch(() => this.video.setSinkId('').catch(() => {}));
  }

  /** Lower (or raise back) the share's sound while friends talk: a factor on its volume; the slider keeps showing yours. */
  setDuck(factor) {
    this.duck = Math.min(1, Math.max(0, factor));
    if (this.sid) this.video.volume = Math.min(1, Math.max(0, this.hooks.volume(this.sid))) * this.duck;
  }

  toggleFullScreen() {
    if (document.fullscreenElement) return document.exitFullscreen().catch(() => {});
    if (this.stage.requestFullscreen) return this.stage.requestFullscreen().catch(() => {});
    this.video.webkitEnterFullscreen?.(); // iPhone: only a video can go full screen
  }

  /** Popped out into picture-in-picture (keeps playing outside this view). */
  get poppedOut() {
    return document.pictureInPictureElement === this.video;
  }

  close() {
    if (this.poppedOut) document.exitPictureInPicture().catch(() => {});
    if (document.fullscreenElement && this.el.contains(document.fullscreenElement)) document.exitFullscreen().catch(() => {});
    clearInterval(this.timer);
    this.timer = null;
    if (this.frameCall != null) this.video.cancelVideoFrameCallback?.(this.frameCall);
    this.frameCall = null;
    this.video.srcObject = null;
    this.sid = null;
  }

  // ── Frames and what to say ────────────────────────────────────────────────

  /** Note each new frame (requestVideoFrameCallback; the frame counter where there isn't one). */
  #watchFrames() {
    if (!this.video.requestVideoFrameCallback || this.frameCall != null) return;
    const onFrame = () => {
      this.frameCall = this.video.requestVideoFrameCallback(onFrame);
      this.#frame();
    };
    this.frameCall = this.video.requestVideoFrameCallback(onFrame);
  }

  #frames = 0;
  #check() {
    if (!this.sid) return;
    if (!this.video.requestVideoFrameCallback) {
      const n = this.video.getVideoPlaybackQuality?.().totalVideoFrames ?? 0;
      if (n !== this.#frames) {
        this.#frames = n;
        this.#frame();
      }
    }
    const still = performance.now() - this.lastFrame > PAUSED_AFTER_MS;
    if (this.state === 'live' && still && document.visibilityState === 'visible') this.#say('paused');
  }

  #frame() {
    this.lastFrame = performance.now();
    if (this.state === 'connecting' || this.state === 'paused') this.#say('live');
  }

  #say(state) {
    this.state = state;
    this.el.dataset.state = state;
    const name = this.name;
    this.el.querySelector('.screen-state').textContent = {
      connecting: `Connecting to ${name}'s screen…`,
      paused: `${name}'s screen is paused.`,
      stopped: `${name} stopped sharing.`,
      live: '',
    }[state];
  }
}
