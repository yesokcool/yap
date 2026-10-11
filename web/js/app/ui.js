// Small pieces the whole page shares: toasts, the popover, the gate card,
// avatars and badges, colour swatches, the message box's size, and words for
// times and people.

import { COLORS } from '../settings.js';
import { esc, formatTime, sameDay, initial } from '../format.js';
import { confirmSheet } from '../sheet.js';
import { $, state } from './state.js';

export function insertAtCaret(input, text) {
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? start;
  const before = input.value.slice(0, start);
  const spacer = before && !/\s$/.test(before) ? ' ' : '';
  input.value = before + spacer + text + input.value.slice(end);
  const pos = start + spacer.length + text.length;
  input.focus();
  input.setSelectionRange(pos, pos);
  autosize();
}

/**
 * Before redrawing part of the page with innerHTML: remember which of its
 * controls has the keyboard, and put the keyboard back on the redrawn one
 * (else focus falls to the page and Tab starts over from the top).
 * Returns the function that puts it back.
 */
export function holdFocus(container) {
  const el = document.activeElement;
  const attrs = el && el !== container && container?.contains(el)
    ? ['data-action', 'data-id', 'data-uid', 'data-view', 'data-sid'].filter((a) => el.hasAttribute(a)).map((a) => `[${a}="${CSS.escape(el.getAttribute(a))}"]`).join('')
    : '';
  const selector = attrs && `${el.tagName.toLowerCase()}${attrs}`;
  return () => {
    if (!selector || (document.activeElement !== document.body && document.activeElement !== null)) return;
    container.querySelector(selector)?.focus({ preventScroll: true });
  };
}

export function avatar(user, cls = '') {
  const color = esc(user?.color ?? '#8d8d8d');
  const face = user?.avatar ? `<img src="/media/${esc(user.avatar)}" alt="" loading="lazy" draggable="false">` : esc(initial(user?.name));
  return `<span class="avatar ${cls}" style="--c:${color}">${face}</span>`;
}

/** The full-screen card: the waiting room (`waiting`: a busy bar and "start over"), or banned / turned away. */
export function showGate(title, text, { waiting = false } = {}) {
  $('gate-title').textContent = title;
  $('gate-text').textContent = text;
  $('gate-wait').hidden = !waiting;
  $('gate-actions').hidden = !waiting;
  $('gate').hidden = false;
}

export function hideGate() {
  $('gate').hidden = true;
}

export function roleBadge(user) {
  if (user?.role === 'owner') return ' <span class="role-badge owner" title="Owner">owner</span>';
  if (user?.role === 'mod') return ' <span class="role-badge" title="Moderator">mod</span>';
  return '';
}

/** "back at 6:00 PM" / "back tomorrow, 9:00 AM" */
export function untilText(ts) {
  const d = new Date(ts);
  const time = formatTime(ts);
  if (sameDay(ts, Date.now())) return `at ${time}`;
  if (sameDay(ts, Date.now() + 86_400_000)) return `tomorrow, ${time}`;
  return `${d.toLocaleDateString(undefined, { weekday: 'long' })}, ${time}`;
}

export function autosize() {
  const input = $('input');
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, 220)}px`;
}

/** What opened each floating panel (by its id), so Esc can put the keyboard back there. */
export const openedBy = new Map();

let popoverFrom = null;

/** The popover was just placed next to `from`: opened from the keyboard, the keyboard goes into it. */
export function popoverOpened(from) {
  popoverFrom = from;
  if (from?.matches(':focus-visible')) $('popover').querySelector('input, select, button')?.focus();
}

export function closePopover() {
  const pop = $('popover');
  if (pop.hidden) return;
  const back = pop.contains(document.activeElement) && popoverFrom?.isConnected ? popoverFrom : null;
  pop.hidden = true;
  popoverFrom = null;
  back?.focus(); // (Esc from inside it: back where you were)
}

let toastTimer;
export function toast(text) {
  const el = $('toast');
  el.textContent = text;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 4000);
}

export function hideToast() {
  clearTimeout(toastTimer);
  $('toast').hidden = true;
}

export function swatches(container, selected, onPick) {
  container.innerHTML = COLORS.map((c) =>
    `<button type="button" class="swatch" style="--c:${c}" data-color="${c}" aria-label="Color ${c}" aria-pressed="${c === selected}"></button>`,
  ).join('');
  container.onclick = (e) => {
    const b = e.target.closest('.swatch');
    if (!b) return;
    for (const s of container.querySelectorAll('.swatch')) s.setAttribute('aria-pressed', s === b);
    onPick(b.dataset.color);
  };
}

export function showInfo(title, text) {
  $('info-title').textContent = title;
  $('info-text').textContent = text;
  $('info-dialog').showModal();
}

/** "Fri 8:00 PM" (or "today 8:00 PM") for a hang. */
export function hangWhen(ts) {
  if (sameDay(ts, Date.now())) return `today ${formatTime(ts)}`;
  if (sameDay(ts, Date.now() + 86_400_000)) return `tomorrow ${formatTime(ts)}`;
  return `${new Date(ts).toLocaleDateString(undefined, { weekday: 'short' })} ${formatTime(ts)}`;
}

export const ago = (ts) => {
  const s = (Date.now() - ts) / 1000;
  return s < 60 ? 'just now' : s < 3600 ? `${Math.round(s / 60)} min ago` : s < 86400 ? `${Math.round(s / 3600)} h ago` : new Date(ts).toLocaleDateString();
};
export const nameOf = (uid) => (uid === 'admin' ? 'The owner (admin key)' : state.users.get(uid)?.name ?? 'Someone');
/** The same, in the middle of a sentence. */
export const nameIn = (uid) => (uid === 'admin' ? 'the owner (admin key)' : nameOf(uid));

export const kb = (n) => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1000))} KB`);

/** When the clipboard says no: the text in a box, selected, to copy yourself. */
export function copyByHand(title, text) {
  return confirmSheet({ title, text: 'Copy it from here:', input: { value: text, readonly: true }, ok: 'Done', cancel: null });
}
