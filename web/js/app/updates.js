// ── Updates without a refresh ─────────────────────────────────────────────
// After a deploy the server's `ready` carries the new build (PROTOCOL.md ›
// Updates). CSS-only changes swap in place; anything else reloads the page
// once nobody would notice: not in voice, nothing typed or still sending, no
// dialog open, and the tab hidden or idle. Until then a bar offers "Restart now".

import { settings } from '../settings.js';
import { BUILD } from '../build.js';
import { $, state } from './state.js';
import { drawBoard, watchParty } from './places.js';

const IDLE_RELOAD_MS = 5 * 60_000;
let liveStyle = BUILD.style;
let lastActivity = Date.now();

export function onBuild(build) {
  if (!build?.app) return;
  if (build.app !== BUILD.app) {
    state.updateReady = true;
    renderUpdateBar();
    return maybeUpdate();
  }
  if (build.style !== liveStyle) {
    liveStyle = build.style;
    for (const link of document.querySelectorAll('link[rel=stylesheet]')) link.href = `${link.getAttribute('href').split('?')[0]}?v=${build.style}`;
  }
}

function renderUpdateBar() {
  const bar = $('update-bar');
  bar.hidden = !state.updateReady;
  if (!state.updateReady) return;
  bar.innerHTML = `<span>Yap has been updated. ${state.voiceWanted ? 'It restarts by itself after you leave voice.' : 'It restarts by itself when you step away.'}</span>
    <span class="spacer"></span><button class="btn small" data-action="update-now">Restart now</button>`;
}

function quietMoment() {
  return !state.voiceWanted && !$('input').value.trim() && !state.outbox.length && !document.querySelector('dialog[open]') && !drawBoard?.drawing && !watchParty.busy()
    && (document.visibilityState === 'hidden' || Date.now() - lastActivity > IDLE_RELOAD_MS);
}

export function maybeUpdate() {
  if (state.updateReady && quietMoment()) reloadForUpdate();
  else if (state.updateReady) renderUpdateBar();
}

export function reloadForUpdate() {
  try {
    sessionStorage.setItem('yap.resume', JSON.stringify({ channel: settings.channel, draft: $('input').value, view: state.view }));
  } catch {}
  location.reload();
}

export function restoreAfterUpdate() {
  try {
    const resume = JSON.parse(sessionStorage.getItem('yap.resume') || 'null');
    sessionStorage.removeItem('yap.resume');
    if (!resume) return;
    if (resume.channel === settings.channel && resume.draft) $('input').value = resume.draft;
    if (resume.view && resume.view !== 'chat') state.restoreView = resume.view;
  } catch {}
}

/** Notice activity, and keep looking for a quiet moment (app.js calls this once, before boot). */
export function bindUpdates() {
  for (const type of ['pointerdown', 'keydown', 'wheel']) addEventListener(type, () => (lastActivity = Date.now()), { passive: true, capture: true });
  document.addEventListener('visibilitychange', maybeUpdate);
  setInterval(maybeUpdate, 60_000);
}
