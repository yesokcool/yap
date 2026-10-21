/*
 * When's good — a weekly when2meet (PROTOCOL.md › When's good).
 *
 * Paint the hours you're free on the left grid; the right grid is everyone,
 * as a heat map. Times travel as half-hour slots since the epoch, so each
 * person sees the same moments in their own time zone. An hour cell sets
 * both of its half-hour slots. This week and next; every Monday starts fresh.
 */

import { esc } from './format.js';
import { confirmSheet } from './sheet.js';

const SLOT_MS = 1_800_000;
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
/** Folded away unless you ask (or someone is free then): 2 AM – 8 AM. */
const NIGHT_FROM = 2;
const NIGHT_TO = 8;
const NIGHT_KEY = 'yap.whenNight';


/** Local Monday 00:00 of the week `offset` weeks from now. */
function mondayOf(offset) {
  const now = new Date();
  const back = (now.getDay() + 6) % 7;
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() - back + offset * 7);
}

/** The slot an hour cell starts at (local time, so DST is handled by Date). */
function slotOf(monday, day, hour) {
  return Math.floor(new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + day, hour).getTime() / SLOT_MS);
}

function hourLabel(h) {
  if (h === 0) return '12 AM';
  if (h === 12) return 'Noon';
  return h < 12 ? `${h} AM` : `${h - 12} PM`;
}

export class WhenPlanner {
  /**
   * @param {HTMLElement} el
   * @param {{ send: (m: object) => void, me: () => string, users: () => Map<string, {name: string, color: string}>,
   *   hangs: () => {id: string, at: number, title: string, channelId: string|null, uid: string}[], canCancel: (h: object) => boolean,
   *   voiceChannels: () => {id: string, name: string}[] }} host
   */
  constructor(el, host) {
    this.el = el;
    this.host = host;
    this.week = 0; // 0 = this week, 1 = next
    this.tab = 'mine'; // narrow screens show one grid at a time
    /** uid → Set of slots */
    this.people = new Map();
    this.loaded = false;
    this.paint = null; // { mode: 'add'|'remove', from: [day, hour], to: [day, hour] }
    this.picked = null; // [day, hour] tapped on the group grid
    this.planning = null; // the slot (ms) a hang is being planned for
    /** Show 2–8 AM too (per device). */
    this.night = (() => { try { return localStorage.getItem(NIGHT_KEY) === '1'; } catch { return false; } })();
    this.bind();
  }

  open() {
    this.host.send({ t: 'when.get' });
    this.render();
  }

  onAll(msg) {
    this.people = new Map(msg.people.map((p) => [p.uid, new Set(p.slots)]));
    this.loaded = true;
    this.render();
  }

  onOne(msg) {
    if (msg.uid === this.host.me() && this.paint) return; // our own echo mid-drag
    if (msg.slots.length) this.people.set(msg.uid, new Set(msg.slots));
    else this.people.delete(msg.uid);
    this.render();
  }

  mine() {
    return this.people.get(this.host.me()) ?? new Set();
  }

  /** My slots with the drag rectangle applied. */
  preview() {
    const set = new Set(this.mine());
    if (!this.paint) return set;
    const monday = mondayOf(this.week);
    const [d1, h1] = this.paint.from;
    const [d2, h2] = this.paint.to;
    for (let d = Math.min(d1, d2); d <= Math.max(d1, d2); d++) {
      for (let h = Math.min(h1, h2); h <= Math.max(h1, h2); h++) {
        const s = slotOf(monday, d, h);
        if (s * SLOT_MS + 2 * SLOT_MS <= Date.now()) continue; // the past stays as it was
        for (const x of [s, s + 1]) this.paint.mode === 'add' ? set.add(x) : set.delete(x);
      }
    }
    return set;
  }

  commit(set) {
    this.people.set(this.host.me(), set);
    if (!set.size) this.people.delete(this.host.me());
    this.host.send({ t: 'when.set', slots: [...set] });
  }

  bind() {
    const cellAt = (e) => {
      const cell = document.elementFromPoint(e.clientX, e.clientY)?.closest?.('.when-cell');
      return cell && this.el.contains(cell) ? cell : null;
    };
    this.el.addEventListener('pointerdown', (e) => {
      const cell = e.target.closest('.when-cell');
      if (!cell || e.button > 0) return;
      const at = [Number(cell.dataset.d), Number(cell.dataset.h)];
      if (cell.closest('.when-group')) {
        this.picked = this.picked && this.picked[0] === at[0] && this.picked[1] === at[1] ? null : at;
        this.render();
        // Who's free then shows above the grids: bring it into view on a phone.
        return this.el.querySelector('.when-foot')?.scrollIntoView?.({ block: 'nearest' });
      }
      if (cell.classList.contains('past')) return;
      e.preventDefault();
      this.paint = { mode: cell.classList.contains('on') ? 'remove' : 'add', from: at, to: at };
      this.el.setPointerCapture?.(e.pointerId);
      this.renderGrids();
    });
    this.el.addEventListener('pointermove', (e) => {
      if (!this.paint) return;
      const cell = cellAt(e);
      if (!cell || cell.closest('.when-group')) return;
      const at = [Number(cell.dataset.d), Number(cell.dataset.h)];
      if (at[0] === this.paint.to[0] && at[1] === this.paint.to[1]) return;
      this.paint.to = at;
      this.renderGrids();
    });
    const finish = () => {
      if (!this.paint) return;
      const set = this.preview();
      this.paint = null;
      this.commit(set);
      this.render();
    };
    this.el.addEventListener('pointerup', finish);
    this.el.addEventListener('pointercancel', finish);
    // Keep what's typed in the plan form across re-renders (others painting their times).
    this.el.addEventListener('input', (e) => {
      const form = e.target.closest('.when-plan');
      if (form) this.planDraft = { title: form.title.value, channel: form.channel.value };
    });
    this.el.addEventListener('change', (e) => {
      const form = e.target.closest('.when-plan');
      if (form) this.planDraft = { title: form.title.value, channel: form.channel.value };
    });
    this.el.addEventListener('submit', (e) => {
      if (!e.target.matches('.when-plan')) return;
      e.preventDefault();
      const form = e.target;
      const title = form.title.value.trim();
      if (!title) return;
      this.host.send({ t: 'hang.set', at: this.planning, title, ...(form.channel.value ? { channelId: form.channel.value } : {}) });
      this.planning = null;
      this.planDraft = null;
      this.picked = null;
      this.render();
    });
    this.el.addEventListener('click', (e) => {
      const b = e.target.closest('[data-when]');
      if (!b) return;
      const what = b.dataset.when;
      if (what === 'plan') {
        this.planning = Number(b.dataset.at);
        this.renderGrids();
        this.el.querySelector('.when-plan input')?.focus();
        return;
      }
      if (what === 'unplan') {
        this.planning = null;
        this.planDraft = null;
        return this.renderGrids();
      }
      if (what === 'cancel-hang') {
        const hang = this.host.hangs().find((h) => h.id === b.dataset.id);
        if (hang) confirmSheet({ title: `Cancel “${hang.title}”?`, text: 'Everyone sees it’s off.', ok: 'Cancel it', cancel: 'Keep it', danger: true }).then((yes) => yes && this.host.send({ t: 'hang.cancel', id: hang.id }));
        return;
      }
      if (what === 'night') {
        this.night = !this.night;
        try { localStorage.setItem(NIGHT_KEY, this.night ? '1' : '0'); } catch {}
      }
      if (what === 'week') this.week = Number(b.dataset.week);
      if (what === 'tab') this.tab = b.dataset.tab;
      if (what === 'clear') {
        const monday = mondayOf(this.week);
        const from = slotOf(monday, 0, 0);
        const to = slotOf(monday, 7, 0);
        const left = new Set([...this.mine()].filter((s) => s < from || s >= to));
        this.commit(left);
      }
      this.picked = null;
      this.render();
    });
  }

  /** Who answered for the week shown (anyone with a slot in it). */
  responders(from, to) {
    return [...this.people].filter(([, set]) => [...set].some((s) => s >= from && s < to)).map(([uid]) => uid);
  }

  /** Where someone is typing in the plan form (a friend painting re-renders it), to put them back after. */
  typing() {
    const el = document.activeElement;
    if (!el?.name || !this.el.contains(el) || !el.closest('.when-plan')) return null;
    return { name: el.name, from: el.selectionStart ?? null, to: el.selectionEnd ?? null };
  }

  render() {
    const caret = this.typing();
    const monday = mondayOf(this.week);
    const sunday = new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + 6);
    const fmt = (d) => d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    this.el.innerHTML = `
      <div class="when-wrap tab-${this.tab}">
        <header class="when-head">
          <div>
            <h2>When's good?</h2>
            <p class="muted">Paint the times you're free. Everyone's answers show as a heat map. Starts fresh every Monday.</p>
          </div>
          <div class="when-controls">
            <div class="when-seg" role="group" aria-label="Week">
              <button type="button" data-when="week" data-week="0" class="btn small ${this.week === 0 ? 'on' : ''}" aria-pressed="${this.week === 0}">This week</button>
              <button type="button" data-when="week" data-week="1" class="btn small ${this.week === 1 ? 'on' : ''}" aria-pressed="${this.week === 1}">Next week</button>
            </div>
            <span class="when-range">${esc(fmt(monday))} – ${esc(fmt(sunday))}</span>
            <button type="button" class="link-btn when-night" data-when="night" hidden>Hide night hours</button>
          </div>
        </header>
        <div class="when-seg when-tabs" role="group" aria-label="Show">
          <button type="button" data-when="tab" data-tab="mine" class="btn small ${this.tab === 'mine' ? 'on' : ''}" aria-pressed="${this.tab === 'mine'}">Your times</button>
          <button type="button" data-when="tab" data-tab="group" class="btn small ${this.tab === 'group' ? 'on' : ''}" aria-pressed="${this.tab === 'group'}">Everyone</button>
        </div>
        <div class="when-hangs"></div>
        <div class="when-foot"></div>
        <div class="when-grids"></div>
      </div>`;
    this.renderGrids(caret);
  }

  /** Planned hangs: one push to whoever's free when each starts (PROTOCOL.md › Hang reminders). */
  renderHangs() {
    const box = this.el.querySelector('.when-hangs');
    if (!box) return;
    const users = this.host.users();
    const channels = new Map(this.host.voiceChannels().map((c) => [c.id, c.name]));
    const hangs = this.host.hangs();
    box.innerHTML = hangs.length ? `<h3>Planned</h3><ul class="hang-list">${hangs.map((h) => {
      const when = new Date(h.at);
      const live = h.at <= Date.now();
      return `<li class="${live ? 'live' : ''}">
          <span class="hang-when">${live ? 'Now' : esc(when.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' }))} · ${esc(when.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }))}</span>
          <b>${esc(h.title)}</b>${h.channelId ? ` <span class="muted">in ${esc(channels.get(h.channelId) ?? 'voice')}</span>` : ''}
          <small class="muted">by ${esc(users.get(h.uid)?.name ?? 'someone')}</small>
          ${this.host.canCancel(h) && !live ? `<button type="button" class="link-btn" data-when="cancel-hang" data-id="${esc(h.id)}">Cancel</button>` : ''}
        </li>`;
    }).join('')}</ul>` : '';
  }

  /** The little "plan a hang at …" form. */
  planForm(at) {
    const when = new Date(at);
    const label = `${DAYS[(when.getDay() + 6) % 7]} ${when.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`;
    const draft = this.planDraft ?? { title: '', channel: this.host.voiceChannels()[0]?.id ?? '' };
    return `<form class="when-plan inline-form" autocomplete="off">
        <span>🗓 <b>${esc(label)}</b></span>
        <input name="title" maxlength="80" placeholder="What's the plan? (Game night…)" required value="${esc(draft.title)}">
        <span class="combo"><select name="channel" aria-label="Voice channel"><option value="">No voice channel</option>${this.host.voiceChannels().map((c) => `<option value="${esc(c.id)}" ${c.id === draft.channel ? 'selected' : ''}>${esc(c.name)}</option>`).join('')}</select></span>
        <button type="submit" class="btn small primary">Plan it</button>
        <button type="button" class="btn small" data-when="unplan">Cancel</button>
        <small class="muted">Whoever's free then gets one reminder when it starts.</small>
      </form>`;
  }

  renderGrids(caret = this.typing()) {
    const box = this.el.querySelector('.when-grids');
    if (!box) return;
    this.renderHangs();
    const monday = mondayOf(this.week);
    const from = slotOf(monday, 0, 0);
    const to = slotOf(monday, 7, 0);
    const mine = this.preview();
    const users = this.host.users();
    const answered = this.responders(from, to);
    if (!answered.includes(this.host.me()) && [...mine].some((s) => s >= from && s < to)) answered.push(this.host.me());
    const total = answered.length;
    const now = Date.now();
    const today = (new Date().getDay() + 6) % 7;
    const head = DAYS.map((d, i) => {
      const date = new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + i);
      return `<div class="when-day ${this.week === 0 && i === today ? 'today' : ''}">${d}<small>${date.getDate()}</small></div>`;
    }).join('');
    const free = (uid, s) => (uid === this.host.me() ? mine : this.people.get(uid))?.has(s);
    // Night hours fold away unless you've asked for them, or someone is free then this week.
    const nightTaken = answered.some((uid) => {
      for (let d = 0; d < 7; d++) for (let h = NIGHT_FROM; h < NIGHT_TO; h++) if (free(uid, slotOf(monday, d, h))) return true;
      return false;
    });
    const folded = !this.night && !nightTaken;
    const hours = Array.from({ length: 24 }, (_, h) => h).filter((h) => !folded || h < NIGHT_FROM || h >= NIGHT_TO);
    const toggle = this.el.querySelector('.when-night');
    if (toggle) toggle.hidden = folded || nightTaken; // (can't hide hours someone is free in)
    const grid = (group) => {
      let rows = '';
      for (const h of hours) {
        if (folded && h === NIGHT_TO) rows += `<button type="button" class="when-fold" data-when="night" title="Show 2 AM – 8 AM">${hourLabel(NIGHT_FROM)} – ${hourLabel(NIGHT_TO)} · Show night hours</button>`;
        rows += `<div class="when-hour">${hourLabel(h)}</div>`;
        for (let d = 0; d < 7; d++) {
          const s = slotOf(monday, d, h);
          const past = (s + 2) * SLOT_MS <= now;
          if (group) {
            const n = answered.filter((uid) => free(uid, s)).length;
            const level = total ? n / total : 0;
            const picked = this.picked && this.picked[0] === d && this.picked[1] === h;
            rows += `<div class="when-cell ${past ? 'past' : ''} ${n && n === total && total > 1 ? 'all' : ''} ${picked ? 'picked' : ''}" data-d="${d}" data-h="${h}" style="--level:${level.toFixed(3)}" title="${n}/${total} free"></div>`;
          } else {
            rows += `<div class="when-cell ${past ? 'past' : ''} ${mine.has(s) ? 'on' : ''}" data-d="${d}" data-h="${h}"></div>`;
          }
        }
      }
      return `<div class="when-grid ${group ? 'when-group' : 'when-mine'}">
          <div class="when-corner"></div>${head}${rows}
        </div>`;
    };
    box.innerHTML = `
      <section class="when-panel mine-panel">
        <h3>Your times <button type="button" class="link-btn" data-when="clear">${this.week ? 'Clear next week' : 'Clear this week'}</button></h3>
        ${grid(false)}
      </section>
      <section class="when-panel group-panel">
        <h3>Everyone <small class="muted">${total ? `${total} answered` : 'nobody yet'}</small></h3>
        ${grid(true)}
      </section>`;

    // Footer: who's free at the picked hour, or the best times this week.
    const foot = this.el.querySelector('.when-foot');
    const names = (uids) => uids.map((uid) => `<b style="color:${esc(users.get(uid)?.color ?? 'inherit')}">${esc(users.get(uid)?.name ?? 'Someone')}</b>`).join(', ');
    if (this.planning) {
      foot.innerHTML = this.planForm(this.planning);
    } else if (this.picked) {
      const [d, h] = this.picked;
      const s = slotOf(monday, d, h);
      const yes = answered.filter((uid) => free(uid, s));
      const no = answered.filter((uid) => !free(uid, s));
      const future = s * SLOT_MS > now + 5 * 60_000;
      foot.innerHTML = `<p><b>${DAYS[d]} ${hourLabel(h)}</b> · ${yes.length}/${total} free${yes.length ? `: ${names(yes)}` : ''}${no.length ? ` <span class="muted">· not: ${names(no)}</span>` : ''}
        ${future ? ` <button type="button" class="btn small" data-when="plan" data-at="${s * SLOT_MS}">🗓 Plan a hang then</button>` : ''}</p>`;
    } else {
      foot.innerHTML = this.loaded ? bestTimes(monday, answered, free, now) : '<p class="muted">Loading…</p>';
    }
    const again = caret && foot.querySelector(`.when-plan [name="${caret.name}"]`);
    if (again) {
      again.focus({ preventScroll: true });
      if (caret.from !== null) again.setSelectionRange?.(caret.from, caret.to);
    }
  }
}

/** The longest runs where the most people are free (at least two). */
function bestTimes(monday, answered, free, now) {
  if (answered.length < 2) return '<p class="muted">Once two people paint their times, the best overlaps show up here. Tap a cell in Everyone to see who’s free.</p>';
  const runs = [];
  for (let d = 0; d < 7; d++) {
    let start = null;
    let count = 0;
    for (let h = 0; h <= 24; h++) {
      const s = h < 24 ? slotOf(monday, d, h) : null;
      const n = s !== null && (s + 2) * SLOT_MS > now ? answered.filter((uid) => free(uid, s)).length : 0;
      if (start !== null && n !== count) {
        if (count >= 2) runs.push({ d, from: start, to: h, n: count });
        start = null;
      }
      if (start === null && n >= 2) {
        start = h;
        count = n;
      }
    }
  }
  if (!runs.length) return '<p class="muted">No overlap yet. Tap a cell in Everyone to see who’s free.</p>';
  runs.sort((a, b) => b.n - a.n || b.to - b.from - (a.to - a.from) || a.d - b.d || a.from - b.from);
  return `<p class="when-best"><b>Best times:</b> ${runs.slice(0, 3).map((r) => {
    const at = slotOf(monday, r.d, r.from) * SLOT_MS;
    const plan = at > now + 5 * 60_000 ? ` <button type="button" class="link-btn" data-when="plan" data-at="${at}" title="Plan a hang then: one reminder for whoever's free">Plan</button>` : '';
    return `<span class="when-run">${DAYS[r.d]} ${hourLabel(r.from)}–${hourLabel(r.to % 24)} <span class="muted">(${r.n} free)</span>${plan}</span>`;
  }).join(' · ')}</p>`;
}
