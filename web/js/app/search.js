// Search, pins and the quick switcher (Ctrl/⌘+K).

import { settings } from '../settings.js';
import { esc, plainHtml, clip } from '../format.js';
import { QuickSwitcher } from '../switcher.js';
import { decryptNote, isEncryptedNote } from '../vault.js';
import { $, conn, noHover, state } from './state.js';
import { VIEWS, setView } from './places.js';
import { joinVoice } from './voice-ui.js';
import { isUnread, loadUntil, openChannel } from './chat.js';
import { openPeople, renderMembers } from './people.js';
import { openCatchup, openShelf } from './extras.js';
import { openSettings } from './settings-ui.js';

let switcher = null;

// ── Search and pins ───────────────────────────────────────────────────────

export function openSearch() {
  const channel = state.channels.get(settings.channel);
  $('search-here').textContent = channel ? `In ${channel.name}` : 'This channel';
  if (!$('search-dialog').open) $('search-dialog').showModal();
  // Opening it shows how to search (it used to open blank until you'd searched once).
  if (!$('search-input').value.trim()) renderResultList($('search-results'), [], {});
  $('search-input').focus();
  $('search-input').select();
}

export function runSearch() {
  const q = $('search-input').value.trim();
  const form = $('search-form');
  const here = form.scope.value === 'here';
  state.search = { q, results: [], pending: Boolean(q), here };
  if (!q) return renderResultList($('search-results'), [], {});
  $('search-results').innerHTML = '<li class="muted">Searching…</li>';
  conn?.send({ t: 'search', q, ...(here ? { channelId: settings.channel } : {}) });
}

/** Server results, plus (when unlocked) your encrypted notes searched right here. */
export async function onSearchResults(msg) {
  if (msg.q !== state.search.q) return;
  let results = msg.messages;
  const personal = `p_${settings.uid}`;
  if (state.vault.key && (!state.search.here || settings.channel === personal)) {
    const words = msg.q.toLowerCase().split(/\s+/).filter((w) => w && !/^(from|has):/.test(w));
    if (words.length) {
      if (!state.history.has(personal)) {
        state.history.set(personal, { messages: [], hasMore: true, loading: true, mode: 'latest' });
        const first = new Promise((r) => (state.history.get(personal).waiters = [r]));
        conn?.send({ t: 'history', channelId: personal, limit: 50 });
        await first;
      }
      await loadUntil(personal, '\u0000'); // every page (up to the cap)
      const notes = [];
      for (const m of state.history.get(personal)?.messages ?? []) {
        if (!isEncryptedNote(m.body)) continue;
        let note = state.notes.get(m.body);
        if (!note || note === 'pending') {
          note = await decryptNote(state.vault.key, m.body).catch(() => null);
          if (note) state.notes.set(m.body, note);
        }
        const text = (note?.text ?? '').toLowerCase();
        if (note && words.every((w) => text.includes(w))) notes.push(m);
      }
      results = [...results, ...notes].sort((a, b) => b.ts - a.ts).slice(0, 60);
    }
  }
  state.search.results = results;
  state.search.pending = false;
  renderResultList($('search-results'), results, { q: msg.q, here: state.search.here });
}

/** One line per message: where · who · when, then the text with matches marked. */
export function renderResultList(list, messages, { q = '', pins = false, here = false }) {
  const words = q.toLowerCase().split(/\s+/).filter((w) => w && !/^(from|has):/.test(w)).map((w) => w.replace(/^"|"$/g, ''));
  const safe = words.filter((w) => !/^(?:amp|lt|gt|quot|#39|#?\d*;?)$/.test(w)); // never split an HTML entity
  const mark = (html) => safe.reduce((out, w) => out.replace(new RegExp(`(${esc(w).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi'), '<mark>$1</mark>'), html);
  list.innerHTML = messages.map((m) => {
    const who = state.users.get(m.uid);
    const where = state.channels.get(m.channelId)?.name ?? 'Personal space';
    let text = m.body;
    if (isEncryptedNote(text)) text = state.notes.get(text)?.text ?? '🔒 Encrypted note';
    // Around the first match, without the formatting marks (custom emojis drawn small).
    const snippet = plainHtml(text, {
      cut: (plain) => {
        const lower = plain.toLowerCase();
        const at = Math.max(0, (words.map((w) => lower.indexOf(w)).filter((i) => i >= 0).sort((a, b) => a - b)[0] ?? 0) - 50);
        return (at ? '…' : '') + clip(plain.slice(at), 220);
      },
      mark,
    });
    const extra = m.image ? '📷 ' : m.gif ? 'GIF ' : '';
    return `
      <li class="result">
        <button type="button" class="result-open" data-action="open-result" data-channel="${esc(m.channelId)}" data-id="${esc(m.id)}">
          <span class="result-meta"><b style="color:${esc(who?.color ?? 'inherit')}">${esc(who?.name ?? 'Someone')}</b> · ${esc(where)} · ${esc(new Date(m.ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }))}</span>
          <span class="result-text">${extra}${snippet || '<span class="muted">(no text)</span>'}</span>
        </button>
        ${pins ? `<button type="button" class="btn small" data-action="pin-message" data-id="${esc(m.id)}">Unpin</button>` : ''}
      </li>`;
  }).join('') || `<li class="muted">${pins ? `Nothing pinned here yet. ${noHover.matches ? 'Tap a message, then the pin.' : 'Hover a message and click the pin.'}`
    : q ? (here ? `Nothing for “${esc(q)}” in ${esc(state.channels.get(settings.channel)?.name ?? 'this channel')}. <button type="button" class="link-btn" data-action="search-everywhere">Search everywhere</button>` : `Nothing for “${esc(q)}” anywhere.`)
      : 'Type a few words. You can also look for <b>from:</b>Name, <b>has:image</b>, <b>has:gif</b> or <b>has:link</b>.'}</li>`;
  // Unpin from the list works on messages that aren't loaded too.
  if (pins) for (const m of messages) state.pinnedLoose.set(m.id, m);
}

export function bindSearch() {
  let timer = null;
  $('search-form').addEventListener('submit', (e) => {
    e.preventDefault();
    runSearch();
  });
  $('search-input').addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(runSearch, 350);
  });
  // Only the scope radios: a text field's "change" fires on blur, which would
  // redraw the results right under the click that caused it.
  $('search-form').addEventListener('change', (e) => e.target.name === 'scope' && runSearch());
  document.addEventListener('keydown', (e) => {
    if (!(e.metaKey || e.ctrlKey) || !state.sid) return;
    const key = e.key.toLowerCase();
    if (key === 'k' && !e.shiftKey) {
      e.preventDefault();
      openSwitcher();
    } else if (key === 'f' && e.shiftKey) {
      e.preventDefault();
      openSearch();
    }
  });
}

export function openSwitcher() {
  switcher ??= new QuickSwitcher($('switcher-dialog'), {
    items: switcherItems,
    search: (q) => {
      openSearch();
      if (q) {
        $('search-input').value = q;
        runSearch();
      }
    },
  });
  for (const d of document.querySelectorAll('dialog[open]')) if (d.id !== 'switcher-dialog') d.close();
  switcher.open();
}

function switcherItems() {
  const channels = [...state.channels.values()].sort((a, b) => a.position - b.position);
  const places = [
    ...Object.entries(VIEWS).filter(([v]) => v !== 'screen' && (v !== 'board' || (state.links ?? []).some((l) => l.view === 'board'))).map(([v, info]) => ({ label: info.title, icon: info.icon, kind: 'place', run: () => setView(v) })),
    ...(state.links ?? []).filter((l) => !l.view && /^https:\/\//.test(l.url)).map((l) => ({ label: l.name, sub: l.url, icon: 'external', kind: 'link', run: () => open(l.url, '_blank', 'noopener') })),
  ];
  return [
    ...channels.filter((c) => c.kind === 'text' || c.kind === 'personal').map((c) => ({ label: c.kind === 'personal' ? 'Personal space' : c.name, icon: c.kind === 'personal' ? 'lock' : 'chat', kind: isUnread(c) ? 'unread' : 'channel', run: () => openChannel(c.id) })),
    ...channels.filter((c) => c.kind === 'voice').map((c) => ({ label: c.name, icon: 'volume', kind: state.voiceChannel === c.id ? 'in voice' : 'voice', run: () => joinVoice(c.id) })),
    ...places,
    ...[...state.users.values()].filter((u) => u.id !== settings.uid).map((u) => ({ label: u.name, sub: u.away?.text, icon: 'users', kind: 'person', run: () => {
      openPeople();
      state.personOpen = u.id;
      renderMembers();
    } })),
    { label: 'Settings', icon: 'settings', kind: 'action', run: () => openSettings() },
    { label: 'Pinned messages', icon: 'pin', kind: 'action', run: () => document.querySelector('[data-action="pins"]').click() },
    { label: 'Pictures & GIFs here', icon: 'images', kind: 'action', run: openShelf },
    { label: 'Catch up', icon: 'sparkles', kind: 'action', run: openCatchup },
  ];
}
