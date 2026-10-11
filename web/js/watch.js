/*
 * Watch together — one shared YouTube player for the whole server
 * (PROTOCOL.md › Watch together).
 *
 * The server keeps what's on (video, playing/paused, a position at a server
 * time, the queue); everyone's own player follows it and seeks when it drifts
 * more than DRIFT seconds. Pausing or seeking in your own player (or with the
 * controls under it) changes it for everyone.
 *
 * The player is a youtube-nocookie embed driven with postMessage only:
 * YouTube's iframe_api script never runs in Yap's page (the CSP forbids it).
 *
 * The iframe lives in one floating "dock" that is never moved in the DOM
 * (moving an iframe reloads it, which would restart the video). On the Watch
 * view the dock sits exactly over the 16:9 slot; anywhere else in the app it
 * becomes a small mini player in the corner, so the video keeps going while
 * you browse. Voice is never touched.
 */

import { esc, youtubeId } from './format.js';
import { icon } from './icons.js';

const YT = 'https://www.youtube-nocookie.com';
// YouTube player states.
const UNSTARTED = -1;
const ENDED = 0;
const PLAYING = 1;
const PAUSED = 2;
const BUFFERING = 3;
const CUED = 5;
const DRIFT = 1.5; // seconds off before we seek to where everyone is
const PREFS_KEY = 'yap.watch'; // { volume 0–100, muted } — this device only
const MIN_SEATS = 6; // empty chairs fill the row so it reads as a theater
const GAP = 10; // mini player ↔ composer / window edge

export { youtubeId };

/** 75 → "1:15", 3725 → "1:02:05". */
export function clock(seconds) {
  const s = Math.max(0, Math.floor(Number(seconds) || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${r}` : `${m}:${r}`;
}

const thumb = (id, size = 'mqdefault') => `https://i.ytimg.com/vi/${encodeURIComponent(id)}/${size}.jpg`;
const titleOf = (item) => item?.title || 'YouTube video';

function loadPrefs() {
  try {
    const p = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}');
    return { volume: Number.isFinite(p.volume) ? Math.min(100, Math.max(0, Math.round(p.volume))) : 70, muted: Boolean(p.muted) };
  } catch {
    return { volume: 70, muted: false };
  }
}

/** What our player last told us. `video` is what we asked it to load. */
const freshPlayer = (video = null) => ({
  ready: false, talking: false, state: UNSTARTED, sampleState: UNSTARTED,
  time: null, timeAt: 0, duration: 0, video, started: false,
});

export class WatchParty {
  /**
   * @param {HTMLElement} el the Watch view (#watch-view)
   * @param {{
   *   send: (m: object) => void, mySid: () => string | null, myUid: () => string,
   *   sessions: () => Map<string, any>, users: () => Map<string, any>,
   *   avatar: (user: any, cls?: string) => string, speaking: (sid: string) => boolean,
   *   open: () => void, toast: (text: string) => void, changed: () => void,
   * }} host
   */
  constructor(el, host) {
    this.el = el;
    this.host = host;
    /** The server's WatchState, and server clock − our clock (ms). */
    this.state = null;
    this.offset = 0;
    this.joined = false; // in the audience (we sent watch.join)
    this.inView = false; // the Watch view is open
    this.mode = 'hidden'; // dock: hidden | view | mini
    this.built = false;
    this.frame = null; // the one YouTube iframe
    this.player = freshPlayer();
    this.prefs = loadPrefs();
    this.blocked = 0; // 0 fine · 1 "Click to join the show" · 2 "press ▶ on the video"
    this.unblockTried = false;
    this.cmdAt = 0; // last play/pause/seek/load we sent our player (its echoes aren't the person's doing)
    this.playAskedAt = 0;
    this.playTries = 0;
    this.pauseAskedAt = 0;
    this.seekAt = 0;
    this.nextAt = 0;
    this.volAt = 0;
    this.touchedAt = -Infinity; // the person last clicked into the player
    this.visibilityAt = -Infinity;
    this.seeking = false; // dragging our seek bar
    this.dragged = null; // mini player moved by hand: { x, y }
    this.drag = null;
    this.note = { text: '', until: 0 };
    this.timer = null;
    this.raf = 0;
    this.rectKey = '';
    this.curtainKey = '';
    this.dock = this.makeDock();

    addEventListener('message', (e) => this.onMessage(e));
    addEventListener('resize', () => this.placeMini());
    // Clicking into the player moves focus into its iframe: that's how we know
    // a pause/seek in there was the person's own doing (not an ad, not the OS).
    addEventListener('blur', () => setTimeout(() => {
      if (this.frame && document.activeElement === this.frame) this.touchedAt = performance.now();
    }));
    document.addEventListener('visibilitychange', () => (this.visibilityAt = performance.now()));
    const composer = document.querySelector('.composer-wrap');
    if (composer && typeof ResizeObserver !== 'undefined') new ResizeObserver(() => this.placeMini()).observe(composer);
    this.bindView();
  }

  // ── Server messages ─────────────────────────────────────────────────────

  /**
   * After every (re)connect: what's on, and back into the audience if we were
   * in it. `ready.sessions` already lists everyone watching, so the seats and
   * the sidebar show them straight away.
   */
  onReady() {
    this.host.send(this.joined ? { t: 'watch.join' } : { t: 'watch.get' });
    this.renderSeats();
    this.host.changed();
  }

  onState(msg) {
    const prev = this.state;
    const s = msg.state;
    this.state = s;
    this.offset = Number(msg.now) - Date.now() || 0;
    if (prev) this.describe(prev, s);
    if (s.playing && (!prev?.playing || prev.videoId !== s.videoId)) {
      this.playAskedAt = 0;
      this.playTries = 0;
    }
    // The show ended while you were off browsing: quietly leave the theater.
    if (this.joined && !s.videoId && prev?.videoId && !this.inView) this.leave();
    this.ensureFrame();
    this.layout();
    this.render();
    this.sync();
    this.host.changed();
  }

  /** Someone joined or left the audience (or disconnected). */
  onAudience() {
    this.renderSeats();
  }

  // ── Places ──────────────────────────────────────────────────────────────

  /** Called by the app on every view change. Opening Watch takes a seat. */
  setView(view) {
    const inView = view === 'watch';
    if (inView === this.inView) return;
    this.inView = inView;
    if (inView) {
      this.build();
      this.join();
    } else if (this.joined && !this.state?.videoId) {
      this.leave(); // nothing on: no point staying in the audience
    }
    this.timerCheck();
    this.layout();
    this.render();
  }

  join() {
    if (this.joined) return;
    this.joined = true;
    this.host.send({ t: 'watch.join' });
    this.timerCheck();
    this.ensureFrame();
    this.layout();
    this.render();
  }

  leave() {
    if (!this.joined) return;
    this.joined = false;
    this.host.send({ t: 'watch.leave' });
    this.dropFrame();
    this.timerCheck();
    this.layout();
    this.render();
  }

  /** Not a good moment to reload the page for an update. */
  busy() {
    return this.joined && Boolean(this.state?.videoId);
  }

  /** Extra bit for the sidebar row: "▶ title". Escaped. */
  linkExtra() {
    const s = this.state;
    if (!s?.videoId) return '';
    return `<span class="watch-now" title="${esc(titleOf(s))}">${s.playing ? '▶' : '❚❚'} ${esc(titleOf(s))}</span>`;
  }

  // ── The player ──────────────────────────────────────────────────────────

  ensureFrame() {
    const id = this.state?.videoId;
    if (!this.joined || !id || this.frame) return;
    const q = new URLSearchParams({
      enablejsapi: '1', playsinline: '1', origin: location.origin, rel: '0', iv_load_policy: '3',
      start: String(Math.floor(this.expected())),
    });
    const f = document.createElement('iframe');
    f.src = `${YT}/embed/${encodeURIComponent(id)}?${q}`;
    f.title = 'YouTube video player';
    f.allow = 'autoplay; encrypted-media; picture-in-picture; fullscreen';
    f.allowFullscreen = true;
    // Yap sends no Referer anywhere; YouTube's player refuses to start without one (error 153).
    f.referrerPolicy = 'strict-origin-when-cross-origin';
    f.addEventListener('load', () => this.listen());
    this.player = freshPlayer(id);
    this.frame = f;
    this.setBlocked(0);
    this.dock.querySelector('.watch-frame').prepend(f);
  }

  dropFrame() {
    clearInterval(this.listenTimer);
    this.frame?.remove();
    this.frame = null;
    this.player = freshPlayer();
    this.setBlocked(0);
  }

  /** The handshake YouTube's own API does: say "listening" until the player answers. */
  listen() {
    clearInterval(this.listenTimer);
    this.player.talking = false;
    const hello = () => this.post({ event: 'listening' });
    hello();
    this.listenTimer = setInterval(() => (this.player.talking ? clearInterval(this.listenTimer) : hello()), 250);
  }

  post(m) {
    try {
      this.frame?.contentWindow?.postMessage(JSON.stringify({ ...m, id: 1, channel: 'widget' }), YT);
    } catch {}
  }

  cmd(func, args = []) {
    if (/^(play|pause|seek|load|cue|stop)/.test(func)) this.cmdAt = performance.now();
    this.post({ event: 'command', func, args });
  }

  onMessage(e) {
    if (!this.frame || e.source !== this.frame.contentWindow || e.origin !== YT) return;
    let data = e.data;
    if (typeof data === 'string') {
      try {
        data = JSON.parse(data);
      } catch {
        return;
      }
    }
    if (!data || typeof data !== 'object') return;
    this.player.talking = true;
    if (data.event === 'onReady') this.onPlayerReady();
    else if (data.event === 'onStateChange') this.onPlayerState(Number(data.info));
    else if (data.event === 'onError') this.onPlayerError(Number(data.info));
    else if ((data.event === 'infoDelivery' || data.event === 'initialDelivery') && data.info && typeof data.info === 'object') this.onInfo(data.info);
  }

  onPlayerReady() {
    const p = this.player;
    if (p.ready) return;
    p.ready = true;
    for (const name of ['onStateChange', 'onError']) this.post({ event: 'command', func: 'addEventListener', args: [name] });
    this.applyVolume();
    this.sync();
  }

  onInfo(info) {
    const p = this.player;
    if (Number.isFinite(info.duration) && info.duration > 0) p.duration = info.duration;
    if (info.playerState !== undefined) this.onPlayerState(Number(info.playerState));
    if (Number.isFinite(info.currentTime)) this.onTime(info.currentTime);
    if (Number.isFinite(info.volume) || typeof info.muted === 'boolean') this.onPlayerVolume(info);
  }

  onPlayerState(st) {
    const p = this.player;
    if (!Number.isFinite(st) || st === p.state) return;
    const prev = p.state;
    if (p.time !== null) {
      p.time = this.localTime(); // keep the estimate honest across the change
      p.timeAt = performance.now();
    }
    p.state = st;
    p.sampleState = st;
    const s = this.state;
    if (st === PLAYING || st === BUFFERING) {
      this.playAskedAt = 0;
      this.playTries = 0;
    }
    if (st === PLAYING) {
      p.started = true;
      this.unblockTried = false;
      this.setBlocked(0);
    }
    if (st === ENDED && s?.videoId && p.video === s.videoId) {
      // Everyone reports the end; the server only acts on the first.
      this.host.send({ t: 'watch.next', expect: s.videoId });
    } else if (s?.videoId && this.byUser()) {
      if (st === PAUSED && (prev === PLAYING || prev === BUFFERING) && s.playing && p.started) this.set({ playing: false, position: this.localTime() });
      else if ((st === PLAYING || st === BUFFERING) && [PAUSED, UNSTARTED, CUED].includes(prev) && !s.playing) this.set({ playing: true, position: this.localTime() });
    }
    this.renderTransport();
  }

  /** A jump the clock can't explain, right after the person used the player = they seeked. */
  onTime(t) {
    const p = this.player;
    const now = performance.now();
    if (p.time !== null && p.started && this.byUser() && [PLAYING, PAUSED, BUFFERING].includes(p.state)) {
      const predicted = p.time + (p.sampleState === PLAYING ? (now - p.timeAt) / 1000 : 0);
      if (Math.abs(t - predicted) > 2.5 && t > 1 && (!p.duration || t < p.duration - 1)) this.set({ position: t });
    }
    p.time = t;
    p.timeAt = now;
    p.sampleState = p.state;
  }

  /** Volume changed with YouTube's own controls: that's your volume now. */
  onPlayerVolume(info) {
    const now = performance.now();
    if (now - this.volAt < 1500 || !(document.activeElement === this.frame || now - this.touchedAt < 5000)) return;
    const volume = Number.isFinite(info.volume) ? Math.round(info.volume) : this.prefs.volume;
    const muted = typeof info.muted === 'boolean' ? info.muted : this.prefs.muted;
    if (volume === this.prefs.volume && muted === this.prefs.muted) return;
    this.prefs = { volume, muted };
    this.savePrefs();
    this.renderTransport();
  }

  onPlayerError(code) {
    const why = { 2: "that link's video id is wrong", 5: "the browser can't play it", 100: 'it was removed or is private', 101: 'its owner turned off embedding', 150: 'its owner turned off embedding', 153: 'the player was set up wrong' }[code];
    this.showNote(`YouTube won't play this one here${why ? `: ${why}` : ''} (error ${code}). Try Next or another link.`);
  }

  /** Was this change in our player the person's own doing (not us applying the shared state)? */
  byUser() {
    const now = performance.now();
    return this.joined && this.player.video === this.state?.videoId && now - this.cmdAt > 1500
      && document.visibilityState === 'visible' && now - this.visibilityAt > 2000
      && (document.activeElement === this.frame || now - this.touchedAt < 5000);
  }

  /** Where the shared video is right now (seconds). */
  expected() {
    const s = this.state;
    if (!s) return 0;
    return Math.max(0, s.position + (s.playing ? (Date.now() + this.offset - s.at) / 1000 : 0));
  }

  /** Where our own player is right now. */
  localTime() {
    const p = this.player;
    if (p.time === null) return 0;
    return p.time + (p.state === PLAYING ? (performance.now() - p.timeAt) / 1000 : 0);
  }

  /** Change the shared player. Applied locally right away, so our player doesn't fight it before the echo. */
  set(patch) {
    const msg = { t: 'watch.set' };
    if (patch.playing !== undefined) msg.playing = Boolean(patch.playing);
    if (patch.position !== undefined) msg.position = Math.max(0, Math.round(patch.position * 100) / 100);
    this.host.send(msg);
    const s = this.state;
    if (s?.videoId) {
      this.state = { ...s, position: msg.position ?? this.expected(), playing: msg.playing ?? s.playing, at: Date.now() + this.offset, by: this.host.myUid() };
    }
  }

  /** Make our player match the shared state. Runs on every update and twice a second. */
  sync() {
    const s = this.state;
    const p = this.player;
    if (!this.joined || !this.frame || !p.ready || !s) return;
    const now = performance.now();
    if (!s.videoId) {
      if ([PLAYING, BUFFERING, PAUSED].includes(p.state) && now - this.pauseAskedAt > 1500) {
        this.cmd('stopVideo');
        this.pauseAskedAt = now;
      }
      return;
    }
    const want = this.expected();
    if (p.video !== s.videoId) {
      // A new video: same iframe, so nothing reloads (and autoplay stays allowed).
      Object.assign(p, { video: s.videoId, time: null, duration: 0, started: false });
      this.setBlocked(0);
      this.cmd(s.playing ? 'loadVideoById' : 'cueVideoById', [s.videoId, Math.floor(want)]);
      this.applyVolume();
      this.playAskedAt = s.playing ? now : 0;
      this.playTries = 1;
      return;
    }
    if (p.duration > 0 && want > p.duration + 2) {
      // It should have ended by now (everyone was away): move on.
      if (p.state !== ENDED && now - this.nextAt > 5000) {
        this.nextAt = now;
        this.host.send({ t: 'watch.next', expect: s.videoId });
      }
      return;
    }
    if (s.playing) {
      if (!this.blocked && ![PLAYING, BUFFERING, ENDED].includes(p.state)) {
        if (!this.playAskedAt) {
          this.cmd('playVideo');
          this.playAskedAt = now;
          this.playTries = 1;
        } else if (now - this.playAskedAt > 2500) {
          // Browsers block sound until you've clicked: ask once more, then ask the person.
          if (this.playTries < 2) {
            this.cmd('playVideo');
            this.playAskedAt = now;
            this.playTries++;
          } else {
            this.setBlocked(this.unblockTried ? 2 : 1);
          }
        }
      }
    } else {
      if (this.blocked) this.setBlocked(0); // paused for everyone: nothing to click into
      if ((p.state === PLAYING || p.state === BUFFERING) && now - this.pauseAskedAt > 1500) {
        this.cmd('pauseVideo');
        this.pauseAskedAt = now;
      }
    }
    // Seeking during the very first buffer knocks YouTube back to "unstarted", so wait until it has played.
    if (p.time !== null && (p.started ? [PLAYING, PAUSED, BUFFERING].includes(p.state) : p.state === PAUSED)
      && Math.abs(this.localTime() - want) > DRIFT && now - this.seekAt > 2500) {
      this.cmd('seekTo', [want, true]);
      this.seekAt = now;
    }
  }

  /** The "Click to join the show" button: a real click, so the browser lets it play with sound. */
  unblock() {
    this.unblockTried = true;
    this.setBlocked(0);
    this.cmd('seekTo', [this.expected(), true]);
    this.cmd('playVideo');
    this.applyVolume();
    this.playAskedAt = performance.now();
    this.playTries = 2;
  }

  setBlocked(level) {
    this.blocked = level;
    this.dock.querySelector('.watch-gate').hidden = level !== 1;
    this.dock.querySelector('.watch-hint').hidden = level !== 2;
  }

  applyVolume() {
    this.volAt = performance.now();
    this.cmd('setVolume', [this.prefs.volume]);
    this.cmd(this.prefs.muted ? 'mute' : 'unMute');
  }

  savePrefs() {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(this.prefs));
    } catch {}
  }

  toggle() {
    const s = this.state;
    if (!s?.videoId) return;
    this.set({ playing: !s.playing });
    if (!s.playing) {
      // A click: the best moment to start our own player (autoplay rules).
      this.setBlocked(0);
      this.playAskedAt = 0;
      this.playTries = 0;
    }
    this.sync();
    this.renderTransport();
  }

  // ── Dock: the player over the slot, or the mini player ─────────────────

  makeDock() {
    const dock = document.createElement('div');
    dock.className = 'watch-dock';
    dock.id = 'watch-dock';
    dock.hidden = true;
    dock.dataset.mode = 'hidden';
    dock.innerHTML = `
      <div class="watch-mini-bar" title="Drag to move">
        ${icon('tv', 'watch-mini-icon')}
        <span class="watch-mini-title"></span>
        <button class="watch-cap" type="button" data-watch="expand" title="Back to Watch together" aria-label="Back to Watch together">${icon('maximize')}</button>
        <button class="watch-cap" type="button" data-watch="close" title="Leave the show (stops the video for you)" aria-label="Leave the show">${icon('x')}</button>
      </div>
      <div class="watch-frame">
        <div class="watch-gate" hidden><button class="btn primary" type="button" data-watch="unblock">${icon('play')}<span>Click to join the show</span></button></div>
        <div class="watch-hint" hidden>Press ▶ on the video to start it: your browser wants a tap on the player itself.</div>
      </div>`;
    document.body.append(dock);
    dock.addEventListener('click', (e) => this.onClick(e));
    const bar = dock.querySelector('.watch-mini-bar');
    bar.addEventListener('dblclick', (e) => !e.target.closest('button') && this.host.open());
    bar.addEventListener('pointerdown', (e) => {
      if (this.mode !== 'mini' || e.button > 0 || e.target.closest('button')) return;
      const r = dock.getBoundingClientRect();
      this.drag = { dx: e.clientX - r.left, dy: e.clientY - r.top };
      bar.setPointerCapture?.(e.pointerId);
      dock.classList.add('dragging');
      e.preventDefault();
    });
    bar.addEventListener('pointermove', (e) => {
      if (!this.drag) return;
      this.dragged = { x: e.clientX - this.drag.dx, y: e.clientY - this.drag.dy };
      this.placeMini();
    });
    const end = () => {
      this.drag = null;
      dock.classList.remove('dragging');
    };
    bar.addEventListener('pointerup', end);
    bar.addEventListener('pointercancel', end);
    return dock;
  }

  layout() {
    const mode = this.joined && this.frame && this.state?.videoId ? (this.inView ? 'view' : 'mini') : 'hidden';
    if (mode !== this.mode) {
      this.mode = mode;
      this.dock.dataset.mode = mode;
      this.dock.hidden = mode === 'hidden';
      this.rectKey = '';
      for (const k of ['left', 'top', 'right', 'bottom', 'width', 'height']) this.dock.style[k] = '';
      cancelAnimationFrame(this.raf);
      if (mode === 'view') this.follow();
      this.renderMini();
    }
    this.placeMini();
  }

  /** On the Watch view the dock tracks the slot (banners, resizes, the sidebar…). */
  follow() {
    if (this.mode !== 'view') return;
    const slot = this.el.querySelector('.watch-slot');
    if (slot) {
      const r = slot.getBoundingClientRect();
      const key = `${r.left},${r.top},${r.width},${r.height}`;
      if (key !== this.rectKey) {
        this.rectKey = key;
        Object.assign(this.dock.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
      }
    }
    this.raf = requestAnimationFrame(() => this.follow());
  }

  /**
   * Phones: the mini player sits right over the newest messages, so the list
   * gets that much room under them (CSS: .watch-covers .message-list) and stays
   * at the bottom if it was there. Not once it's been dragged somewhere else.
   */
  makeRoom() {
    const on = this.mode === 'mini' && !this.dragged && matchMedia('(max-width: 720px)').matches;
    const root = document.documentElement;
    const h = on ? `${Math.round(this.dock.offsetHeight + GAP)}px` : '';
    if (root.classList.contains('watch-covers') === on && root.style.getPropertyValue('--watch-mini-h') === h) return;
    const list = document.getElementById('messages');
    const atBottom = list && list.scrollHeight - list.scrollTop - list.clientHeight < 48;
    root.classList.toggle('watch-covers', on);
    if (on) root.style.setProperty('--watch-mini-h', h);
    else root.style.removeProperty('--watch-mini-h');
    if (atBottom) list.scrollTop = list.scrollHeight;
  }

  /** Bottom-right of the chat pane, above the composer (or wherever it was dragged). */
  placeMini() {
    this.makeRoom();
    if (this.mode !== 'mini') return;
    const d = this.dock.style;
    if (this.dragged) {
      const x = Math.min(Math.max(4, this.dragged.x), innerWidth - this.dock.offsetWidth - 4);
      const y = Math.min(Math.max(4, this.dragged.y), innerHeight - this.dock.offsetHeight - 4);
      Object.assign(d, { left: `${x}px`, top: `${y}px`, right: 'auto', bottom: 'auto' });
      return;
    }
    const chat = document.querySelector('.chat')?.getBoundingClientRect();
    const composer = document.querySelector('.composer-wrap');
    let floor = chat?.height ? chat.bottom : innerHeight;
    if (composer && composer.offsetParent !== null) floor = Math.min(floor, composer.getBoundingClientRect().top);
    const right = chat?.width ? Math.max(0, innerWidth - chat.right) : 0;
    Object.assign(d, { left: 'auto', top: 'auto', right: `${right + GAP}px`, bottom: `${Math.max(0, innerHeight - floor) + GAP}px` });
  }

  timerCheck() {
    const want = this.joined || this.inView;
    if (want && !this.timer) this.timer = setInterval(() => this.tick(), 500);
    else if (!want && this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  tick() {
    this.sync();
    this.renderTransport();
    this.placeMini();
    if (this.note.text && performance.now() > this.note.until) this.showNote('');
  }

  // ── The Watch view ──────────────────────────────────────────────────────

  build() {
    if (this.built || !this.el) return;
    this.built = true;
    this.el.innerHTML = `
      <div class="watch-stage">
        <div class="watch-slot"><div class="watch-curtain"></div></div>
      </div>
      <div class="watch-seats"><ul class="watch-seat-row" aria-label="Audience"></ul></div>
      <div class="watch-panel">
        <div class="watch-head">
          <div class="watch-heading">
            <div class="watch-title"></div>
            <div class="watch-note" aria-live="polite"></div>
          </div>
          <span class="watch-count muted"></span>
          <span class="watch-seat-btn"></span>
        </div>
        <div class="watch-transport">
          <button class="btn small watch-play" type="button" data-watch="toggle"></button>
          <input class="watch-seek" type="range" min="0" max="0" step="any" value="0" aria-label="Seek (for everyone)">
          <span class="watch-time">0:00</span>
          <button class="btn small watch-next" type="button" data-watch="next" title="Skip to the next video in the queue">${icon('skip')}<span>Next</span></button>
          <span class="watch-volume" title="Your volume (only you)">
            <button class="icon-btn watch-mute" type="button" data-watch="mute"></button>
            <input class="watch-vol" type="range" min="0" max="100" step="1" aria-label="Your volume (only you)">
          </span>
        </div>
        <form class="watch-add" autocomplete="off">
          <input name="link" type="text" inputmode="url" enterkeyhint="go" spellcheck="false" placeholder="Paste a YouTube link" aria-label="YouTube link">
          <span class="watch-add-btns"></span>
        </form>
        <div class="watch-queue">
          <h3>Up next</h3>
          <ol class="watch-queue-list"></ol>
        </div>
      </div>`;
  }

  bindView() {
    if (!this.el) return;
    this.el.addEventListener('click', (e) => this.onClick(e));
    this.el.addEventListener('submit', (e) => {
      if (!e.target.matches('.watch-add')) return;
      e.preventDefault();
      const input = e.target.elements.link;
      const id = youtubeId(input.value);
      if (!id) {
        this.host.toast("That doesn't look like a YouTube link.");
        return input.focus();
      }
      const now = e.submitter?.value === 'play' || !this.state?.videoId;
      this.host.send(now ? { t: 'watch.set', videoId: id } : { t: 'watch.queue', add: id });
      input.value = '';
      this.join();
    });
    this.el.addEventListener('input', (e) => {
      if (e.target.matches('.watch-seek')) {
        this.seeking = true;
        this.renderTime(Number(e.target.value));
      } else if (e.target.matches('.watch-vol')) {
        this.prefs = { volume: Number(e.target.value), muted: this.prefs.muted && Number(e.target.value) === 0 };
        this.savePrefs();
        this.applyVolume();
        this.renderTransport();
      }
    });
    this.el.addEventListener('change', (e) => {
      if (!e.target.matches('.watch-seek')) return;
      this.seeking = false;
      if (!this.state?.videoId) return;
      this.set({ position: Number(e.target.value) });
      this.seekAt = 0;
      this.sync();
    });
  }

  onClick(e) {
    const el = e.target.closest('[data-watch]');
    if (!el) return;
    switch (el.dataset.watch) {
      case 'toggle': return this.toggle();
      case 'next':
        if (this.state?.videoId) this.host.send({ t: 'watch.next', expect: this.state.videoId });
        return;
      case 'mute':
        this.prefs = { ...this.prefs, muted: !this.prefs.muted };
        this.savePrefs();
        this.applyVolume();
        return this.renderTransport();
      case 'remove': return this.host.send({ t: 'watch.queue', remove: Number(el.dataset.index) });
      case 'join': return this.join();
      case 'leave':
      case 'close': return this.leave();
      case 'expand': return this.host.open();
      case 'unblock': return this.unblock();
    }
  }

  name(uid) {
    if (uid && uid === this.host.myUid()) return 'You';
    return this.host.users().get(uid)?.name ?? 'Someone';
  }

  /** A small "Ada paused" line for whatever just changed. */
  describe(prev, s) {
    const who = this.name(s.by);
    let text = '';
    if (s.videoId !== prev.videoId) {
      if (!s.videoId) text = 'That was the last one. Paste a link to keep the show going.';
      else if (prev.queue?.[0]?.id === s.videoId && s.queue.length === prev.queue.length - 1) text = `Up next: ${titleOf(s)}`;
      else text = `${who} put on ${titleOf(s)}`;
    } else if (s.videoId && s.playing !== prev.playing) {
      text = `${who} ${s.playing ? 'pressed play' : 'paused'}`;
    } else if (s.videoId && s.at !== prev.at) {
      const was = prev.position + (prev.playing ? (s.at - prev.at) / 1000 : 0);
      if (Math.abs(s.position - was) > 2) text = `${who} skipped to ${clock(s.position)}`;
    } else if (s.queue.length > prev.queue.length) {
      const item = s.queue.at(-1);
      text = `${this.name(item.by)} queued ${titleOf(item)}`;
    }
    if (text && !(s.by === this.host.myUid() && /^You (pressed play|paused|skipped)/.test(text))) this.showNote(text);
  }

  showNote(text) {
    this.note = { text, until: performance.now() + 6000 };
    if (this.built) {
      const el = this.el.querySelector('.watch-note');
      el.textContent = text;
      el.classList.toggle('show', Boolean(text));
    }
    this.renderMini();
  }

  render() {
    if (!this.built) return;
    const s = this.state;
    const id = s?.videoId ?? null;
    this.el.querySelector('.watch-slot').classList.toggle('live', this.mode === 'view');

    // What shows in the slot when the player isn't there.
    const curtain = this.el.querySelector('.watch-curtain');
    const key = this.mode === 'view' ? 'live' : !id ? 'empty' : this.joined ? `loading:${id}` : `seat:${id}:${s.title}`;
    if (key !== this.curtainKey) {
      this.curtainKey = key;
      curtain.innerHTML = key === 'live' ? ''
        : !id ? `<div class="watch-curtain-text"><p class="watch-big">Nothing's on.</p><p>Paste a YouTube link below to start the show. Everyone watches it at the same spot, and it keeps playing while you browse.</p></div>`
          : this.joined ? '<div class="watch-curtain-text"><p>Loading the video…</p></div>'
            : `<img class="watch-poster" src="${esc(thumb(id, 'hqdefault'))}" alt="">
               <div class="watch-curtain-text"><p class="watch-big">${esc(titleOf(s))}</p>
               <button class="btn primary" type="button" data-watch="join">${icon('play')}<span>Take a seat</span></button></div>`;
    }

    // Nothing on: the curtain says so once; no title, transport or queue until something plays.
    this.el.querySelector('.watch-title').innerHTML = id
      ? `<a href="https://www.youtube.com/watch?v=${esc(encodeURIComponent(id))}" target="_blank" rel="noopener noreferrer" title="Open on YouTube">${esc(titleOf(s))}</a>`
      : '';
    this.el.querySelector('.watch-transport').hidden = !id;
    this.el.querySelector('.watch-queue').hidden = !id;
    this.el.querySelector('.watch-panel').classList.toggle('idle', !id);
    this.el.querySelector('.watch-seat-btn').innerHTML = this.joined
      ? `<button class="btn small" type="button" data-watch="leave" title="Stop watching (the others keep going)">Leave</button>`
      : `<button class="btn small primary" type="button" data-watch="join">Take a seat</button>`;
    this.el.querySelector('.watch-add-btns').innerHTML = id
      ? `<button class="btn small primary" type="submit" value="queue">${icon('plus')}<span>Add to queue</span></button>
         <button class="btn small" type="submit" value="play" title="Replace what's playing for everyone">Play now</button>`
      : `<button class="btn small primary" type="submit" value="play">${icon('play')}<span>Play</span></button>`;

    const queue = s?.queue ?? [];
    this.el.querySelector('.watch-queue-list').innerHTML = queue.length
      ? queue.map((item, i) => `
        <li class="watch-q-item">
          <img class="watch-thumb" src="${esc(thumb(item.id))}" alt="" loading="lazy">
          <span class="watch-q-text"><span class="watch-q-title">${esc(titleOf(item))}</span><small class="muted">added by ${esc(this.name(item.by))}</small></span>
          <button class="icon-btn tiny" type="button" data-watch="remove" data-index="${i}" title="Remove from the queue" aria-label="Remove ${esc(titleOf(item))}">${icon('x')}</button>
        </li>`).join('')
      : '<li class="watch-q-empty muted">Nothing queued. Paste a link above and press Add to queue.</li>';

    this.renderSeats();
    this.renderTransport();
    this.renderMini();
  }

  renderSeats() {
    if (!this.built) return;
    const byUid = new Map(); // one head per person (phone + laptop = one seat)
    for (const s of this.host.sessions().values()) {
      if (!s.watching) continue;
      const had = byUid.get(s.uid);
      if (!had || (!had.voice && s.voice)) byUid.set(s.uid, s);
    }
    const heads = [...byUid.values()];
    const users = this.host.users();
    const chair = '<li class="watch-seat empty" aria-hidden="true"><span class="watch-chair"></span><span class="watch-seat-name"></span></li>';
    const empty = Math.max(0, MIN_SEATS - heads.length);
    const seats = heads.map((s) => {
      const user = users.get(s.uid);
      const name = user?.name ?? 'Someone';
      return `<li class="watch-seat${this.host.speaking(s.sid) ? ' speaking' : ''}" data-voice-sid="${esc(s.sid)}" title="${esc(name)}${s.voice ? ' · in voice' : ''}">
        ${this.host.avatar(user, 'watch-face')}<span class="watch-chair"></span><span class="watch-seat-name">${esc(name)}</span></li>`;
    });
    this.el.querySelector('.watch-seat-row').innerHTML = chair.repeat(Math.floor(empty / 2)) + seats.join('') + chair.repeat(Math.ceil(empty / 2));
    this.el.querySelector('.watch-count').textContent = heads.length ? `${heads.length} watching` : '';
  }

  renderTime(t) {
    if (!this.built) return;
    const dur = this.player.duration;
    this.el.querySelector('.watch-time').textContent = dur ? `${clock(t)} / ${clock(dur)}` : clock(t);
  }

  /** The bits that move: cheap enough to run twice a second. */
  renderTransport() {
    this.renderMini();
    if (!this.built || !this.inView) return;
    const s = this.state;
    const id = s?.videoId;
    const play = this.el.querySelector('.watch-play');
    const label = s?.playing ? 'Pause' : 'Play';
    if (play.dataset.label !== label) {
      play.dataset.label = label;
      play.innerHTML = `${icon(s?.playing ? 'pause' : 'play')}<span>${label}</span>`;
    }
    play.disabled = !id;
    const seek = this.el.querySelector('.watch-seek');
    const dur = this.player.duration || 0;
    seek.disabled = !id || !dur || !this.joined;
    if (!this.seeking) {
      const at = Math.min(this.expected(), dur || Infinity);
      seek.max = String(dur);
      seek.value = String(id ? at : 0);
      this.renderTime(id ? at : 0);
    }
    this.el.querySelector('.watch-next').disabled = !id || !s.queue?.length;
    const mute = this.el.querySelector('.watch-mute');
    const muted = this.prefs.muted || this.prefs.volume === 0;
    if (mute.dataset.muted !== String(muted)) {
      mute.dataset.muted = String(muted);
      mute.innerHTML = icon(muted ? 'volume-x' : 'volume');
      mute.title = muted ? 'Unmute (only you)' : 'Mute (only you)';
      mute.setAttribute('aria-label', mute.title);
    }
    const vol = this.el.querySelector('.watch-vol');
    if (document.activeElement !== vol) vol.value = String(this.prefs.muted ? 0 : this.prefs.volume);
  }

  renderMini() {
    if (this.mode !== 'mini') return;
    const fresh = this.note.text && performance.now() < this.note.until;
    const text = fresh ? this.note.text : `${this.state?.playing ? '▶' : '❚❚'} ${titleOf(this.state)}`;
    const el = this.dock.querySelector('.watch-mini-title');
    if (el.textContent !== text) el.textContent = text;
  }
}
