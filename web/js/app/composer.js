// Writing: the message box, editing, replies, the GIF, emoji and reaction
// pickers, a picture or GIF waiting to be sent, and deleting.

import { settings, save } from '../settings.js';
import { icon } from '../icons.js';
import { esc, plainHtml, clip } from '../format.js';
import { placeNear } from '../place.js';
import { GifPicker, gifHtml, watchGifs, forgetGifs, reportShare } from '../gifs.js';
import { EmojiPicker, EmojiSuggest, encodeEmojis, plainEmojis, uploadEmoji } from '../emoji.js';
import { confirmSheet } from '../sheet.js';
import { encryptBlob, isEncryptedNote } from '../vault.js';
import { $, MAX_BODY, MAX_PICTURE, TYPING_SEND_EVERY, coarsePointer, conn, emojiList, emojisByName, findMessage, isPersonalChannel, personalId, sealedHere, state } from './state.js';
import { autosize, avatar, hideToast, insertAtCaret, kb, openedBy, toast } from './ui.js';
import { postBlob, postImage } from './uploads.js';
import { nodeCache, renderMessages } from './messages.js';
import { askUnlock, sealNote } from './personal.js';
import { queueSend } from './chat.js';

let lastTypingSent = 0;

export let gifPicker = null;
export let emojiPicker = null;
let emojiSuggest = null;

/** The message box: pictures attached, pasted or dropped, suggestions, typing, keys, Send (bindUi calls this). */
export function bindComposer() {
  const input = $('input');
  $('image-file').addEventListener('change', (e) => {
    const file = e.target.files[0];
    e.target.value = '';
    stagePicture(file);
  });
  input.addEventListener('paste', (e) => {
    const file = [...(e.clipboardData?.files ?? [])].find((f) => f.type.startsWith('image/'));
    if (!file) return;
    e.preventDefault();
    stagePicture(file);
  });
  const chat = document.querySelector('.chat');
  chat.addEventListener('dragover', (e) => {
    if ([...(e.dataTransfer?.items ?? [])].some((i) => i.type.startsWith('image/'))) e.preventDefault();
  });
  chat.addEventListener('drop', (e) => {
    const file = [...(e.dataTransfer?.files ?? [])].find((f) => f.type.startsWith('image/'));
    if (!file || state.view !== 'chat') return;
    e.preventDefault();
    stagePicture(file);
  });
  emojiSuggest = new EmojiSuggest($('emoji-suggest'), input, emojiList, () =>
    [...state.users.values()].filter((u) => u.id !== settings.uid).sort((a, b) => a.name.localeCompare(b.name)), (user) => avatar(user, 'sm'));
  input.addEventListener('blur', () => setTimeout(() => emojiSuggest.close(), 100));
  input.addEventListener('click', () => emojiSuggest.update());
  input.addEventListener('input', () => {
    autosize();
    emojiSuggest.update();
    if (input.value.trim() && Date.now() - lastTypingSent > TYPING_SEND_EVERY) {
      lastTypingSent = Date.now();
      conn?.send({ t: 'typing', channelId: settings.channel });
    }
  });
  input.addEventListener('keydown', (e) => {
    if (emojiSuggest.handleKey(e)) return;
    if (e.key === 'Escape' && state.editing) {
      e.preventDefault();
      return cancelEdit();
    }
    if (e.key === 'Escape' && state.staged) {
      e.preventDefault();
      return clearStaged();
    }
    if (e.key === 'Escape' && state.replying) {
      e.preventDefault();
      return cancelReply();
    }
    if (e.key === 'ArrowUp' && !input.value && !state.editing) {
      const mine = (state.history.get(settings.channel)?.messages ?? []).filter((m) => m.uid === settings.uid && m.body);
      if (mine.length) {
        e.preventDefault();
        return startEdit(mine[mine.length - 1].id);
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !coarsePointer) {
      e.preventDefault();
      sendMessage();
    }
  });
  $('composer').addEventListener('submit', (e) => {
    e.preventDefault();
    sendMessage();
    if (coarsePointer) input.focus();
  });
}

function sendMessage() {
  const input = $('input');
  const body = encodeEmojis(input.value.replace(/\s+$/, ''), emojisByName());
  if (body.length > MAX_BODY && !sealedHere()) return toast(`That's ${body.length.toLocaleString()} characters — the limit is ${MAX_BODY.toLocaleString()}. Split it into two?`);
  if (state.editing) return saveEdit(body);
  if (sealedHere() && !state.vault.key) return askUnlock();
  if (state.staged) return sendStaged(body);
  if (!body.trim()) return;
  emojiSuggest?.close();
  const nonce = crypto.randomUUID();
  const channelId = settings.channel;
  const draft = plainEmojis(body); // back in the box if the server turns it away
  // /poll Question? | option | option …  (the server rolls /roll and /flip itself)
  const poll = /^\/poll\s+([\s\S]+)$/i.exec(body.trim());
  if (poll) {
    const [question, ...options] = poll[1].split('|').map((x) => x.trim()).filter(Boolean);
    if (sealedHere() || isPersonalChannel(channelId)) return toast('Polls are for channels.');
    if (!question || options.length < 2 || options.length > 10) return toast('Polls look like: /poll Pizza or tacos? | Pizza | Tacos (2–10 options).');
    input.value = '';
    autosize();
    queueSend({ t: 'send', channelId, body: question, poll: { options }, nonce, replyTo: takeReply(channelId) }, { draft });
    return;
  }
  const replyTo = takeReply(channelId);
  input.value = '';
  autosize();
  lastTypingSent = 0;
  if (sealedHere()) {
    sealNote({ text: body }).then((sealed) => {
      queueSend({ t: 'send', channelId, body: sealed, nonce }, { copy: { id: nonce, channelId, uid: settings.uid, body: sealed, ts: Date.now(), pending: true }, draft });
      renderMessages({ keep: 'bottom' });
    });
  } else {
    queueSend({ t: 'send', channelId, body, nonce, replyTo }, { copy: { id: nonce, channelId, uid: settings.uid, body, ts: Date.now(), pending: true, replyTo, reply: replyTo ? replyRef(replyTo) : undefined }, draft });
  }
  renderMessages({ keep: 'bottom' });
}

// ── Editing your own messages ─────────────────────────────────────────────
// Edit (hover button) or Up-arrow in an empty composer loads the message into
// the composer; Enter saves, Esc puts back whatever you were typing before.

export function startEdit(id) {
  const m = findMessage(id);
  if (!m || m.uid !== settings.uid || m.pending) return;
  const input = $('input');
  let text = m.body;
  let note = null;
  if (isEncryptedNote(m.body)) {
    note = state.notes.get(m.body);
    if (!note || note === 'pending' || note.error) return toast('Unlock your Personal space to edit that.');
    text = note.text ?? '';
  }
  state.editing = { id: m.id, channelId: m.channelId, note, draft: state.editing ? state.editing.draft : input.value };
  input.value = plainEmojis(text);
  autosize();
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  renderEditBar();
}

export function cancelEdit() {
  if (!state.editing) return;
  $('input').value = state.editing.draft ?? '';
  state.editing = null;
  autosize();
  renderEditBar();
}

function saveEdit(body) {
  const { id, note } = state.editing;
  const m = findMessage(id);
  if (!body.trim() && !m?.gif && !m?.image && !note?.gif && !note?.image) {
    cancelEdit();
    return askDelete(id);
  }
  if (m && note) {
    // Encrypted note: re-seal the new text with whatever GIF/picture it had.
    if (body !== note.text) {
      sealNote({ ...note, text: body }).then((sealed) => {
        conn?.send({ t: 'edit', id, body: sealed });
        Object.assign(m, { body: sealed, editedAt: Date.now() });
        renderMessages();
      });
    }
    cancelEdit();
    return;
  }
  if (m && body !== m.body && m.channelId === personalId() && state.vault.wrap && body) {
    // A plain note (e.g. a saved bookmark) in an encrypted space: seal what you add.
    sealNote({ text: body }).then((sealed) => {
      conn?.send({ t: 'edit', id, body: sealed });
      Object.assign(m, { body: sealed, editedAt: Date.now() });
      renderMessages();
    });
    cancelEdit();
    return;
  }
  if (m && body !== m.body) {
    conn?.send({ t: 'edit', id, body });
    Object.assign(m, { body, editedAt: Date.now() });
  }
  cancelEdit();
  renderMessages();
}

function renderEditBar() {
  $('edit-bar').hidden = !state.editing;
  $('composer').classList.toggle('editing', !!state.editing);
  markEditing();
  // After the edit bar has changed the chat's height (which keeps you at the end if you were there);
  // only if it isn't fully in view already, and then in the middle, not flush under the header.
  if (state.editing) requestAnimationFrame(() => requestAnimationFrame(() => {
    const row = nodeCache.get(`msg:${state.editing?.id}`)?.el;
    const box = $('messages').getBoundingClientRect();
    const r = row?.getBoundingClientRect();
    if (r && (r.top < box.top || r.bottom > box.bottom)) row.scrollIntoView({ block: 'center' });
  }));
}

/** The message you're editing is outlined, so you can see which one it is. */
export function markEditing() {
  for (const row of document.querySelectorAll('.msg.editing')) row.classList.remove('editing');
  if (state.editing) nodeCache.get(`msg:${state.editing.id}`)?.el.classList.add('editing');
}


export function toggleGifs() {
  gifPicker ??= new GifPicker($('gif-picker'), stageGif);
  emojiPicker?.close();
  if (gifPicker.isOpen) gifPicker.close();
  else gifPicker.open();
}

export function toggleEmojis() {
  emojiPicker ??= new EmojiPicker($('emoji-picker'), {
    emojis: emojiList,
    onPick: ({ char, emoji }, keepOpen) => {
      insertAtCaret($('input'), char ?? `:${emoji.name}: `);
      if (!keepOpen) emojiPicker.close();
    },
    onDelete: (emoji) => conn?.send({ t: 'emoji.delete', id: emoji.id }),
    onRename: (emoji, name) => conn?.send({ t: 'emoji.rename', id: emoji.id, name }),
    onUpload: async (blob, name) => {
      if (!state.sid) throw new Error('Not connected yet.');
      const emoji = await uploadEmoji(blob, name, state.sid, state.uploadToken);
      toast(`:${emoji.name}: added.`);
    },
    toast,
  });
  gifPicker?.close();
  if (emojiPicker.isOpen) emojiPicker.close();
  else emojiPicker.open();
}

// ── Replies, deletes, long messages ───────────────────────────────────────

/** What a reply quote shows, from a loaded message (the server fills it in for real). */
function replyRef(id) {
  const m = findMessage(id);
  if (!m) return undefined;
  return { id, uid: m.uid, body: m.body.slice(0, 200), ...(m.gif ? { gif: true } : {}), ...(m.image ? { image: true } : {}) };
}

export function startReply(id) {
  const m = findMessage(id);
  if (!m || m.pending || isPersonalChannel(m.channelId)) return;
  if (state.editing) cancelEdit();
  state.replying = { id: m.id, uid: m.uid, channelId: m.channelId };
  renderReplyBar();
  $('input').focus();
}

export function cancelReply() {
  state.replying = null;
  renderReplyBar();
}

/** The reply to attach to a message being sent now (and clear it). */
function takeReply(channelId) {
  const r = state.replying;
  if (!r || r.channelId !== channelId) return undefined;
  cancelReply();
  return r.id;
}

/**
 * The server turned a message away: its text goes back in the message box
 * (and what it answered, back in the reply bar), so nothing is lost — but
 * only into an empty box, never over something new (in another channel: into
 * that channel's draft).
 */
export function putBack({ channelId, text, replyTo }) {
  const input = $('input');
  if (!text) return;
  // You've moved on to another channel: it waits in its own channel's box (unless something new is there).
  if (settings.channel !== channelId) {
    if (state.channels.has(channelId) && !state.drafts.get(channelId)?.trim()) state.drafts.set(channelId, text);
    return;
  }
  if (input.value.trim() || state.editing || state.staged) return;
  input.value = text;
  autosize();
  if (replyTo && !state.replying) startReply(replyTo);
}

function renderReplyBar() {
  const bar = $('reply-bar');
  const r = state.replying;
  bar.hidden = !r;
  if (!r) return;
  const who = r.uid === settings.uid ? 'yourself' : state.users.get(r.uid)?.name ?? 'someone';
  // What you're answering, in a word or two (like the quote it will become).
  const m = findMessage(r.id);
  const snippet = !m ? '' : isEncryptedNote(m.body) ? '🔒 Encrypted note' : plainHtml(m.body, { cut: (t) => clip(t, 120) }) || (m.image ? '📷 A picture' : m.gif ? 'GIF' : '');
  bar.innerHTML = `${icon('reply')}<span class="reply-to">Replying to <b>${esc(who)}</b>${snippet ? `<span class="reply-snippet">${snippet}</span>` : ''}</span><span class="spacer"></span><small class="muted key-hint">Esc to cancel</small><button type="button" class="link-btn" data-action="cancel-reply">Cancel</button>`;
}

/** Bookmark a message in your Personal space (a reference, not a copy). */
export function saveToPersonal(id) {
  const m = findMessage(id);
  if (!m || m.pending) return;
  queueSend({ t: 'send', channelId: personalId(), body: '', replyTo: id });
  if (state.vault.wrap && !settings.savedNoteSeen) {
    save({ savedNoteSeen: true });
    return toast('Saved to your Personal space. Yap’s server can see which message you saved (never your notes).');
  }
  toast('Saved to your Personal space.');
}

export function askDelete(id) {
  const skip = coarsePointer ? '' : ' (Shift-click the trash can to skip this question.)';
  return confirmSheet({ title: 'Delete this message?', text: `It’s erased for good, for everyone.${skip}`, ok: 'Delete', danger: true })
    .then((yes) => yes && deleteMessage(id));
}

/** Deleting erases it on the server; encrypted notes also name their pictures. */
export function deleteMessage(id) {
  const m = findMessage(id);
  const note = m && isEncryptedNote(m.body) ? state.notes.get(m.body) : null;
  const blobs = note?.image?.id ? [note.image.id] : undefined;
  if (state.replying?.id === id) cancelReply();
  conn?.send({ t: 'delete', id, blobs });
}

// ── Pictures in messages ──────────────────────────────────────────────────
// Attach, paste or drop. By default pictures are compressed (WebP, ≤ 1600 px,
// ~150–300 KB for a phone photo) to save space; "Full res" in the preview
// sends the original bytes when they fit the 1.9 MB cap (else the sharpest
// WebP that does). Small GIFs always stay as they are (still animated).

const COMPRESSED_STEPS = [[1600, 0.78], [1280, 0.72], [1024, 0.66]];
const FULL_RES_STEPS = [[Infinity, 0.92], [3072, 0.9], [2048, 0.88], [1600, 0.82]];

async function preparePicture(file, { full = false } = {}) {
  const bmp = await createImageBitmap(file);
  // (Decoded, a phone photo is ~50 MB: let it go as soon as we're done, not whenever the browser gets round to it.)
  try {
    const dims = { width: bmp.width, height: bmp.height };
    if (file.type === 'image/gif' && file.size <= MAX_PICTURE) return { blob: file, ...dims };
    // Full res: the original, untouched, if it fits.
    if (full && /^image\/(jpeg|png|webp)$/.test(file.type) && file.size <= MAX_PICTURE) return { blob: file, ...dims };
    for (const [side, quality] of full ? FULL_RES_STEPS : COMPRESSED_STEPS) {
      const scale = Math.min(1, side / Math.max(bmp.width, bmp.height));
      const canvas = Object.assign(document.createElement('canvas'), { width: Math.round(bmp.width * scale), height: Math.round(bmp.height * scale) });
      const ctx = canvas.getContext('2d');
      ctx.imageSmoothingQuality = 'high';
      ctx.drawImage(bmp, 0, 0, canvas.width, canvas.height);
      let blob = await new Promise((r) => canvas.toBlob(r, 'image/webp', quality));
      if (blob?.type !== 'image/webp') blob = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', quality));
      if (blob && blob.size <= MAX_PICTURE) return { blob, width: canvas.width, height: canvas.height };
    }
    throw new Error('That picture is too big.');
  } finally {
    bmp.close();
  }
}

// ── Preview before sending ────────────────────────────────────────────────
// A GIF you pick or a picture you attach/paste/drop waits in a tray above
// the message box: see it, add a caption, then Send (or × / Esc).

function stageGif(gif, query) {
  clearStaged();
  state.staged = { kind: 'gif', gif, query };
  gifPicker?.close();
  renderStaged();
  if (!coarsePointer) $('input').focus();
}

function stagePicture(file) {
  if (!file || !file.type.startsWith('image/')) return;
  if (state.editing) cancelEdit();
  clearStaged();
  const staged = { kind: 'image', file, url: URL.createObjectURL(file), full: false };
  shrinkStaged(staged);
  state.staged = staged;
  renderStaged();
  if (!coarsePointer) $('input').focus();
}

/** Prepare the staged picture (compressed or full res) while you type; show its size when done. */
export function shrinkStaged(staged) {
  staged.size = null;
  staged.ready = preparePicture(staged.file, { full: staged.full }).catch((err) => err);
  staged.ready.then((r) => {
    if (state.staged !== staged || r instanceof Error) return;
    staged.size = r.blob.size;
    renderStaged();
  });
}

export function clearStaged() {
  if (state.staged?.url) URL.revokeObjectURL(state.staged.url);
  state.staged = null;
  renderStaged();
}

export function renderStaged() {
  const el = $('staged');
  const s = state.staged;
  el.hidden = !s;
  $('composer').classList.toggle('has-staged', Boolean(s));
  if (!s) {
    forgetGifs(el);
    el.innerHTML = '';
    return;
  }
  const preview = s.kind === 'gif' ? gifHtml(s.gif) : `<img src="${esc(s.url)}" alt="Picture to send">`;
  const picture = s.kind === 'image'
    ? `<label class="full-res" title="Send the original picture instead of a smaller copy (uses more space)"><input type="checkbox" data-action="toggle-full-res" ${s.full ? 'checked' : ''}> Full res</label>`
    : '';
  const size = s.kind === 'image' ? (s.size ? ` · ${kb(s.size)}${s.full ? '' : ', compressed'}` : ' · shrinking…') : '';
  el.innerHTML = `
    <div class="staged-preview">${preview}</div>
    <div class="staged-text"><b>${s.kind === 'gif' ? 'GIF' : 'Picture'}</b><small class="muted">${s.kind === 'gif' ? esc(s.gif.title || 'from KLIPY') : 'Add a message if you like'}${size} · Enter to send</small>${picture}</div>
    <button type="button" class="icon-btn" data-action="unstage" aria-label="Don't send this" title="Don't send this (Esc)">${icon('x')}</button>`;
  watchGifs(el);
}

/** Send what's in the tray, with the message box as its caption. */
async function sendStaged(body) {
  const s = state.staged;
  // A picture is uploaded first, which needs the connection: until then the
  // picture and its caption stay in the box.
  if (s.kind !== 'gif' && state.status !== 'online') return toast('Not connected right now. Your picture is still here; send it again in a moment.');
  const channelId = settings.channel;
  const input = $('input');
  clearStaged();
  input.value = '';
  autosize();
  emojiSuggest?.close();
  const sealed = sealedHere();
  const draft = plainEmojis(body); // the caption goes back in the box if the server turns it away
  if (s.kind === 'gif') {
    const nonce = crypto.randomUUID();
    if (sealed) {
      // Encrypted: the GIF rides inside the note (and KLIPY isn't told).
      const note = await sealNote({ text: body, gif: s.gif });
      queueSend({ t: 'send', channelId, body: note, nonce }, { copy: { id: nonce, channelId, uid: settings.uid, body: note, ts: Date.now(), pending: true }, draft });
      return renderMessages({ keep: 'bottom' });
    }
    const replyTo = takeReply(channelId);
    queueSend({ t: 'send', channelId, body, gif: s.gif, nonce, replyTo }, { copy: { id: nonce, channelId, uid: settings.uid, body, gif: s.gif, ts: Date.now(), pending: true, replyTo, reply: replyTo ? replyRef(replyTo) : undefined }, draft });
    reportShare(s.gif.slug, s.query);
    return renderMessages({ keep: 'bottom' });
  }
  toast('Sending picture…');
  let uploading = false;
  state.sendingPictures++; // (an update waits until it's sent: updates.js)
  try {
    const prepared = await s.ready;
    if (prepared instanceof Error) throw prepared;
    const { blob, width, height } = prepared;
    uploading = true;
    if (sealed) {
      // Encrypted: the picture is sealed before upload; the note says where it is.
      const bytes = await encryptBlob(state.vault.key, new Uint8Array(await blob.arrayBuffer()));
      const { id } = await postBlob(bytes);
      state.sealedImages.set(id, URL.createObjectURL(blob));
      const note = await sealNote({ text: body, image: { id, width, height, mime: blob.type } });
      hideToast();
      return queueSend({ t: 'send', channelId, body: note }, { draft });
    }
    const { id } = await postImage('/api/image', blob);
    hideToast();
    queueSend({ t: 'send', channelId, body, image: { id, width, height }, replyTo: takeReply(channelId) }, { draft });
  } catch (err) {
    toast(`Couldn't send that picture: ${err.message}`);
    // The caption goes back in the box, and if it was the upload that failed (not the
    // picture), the picture goes back in the tray: Send tries again.
    if (settings.channel !== channelId || state.editing) return;
    input.value = input.value || plainEmojis(body);
    autosize();
    if (uploading && !state.staged) {
      state.staged = { ...s, url: URL.createObjectURL(s.file) };
      renderStaged();
    }
  } finally {
    state.sendingPictures--;
  }
}

export let reactPicker = null;
let reactTarget = null;

/** Emoji picker floating next to a message; picking adds a reaction. */
export function openReactPicker(button) {
  reactTarget = button.dataset.id;
  openedBy.set('react-picker', button);
  reactPicker ??= new EmojiPicker($('react-picker'), {
    readonly: true, // picking only: no upload/edit controls here
    emojis: emojiList,
    onPick: ({ char, emoji }) => {
      conn?.send({ t: 'react', id: reactTarget, emoji: char ?? `<:${emoji.name}:${emoji.id}>`, on: true });
      reactPicker.close();
    },
    toast,
  });
  const el = $('react-picker');
  reactPicker.open();
  placeNear(el, button, { align: 'right' });
  el.style.bottom = 'auto';
  el.style.right = 'auto';
}
