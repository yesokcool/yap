// Helpers shared by tools/*.mjs.
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** The repo root. (fileURLToPath, never URL.pathname: the path may contain spaces. PATTERNS.md.) */
export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** git in this repo; its output, trimmed. */
export const git = (...args) => execFileSync('git', ['-C', ROOT, ...args], { encoding: 'utf8' }).trim();

/** A secret from the macOS Keychain, by service name; '' when it isn't there (or not on a Mac). Never printed. */
export function keychain(service) {
  if (process.platform !== 'darwin') return '';
  try {
    // stderr ignored: `security` only ever says "not found" there; stdout is captured, never shown.
    return execFileSync('security', ['find-generic-password', '-s', service, '-w'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return '';
  }
}

/** The friendly, user-facing lines in commit messages ("Changelog: …"), in order, each once. */
export const changelogLines = (text) => [...new Set(text.split('\n').map((line) => /^Changelog:\s*(.+)$/.exec(line)?.[1].trim()).filter(Boolean))];

/**
 * What a desktop update's v2 signature (`sig2` in latest.json) signs, as
 * UTF-8: `yap-update-v2`, the platform key, the version exactly as the
 * manifest has it and the file's SHA-256 in lowercase hex, joined by "\n",
 * nothing after. The apps check the same bytes (`signed_message` in
 * desktop/src/update.rs), so a signed build can't be relabelled as another
 * version or handed to another platform.
 */
export const updateMessage = (platform, version, sha256) => `yap-update-v2\n${platform}\n${version}\n${sha256.toLowerCase()}`;

/** Stop, saying why (exit 1). */
export function fail(message, status = 1) {
  console.error(message);
  process.exit(status);
}

/** Stop: the tool was called wrong (exit 2). */
export const usage = (text) => fail(`usage: ${text}`, 2);

/** fn's result, or stop with its error's message (no stack trace for "which server?"). */
export function orFail(fn) {
  try {
    return fn();
  } catch (err) {
    return fail(err.message);
  }
}

/**
 * The stored files (media ids) a backup refers to, which a restore of it
 * needs: custom emojis (deleted ones too: old messages still show them),
 * avatars, the pictures and doodle reactions of posts that weren't deleted,
 * saved drawings, and soundboard clips and chimes. Sealed (encrypted)
 * pictures aren't here: their ids are inside the notes, which only their
 * owner can open (backups keep them as .bin files).
 */
export function referencedMedia(backup) {
  const ids = new Set();
  const add = (id) => {
    if (typeof id === 'string' && /^[a-f0-9]{32}$/.test(id)) ids.add(id);
  };
  for (const e of backup.emojis ?? []) add(e.id);
  for (const u of backup.users ?? []) add(u.avatar);
  const live = new Set();
  for (const m of backup.messages ?? []) {
    if (m.deletedAt) continue;
    live.add(m.id);
    add(m.image?.id);
  }
  for (const r of backup.reactions ?? []) if (live.has(r.message_id)) add(/^<d:([a-f0-9]{32})>$/.exec(r.emoji)?.[1]);
  for (const d of backup.drawings ?? []) add(d.media_id);
  for (const s of backup.sounds ?? []) add(s.media_id);
  return ids;
}
