#!/usr/bin/env node
// Builds web/emoji-data.json — the standard emoji set for the picker and
// :shortcode: autocomplete — from emojibase (a devDependency). Rerun after
// bumping emojibase-data:  node tools/build-emoji-data.mjs
//
// Format: { groups: [name…], emojis: [[char, groupIndex, "short codes", "search words", "5 skin tones"?], …] }
// The 5th field (light → dark, space-separated) only exists for emojis with one skin tone.
// Shortcodes are Slack/Discord-style (iamcal) plus emojibase's extras.
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { ROOT } from './lib.mjs';

const require = createRequire(import.meta.url);
const data = require('emojibase-data/en/data.json');
const iamcal = require('emojibase-data/en/shortcodes/iamcal.json');
const extra = require('emojibase-data/en/shortcodes/emojibase.json');
const { version } = require('emojibase-data/package.json');

// Emoji 16+ still shows as empty boxes on many systems (e.g. Windows 10/11 fonts).
const MAX_EMOJI_VERSION = 15.1;
// emojibase group id → picker section (2 = skin-tone components; skipped).
const GROUPS = [[0, 'Smileys'], [1, 'People'], [3, 'Animals & nature'], [4, 'Food & drink'], [5, 'Travel & places'], [6, 'Activities'], [7, 'Objects'], [8, 'Symbols'], [9, 'Flags']];

const list = (v) => (v === undefined ? [] : Array.isArray(v) ? v : [v]);
const emojis = [];
for (const e of [...data].sort((a, b) => a.order - b.order)) {
  const group = GROUPS.findIndex(([id]) => id === e.group);
  if (group < 0 || e.version > MAX_EMOJI_VERSION) continue;
  const codes = [...new Set([...list(iamcal[e.hexcode]), ...list(extra[e.hexcode])])];
  if (!codes.length) continue;
  const words = [...new Set([...e.label.toLowerCase().split(/[\s:,“”"()]+/), ...list(e.tags)])].filter((w) => w && !codes.includes(w));
  const tones = (e.skins ?? []).filter((s) => typeof s.tone === 'number').sort((a, b) => a.tone - b.tone).map((s) => s.emoji);
  emojis.push(tones.length === 5 ? [e.emoji, group, codes.join(' '), words.join(' '), tones.join(' ')] : [e.emoji, group, codes.join(' '), words.join(' ')]);
}

const out = path.join(ROOT, 'web/emoji-data.json');
fs.writeFileSync(out, JSON.stringify({ source: `emojibase-data ${version}`, groups: GROUPS.map(([, name]) => name), emojis }));
console.log(`wrote ${emojis.length} emojis, ${(fs.statSync(out).size / 1024).toFixed(0)} KB`);
