// Light or dark before first paint (dark unless the person picked otherwise).
// Yap has one look, Windows 98: a skin saved by an older version (Apple, Arch)
// is ignored here and dropped by js/settings.js.
// A separate file (not inline) so the page's Content-Security-Policy can forbid inline scripts.
(() => {
  let saved = {};
  try {
    saved = JSON.parse(localStorage.getItem('yap.settings') || '{}');
  } catch {}
  const root = document.documentElement;
  root.dataset.theme = saved.theme || 'dark';
  // Inside the iPhone/Mac app (Draw, Space: #…&embed=1) the app has its own
  // bar and sidebar, so ours stay hidden — for the rest of this tab, reloads too.
  try {
    if (/[#&]embed=1(?:&|$)/.test(location.hash)) sessionStorage.setItem('yap.embed', '1');
    if (sessionStorage.getItem('yap.embed')) root.dataset.embed = 'app';
  } catch {}
})();
