import { esc } from './format.js';

const log = await (await fetch('/changelog.json')).json();

// The biggest things from the past week, curated in changelog.json › highlights.
const weekAgo = Date.now() - 7 * 86_400_000;
const recent = (log.highlights ?? []).filter((h) => Date.parse(h.date) >= weekAgo - 86_400_000);
const box = document.getElementById('highlights');
box.hidden = !recent.length;
box.innerHTML = recent.length
  ? `<h2>Highlights from the past week</h2><ul>${recent.map((h) => `
      <li><span class="hl-emoji">${esc(h.emoji)}</span><div><b>${esc(h.title)}</b><p>${esc(h.text)}</p>${h.where ? `<small>${esc(h.where)}</small>` : ''}</div></li>`).join('')}</ul>`
  : '';
document.getElementById('releases').innerHTML = log.releases
  .map((r) => `
    <section class="release">
      <header><span class="ver">v${esc(r.version)}</span><h2>${esc(r.title ?? '')}</h2><time>${esc(r.date)}</time></header>
      <ul>${r.items.map((i) => `<li>${esc(i)}</li>`).join('')}</ul>
    </section>`)
  .join('');
