// ── Encrypted Personal space ──────────────────────────────────────────────
// Notes are sealed in the browser with a key only your passcode opens
// (js/vault.js). Each device asks once, then keeps the key (it can't be
// read back out). The server and backups see "e1:…" and opaque pictures.

import { settings } from '../settings.js';
import { icon } from '../icons.js';
import { esc } from '../format.js';
import { createVault, openVault, rewrapVault, encryptNote, decryptNote, encryptBlob, decryptBlob, sealBatches, rememberKey, recallKey, forgetKey } from '../vault.js';
import { renderAll } from '../app.js';
import { $, MAX_SIDE_PX, conn, state } from './state.js';
import { toast } from './ui.js';
import { postBlob } from './uploads.js';
import { renderHeader } from './sidebar.js';
import { imageHtml, renderMessages } from './messages.js';

export async function syncVault(wrap) {
  state.vault.wrap = wrap;
  // Changed on another device (new passcode, or "start over" with a new key —
  // there's no telling which without the passcode): lock and ask again.
  if (!wrap || (state.vault.key && state.vault.keyCt !== wrap.ct)) {
    state.vault.key = null;
    state.vault.keyCt = null;
    forgetSealedImages();
    forgetKey(settings.uid);
  }
  if (wrap && !state.vault.key) {
    const saved = await recallKey(settings.uid);
    if (saved?.key && saved.ct === wrap.ct) Object.assign(state.vault, { key: saved.key, keyCt: wrap.ct });
    else if (saved) forgetKey(settings.uid);
  }
  state.notes.clear();
  renderHeader();
  renderMessages();
}

export async function sealNote(payload) {
  const body = await encryptNote(state.vault.key, payload);
  state.notes.set(body, payload); // we know what it says
  return body;
}

/** What an encrypted note says (null while decrypting or locked). */
export function openNote(body) {
  const cached = state.notes.get(body);
  if (cached && cached !== 'pending') return cached;
  if (!cached && state.vault.key) {
    state.notes.set(body, 'pending');
    decryptNote(state.vault.key, body).then(
      (note) => {
        state.notes.set(body, note);
        if (note.image) loadSealedImage(note.image);
        renderMessages();
      },
      () => {
        state.notes.set(body, { error: true });
        renderMessages();
      },
    );
  }
  return null;
}

/** Locked: the pictures opened so far go too (blob: copies, readable until they're let go). */
export function forgetSealedImages() {
  for (const url of state.sealedImages.values()) if (url.startsWith('blob:')) URL.revokeObjectURL(url);
  state.sealedImages.clear();
}

function loadSealedImage(img) {
  if (state.sealedImages.has(img.id) || !state.vault.key) return;
  state.sealedImages.set(img.id, 'pending');
  fetch(`/media/${img.id}`)
    .then((res) => (res.ok ? res.arrayBuffer() : Promise.reject(new Error(res.status))))
    .then((bytes) => decryptBlob(state.vault.key, bytes))
    .then((plain) => {
      state.sealedImages.set(img.id, URL.createObjectURL(new Blob([plain], { type: img.mime || 'image/webp' })));
      renderMessages();
    })
    .catch(() => state.sealedImages.set(img.id, 'error'));
}

export function sealedImageHtml(img, msgId) {
  const url = state.sealedImages.get(img.id);
  if (!url) loadSealedImage(img);
  if (!url || url === 'pending' || url === 'error') {
    const scale = Math.min(1, MAX_SIDE_PX / img.width, MAX_SIDE_PX / img.height);
    return `<div class="msg-image sealed-image" style="width:${Math.round(img.width * scale)}px;aspect-ratio:${img.width}/${img.height}">${url === 'error' ? '🔒 Can’t open' : '🔒'}</div>`;
  }
  return imageHtml(img, msgId).replaceAll(`/media/${esc(img.id)}`, esc(url));
}

export function personalStartHtml() {
  const head = `<div class="channel-start-icon">${icon('lock')}</div><h2>Your Personal space</h2><p>Only you can see this. Send yourself links, pictures and notes to grab on your other devices.</p>`;
  if (state.vault.key) {
    return `<div class="channel-start">${head}<p class="personal-note">🔒 <b>End-to-end encrypted</b> with your passcode: this device and your others that know it can read these notes; Yap's server and its backups only ever see scrambled text. Forget the passcode and they're gone for good. (Someone holding your unlocked device can still read it.)</p></div>`;
  }
  if (state.vault.wrap) {
    return `<div class="channel-start">${head}<p class="personal-note">🔒 <b>Locked on this device.</b> <button class="btn small" data-action="vault-unlock">Unlock</button></p></div>`;
  }
  return `<div class="channel-start">${head}<p class="personal-warning">⚠️ Not encrypted yet: Yap's server (and its backups) can read it like any other message. <button class="btn small" data-action="vault-setup">🔒 Encrypt it with a passcode</button></p></div>`;
}

/**
 * Turning encryption on also seals the notes you wrote before: each one
 * (and its picture) is encrypted here, and the server swaps out the plain copy.
 */
async function sealOldNotes() {
  const key = state.vault.key;
  if (!key || !conn?.open) return;
  const notes = await new Promise((resolve) => {
    state.plainWaiter = resolve;
    conn.send({ t: 'personal.plain' });
    setTimeout(() => resolve([]), 10_000);
  });
  state.plainWaiter = null;
  if (!notes.length) return;
  toast(`Encrypting your ${notes.length} earlier ${notes.length === 1 ? 'note' : 'notes'}…`);
  const sealed = [];
  let failed = 0;
  for (const n of notes) {
    try {
      const payload = { text: n.body };
      if (n.gif) payload.gif = n.gif;
      if (n.image) {
        const res = await fetch(`/media/${n.image.id}`);
        if (!res.ok) throw new Error(`picture ${res.status}`);
        const plain = new Uint8Array(await res.arrayBuffer());
        const { id } = await postBlob(await encryptBlob(key, plain));
        payload.image = { id, width: n.image.width, height: n.image.height, mime: res.headers.get('Content-Type') || 'image/webp' };
      }
      const body = await encryptNote(key, payload);
      state.notes.set(body, payload);
      sealed.push({ id: n.id, body });
    } catch {
      failed++;
    }
  }
  // Batches by size, not count: one frame is at most 384 KB, and long notes
  // add up (25 at a time could pass that and be refused whole). A note only
  // counts as encrypted once the server says so (`edited {sealed: true}`).
  const { batches, tooBig } = sealBatches(sealed);
  failed += tooBig.length;
  const confirmed = waitSealed(batches.flat().map((n) => n.id), 20_000);
  for (const batch of batches) conn?.send({ t: 'personal.seal', notes: batch });
  failed += batches.flat().length - (await confirmed);
  toast(failed ? `Encrypted your earlier notes, but ${failed === 1 ? '1 couldn’t be' : `${failed} couldn’t be`} (try turning it on again later).` : 'Your earlier notes are encrypted too.');
}

/** Resolves with how many of `ids` the server confirmed sealed within `ms`. */
function waitSealed(ids, ms) {
  return new Promise((resolve) => {
    const left = new Set(ids);
    let done = 0;
    const finish = () => {
      clearTimeout(timer);
      state.sealWaiter = null;
      resolve(done);
    };
    const timer = setTimeout(finish, ms);
    state.sealWaiter = (id) => {
      if (!left.delete(id)) return;
      done++;
      if (!left.size) finish();
    };
    if (!left.size) finish();
  });
}

export function askUnlock() {
  if (!state.vault.wrap) return showVaultDialog('setup');
  showVaultDialog('unlock');
}

/** setup · unlock · change · reset (forgot: new key, old notes stay locked) */
export function showVaultDialog(mode) {
  const form = $('vault-form');
  form.reset();
  form.dataset.mode = mode;
  const texts = {
    setup: ['Encrypt your Personal space', 'Pick a passcode only you know: a short phrase works best (like "purple tiger river 42"), at least 8 characters. Yap can\'t see it and can\'t recover it — forget it and your encrypted notes are gone. Each of your devices asks for it once.', 'Encrypt'],
    unlock: ['Unlock your Personal space', 'Enter your Personal space passcode. This device remembers it until you lock it.', 'Unlock'],
    change: ['Change your passcode', 'Your notes stay as they are; only the passcode that opens them changes. Your other devices will ask for the new one.', 'Change'],
    reset: ['Start over?', 'Without the passcode, nobody can open your old encrypted notes — not you, not Yap. Starting over makes a new passcode for new notes; the old ones stay locked.', 'Start over'],
  };
  const [title, text, ok] = texts[mode];
  $('vault-title').textContent = title;
  $('vault-text').textContent = text;
  $('vault-ok').textContent = ok;
  $('vault-old-field').hidden = mode !== 'change';
  $('vault-again-field').hidden = mode === 'unlock';
  $('vault-pass-label').textContent = mode === 'change' ? 'New passcode' : 'Passcode';
  form.pass.autocomplete = mode === 'unlock' ? 'current-password' : 'new-password';
  form.username.value = `${settings.name} · Yap Personal space`;
  $('vault-forgot').hidden = mode !== 'unlock';
  $('vault-error').hidden = true;
  if (!$('vault-dialog').open) $('vault-dialog').showModal();
  (mode === 'change' ? form.old : form.pass).focus();
}

export function bindVaultDialog() {
  const form = $('vault-form');
  form.addEventListener('submit', async (e) => {
    if (e.submitter?.value === 'cancel') return; // (Cancel closes it; it used to complain about the passcode and stay)
    e.preventDefault();
    const mode = form.dataset.mode;
    const fail = (text) => {
      $('vault-error').textContent = text;
      $('vault-error').hidden = false;
    };
    const pass = form.pass.value;
    if (mode !== 'unlock') {
      if (pass.length < 8) return fail('Use at least 8 characters (a short phrase is easiest to remember).');
      if (/^\d+$/.test(pass) && pass.length < 12) return fail('Numbers alone are easy to guess. Add some words.');
      if (pass !== form.again.value) return fail('Those two don’t match.');
    }
    $('vault-ok').disabled = true;
    $('vault-ok').textContent = 'Working…';
    try {
      if (mode === 'unlock') {
        const key = await openVault(state.vault.wrap, pass);
        Object.assign(state.vault, { key, keyCt: state.vault.wrap.ct });
        await rememberKey(settings.uid, key, state.vault.wrap.ct);
      } else if (mode === 'change') {
        const wrap = await rewrapVault(state.vault.wrap, form.old.value, pass);
        conn?.send({ t: 'vault.set', wrap });
        Object.assign(state.vault, { wrap, keyCt: wrap.ct });
        if (state.vault.key) await rememberKey(settings.uid, state.vault.key, wrap.ct);
        toast('Passcode changed.');
      } else {
        const { wrap, key } = await createVault(pass);
        conn?.send({ t: 'vault.set', wrap });
        state.vault = { wrap, key, keyCt: wrap.ct };
        await rememberKey(settings.uid, key, wrap.ct);
        toast('Your Personal space is encrypted. New notes are sealed with your passcode.');
        sealOldNotes();
      }
      $('vault-dialog').close();
      state.notes.clear();
      renderAll();
    } catch (err) {
      fail(err.name === 'WrongPasscode' ? 'That passcode doesn’t open it.' : `Something went wrong: ${err.message}`);
    } finally {
      $('vault-ok').disabled = false;
      $('vault-ok').textContent = { setup: 'Encrypt', unlock: 'Unlock', change: 'Change', reset: 'Start over' }[mode];
    }
  });
}
