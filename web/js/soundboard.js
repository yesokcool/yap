/*
 * The soundboard (PROTOCOL.md › Extras › Soundboard): short clips anyone in
 * a voice channel can play for everyone in it. The loudness cap lives in
 * sounds.js; this is the panel: play, add, remove, your volume.
 */
import { esc } from './format.js';
import { settings, save } from './settings.js';
import { makeClip, playClip } from './sounds.js';
import { placeNear } from './place.js';
import { confirmSheet } from './sheet.js';

export class Soundboard {
  /**
   * @param {HTMLElement} el floating panel
   * @param {{ sounds: () => object[], inVoice: () => boolean, play: (id: string) => void, upload: (blob: Blob, name: string, emoji: string) => Promise<void>, remove: (sound: object) => void, canRemove: (sound: object) => boolean, toast: (t: string) => void }} hooks
   */
  constructor(el, hooks) {
    this.el = el;
    this.hooks = hooks;
    this.adding = false;
    this.editing = false; // × on your own clips (any for owner/mods) only while editing
    el.addEventListener('click', (e) => this.onClick(e));
    el.addEventListener('input', (e) => {
      if (e.target.name !== 'sb-volume') return;
      save({ soundboardVolume: Number(e.target.value) / 100 });
      e.target.closest('label').querySelector('output').textContent = `${e.target.value}%`;
    });
    el.addEventListener('change', async (e) => {
      if (e.target.name === 'sb-muted') return save({ soundboardMuted: e.target.checked });
      if (e.target.name !== 'sb-file') return;
      const file = e.target.files[0];
      e.target.value = '';
      if (!file) return;
      try {
        this.clip = await makeClip(file, 5000);
        this.adding = true;
        this.render();
        this.el.querySelector('[name=sb-name]').value = file.name.replace(/\.[^.]+$/, '').slice(0, 24);
        this.el.querySelector('[name=sb-name]').focus();
        if (this.clip.cut) this.hooks.toast('Trimmed to the first 5 seconds.');
      } catch (err) {
        this.hooks.toast(`Couldn't use that file: ${err.message}`);
      }
    });
  }

  get isOpen() {
    return !this.el.hidden;
  }

  toggle(near) {
    if (this.isOpen) return this.close();
    this.adding = false;
    this.editing = false;
    this.el.hidden = false;
    this.render();
    placeNear(this.el, near);
  }

  close() {
    this.el.hidden = true;
  }

  render() {
    if (this.el.hidden) return;
    const list = this.hooks.sounds().filter((s) => s.kind === 'board');
    const live = this.hooks.inVoice();
    const removable = list.some((s) => this.hooks.canRemove(s));
    if (!removable) this.editing = false;
    this.el.classList.toggle('editing', this.editing);
    this.el.innerHTML = `
      <div class="sb-head"><b>Soundboard</b><span class="spacer"></span>${removable ? `<button type="button" class="link-btn sb-edit" data-sb="edit" aria-pressed="${this.editing}">${this.editing ? 'Done' : 'Edit'}</button>` : ''}<button type="button" class="icon-btn tiny" data-sb="close" aria-label="Close" title="Close">×</button></div>
      ${live ? '' : '<p class="muted sb-note">Join a voice channel to play sounds for everyone in it.</p>'}
      <div class="sb-grid">
        ${list.map((s) => `
          <div class="sb-item">
            <button type="button" class="btn sb-play" data-sb="play" data-id="${esc(s.id)}" ${live ? '' : 'disabled'} title="${esc(s.name)} · ${(s.ms / 1000).toFixed(1)} s">
              <span class="sb-emoji">${esc(s.emoji ?? '🔊')}</span><span class="sb-name">${esc(s.name)}</span>
            </button>
            <button type="button" class="sb-mini" data-sb="preview" data-id="${esc(s.id)}" title="Preview (only you hear it)" aria-label="Preview ${esc(s.name)}">▶</button>
            ${this.editing && this.hooks.canRemove(s) ? `<button type="button" class="sb-mini sb-remove" data-sb="remove" data-id="${esc(s.id)}" title="Delete" aria-label="Delete ${esc(s.name)}">×</button>` : ''}
          </div>`).join('') || '<p class="muted sb-note">No sounds yet. Add the first one!</p>'}
      </div>
      ${this.adding ? `
        <div class="sb-add inline-form">
          <input name="sb-emoji" maxlength="8" placeholder="🔊" aria-label="Emoji" class="sb-emoji-input">
          <input name="sb-name" maxlength="24" placeholder="Name" aria-label="Name">
          <button type="button" class="btn small" data-sb="test">Test</button>
          <button type="button" class="btn small primary" data-sb="save">Add</button>
        </div>` : `
        <label class="btn small sb-upload">+ Add a sound<input type="file" name="sb-file" accept="audio/*,.mp3,.m4a,.wav,.ogg,.webm" hidden></label>`}
      <div class="sb-volume">
        <label class="field"><span>Your volume <output>${Math.round(settings.soundboardVolume * 100)}%</output></span><input type="range" name="sb-volume" min="0" max="100" step="5" value="${Math.round(settings.soundboardVolume * 100)}"></label>
        <label class="toggle"><input type="checkbox" name="sb-muted" ${settings.soundboardMuted ? 'checked' : ''}><span>Mute the soundboard</span></label>
        <small class="muted">Clips are 5 s at most and loud ones are turned down for everyone.</small>
      </div>`;
  }

  async onClick(e) {
    const b = e.target.closest('[data-sb]');
    if (!b) return;
    const sound = this.hooks.sounds().find((s) => s.id === b.dataset.id);
    switch (b.dataset.sb) {
      case 'close': return this.close();
      case 'edit':
        this.editing = !this.editing;
        return this.render();
      case 'play': return sound && this.hooks.play(sound.id);
      case 'preview': return sound && playClip(sound, { force: true });
      case 'remove':
        if (sound) confirmSheet({ title: `Delete “${sound.name}”?`, text: 'It goes from the soundboard for everyone.', ok: 'Delete', danger: true }).then((yes) => yes && this.hooks.remove(sound));
        return;
      case 'test': {
        // The server measures loudness on upload; until then, play it quietly.
        const url = URL.createObjectURL(this.clip.blob);
        const audio = new Audio(url);
        audio.volume = 0.3 * settings.soundboardVolume;
        audio.onended = () => URL.revokeObjectURL(url);
        return audio.play().catch(() => URL.revokeObjectURL(url));
      }
      case 'save': {
        const name = this.el.querySelector('[name=sb-name]').value.trim();
        const emoji = this.el.querySelector('[name=sb-emoji]').value.trim();
        if (!name) return this.hooks.toast('Give it a name.');
        b.disabled = true;
        try {
          await this.hooks.upload(this.clip.blob, name, emoji);
          this.adding = false;
          this.clip = null;
          this.render();
        } catch (err) {
          this.hooks.toast(`Couldn't add it: ${err.message}`);
          b.disabled = false;
        }
      }
    }
  }
}
