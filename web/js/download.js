import { esc } from './format.js';

// Desktop builds from /downloads/latest.json (written by tools/publish-desktop.mjs).
const PLATFORMS = [
  ['windows-x86_64', '🪟', 'Windows', '64-bit, Windows 10 or newer'],
  ['macos-aarch64', '🍎', 'Mac', 'Apple silicon (M1 or newer)'],
  ['linux-x86_64', '🐧', 'Linux', 'x86-64, glibc 2.28 or newer'],
];

const el = document.getElementById('platforms');
try {
  const res = await fetch('/downloads/latest.json', { cache: 'no-cache' });
  if (!res.ok) throw new Error(res.status);
  const latest = await res.json();
  el.innerHTML = PLATFORMS.filter(([key]) => latest.files?.[key])
    .map(([key, glyph, name, detail]) => {
      const f = latest.files[key];
      return `<div class="platform">
        <span class="platform-icon" aria-hidden="true">${glyph}</span>
        <span><b>${esc(name)}</b><span class="muted">${esc(detail)} · ${(f.size / 1e6).toFixed(1)} MB · v${esc(latest.version)}</span></span>
        <a class="btn" href="${esc(f.path)}" download>Download</a>
      </div>`;
    })
    .join('');
} catch {
  el.innerHTML = '<p class="muted">No desktop builds are published yet.</p>';
}
