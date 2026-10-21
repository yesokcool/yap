/*
 * "Space": a little top-down world everyone can walk around in, with a
 * CS:GO-style radar. Positions travel as space.move → space.pos (relayed,
 * last position kept per session on the server). The map lives in
 * space-map.js, the drawing in space-art.js, the instruments in space-sound.js.
 *
 * Rooms are voice zones. On "This room only" (the default) you hear the
 * people in your room at full volume and nobody outside it; the Plaza
 * (everywhere that isn't a room) is a zone of its own. "Nearby is louder"
 * and "Everyone" are the other choices. In "Nearby is louder" and inside your
 * room, voices also come from the left or right, where people stand. People
 * in the call who aren't on Space are heard as usual.
 *
 * Mini-games ride on space.game {g, d}: relayed to everyone else on Space,
 * never stored. Whoever owns a piece of state answers 'sync' (sent whenever
 * space.state arrives), so newcomers catch up. Payloads: PROTOCOL.md › Space.
 *   ball  — a football on the Pitch; the last kicker simulates and broadcasts it.
 *   tag   — someone is "it"; touching someone passes it; least time as it wins.
 *   fx    — your own effects: on fire (walk into the campfire) or dripping
 *           (the fountain, the pond, a water balloon). Water puts fire out.
 *   throw — a water balloon you threw; whoever it hits gets wet and says so.
 *   lap   — a lap of the race track round the edge, timed by the runner.
 * The floor piano and drum pads need no messages: everyone sees where people
 * stand, so everyone in the Music room hears the same notes.
 *
 * The Gallery hangs the drawings saved in Draw (draw.gallery, then each new
 * `drawing` as it's saved), twelve to a wall, newest first. Walk up to one
 * (or press E) for a big viewer with the real image, so GIFs wiggle.
 */
import { esc, initial } from './format.js';
import { icon } from './icons.js';
import {
  WORLD, RADIUS, PLAZA, PLAZA_SPOT, SPAWN, ROOMS, SOLID, MID, BALL_R, TAG_SPOT, TRACK, CORNERS, MUSIC_ZONE, GALLERY_ZONE, ART, SEATS, DECOR,
  seatAt, zoneOf, zoneName, blocked, netAt, onFire, inWater, onTrack, inRect, instrumentAt, frameAt, voicePan,
} from './space-map.js';
import {
  FONT, clock, drawFloor, drawRoomNames, drawWalls, drawDecor, drawBall, drawBalloon, drawPuddle, drawGlow, drawFlames, drawWet, drawBurst,
} from './space-art.js';
import { playNote, playDrum } from './space-sound.js';

export { WORLD };
const SPEED = 280; // units per second
const SEND_EVERY_MS = 80;

/** Quick emotes over your head (keys 1–6). Same list as the server's. */
const EMOTES = ['👋', '😂', '❤️', '👍', '🎉', '💤'];
const EMOTE_MS = 3000;

/** Characters you can pick (or your picture). */
const LOOKS = ['🙂', '😎', '🤠', '🤖', '👻', '👽', '🐸', '🐱', '🐶', '🦊', '🐼', '🐧', '🦄', '🐙', '🐢', '🦆', '🐝', '🦖', '🍄', '🌵', '🎃', '🧙', '🥷', '🧑‍🚀'];

// ── Mini-games ────────────────────────────────────────────────────────────
const FRICTION = 1.3; // per second, exponential
const BOUNCE = 0.72;
const KICK_MAX = 1100;
const TAG_MS = 90_000; // one round
const TAG_GRACE_MS = 2500; // no tag-backs
const TAG_BEAT_MS = 3000; // "it" re-announces the round this often
const TAG_SILENT_MS = 10_000; // no word from "it" for this long: someone else takes over
const FIRE_MS = 5000; // how long you burn after leaving the fire
const WET_MS = 6000; // how long you drip after leaving the water
const FX_MAX = 10_000;
const SHOT = { speed: 650, range: 520, cooldown: 700, r: 8 };
const LAP_MAX_MS = 180_000;
const LOOK_MS = 450; // stand in front of a drawing this long and it opens up
const MEDIA_RE = /^[0-9a-f]{16,64}$/i;
/** Phones and tablets: no keyboard, so no Shift-to-run, and a hint to tap where you want to go. */
const TOUCH = matchMedia('(pointer: coarse)').matches;
const TAP_KEY = 'yap.spaceTapped';
const NARROW = matchMedia('(max-width: 720px)');
/** Games name people by the first 8 characters of their session id (sids are UUIDs). */
const short = (sid) => String(sid ?? '').slice(0, 8);
const SHORT_RE = /^[0-9a-f-]{8}$/;
const num = (v, lo, hi) => Math.min(hi, Math.max(lo, Number.isFinite(Number(v)) ? Number(v) : 0));
const secs = (ms) => `${(ms / 1000).toFixed(1)} s`;
const VOICE_HINTS = {
  room: (where) => (where ? `You hear everyone in the ${where}, and nobody outside it.` : 'You hear everyone on the Plaza, and nobody in the rooms.'),
  near: () => 'You hear everyone in the call; nearby is louder.',
  everyone: () => 'You hear everyone in the call at full volume.',
};

export class SpaceWorld {
  /**
   * @param {HTMLElement} root
   * @param {{ send: (m: object) => void, mySid: () => string, who: (sid: string) => {name: string, color: string, avatar: string|null, look: string|null} | null,
   *           proximity: (sid: string, factor: number, pan?: number|null) => void, setLook: (look: string|null) => void,
   *           inCall?: (sid: string) => {muted: boolean, deafened: boolean} | null, user?: (uid: string) => {name: string, color: string} | null }} hooks
   *   inCall: that session's voice state if it's in your voice call (yours if you're in one), else null.
   *   user: a member by uid (who made a drawing).
   */
  constructor(root, hooks) {
    this.root = root;
    this.hooks = hooks;
    this.players = new Map(); // sid -> { x, y, d, tx, ty, vol }
    this.me = null;
    this.keys = new Set();
    this.target = null;
    this.images = new Map();
    this.running = false;
    this.lastSent = 0;
    this.emotes = new Map(); // sid -> { e, at }
    this.vel = { x: 0, y: 0 };
    this.ball = freshBall();
    this.tag = { it: null, by: null, k: 0, endAt: 0, t: {}, at: 0, since: 0, heard: 0, beat: 0, over: true, result: null };
    // Effects: until when (performance.now) someone burns / drips.
    this.myFx = { fire: 0, wet: 0, safe: 0 };
    this.fxSent = { fire: 0, wet: 0 };
    this.fx = new Map(); // sid -> { fire, wet }
    // Water balloons.
    this.shots = []; // { sid, id, x, y, vx, vy, left }
    this.shotId = 0;
    this.lastThrow = 0;
    this.mySplashes = 0;
    this.splashes = new Map(); // sid -> balloons they've landed
    this.splashBoard = 0; // show the splash count until then
    // Race laps.
    this.race = { start: 0, cp: 0, best: 0, laps: 0, lastY: null, offAt: 0 };
    this.laps = new Map(); // sid -> { best, n }
    // Instruments and decoration.
    this.inst = new Map(); // sid -> id of the key or pad they stand on
    this.heard = 0; // notes played here (for tests)
    // The Gallery: every saved drawing, newest first; which wall you're looking at; the viewer.
    this.gallery = []; // { id, media, uid, ts, when }
    this.galleryPage = 0;
    this.viewing = -1; // index into gallery while the viewer is open
    this.viewAnchor = null; // where you stood when it opened: walk away and it closes
    this.dwell = { frame: -1, since: 0 };
    this.dismissed = -1; // the frame you closed the viewer on (no reopening until you step away)
    this.bursts = []; // { kind, x, y, at, sid?, glyph?, color? }
    this.puddles = []; // { x, y, at }
    this.dripFrom = new Map(); // sid -> where they left their last puddle
    this.flash = null; // a short line over the map: { text, until }
    this.onSpot = false;
    this.panelAt = 0;
    this.panelSig = '';
    this.gameSig = '';
    // Voice in Space: only your room (default), quieter with distance, or everyone at full volume.
    this.voiceMode = (() => {
      try {
        const saved = localStorage.getItem('yap.spaceVoice') ?? (localStorage.getItem('yap.proximity') === '1' ? 'near' : 'room');
        return ['room', 'near', 'everyone'].includes(saved) ? saved : 'room';
      } catch {
        return 'room';
      }
    })();

    root.innerHTML = `
      <canvas class="space-canvas" tabindex="0" aria-label="${TOUCH ? 'Space: tap where you want to walk' : 'Space: move with WASD or arrow keys, or click where to go'}"></canvas>
      <canvas class="space-radar" width="220" height="147" aria-hidden="true"></canvas>
      <section class="space-room" aria-label="Your room">
        <h2 class="space-room-title"><span class="space-room-name">${PLAZA}</span><span class="space-room-count"></span></h2>
        <ul class="space-room-people"></ul>
        <label class="space-voice"><span>Voice</span><select aria-label="Voice in Space">
          <option value="room">This room only</option>
          <option value="near">Nearby is louder</option>
          <option value="everyone">Everyone</option>
        </select></label>
        <p class="space-room-hint"></p>
        <div class="space-wall" hidden><span class="space-wall-label"></span>
          <button type="button" data-wall="-1" title="Newer drawings">◀ Newer</button><button type="button" data-wall="1" title="Older drawings">Older ▶</button></div>
      </section>
      <div class="space-viewer" role="dialog" aria-label="A drawing from the gallery" hidden>
        <div class="space-viewer-bar"><span class="space-viewer-title">Gallery</span><button type="button" class="space-viewer-close" data-viewer="close" aria-label="Close (Esc)">✕</button></div>
        <div class="space-viewer-body">
          <button type="button" class="space-viewer-nav" data-viewer="-1" aria-label="Newer (←)">‹</button>
          <img class="space-viewer-img" alt="A drawing from the gallery">
          <button type="button" class="space-viewer-nav" data-viewer="1" aria-label="Older (→)">›</button>
        </div>
        <p class="space-viewer-caption"><span class="space-viewer-dot"></span><b class="space-viewer-who"></b> <span class="space-viewer-when"></span> <span class="space-viewer-pos"></span></p>
      </div>
      <div class="space-game" role="status" hidden></div>
      <div class="space-hud">
        <button type="button" class="look-btn" data-space="looks" aria-expanded="false">Character <span class="look-now"></span></button>
        <span class="space-keys"><kbd>WASD</kbd> move · <kbd>Shift</kbd> run · <kbd>E</kbd> sit · <kbd>1</kbd>–<kbd>6</kbd> emote · <kbd>T</kbd> tag · <kbd>F</kbd> throw</span>
        <span class="space-emotes">${EMOTES.map((e, i) => `<button type="button" data-emote="${e}" title="${e} (${i + 1})">${e}</button>`).join('')}</span>
        <button type="button" class="look-btn space-tag" data-game="tag" title="Start a round of tag (T): you're it, touch someone to pass it on">🏃 Tag</button>
        <button type="button" class="look-btn space-throw" data-game="throw" title="Throw a water balloon the way you're facing (F)">💧 Throw</button>
      </div>
      <div class="space-looks" hidden>${LOOKS.map((l) => `<button type="button" data-look="${l}" aria-label="Be ${l}">${l}</button>`).join('')}
        <button type="button" class="picture" data-look="">Use my picture</button></div>`;
    this.canvas = root.querySelector('.space-canvas');
    this.radar = root.querySelector('.space-radar');
    const looks = root.querySelector('.space-looks');
    root.addEventListener('click', (e) => {
      if (e.target.closest('[data-space="looks"]')) {
        looks.hidden = !looks.hidden;
        // Just above the bar of buttons, however tall it is (two rows on a phone, where a fixed
        // spot covered the Character button that opened it).
        const hud = this.root.querySelector('.space-hud');
        if (!looks.hidden) looks.style.bottom = `${this.root.clientHeight - hud.offsetTop + 6}px`;
        this.#syncLooks();
        return;
      }
      const emote = e.target.closest('[data-emote]');
      if (emote) {
        this.hooks.send({ t: 'space.emote', e: emote.dataset.emote });
        this.canvas.focus();
        return;
      }
      const viewer = e.target.closest('[data-viewer]');
      if (viewer) {
        if (viewer.dataset.viewer === 'close') this.#closeViewer(true);
        else this.#showDrawing(this.viewing + Number(viewer.dataset.viewer));
        this.canvas.focus();
        return;
      }
      const wall = e.target.closest('[data-wall]');
      if (wall) {
        this.#turnWall(Number(wall.dataset.wall));
        this.canvas.focus();
        return;
      }
      const game = e.target.closest('[data-game]');
      if (game) {
        if (game.dataset.game === 'tag') this.#startTag();
        if (game.dataset.game === 'throw') this.#throw();
        this.canvas.focus();
        return;
      }
      const pick = e.target.closest('[data-look]');
      if (pick) {
        this.hooks.setLook(pick.dataset.look || null);
        looks.hidden = true;
        this.canvas.focus();
      }
    });
    const voice = root.querySelector('.space-voice select');
    voice.value = this.voiceMode;
    voice.addEventListener('change', (e) => {
      this.voiceMode = e.target.value;
      try { localStorage.setItem('yap.spaceVoice', this.voiceMode); } catch {}
      if (this.voiceMode === 'everyone') {
        for (const [sid, p] of this.players) {
          p.vol = 1;
          p.pan = null;
          this.hooks.proximity(sid, 1, null);
        }
      }
      this.panelAt = 0;
      this.canvas.focus();
    });
    addEventListener('keydown', (e) => {
      if (!this.running || /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) return;
      // Ctrl/⌘/Alt + a key is a shortcut (Ctrl+Shift+D deafens, Ctrl+F finds), never a step or a throw.
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (this.viewing >= 0 && ['escape', 'arrowleft', 'arrowright'].includes(k)) {
        e.preventDefault();
        if (k === 'escape') return this.#closeViewer(true);
        return this.#showDrawing(this.viewing + (k === 'arrowleft' ? -1 : 1));
      }
      if (/^[1-6]$/.test(k) && !e.repeat) return this.hooks.send({ t: 'space.emote', e: EMOTES[Number(k) - 1] });
      if (k === 'e' && !e.repeat) return this.#lookOrSit();
      if (k === 't' && !e.repeat) return this.#startTag();
      if (k === 'f' && !e.repeat) return this.#throw();
      if (['w', 'a', 's', 'd', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright', 'shift'].includes(k)) {
        this.keys.add(k);
        this.target = null;
        if (k.startsWith('arrow')) e.preventDefault();
      }
    });
    addEventListener('keyup', (e) => {
      // macOS sends no keyup for keys let go while ⌘ is down: stop rather than walk on forever.
      if (e.key === 'Meta') this.keys.clear();
      else this.keys.delete(e.key.toLowerCase());
    });
    addEventListener('blur', () => this.keys.clear());
    this.canvas.addEventListener('pointerdown', (e) => {
      if (this.tapHint) {
        // The first tap on a phone: the "tap where you want to walk" line has done its job.
        this.tapHint = false;
        this.root.querySelector('.space-keys')?.classList.remove('space-tap-hint');
        try { localStorage.setItem(TAP_KEY, '1'); } catch {}
      }
      const r = this.canvas.getBoundingClientRect();
      this.target = { x: this.cam.x + (e.clientX - r.left), y: this.cam.y + (e.clientY - r.top) };
      this.canvas.focus();
    });
    this.cam = { x: 0, y: 0 };
    this.tapHint = TOUCH && (() => { try { return localStorage.getItem(TAP_KEY) !== '1'; } catch { return true; } })();
    // On a touch screen the keyboard line at the bottom says how to walk instead, until the first tap
    // (at the top it sat on the lap board, right where you start).
    if (this.tapHint) {
      const keys = this.root.querySelector('.space-keys');
      keys.textContent = 'Tap where you want to walk';
      keys.classList.add('space-tap-hint');
    }
  }

  open() {
    this.running = true;
    // Where we were last time comes back in space.state (see onState).
    this.hooks.send({ t: 'space.state' });
    if (this.me) this.#send(true);
    this.#syncLooks();
    this.last = performance.now();
    this.panelSig = this.gameSig = '';
    this.canvas.focus();
    // One loop at a time, even if Space closes and opens again before the last frame ran.
    const run = (this.run = (this.run ?? 0) + 1);
    const loop = (now) => {
      if (!this.running || run !== this.run) return;
      this.#tick(Math.min(0.05, Math.max(0, now - this.last) / 1000), now);
      this.last = now;
      this.#render();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
    // Hidden tabs get no animation frames, but voice zones and games must keep going.
    clearInterval(this.idleTimer);
    this.idleTimer = setInterval(() => {
      if (!this.running || !document.hidden) return;
      const now = performance.now();
      for (let left = Math.min(2, (now - this.last) / 1000); left > 0; left -= 0.05) this.#tick(Math.min(0.05, left), now, true);
      this.last = now;
    }, 250);
  }

  close() {
    this.running = false;
    clearInterval(this.idleTimer);
    this.keys.clear();
    for (const [sid, p] of this.players) {
      delete p.vol;
      delete p.pan;
      this.hooks.proximity(sid, 1, null);
    }
    // The others take over whatever we owned once they see us go; next time we sync afresh.
    this.ball = freshBall();
    if (!this.tag.over) Object.assign(this.tag, { over: true, it: null, result: null });
    Object.assign(this.race, { start: 0, cp: 0, lastY: null, offAt: 0 });
    this.fx.clear();
    this.shots = [];
    this.bursts = [];
    this.puddles = [];
    this.inst.clear();
    this.#closeViewer(false);
  }

  onState({ players, me }) {
    // The full list: anyone we still have who isn't on it left while we weren't listening (a reconnect).
    const listed = new Set(players.map((p) => p.sid));
    for (const sid of [...this.players.keys()]) if (!listed.has(sid)) this.remove(sid);
    for (const p of players) if (p.sid !== this.hooks.mySid()) this.#upsert(p);
    if (!this.me) {
      // Your saved spot, or the plaza in the middle (a little spread out) the first time.
      const spread = () => (Math.random() - 0.5) * SPAWN.spread;
      this.me = me ? { ...me } : { x: SPAWN.x + spread(), y: SPAWN.y + spread() / 3, d: Math.PI / 2 };
      if (blocked(this.me.x, this.me.y)) Object.assign(this.me, { x: SPAWN.x, y: SPAWN.y });
    }
    // Always say where we are: after a reconnect (every deploy) this is a new session, which
    // nobody sees on the map until it has sent a position.
    this.#send(true);
    // Catch up on the games (whoever owns something answers), and tell everyone our own bits.
    this.#game('sync', {});
    this.#announceMine();
    // The Gallery's drawings (new ones arrive as `drawing`).
    this.hooks.send({ t: 'draw.gallery' });
  }

  /** draw.gallery: every saved drawing, newest first. */
  onGallery({ gallery }) {
    if (!Array.isArray(gallery)) return;
    this.gallery = gallery.map(cleanDrawing).filter(Boolean).slice(0, 60);
    this.galleryPage = Math.min(this.galleryPage, this.#pages() - 1);
    this.panelSig = '';
  }

  /** Someone just saved a drawing: it goes up first on the first wall. */
  onDrawing({ drawing }) {
    const d = cleanDrawing(drawing);
    if (!d || this.gallery.some((x) => x.id === d.id)) return;
    this.gallery.unshift(d);
    if (this.viewing >= 0) this.viewing += 1; // keep showing the same one
    this.panelSig = '';
    if (this.running) this.#flashText(`🖼 ${this.hooks.user?.(d.uid)?.name ?? 'Someone'} hung a new drawing in the Gallery`, 3000);
  }

  /** Refresh the HUD after my character changes. */
  refreshLook() {
    this.#syncLooks();
  }

  #syncLooks() {
    const me = this.hooks.who(this.hooks.mySid());
    this.root.querySelector('.look-now').textContent = me?.look ?? '🖼️';
    for (const b of this.root.querySelectorAll('[data-look]')) b.setAttribute('aria-pressed', String((b.dataset.look || null) === (me?.look ?? null)));
    this.root.querySelector('[data-space="looks"]').setAttribute('aria-expanded', String(!this.root.querySelector('.space-looks').hidden));
  }

  onPos(p) {
    if (p.sid !== this.hooks.mySid()) this.#upsert(p);
  }

  remove(sid) {
    const gone = this.players.get(sid);
    this.players.delete(sid);
    this.emotes.delete(sid);
    this.fx.delete(sid);
    this.laps.delete(sid);
    this.splashes.delete(sid);
    this.inst.delete(sid);
    this.dripFrom.delete(sid);
    this.hooks.proximity(sid, 1);
    if (!this.running) return;
    // Whatever they owned is picked up by the lowest sid still here, so everyone agrees who.
    if (this.ball.owner === sid) {
      this.ball.owner = null;
      if (this.#lowestSid() === this.hooks.mySid()) {
        this.ball.owner = this.hooks.mySid();
        this.ball.dirty = true;
      }
    }
    if (!this.tag.over && short(sid) === this.tag.it) this.#reassignIt(performance.now(), gone);
  }

  onEmote({ sid, e }) {
    this.emotes.set(sid, { e, at: performance.now() });
  }

  /** A mini-game update from someone else on Space (space.game). */
  onGame({ sid, g, d }) {
    if (sid === this.hooks.mySid() || !d || typeof d !== 'object') return;
    if (g === 'sync') return this.#answerSync();
    if (g === 'ball') return this.#onBall(sid, d);
    if (g === 'tag') return this.#onTag(sid, d);
    if (g === 'fx') return this.#onFx(sid, d);
    if (g === 'throw') return this.#onThrow(sid, d);
    if (g === 'lap') return this.#onLap(sid, d);
  }

  /** E: sit on the nearest free seat, or stand up. */
  #toggleSit() {
    const me = this.me;
    if (!me) return;
    const seat = seatAt(me);
    if (seat) {
      me.y += blocked(me.x, me.y + 40) ? -40 : 40;
      return this.#send(true);
    }
    const taken = (s) => [...this.players.values()].some((p) => seatAt(p) === s);
    const free = SEATS.filter((s) => !taken(s)).sort((a, b) => Math.hypot(a.x - me.x, a.y - me.y) - Math.hypot(b.x - me.x, b.y - me.y))[0];
    if (!free || Math.hypot(free.x - me.x, free.y - me.y) > 110) return;
    Object.assign(me, { x: free.x, y: free.y, d: free.d });
    this.target = null;
    this.#send(true);
  }

  #upsert({ sid, x, y, d }) {
    const known = this.players.get(sid);
    if (known) Object.assign(known, { tx: x, ty: y, d });
    else this.players.set(sid, { x, y, d, tx: x, ty: y });
  }

  /** Me, or someone on Space right now. */
  #here(sid) {
    return Boolean(sid) && (sid === this.hooks.mySid() || this.players.has(sid));
  }

  #lowestSid() {
    return [this.hooks.mySid(), ...this.players.keys()].sort()[0];
  }

  #name(sid) {
    return sid ? this.hooks.who(sid)?.name ?? null : null;
  }

  /** The full sid (me or someone on Space) behind a game's 8-character id. */
  #sidOf(id) {
    const mine = this.hooks.mySid();
    if (id && short(mine) === id) return mine;
    for (const sid of this.players.keys()) if (short(sid) === id) return sid;
    return null;
  }

  /** Where someone is drawn right now (me or anyone on Space). */
  #spot(sid) {
    return sid === this.hooks.mySid() ? this.me : this.players.get(sid) ?? null;
  }

  #game(g, d) {
    this.hooks.send({ t: 'space.game', g, d });
  }

  #answerSync() {
    const mine = this.hooks.mySid();
    if (this.ball.owner === mine) this.ball.dirty = true; // goes out on the next tick
    if (!this.tag.over && this.tag.it === short(mine)) this.#sendTag(performance.now());
    this.#announceMine();
  }

  /** Our own effects, splash count and best lap, for whoever just arrived. */
  #announceMine() {
    const now = performance.now();
    if (this.myFx.fire > now || this.myFx.wet > now || this.mySplashes) this.#sendFx(now);
    if (this.race.best) this.#game('lap', { best: this.race.best, n: this.race.laps });
  }

  #flashText(text, ms = 2500) {
    this.flash = { text, until: performance.now() + ms };
    this.gameSig = '';
  }

  #burst(kind, x, y, extra = {}) {
    this.bursts.push({ kind, x, y, at: performance.now(), ...extra });
    if (this.bursts.length > 60) this.bursts.shift();
  }

  // ── Simulation ───────────────────────────────────────────────────────────

  #tick(dt, now, hidden = false) {
    this.#step(dt);
    this.#stepBall(dt, now);
    this.#stepShots(dt, now);
    this.#stepEffects(now);
    this.#stepRace(now);
    this.#stepTag(now, hidden);
    this.#stepVoice(dt);
    if (hidden) return;
    this.#stepInstruments(now);
    this.#stepPuddles(now);
    this.#stepGallery(now);
    this.#refreshPanels(now);
  }

  #step(dt) {
    const me = this.me;
    if (!me) return;
    let dx = 0;
    let dy = 0;
    const k = this.keys;
    if (k.has('a') || k.has('arrowleft')) dx -= 1;
    if (k.has('d') || k.has('arrowright')) dx += 1;
    if (k.has('w') || k.has('arrowup')) dy -= 1;
    if (k.has('s') || k.has('arrowdown')) dy += 1;
    if (!dx && !dy && this.target) {
      const tx = this.target.x - me.x;
      const ty = this.target.y - me.y;
      const dist = Math.hypot(tx, ty);
      if (dist < 8) this.target = null;
      else {
        dx = tx / dist;
        dy = ty / dist;
      }
    }
    this.vel = { x: 0, y: 0 };
    if (dx || dy) {
      const len = Math.hypot(dx, dy);
      // "It" gets a hair more speed, so a chase can end.
      const it = !this.tag.over && this.tag.it === short(this.hooks.mySid());
      const speed = SPEED * (k.has('shift') ? 1.7 : 1) * (it ? 1.08 : 1);
      this.vel = { x: (dx / len) * speed, y: (dy / len) * speed };
      const nx = me.x + this.vel.x * dt;
      const ny = me.y + this.vel.y * dt;
      const before = [me.x, me.y];
      if (!blocked(nx, me.y)) me.x = nx;
      if (!blocked(me.x, ny)) me.y = ny;
      me.d = Math.atan2(dy, dx);
      if (this.target && me.x === before[0] && me.y === before[1]) this.target = null; // walked into a wall
      this.#send(false);
    }
    // Stepping onto the TAG spot starts a round (you're it).
    const onSpot = Math.hypot(me.x - TAG_SPOT.x, me.y - TAG_SPOT.y) < TAG_SPOT.r;
    if (onSpot && !this.onSpot) this.#startTag();
    this.onSpot = onSpot;
    for (const p of this.players.values()) {
      p.x += (p.tx - p.x) * Math.min(1, dt * 12);
      p.y += (p.ty - p.y) * Math.min(1, dt * 12);
    }
  }

  /**
   * Per-person volume for Space voice, and where it comes from: in "Nearby
   * is louder" and inside your own room, left or right by where they stand
   * on screen (voicePan). Both eased, so walking through a door crossfades
   * and someone walking past moves across smoothly.
   */
  #stepVoice(dt) {
    const me = this.me;
    if (!me) return;
    const myZone = zoneOf(me.x, me.y);
    const ease = Math.min(1, dt * 8);
    for (const [sid, p] of this.players) {
      let target = 1;
      let pan = null; // null: not panned ("Everyone")
      const dx = p.x - me.x;
      const dy = p.y - me.y;
      if (this.voiceMode === 'near') {
        const dist = Math.hypot(dx, dy);
        target = dist < 260 ? 1 : dist > 1100 ? 0.12 : 1 - ((dist - 260) / 840) * 0.88;
        pan = voicePan(dx, dy);
      } else if (this.voiceMode === 'room') {
        const here = zoneOf(p.tx, p.ty) === myZone;
        target = here ? 1 : 0;
        pan = here ? voicePan(dx, dy) : 0;
      }
      p.vol = p.vol === undefined ? target : p.vol + (target - p.vol) * ease;
      if (Math.abs(target - p.vol) < 0.03) p.vol = target;
      if (pan === null) p.pan = null;
      else {
        p.pan = p.pan == null ? pan : p.pan + (pan - p.pan) * ease;
        if (Math.abs(pan - p.pan) < 0.02) p.pan = pan;
      }
      this.hooks.proximity(sid, p.vol, p.pan);
    }
  }

  #send(force) {
    if (!force && Date.now() - this.lastSent < SEND_EVERY_MS) return;
    const move = { t: 'space.move', x: Math.round(this.me.x), y: Math.round(this.me.y), d: Math.round(this.me.d * 100) / 100 };
    // Walking into a wall (or a key held down) goes nowhere: nothing to tell anyone.
    const at = `${move.x},${move.y},${move.d}`;
    if (!force && at === this.sentAt) return;
    this.sentAt = at;
    this.lastSent = Date.now();
    this.hooks.send(move);
  }

  // ── Fire and water ───────────────────────────────────────────────────────
  // You're the authority on yourself: your client sees you walk into the fire
  // or the water and says so (fx), and everyone draws it until it runs out.

  #stepEffects(now) {
    const me = this.me;
    if (!me) return;
    if (inWater(me.x, me.y)) this.#soak(now);
    else if (onFire(me.x, me.y) && !seatAt(me)) this.#ignite(now);
    // Standing in it keeps it going: tell the others when that drifts by more than 1.5 s.
    const fx = this.myFx;
    const sent = this.fxSent;
    const drift = (a, b) => a > now !== b > now || Math.abs(a - b) > 1500;
    if (drift(fx.fire, sent.fire) || drift(fx.wet, sent.wet)) this.#sendFx(now);
  }

  /** Water: you drip for a while, and any fire on you goes out. */
  #soak(now, quiet = false) {
    const fx = this.myFx;
    if (fx.fire > now) {
      fx.fire = 0;
      this.#burst('steam', this.me.x, this.me.y);
      if (!quiet) this.#flashText('Pssshh. The water put you out.');
    }
    fx.wet = now + WET_MS;
  }

  /** Fire: you burn for a while. If you're wet, it dries you off instead (and you're safe a moment). */
  #ignite(now) {
    const fx = this.myFx;
    if (now < fx.safe) return;
    if (fx.wet > now) {
      fx.wet = 0;
      fx.safe = now + 1200;
      this.#burst('steam', this.me.x, this.me.y);
      return this.#flashText('Sizzle! The fire dried you off.');
    }
    if (fx.fire <= now) this.#flashText("🔥 You're on fire! Water puts it out.");
    fx.fire = now + FIRE_MS;
  }

  #sendFx(now, extra = {}) {
    const fx = this.myFx;
    this.fxSent = { fire: fx.fire, wet: fx.wet };
    const d = { fire: Math.round(num(fx.fire - now, 0, FX_MAX)), wet: Math.round(num(fx.wet - now, 0, FX_MAX)), ...extra };
    if (this.mySplashes) d.n = this.mySplashes;
    this.#game('fx', d);
  }

  #onFx(sid, d) {
    const now = performance.now();
    const prev = this.fx.get(sid) ?? { fire: 0, wet: 0 };
    const fire = num(d.fire, 0, FX_MAX);
    const wet = num(d.wet, 0, FX_MAX);
    const next = { fire: fire ? now + fire : 0, wet: wet ? now + wet : 0 };
    const p = this.players.get(sid);
    // Fire meeting water (either way round) puffs steam.
    if (p && ((prev.fire > now && !next.fire) || (prev.wet > now + 400 && !next.wet))) this.#burst('steam', p.x, p.y);
    this.fx.set(sid, next);
    if (d.n !== undefined) this.splashes.set(sid, Math.floor(num(d.n, 0, 99_999)));
    // A water balloon hit them: pop it, and credit whoever threw it.
    if (SHORT_RE.test(short(d.by)) && Number.isInteger(d.hit)) {
      const by = short(d.by);
      const i = this.shots.findIndex((s) => short(s.sid) === by && s.id === d.hit);
      if (i >= 0) this.shots.splice(i, 1);
      if (p) this.#burst('splash', p.x, p.y);
      const thrower = this.#sidOf(by);
      const mine = thrower && thrower === this.hooks.mySid();
      if (mine) this.mySplashes += 1;
      else if (thrower) this.splashes.set(thrower, (this.splashes.get(thrower) ?? 0) + 1);
      const target = this.#name(sid) ?? 'someone';
      this.#flashText(mine ? `💦 You splashed ${target}! (${this.mySplashes})` : `💦 ${this.#name(thrower) ?? 'Someone'} splashed ${target}`, 2500);
      this.splashBoard = now + 5000;
    }
  }

  #fxOf(sid) {
    return sid === this.hooks.mySid() ? this.myFx : this.fx.get(sid);
  }

  /** Wet people leave puddles behind them; old ones dry up. */
  #stepPuddles(now) {
    const leave = (sid, p) => {
      const last = this.dripFrom.get(sid);
      if (last && Math.hypot(p.x - last.x, p.y - last.y) < 34) return;
      this.dripFrom.set(sid, { x: p.x, y: p.y });
      this.puddles.push({ x: p.x + (Math.random() - 0.5) * 10, y: p.y + RADIUS - 2, at: now });
    };
    const mine = this.hooks.mySid();
    if (this.me && this.myFx.wet > now) leave(mine, this.me);
    for (const [sid, p] of this.players) if ((this.fx.get(sid)?.wet ?? 0) > now) leave(sid, p);
    while (this.puddles.length && (now - this.puddles[0].at > 4500 || this.puddles.length > 160)) this.puddles.shift();
  }

  // ── Water balloons ───────────────────────────────────────────────────────
  // The thrower says where it starts and which way it flies; everyone flies it
  // the same way. If it hits you, your client soaks you and says who did it.

  #throw() {
    const me = this.me;
    const now = performance.now();
    if (!me || now - this.lastThrow < SHOT.cooldown) return;
    this.lastThrow = now;
    this.shotId = (this.shotId + 1) % 1000;
    const a = Math.round(me.d * 100) / 100;
    const x = Math.round(me.x + Math.cos(a) * (RADIUS + 12));
    const y = Math.round(me.y + Math.sin(a) * (RADIUS + 12));
    this.#launch(this.hooks.mySid(), this.shotId, x, y, a);
    this.#game('throw', { x, y, a, id: this.shotId });
  }

  #launch(sid, id, x, y, a) {
    this.shots.push({ sid, id, x, y, vx: Math.cos(a) * SHOT.speed, vy: Math.sin(a) * SHOT.speed, left: SHOT.range });
    if (this.shots.length > 40) this.shots.shift();
  }

  #onThrow(sid, d) {
    const id = Math.floor(num(d.id, 0, 999));
    const a = Number(d.a);
    if (!Number.isFinite(a)) return;
    this.#launch(sid, id, num(d.x, 0, WORLD.w), num(d.y, 0, WORLD.h), a);
  }

  #stepShots(dt, now) {
    const mine = this.hooks.mySid();
    const me = this.me;
    for (const s of [...this.shots]) {
      s.x += s.vx * dt;
      s.y += s.vy * dt;
      s.left -= SHOT.speed * dt;
      const hitMe = me && s.sid !== mine && Math.hypot(s.x - me.x, s.y - me.y) < RADIUS + SHOT.r;
      if (!hitMe && s.left > 0 && !blocked(s.x, s.y, SHOT.r)) continue;
      this.shots.splice(this.shots.indexOf(s), 1);
      this.#burst('splash', s.x, s.y);
      this.puddles.push({ x: s.x, y: s.y + 8, at: now });
      if (hitMe) this.#hitBy(s, now);
    }
  }

  #hitBy(s, now) {
    this.#soak(now, true);
    this.splashes.set(s.sid, (this.splashes.get(s.sid) ?? 0) + 1);
    this.splashBoard = now + 5000;
    this.#flashText(`💦 ${this.#name(s.sid) ?? 'Someone'} splashed you!`);
    this.#sendFx(now, { by: short(s.sid), hit: s.id });
  }

  // ── Race laps ────────────────────────────────────────────────────────────
  // Each runner times their own laps and announces them; the board shows the
  // best lap of everyone here.

  #stepRace(now) {
    const me = this.me;
    const r = this.race;
    if (!me) return;
    const y0 = r.lastY;
    r.lastY = me.y;
    if (r.start) {
      if (onTrack(me.x, me.y)) r.offAt = 0;
      else r.offAt ||= now;
      if ((r.offAt && now - r.offAt > 4000) || now - r.start > LAP_MAX_MS) {
        r.start = 0;
        this.gameSig = '';
      } else if (r.cp < CORNERS.length && inRect(me.x, me.y, CORNERS[r.cp])) r.cp += 1;
    }
    // The start/finish line runs across the lane on the left; cross it going up.
    if (y0 === null || me.x >= TRACK.lane || !(y0 >= TRACK.startY && me.y < TRACK.startY)) return;
    if (r.start && r.cp === CORNERS.length) {
      const ms = Math.round(now - r.start);
      const record = this.#lapRows()[0];
      r.laps += 1;
      const best = !r.best || ms < r.best;
      if (best) r.best = ms;
      this.#game('lap', { ms, best: r.best, n: r.laps });
      const fastest = !record || ms < record.ms;
      this.#flashText(`🏁 Lap ${r.laps}: ${secs(ms)}${fastest ? ', the fastest here!' : best ? ', your best!' : ''}`, 3500);
    } else if (!r.start) {
      this.#flashText('🏁 Go! Run the lane clockwise, round all four corners.', 3000);
    }
    Object.assign(r, { start: now, cp: 0, offAt: 0 });
  }

  #onLap(sid, d) {
    const best = Math.round(num(d.best, 0, LAP_MAX_MS));
    if (!best) return;
    const record = this.#lapRows()[0];
    this.laps.set(sid, { best, n: Math.floor(num(d.n, 0, 99_999)) });
    const ms = Math.round(num(d.ms, 0, LAP_MAX_MS));
    if (ms) this.#flashText(`🏁 ${this.#name(sid) ?? 'Someone'} ran a lap in ${secs(ms)}${!record || ms < record.ms ? ', the fastest here!' : ''}`, 3000);
  }

  /** Best laps of everyone here, fastest first. */
  #lapRows() {
    const rows = [];
    const mine = this.hooks.mySid();
    if (this.race.best) rows.push({ sid: mine, ms: this.race.best });
    for (const [sid, l] of this.laps) if (this.players.has(sid)) rows.push({ sid, ms: l.best });
    return rows
      .map((r) => ({ ...r, name: this.#name(r.sid) }))
      .filter((r) => r.name)
      .sort((a, b) => a.ms - b.ms);
  }

  // ── Instruments ──────────────────────────────────────────────────────────
  // Nothing is sent: everyone sees where everyone stands, so each client plays
  // the key or pad someone steps onto, if it's in the Music room too.

  #stepInstruments(now) {
    const mine = this.hooks.mySid();
    const hear = this.me && zoneOf(this.me.x, this.me.y) === MUSIC_ZONE && !this.hooks.inCall?.(mine)?.deafened;
    const check = (sid, p) => {
      const hit = instrumentAt(p.x, p.y);
      const id = hit?.id ?? null;
      if (id === (this.inst.get(sid) ?? null)) return;
      this.inst.set(sid, id);
      if (!hit) return;
      if (hear) {
        hit.drum ? playDrum(hit.drum) : playNote(hit.midi);
        this.heard += 1;
      }
      const color = hit.drum ? '#ffd166' : `hsl(${(hit.midi % 12) * 30} 90% 70%)`;
      this.#burst('note', p.x, p.y - RADIUS - 24, { glyph: hit.drum ? '✦' : hit.midi % 2 ? '♫' : '♪', color });
    };
    if (this.me) check(mine, this.me);
    for (const [sid, p] of this.players) check(sid, p);
  }

  // ── Gallery ──────────────────────────────────────────────────────────────
  // Paging is yours alone: nothing is sent, the walls just show another twelve.

  #pages() {
    return Math.max(1, Math.ceil(this.gallery.length / ART.perWall));
  }

  #turnWall(step) {
    this.galleryPage = Math.max(0, Math.min(this.#pages() - 1, this.galleryPage + step));
    this.panelSig = '';
  }

  /** The drawing hanging in a frame on the wall you're looking at, or -1. */
  #drawingIn(frame) {
    const i = frame ? this.galleryPage * ART.perWall + frame.i : -1;
    return i >= 0 && i < this.gallery.length ? i : -1;
  }

  /** Stop in front of a drawing and it opens; walk away and it closes. */
  #stepGallery(now) {
    const me = this.me;
    if (!me) return;
    if (this.viewing >= 0) {
      const a = this.viewAnchor;
      if (zoneOf(me.x, me.y) !== GALLERY_ZONE || !a || Math.hypot(me.x - a.x, me.y - a.y) > 110) this.#closeViewer(false);
      return;
    }
    const frame = zoneOf(me.x, me.y) === GALLERY_ZONE ? frameAt(me.x, me.y) : null;
    const index = frame ? this.#drawingIn(frame) : -1;
    if (index < 0) {
      this.dwell.frame = -1;
      this.dismissed = -1;
      return;
    }
    if (frame.i !== this.dwell.frame || this.vel.x || this.vel.y) this.dwell = { frame: frame.i, since: now };
    if (frame.i !== this.dismissed && !seatAt(me) && now - this.dwell.since >= LOOK_MS) this.#openViewer(index, frame);
  }

  /** E: look at the drawing in front of you, unless a seat is right there. */
  #lookOrSit() {
    const me = this.me;
    if (me && !seatAt(me) && zoneOf(me.x, me.y) === GALLERY_ZONE) {
      const near = SEATS.some((s) => Math.hypot(s.x - me.x, s.y - me.y) < 50);
      const frame = frameAt(me.x, me.y);
      const index = frame ? this.#drawingIn(frame) : -1;
      if (!near && index >= 0) return this.#openViewer(index, frame);
    }
    this.#toggleSit();
  }

  #openViewer(index, frame) {
    this.viewAnchor = { x: this.me.x, y: this.me.y };
    this.dismissed = frame?.i ?? -1;
    this.#showDrawing(index);
  }

  #showDrawing(index) {
    if (!this.gallery.length) return;
    this.viewing = Math.max(0, Math.min(this.gallery.length - 1, index));
    const d = this.gallery[this.viewing];
    // Keep the walls on the same twelve as the viewer.
    this.galleryPage = Math.floor(this.viewing / ART.perWall);
    const who = this.hooks.user?.(d.uid);
    const root = this.root;
    const img = root.querySelector('.space-viewer-img');
    if (img.getAttribute('src') !== `/media/${d.media}`) img.src = `/media/${d.media}`;
    root.querySelector('.space-viewer-title').textContent = `Gallery · ${who?.name ?? 'Someone'}`;
    root.querySelector('.space-viewer-who').textContent = who?.name ?? 'Someone';
    root.querySelector('.space-viewer-dot').style.background = /^#[0-9a-f]{3,8}$/i.test(who?.color ?? '') ? who.color : '#888888';
    root.querySelector('.space-viewer-when').textContent = new Date(d.ts).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
    root.querySelector('.space-viewer-pos').textContent = `${this.viewing + 1} of ${this.gallery.length}`;
    root.querySelector('[data-viewer="-1"]').disabled = this.viewing === 0;
    root.querySelector('[data-viewer="1"]').disabled = this.viewing === this.gallery.length - 1;
    root.querySelector('.space-viewer').hidden = false;
    this.panelSig = '';
  }

  /** `byHand`: Esc or ✕ — then it stays shut until you step to another drawing. */
  #closeViewer(byHand) {
    if (this.viewing < 0) return;
    this.viewing = -1;
    this.viewAnchor = null;
    if (!byHand) this.dismissed = -1;
    this.root.querySelector('.space-viewer').hidden = true;
    this.panelSig = '';
  }

  // ── Ball ─────────────────────────────────────────────────────────────────
  // Everyone rolls the ball forward locally; the last kicker's copy is the
  // truth and goes out ~10×/s while it moves, plus once when it stops. `k`
  // counts kicks: a higher k wins, a tie goes to the lower sid.

  #stepBall(dt, now) {
    const b = this.ball;
    this.#kick(now);
    const moving = this.#rollBall(dt);
    const settle = Math.exp(-dt * 10);
    b.ox *= settle;
    b.oy *= settle;
    const net = netAt(b.x, b.y);
    const entered = net >= 0 && b.inNet < 0;
    b.inNet = net;
    if (b.owner !== this.hooks.mySid()) return;
    if (entered) {
      // Into Red's net (right) scores for Blue, and the other way round.
      b.s[1 - net] += 1;
      b.vx = b.vy = 0;
      b.goal = net;
      b.resetAt = now + 1800;
      b.dirty = true;
      this.#goalFlash(net);
    }
    if (b.resetAt && now >= b.resetAt) {
      Object.assign(b, { x: MID.x, y: MID.y, vx: 0, vy: 0, resetAt: 0, inNet: -1 });
      b.dirty = true;
    }
    if (moving === 'stopped') b.dirty = true;
    if ((b.dirty && now - b.sentAt >= 70) || (moving === true && now - b.sentAt >= 100)) this.#sendBall(now);
  }

  /** Walking into the ball kicks it: harder the faster you're going into it. */
  #kick(now) {
    const me = this.me;
    const b = this.ball;
    if (!me || now - b.kickAt < 150 || seatAt(me)) return;
    const dx = b.x - me.x;
    const dy = b.y - me.y;
    const dist = Math.hypot(dx, dy);
    if (dist >= RADIUS + BALL_R) return;
    const nx = dist > 0.5 ? dx / dist : Math.cos(me.d);
    const ny = dist > 0.5 ? dy / dist : Math.sin(me.d);
    const into = Math.max(0, this.vel.x * nx + this.vel.y * ny);
    const power = Math.min(KICK_MAX, 150 + into * 1.9 + Math.hypot(b.vx, b.vy) * 0.3);
    b.vx = nx * power;
    b.vy = ny * power;
    const px = me.x + nx * (RADIUS + BALL_R + 1);
    const py = me.y + ny * (RADIUS + BALL_R + 1);
    if (!blocked(px, py, BALL_R)) {
      b.ox += b.x - px;
      b.oy += b.y - py;
      b.x = px;
      b.y = py;
    }
    Object.assign(b, { k: b.k + 1, owner: this.hooks.mySid(), kickAt: now, dirty: true, resetAt: 0 });
  }

  /** Physics: friction and bouncy walls. true = moving, 'stopped' = just came to rest. */
  #rollBall(dt) {
    const b = this.ball;
    if (!b.vx && !b.vy) return false;
    const steps = Math.max(1, Math.ceil((Math.hypot(b.vx, b.vy) * dt) / 8));
    const h = dt / steps;
    for (let i = 0; i < steps; i++) {
      // Bounce only when moving into a wall, so a ball that starts overlapping one can get out.
      const free = !blocked(b.x, b.y, BALL_R);
      const nx = b.x + b.vx * h;
      if (free && blocked(nx, b.y, BALL_R)) b.vx *= -BOUNCE;
      else b.x = nx;
      const ny = b.y + b.vy * h;
      if (free && blocked(b.x, ny, BALL_R)) b.vy *= -BOUNCE;
      else b.y = ny;
    }
    b.x = num(b.x, BALL_R, WORLD.w - BALL_R);
    b.y = num(b.y, BALL_R, WORLD.h - BALL_R);
    const f = Math.exp(-FRICTION * dt);
    b.vx *= f;
    b.vy *= f;
    if (Math.hypot(b.vx, b.vy) < 12) {
      b.vx = b.vy = 0;
      return 'stopped';
    }
    return true;
  }

  #sendBall(now) {
    const b = this.ball;
    b.sentAt = now;
    b.dirty = false;
    const d = { x: Math.round(b.x), y: Math.round(b.y), vx: Math.round(b.vx), vy: Math.round(b.vy), k: b.k, s: b.s };
    if (b.goal !== null) d.goal = b.goal;
    b.goal = null;
    this.#game('ball', d);
  }

  #onBall(sid, d) {
    const b = this.ball;
    const k = Math.floor(num(d.k, 0, 1e9));
    // Same k from someone else: two kicks at once (lower sid wins) or an adoption after the owner left.
    if (k < b.k || (k === b.k && sid !== b.owner && this.#here(b.owner) && sid > b.owner)) return;
    const x = num(d.x, BALL_R, WORLD.w - BALL_R);
    const y = num(d.y, BALL_R, WORLD.h - BALL_R);
    // Keep drawing it where it was and glide over, rather than jump.
    const rx = b.x + b.ox;
    const ry = b.y + b.oy;
    Object.assign(b, { x, y, vx: num(d.vx, -KICK_MAX, KICK_MAX), vy: num(d.vy, -KICK_MAX, KICK_MAX), k, owner: sid, resetAt: 0, inNet: netAt(x, y) });
    const far = Math.hypot(rx - x, ry - y) > 200;
    b.ox = far ? 0 : rx - x;
    b.oy = far ? 0 : ry - y;
    if (Array.isArray(d.s)) b.s = [Math.floor(num(d.s[0], 0, 999)), Math.floor(num(d.s[1], 0, 999))];
    if (d.goal === 0 || d.goal === 1) this.#goalFlash(d.goal);
  }

  #goalFlash(net) {
    const [blue, red] = this.ball.s;
    this.#flashText(`⚽ GOAL for ${net ? 'Blue' : 'Red'}! Blue ${blue} – ${red} Red`, 3500);
    // Confetti over whoever kicked it in.
    if (this.ball.owner) this.#burst('confetti', 0, 0, { sid: this.ball.owner });
  }

  // ── Tag ──────────────────────────────────────────────────────────────────
  // Whoever is "it" owns the round: they pass it on by touching someone and
  // re-announce it every few seconds. `t` = deciseconds each person has been
  // it; `k` goes up with every change (a new round starts at the clock's
  // seconds), so the newest state wins and a tie goes to the lower sid.

  #startTag() {
    if (!this.me || !this.tag.over) return;
    if (!this.players.size) return this.#flashText('Tag needs someone else on Space.', 3000);
    const now = performance.now();
    const k = Math.max(this.tag.k + 1, Math.floor(Date.now() / 1000));
    Object.assign(this.tag, { it: short(this.hooks.mySid()), by: null, k, endAt: now + TAG_MS, t: {}, at: now, since: now, heard: now, over: false, result: null });
    this.#sendTag(now);
    this.#flashText("You're it! Touch someone to pass it on.");
  }

  #stepTag(now, hidden) {
    const tg = this.tag;
    if (tg.over) return;
    if (now >= tg.endAt) return this.#endTag(now);
    const mine = this.hooks.mySid();
    if (tg.it !== short(mine)) {
      if (now - tg.heard > TAG_SILENT_MS) this.#reassignIt(now, null);
      return;
    }
    if (now - tg.beat >= TAG_BEAT_MS) this.#sendTag(now);
    const me = this.me;
    if (hidden || !me || now - tg.since < 600) return;
    for (const [sid, p] of this.players) {
      if (Math.hypot(p.x - me.x, p.y - me.y) > RADIUS * 2 + 4) continue;
      if (seatAt({ x: p.tx, y: p.ty })) continue; // sitting is safe
      if (short(sid) === tg.by && now - tg.since < TAG_GRACE_MS) continue; // no tag-backs
      this.#foldTagTime(now);
      Object.assign(tg, { it: short(sid), by: short(mine), k: tg.k + 1, since: now, heard: now });
      this.#sendTag(now);
      this.#flashText(`You tagged ${this.#name(sid) ?? 'them'}!`, 2000);
      return;
    }
  }

  /** Add the running "it" time into the table. */
  #foldTagTime(now) {
    const tg = this.tag;
    if (tg.it && !tg.over) tg.t[tg.it] = Math.round((tg.t[tg.it] ?? 0) + Math.max(0, Math.min(now, tg.endAt) - tg.at) / 100);
    tg.at = now;
  }

  #sendTag(now) {
    const tg = this.tag;
    this.#foldTagTime(now);
    tg.beat = now;
    // Stay well under the relay's 512 bytes: drop people who've left if the table grows.
    if (Object.keys(tg.t).length > 16) {
      const here = new Set([short(this.hooks.mySid()), ...[...this.players.keys()].map(short)]);
      for (const id of Object.keys(tg.t)) if (!here.has(id)) delete tg.t[id];
    }
    const d = { it: tg.it, k: tg.k, left: tg.over ? 0 : Math.max(0, Math.round(tg.endAt - now)), t: tg.t };
    if (tg.by) d.by = tg.by;
    this.#game('tag', d);
  }

  #endTag(now) {
    const tg = this.tag;
    this.#foldTagTime(now);
    tg.over = true;
    tg.result = { until: now + 9000 };
    if (tg.it === short(this.hooks.mySid())) this.#sendTag(now); // the final scores
    this.gameSig = '';
  }

  /** "It" left or went silent: the lowest sid still here hands it to whoever was nearest. */
  #reassignIt(now, gone) {
    const tg = this.tag;
    const mine = this.hooks.mySid();
    const ids = [mine, ...this.players.keys()].filter((sid) => short(sid) !== tg.it);
    if (ids.length < 2) {
      this.#endTag(now);
      return this.#flashText('Everyone else left: tag is over.', 3000);
    }
    tg.heard = now;
    if (ids.sort()[0] !== mine) return;
    const holder = [...this.players].find(([sid]) => short(sid) === tg.it)?.[1];
    const from = gone ?? holder ?? this.me;
    const at = (sid) => this.#spot(sid);
    const next = ids.filter(at).sort((a, b) => Math.hypot(at(a).x - from.x, at(a).y - from.y) - Math.hypot(at(b).x - from.x, at(b).y - from.y))[0];
    this.#foldTagTime(now);
    Object.assign(tg, { it: short(next), by: null, k: tg.k + 1, since: now });
    this.#sendTag(now);
    if (next === mine) this.#flashText("You're it now! Touch someone to pass it on.");
  }

  #onTag(sid, d) {
    const tg = this.tag;
    const now = performance.now();
    const k = Math.floor(num(d.k, 0, 1e12));
    const it = short(d.it);
    if (!SHORT_RE.test(it) || k < tg.k) return;
    const left = num(d.left, 0, TAG_MS);
    if (k === tg.k) {
      if (tg.over && left > 0) return; // a late beat from a finished round
      if (short(sid) !== tg.it && short(sid) > (tg.it ?? '')) return; // two rounds started at once: lower sid wins
    }
    const t = {};
    if (d.t && typeof d.t === 'object') {
      for (const [id, v] of Object.entries(d.t).slice(0, 40)) if (SHORT_RE.test(id)) t[id] = Math.floor(num(v, 0, 1e6));
    }
    const fresh = tg.over; // a new round, or one we're joining
    const changed = it !== tg.it || fresh;
    Object.assign(tg, { it, by: SHORT_RE.test(short(d.by)) ? short(d.by) : null, k, t, at: now, endAt: now + left, heard: now });
    if (changed) tg.since = now;
    this.gameSig = '';
    if (left <= 0) {
      tg.over = true;
      tg.result = { until: now + 9000 };
      return;
    }
    tg.over = false;
    tg.result = null;
    if (changed && it === short(this.hooks.mySid())) {
      const by = this.#name(this.#sidOf(tg.by));
      this.#flashText(by ? `${by} tagged you: you're it!` : "You're it! Touch someone to pass it on.");
    } else if (fresh) {
      this.#flashText(`${this.#name(this.#sidOf(it)) ?? 'Someone'} is it: run!`);
    }
  }

  /** Everyone here with their time as "it", least first. */
  #tagRows(now) {
    const tg = this.tag;
    return [this.hooks.mySid(), ...this.players.keys()]
      .map((sid) => {
        const id = short(sid);
        let ds = tg.t[id] ?? 0;
        if (!tg.over && id === tg.it) ds += Math.max(0, Math.min(now, tg.endAt) - tg.at) / 100;
        return { sid, name: this.#name(sid), secs: Math.floor(ds / 10) };
      })
      .filter((r) => r.name)
      .sort((a, b) => a.secs - b.secs || a.name.localeCompare(b.name));
  }

  // ── Panels: your room, the game line, the Tag button ─────────────────────

  #refreshPanels(now) {
    if (now - this.panelAt < 200 || !this.me) return;
    this.panelAt = now;
    const mine = this.hooks.mySid();
    const zone = zoneOf(this.me.x, this.me.y);
    const here = [mine, ...[...this.players].filter(([, p]) => zoneOf(p.tx, p.ty) === zone).map(([sid]) => sid)];
    const away = this.players.size - (here.length - 1);
    const people = here
      .map((sid) => {
        const who = this.hooks.who(sid);
        if (!who) return '';
        const call = this.hooks.inCall?.(sid);
        const color = /^#[0-9a-f]{3,8}$/i.test(who.color ?? '') ? who.color : '#888888';
        const mic = call ? `<span class="space-mic${call.muted ? ' muted' : ''}" title="${call.muted ? 'In the call, muted' : 'In the call'}">${icon(call.muted ? 'mic-off' : 'volume')}</span>` : '';
        const fx = this.#fxOf(sid);
        const state = fx?.fire > now ? ' 🔥' : fx?.wet > now ? ' 💧' : '';
        return `<li><span class="space-dot" style="--c:${color}"></span><span class="space-who">${esc(who.name)}${sid === mine ? ' <small>(you)</small>' : ''}${state}</span>${mic}</li>`;
      })
      .join('');
    const count = `${here.length} here${away ? ` · ${away} elsewhere` : ''}`;
    const hint = this.hooks.inCall?.(mine) ? VOICE_HINTS[this.voiceMode](zone < 0 ? null : zoneName(zone)) : 'Join a voice channel to talk with people here.';
    const wall = zone === GALLERY_ZONE ? `${this.galleryPage}/${this.#pages()}/${this.gallery.length}` : '';
    const sig = [zone, people, count, hint, wall].join('\n');
    if (sig !== this.panelSig) {
      this.panelSig = sig;
      this.root.querySelector('.space-room-name').textContent = zoneName(zone);
      this.root.querySelector('.space-room-count').textContent = count;
      this.root.querySelector('.space-room-people').innerHTML = people;
      this.root.querySelector('.space-room-hint').textContent = hint;
      // In the Gallery: which wall, and buttons to turn to the next one.
      const box = this.root.querySelector('.space-wall');
      box.hidden = !wall;
      if (wall) {
        const pages = this.#pages();
        box.querySelector('.space-wall-label').textContent = this.gallery.length ? `🖼 Wall ${this.galleryPage + 1} of ${pages} · ${this.gallery.length} drawing${this.gallery.length === 1 ? '' : 's'}` : '🖼 No drawings yet: save one in Draw';
        box.querySelector('[data-wall="-1"]').disabled = this.galleryPage === 0;
        box.querySelector('[data-wall="1"]').disabled = this.galleryPage >= pages - 1;
      }
    }
    this.#refreshGame(now);
  }

  #refreshGame(now) {
    const tg = this.tag;
    const lines = [];
    if (this.flash && now < this.flash.until) lines.push(`<strong>${esc(this.flash.text)}</strong>`);
    const board = (rows) => rows.slice(0, 6).map((r) => `${esc(r.name)} ${r.secs}s`).join(' · ');
    if (!tg.over) {
      const left = Math.max(0, Math.ceil((tg.endAt - now) / 1000));
      const clockText = `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
      const mine = tg.it === short(this.hooks.mySid());
      const who = mine ? "<b>You're it!</b>" : `<b>${esc(this.#name(this.#sidOf(tg.it)) ?? 'Someone')}</b> is it`;
      lines.push(`🏃 Tag · ${who} · <span class="space-clock">${clockText}</span>`);
      lines.push(`<small>${board(this.#tagRows(now))}</small>`);
      lines.push('<small class="space-game-rules">Touch to pass it · sitting is safe · least time as it wins</small>');
    } else if (tg.result && now < tg.result.until) {
      const rows = this.#tagRows(now);
      const best = rows.filter((r) => r.secs === rows[0]?.secs).map((r) => esc(r.name));
      if (best.length) lines.push(`🏁 Time! ${best.join(' &amp; ')} ${best.length > 1 ? 'win' : 'wins'}: least time as it`);
      lines.push(`<small>${board(rows)}</small>`);
    }
    const r = this.race;
    if (r.start) {
      const t = now - r.start;
      const lapClock = `${Math.floor(t / 60000)}:${((t % 60000) / 1000).toFixed(1).padStart(4, '0')}`;
      lines.push(`⏱ <span class="space-clock">${lapClock}</span> · corner ${r.cp}/${CORNERS.length}${r.best ? ` · best ${secs(r.best)}` : ''}`);
    }
    if (now < this.splashBoard) {
      const mine = this.hooks.mySid();
      const rows = [[mine, this.mySplashes], ...[...this.splashes].filter(([sid]) => this.players.has(sid))]
        .filter(([, n]) => n > 0)
        .map(([sid, n]) => [this.#name(sid), n])
        .filter(([name]) => name)
        .sort((a, b) => b[1] - a[1]);
      if (rows.length) lines.push(`<small>💦 ${rows.slice(0, 6).map(([name, n]) => `${esc(name)} ${n}`).join(' · ')}</small>`);
    }
    const html = lines.map((l) => `<div>${l}</div>`).join('');
    const btn = this.root.querySelector('[data-game="tag"]');
    if (btn.disabled === tg.over) {
      btn.disabled = !tg.over;
      btn.title = tg.over ? "Start a round of tag (T): you're it, touch someone to pass it on" : 'A round of tag is on';
    }
    const bar = this.root.querySelector('.space-game');
    // Phones: right under the room window, however tall it is now (it grows in the Gallery).
    const room = this.root.querySelector('.space-room');
    const top = NARROW.matches && html ? `${room.offsetTop + room.offsetHeight + 8}px` : '';
    if (bar.style.top !== top) bar.style.top = top;
    if (html === this.gameSig) return;
    this.gameSig = html;
    bar.innerHTML = html;
    bar.hidden = !html;
  }

  // ── Drawing ──────────────────────────────────────────────────────────────

  #image(id) {
    if (!id) return null;
    if (!this.images.has(id)) {
      const img = new Image();
      img.src = `/media/${id}`;
      this.images.set(id, img);
    }
    const img = this.images.get(id);
    return img.complete && img.naturalWidth ? img : null;
  }

  #render() {
    const c = this.canvas;
    const dpr = devicePixelRatio || 1;
    const w = c.clientWidth;
    const h = c.clientHeight;
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const ctx = c.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const me = this.me;
    if (!me) {
      ctx.fillStyle = '#14122a';
      ctx.fillRect(0, 0, w, h);
      return;
    }
    this.cam.x = Math.max(0, Math.min(WORLD.w - w, me.x - w / 2));
    this.cam.y = Math.max(0, Math.min(WORLD.h - h, me.y - h / 2));
    if (w > WORLD.w) this.cam.x = (WORLD.w - w) / 2;
    if (h > WORLD.h) this.cam.y = (WORLD.h - h) / 2;
    ctx.fillStyle = '#14122a';
    ctx.fillRect(0, 0, w, h);
    ctx.save();
    ctx.translate(-this.cam.x, -this.cam.y);
    this.#drawWorld(ctx);
    const t = clock();
    for (const pd of this.puddles) {
      const age = Math.max(0, t - pd.at);
      if (age < 4500) drawPuddle(ctx, pd.x, pd.y, 0.5 * Math.min(1, (4500 - age) / 1500));
    }
    const b = this.ball;
    drawBall(ctx, b.x + b.ox, b.y + b.oy, (b.x + b.y) / BALL_R);
    const everyone = [...this.players.entries()];
    everyone.push([this.hooks.mySid(), me]);
    everyone.sort((a, b2) => a[1].y - b2[1].y);
    for (const [sid, p] of everyone) this.#drawPlayer(ctx, sid, p, sid === this.hooks.mySid());
    for (const s of this.shots) drawBalloon(ctx, s.x, s.y, Math.sin((1 - s.left / SHOT.range) * Math.PI) * 26 + 6);
    this.bursts = this.bursts.filter((bu) => {
      const at = bu.sid ? this.#spot(bu.sid) : bu;
      return at ? drawBurst(ctx, bu, at.x, at.y, Math.max(0, t - bu.at)) : false;
    });
    if (this.target) {
      ctx.strokeStyle = 'rgba(255,255,255,0.6)';
      ctx.setLineDash([6, 6]);
      ctx.beginPath();
      ctx.arc(this.target.x, this.target.y, 14, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    ctx.restore();
    this.#drawRadar(w, h);
  }

  #drawWorld(ctx) {
    drawFloor(ctx);
    drawRoomNames(ctx);
    const env = {
      tagOn: !this.tag.over,
      score: this.ball.s,
      keysDown: new Set([...this.inst.values()].filter(Boolean)),
      laps: this.#lapRows(),
      touch: TOUCH,
      gallery: {
        items: this.gallery.slice(this.galleryPage * ART.perWall, (this.galleryPage + 1) * ART.perWall),
        image: (media) => this.#image(media),
        user: (uid) => this.hooks.user?.(uid) ?? null,
        focus: this.viewing >= 0 && Math.floor(this.viewing / ART.perWall) === this.galleryPage ? this.viewing % ART.perWall : this.dwell.frame,
        page: this.galleryPage,
        pages: this.#pages(),
        total: this.gallery.length,
      },
    };
    for (const [kind, x, y, extra] of DECOR) drawDecor(ctx, kind, x, y, extra, env);
    drawWalls(ctx);
  }

  #drawPlayer(ctx, sid, p, isMe) {
    const who = this.hooks.who(sid);
    if (!who) return;
    this.#drawEmote(ctx, sid, p);
    // Sitting: a little lower and squashed, no bobbing.
    if (seatAt({ x: p.tx ?? p.x, y: p.ty ?? p.y })) {
      ctx.save();
      ctx.translate(p.x, p.y + 8);
      ctx.scale(0.95, 0.82);
      ctx.translate(-p.x, -p.y);
      this.#drawBody(ctx, sid, { ...p, sitting: true }, isMe, who);
      ctx.restore();
      return;
    }
    this.#drawBody(ctx, sid, p, isMe, who);
  }

  /** Emote bubble over someone's head: pops in, floats up, fades out. */
  #drawEmote(ctx, sid, p) {
    const em = this.emotes.get(sid);
    if (!em) return;
    const t = performance.now() - em.at;
    if (t > EMOTE_MS) return this.emotes.delete(sid);
    const pop = Math.min(1, t / 160);
    const scale = 0.6 + 0.4 * pop + (t < 300 ? Math.sin((t / 300) * Math.PI) * 0.15 : 0);
    const alpha = t > EMOTE_MS - 400 ? (EMOTE_MS - t) / 400 : 1;
    const y = p.y - RADIUS - 58 - Math.min(14, t / 120);
    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.translate(p.x, y);
    ctx.scale(scale, scale);
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.arc(0, 0, 24, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.moveTo(-6, 20);
    ctx.lineTo(0, 32);
    ctx.lineTo(6, 20);
    ctx.fill();
    ctx.font = '28px "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(em.e, 0, 2);
    ctx.restore();
  }

  #drawBody(ctx, sid, p, isMe, who) {
    ctx.save();
    const now = performance.now();
    const it = !this.tag.over && short(sid) === this.tag.it;
    const fx = this.#fxOf(sid);
    const burning = fx?.fire > now;
    const wet = fx?.wet > now;
    // shadow
    ctx.fillStyle = 'rgba(0,0,0,0.3)';
    ctx.beginPath();
    ctx.ellipse(p.x, p.y + RADIUS - 2, RADIUS * 0.9, 7, 0, 0, Math.PI * 2);
    ctx.fill();
    if (burning) drawGlow(ctx, p.x, p.y);
    // "It" in tag: a red ring that pulses.
    if (it) {
      const beat = (clock() % 1000) / 1000;
      ctx.strokeStyle = '#ff4d4f';
      ctx.lineWidth = 4;
      ctx.beginPath();
      ctx.arc(p.x, p.y, RADIUS + 9 + Math.sin(beat * Math.PI * 2) * 3, 0, Math.PI * 2);
      ctx.stroke();
    }
    // facing nub
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.arc(p.x + Math.cos(p.d) * (RADIUS + 6), p.y + Math.sin(p.d) * (RADIUS + 6), 5, 0, Math.PI * 2);
    ctx.fill();
    // body: an emoji character, or a round sticker with their picture/initial
    if (who.look) {
      ctx.font = `${RADIUS * 2.1}px "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      const bob = p.sitting ? 0 : Math.sin(now / 260 + p.x) * 2;
      ctx.fillText(who.look, p.x, p.y + 2 + bob);
      if (isMe) {
        ctx.strokeStyle = 'rgba(255,255,255,0.7)';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(p.x, p.y, RADIUS + 4, 0, Math.PI * 2);
        ctx.stroke();
      }
    } else {
      ctx.fillStyle = who.color;
      ctx.beginPath();
      ctx.arc(p.x, p.y, RADIUS, 0, Math.PI * 2);
      ctx.fill();
      const img = this.#image(who.avatar);
      if (img) {
        ctx.save();
        ctx.beginPath();
        ctx.arc(p.x, p.y, RADIUS - 3, 0, Math.PI * 2);
        ctx.clip();
        ctx.drawImage(img, p.x - RADIUS + 3, p.y - RADIUS + 3, (RADIUS - 3) * 2, (RADIUS - 3) * 2);
        ctx.restore();
      } else {
        ctx.fillStyle = '#fff';
        ctx.font = '700 20px ui-sans-serif, system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(initial(who.name), p.x, p.y + 1);
      }
      ctx.lineWidth = 3;
      ctx.strokeStyle = isMe ? '#ffffff' : '#0d0b1e';
      ctx.beginPath();
      ctx.arc(p.x, p.y, RADIUS, 0, Math.PI * 2);
      ctx.stroke();
    }
    if (wet) drawWet(ctx, p.x, p.y);
    if (burning) drawFlames(ctx, p.x, p.y);
    this.#nameTag(ctx, p, `${it ? 'IT · ' : ''}${who.name}${isMe ? ' (you)' : ''}`, it);
    ctx.restore();
  }

  #nameTag(ctx, p, label, it = false) {
    ctx.font = '700 13px ui-sans-serif, system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'alphabetic';
    const tw = ctx.measureText(label).width + 12;
    ctx.fillStyle = it ? '#c62828' : 'rgba(13,11,30,0.85)';
    ctx.fillRect(p.x - tw / 2, p.y - RADIUS - 30, tw, 20);
    ctx.strokeStyle = '#06050d';
    ctx.lineWidth = 2;
    ctx.strokeRect(p.x - tw / 2, p.y - RADIUS - 30, tw, 20);
    ctx.fillStyle = '#fff';
    ctx.fillText(label, p.x, p.y - RADIUS - 15);
  }

  /**
   * CS:GO-style radar: north-up, rooms with their names (yours lit up), the
   * track round the edge, walls, the ball, everyone as dots ("it" ringed red,
   * burning orange, dripping blue), you as an arrow, your view as a box.
   */
  #drawRadar(viewW, viewH) {
    const r = this.radar;
    const dpr = devicePixelRatio || 1;
    const rw = r.clientWidth || 220;
    const rh = r.clientHeight || 147;
    if (r.width !== Math.round(rw * dpr) || r.height !== Math.round(rh * dpr)) {
      r.width = Math.round(rw * dpr);
      r.height = Math.round(rh * dpr);
    }
    const ctx = r.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const s = rw / WORLD.w;
    ctx.clearRect(0, 0, rw, rh);
    ctx.fillStyle = 'rgba(8, 10, 18, 0.78)';
    ctx.fillRect(0, 0, rw, rh);
    ctx.strokeStyle = 'rgba(200, 110, 80, 0.55)';
    ctx.lineWidth = Math.max(2, TRACK.lane * s * 0.6);
    ctx.strokeRect(56 * s, 56 * s, (WORLD.w - 112) * s, (WORLD.h - 112) * s);
    const me = this.me;
    const myZone = me ? zoneOf(me.x, me.y) : -2;
    ROOMS.forEach((room, i) => {
      ctx.fillStyle = i === myZone ? 'rgba(120, 190, 255, 0.30)' : 'rgba(255,255,255,0.10)';
      ctx.fillRect(room.x * s, room.y * s, room.w * s, room.h * s);
    });
    ctx.fillStyle = 'rgba(190, 220, 255, 0.55)';
    for (const wall of SOLID) ctx.fillRect(wall.x * s, wall.y * s, Math.max(1, wall.w * s), Math.max(1, wall.h * s));
    // Room names, small; the one you're in brighter.
    const small = rw < 180;
    ctx.font = `600 ${small ? 7 : 9}px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ROOMS.forEach((room, i) => {
      ctx.fillStyle = i === myZone ? '#ffffff' : 'rgba(255,255,255,0.6)';
      ctx.fillText(small ? room.short ?? room.name : room.name, (room.x + room.w / 2) * s, (room.y + room.h / 2) * s);
    });
    ctx.fillStyle = myZone === -1 ? '#ffffff' : 'rgba(255,255,255,0.45)';
    ctx.fillText(PLAZA, PLAZA_SPOT.x * s, PLAZA_SPOT.y * s);
    ctx.textBaseline = 'alphabetic';
    // view box
    ctx.strokeStyle = 'rgba(255,255,255,0.55)';
    ctx.lineWidth = 1;
    ctx.strokeRect(this.cam.x * s, this.cam.y * s, Math.min(viewW, WORLD.w) * s, Math.min(viewH, WORLD.h) * s);
    // the ball
    ctx.fillStyle = '#ffffff';
    ctx.strokeStyle = '#000';
    ctx.beginPath();
    ctx.arc((this.ball.x + this.ball.ox) * s, (this.ball.y + this.ball.oy) * s, 2.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    const now = performance.now();
    const itSid = this.tag.over ? null : this.tag.it;
    const ring = (x, y, color, size) => {
      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(x * s, y * s, size, 0, Math.PI * 2);
      ctx.stroke();
      ctx.lineWidth = 1;
    };
    const marks = (sid, p, size) => {
      const fx = this.#fxOf(sid);
      if (short(sid) === itSid) ring(p.x, p.y, '#ff4d4f', size);
      else if (fx?.fire > now) ring(p.x, p.y, '#ff9f45', size);
      else if (fx?.wet > now) ring(p.x, p.y, '#6cc4ff', size);
    };
    for (const [sid, p] of this.players) {
      const who = this.hooks.who(sid);
      if (!who) continue;
      ctx.fillStyle = who.color;
      ctx.beginPath();
      ctx.arc(p.x * s, p.y * s, 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = '#000';
      ctx.stroke();
      marks(sid, p, 7);
    }
    // me: white arrow pointing where I face
    if (!me) return;
    marks(this.hooks.mySid(), me, 8);
    ctx.save();
    ctx.translate(me.x * s, me.y * s);
    ctx.rotate(me.d);
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.moveTo(8, 0);
    ctx.lineTo(-5, -5);
    ctx.lineTo(-2, 0);
    ctx.lineTo(-5, 5);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
    ctx.strokeStyle = 'rgba(255,255,255,0.35)';
    ctx.lineWidth = 2;
    ctx.strokeRect(1, 1, rw - 2, rh - 2);
  }
}

/** A gallery entry from the server, checked; `when` = the short date on its name plate. */
function cleanDrawing(d) {
  if (!d || typeof d !== 'object' || !MEDIA_RE.test(String(d.media ?? ''))) return null;
  const ts = Number(d.ts) || 0;
  return { id: String(d.id ?? d.media), media: String(d.media), uid: String(d.uid ?? ''), ts, when: ts ? new Date(ts).toLocaleDateString([], { month: 'short', day: 'numeric' }) : '' };
}

function freshBall() {
  // ox/oy: drawing offset that glides to 0 after a correction from the network.
  return { x: MID.x, y: MID.y, vx: 0, vy: 0, k: 0, s: [0, 0], owner: null, ox: 0, oy: 0, sentAt: 0, dirty: false, inNet: -1, goal: null, resetAt: 0, kickAt: 0 };
}
