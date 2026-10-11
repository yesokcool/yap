// Settings › Look: light or dark. Yap has one look, Windows 98 (style.css);
// js/theme.js sets the theme before first paint, this keeps it in step after.

import { settings } from '../settings.js';
import { $ } from './state.js';

/**
 * Dark by default. `changed`: the person just picked a theme, so the board
 * inside Yap follows (otherwise the board keeps whatever it was set to).
 */
export function applyTheme({ changed = false } = {}) {
  document.documentElement.dataset.theme = settings.theme;
  const dark = settings.theme === 'dark' || (settings.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  try {
    if (changed || !localStorage.getItem('gk-theme')) localStorage.setItem('gk-theme', dark ? 'dark' : 'light');
    const board = $('board-frame')?.contentDocument?.documentElement;
    if (board && changed) board.dataset.theme = dark ? 'dark' : 'light';
  } catch {}
}
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => settings.theme === 'system' && applyTheme({ changed: true }));
