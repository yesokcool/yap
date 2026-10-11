// Signing up and in, your password (and the once-per-device "Choose a
// password" prompt), your devices, sign-in alerts, starting over from the
// waiting room, and Add to Home Screen.

import { settings, save } from '../settings.js';
import { esc, formatTime } from '../format.js';
import { applySignInLink, checkKey, parseKey, sendHello, start } from '../app.js';
import { $, coarsePointer, conn, state } from './state.js';
import { swatches } from './ui.js';
import { renderUserPanel } from './sidebar.js';

/** Waiting room › "Not you? Start over": forget this sign-in on this device and go back to the welcome form. */
export function startOver() {
  conn?.stop();
  save({ token: '', uid: '', name: '' });
  location.reload();
}

let welcomeColor = settings.color;

/** The server's rule for a new password (PROTOCOL.md › Identity), checked here first so the form can say so at once. */
const PASSWORD_MIN = 8;
const TOO_SHORT = 'Use a password of at least 8 characters.';
/** Length in characters, the way the server counts them (an emoji is one). */
const chars = (s) => [...s].length;

export function onHelloError(msg) {
  if (msg.code === 'bad_token' && state.spareHandoff) {
    save({ token: state.spareHandoff });
    state.spareHandoff = null;
    return sendHello();
  }
  const tried = state.helloPayload;
  state.helloPayload = null;
  const typed = tried?.login?.name ?? tried?.signup?.name;
  if (msg.code === 'bad_token') save({ token: '' });
  const loginCodes = ['bad_token', 'pin_required', 'bad_login', 'locked', 'no_pin'];
  let mode = loginCodes.includes(msg.code) || tried?.login ? 'login' : 'signup';
  let error = msg.message;
  if (msg.code === 'name_taken' && tried?.signup) {
    // Probably them on a new device: flip to sign-in.
    mode = 'login';
    error = `Someone already has the name ${tried.signup.name}. If that's you, sign in with your password.`;
  }
  // A wrong password was a dead end: say who can help.
  // Someone still on an old PIN only sees a Password field: say the PIN works there.
  if (msg.code === 'bad_login') error = `${error} An old PIN works here too. Forgot it? The owner or a mod can reset it.`;
  else if (msg.code === 'locked') error = `${error} Forgot your password? The owner or a mod can reset it.`;
  showWelcome({ needKey: false, error, mode, ...(typed ? { name: typed } : {}) });
}

export function showWelcome({ needKey, error = '', mode, name = settings.name }) {
  const dialog = $('welcome-dialog');
  const form = $('welcome-form');
  // Signed out: nothing of the app behind the form (no "…" user, no empty lists).
  if (!state.sid) document.documentElement.classList.add('signed-out');
  form.name.value = name;
  form.pin.value = '';
  hideSecrets(form);
  form.mode.value = mode ?? (settings.name || settings.token ? 'login' : 'signup');
  $('welcome-key-field').hidden = !needKey && !(isStandalone() && !settings.key);
  $('welcome-paste').hidden = !(isStandalone() && !settings.token);
  form.key.required = needKey;
  form.key.value = '';
  $('welcome-error').hidden = !error;
  $('welcome-error').textContent = error;
  swatches(dialog.querySelector('.swatches'), settings.color, (c) => (welcomeColor = c));
  syncWelcomeMode();
  if (!dialog.open) dialog.showModal();
}

export function syncWelcomeMode() {
  const form = $('welcome-form');
  const signup = form.mode.value === 'signup';
  for (const el of form.querySelectorAll('[data-signup-only]')) el.hidden = !signup;
  // So password managers offer to make one (sign-up) or fill the saved one (sign-in).
  form.pin.autocomplete = signup ? 'new-password' : 'current-password';
  $('welcome-hint').textContent = signup
    ? 'Pick a name friends will see and a password. It’s how you sign in on your other devices.'
    : 'Sign in with your name and password — same you, same messages, on any device.';
  form.querySelector('button[type=submit]').textContent = signup ? 'Sign Up' : 'Sign On';
}

export function bindWelcome() {
  const dialog = $('welcome-dialog');
  const form = $('welcome-form');
  dialog.addEventListener('cancel', (e) => e.preventDefault()); // must finish setup
  form.addEventListener('change', (e) => {
    if (e.target.name !== 'mode') return;
    $('welcome-error').hidden = true; // an old error is about the other form
    syncWelcomeMode();
  });
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = form.name.value.trim();
    const pin = form.pin.value; // (never trimmed: a password is exactly what was typed)
    const error = (text) => {
      $('welcome-error').textContent = text;
      $('welcome-error').hidden = false;
    };
    if (!name || !pin) return;
    // Signing in takes any password (or an old 6-digit PIN); a new one has to be long enough.
    if (form.mode.value === 'signup' && chars(pin) < PASSWORD_MIN) return error(TOO_SHORT);
    if (!$('welcome-key-field').hidden && applySignInLink(form.key.value)) return;
    if (!$('welcome-key-field').hidden) {
      const key = parseKey(form.key.value);
      const check = await checkKey(key);
      if (check && !check.ok) return error("That invite link didn't work. Ask for a new one.");
      const keyChanged = key !== settings.key;
      save({ key });
      if (keyChanged && conn) location.reload(); // simplest way to re-open the socket with the new key
    }
    const signup = form.mode.value === 'signup';
    save({ color: welcomeColor, token: '' }); // the name is kept once the server says yes (ready / pending)
    state.helloPayload = signup ? { signup: { name, pin, color: welcomeColor } } : { login: { name, pin } };
    hideSecrets(form);
    dialog.close();
    renderUserPanel();
    if (conn?.open && !state.sid) sendHello();
    else if (conn?.open) location.reload(); // switching accounts mid-session: start clean
    start();
  });
}

/** Settings › Account › Change password (Set a password while on an old PIN, or with none). */
export function showPinDialog() {
  const form = $('pin-form');
  form.reset();
  hideSecrets(form);
  $('pin-error').hidden = true;
  const onPin = state.secretKind !== 'password';
  form.username.value = settings.name;
  $('pin-current-field').hidden = !state.hasPin;
  form.current.required = Boolean(state.hasPin);
  $('pin-current-label').textContent = onPin ? 'Current password or PIN' : 'Current password';
  $('pin-title').textContent = passwordAction();
  $('pin-dialog').showModal();
  (state.hasPin ? form.current : form.pin).focus();
}

/** "Change password", or "Set a password" for someone still on a PIN (or without one). */
export const passwordAction = () => (state.hasPin && state.secretKind === 'password' ? 'Change password' : 'Set a password');

export function bindPinDialog() {
  const form = $('pin-form');
  form.addEventListener('submit', (e) => {
    if (e.submitter?.value === 'cancel') return; // (Cancel closes it)
    e.preventDefault();
    const fail = (text) => {
      $('pin-error').textContent = text;
      $('pin-error').hidden = false;
    };
    // (Our own words, not the browser's: the form is novalidate.)
    if (form.current.required && !form.current.value) return fail(state.secretKind === 'password' ? 'Type your current password first.' : 'Type your current password or PIN first.');
    if (chars(form.pin.value) < PASSWORD_MIN) return fail(TOO_SHORT);
    $('pin-error').hidden = true;
    conn?.send({ t: 'account.pin', pin: form.pin.value, current: form.current.value || undefined });
  });
}

// ── Passwords: show/hide, and the once-per-device prompt ─────────────────

/** Show or hide what's typed in the password field next to this button. */
export function toggleReveal(button) {
  const input = button.closest('.field')?.querySelector('input');
  if (!input) return;
  const show = input.type === 'password';
  input.type = show ? 'text' : 'password';
  button.textContent = show ? 'Hide' : 'Show';
  button.setAttribute('aria-pressed', String(show));
  input.focus();
}

/** Back to dots, in every password field of a form (so a password is never left showing). */
function hideSecrets(form) {
  for (const button of form.querySelectorAll('[data-action="reveal"]')) {
    const input = button.closest('.field')?.querySelector('input');
    if (input) input.type = 'password';
    button.textContent = 'Show';
    button.setAttribute('aria-pressed', 'false');
  }
}

/** The prompt's words, by ready.weakReason. */
const PROMPTS = {
  pin: 'Yap uses passwords now. Your PIN still works until you choose one.',
  temporary: 'You signed in with a temporary password. Choose your own.',
  guessable: 'Your password is easy to guess. Choose a stronger one.',
};

/**
 * Why to ask this person to choose a password (ready.weakPin, ready.weakReason),
 * or null: an old PIN, a temporary password from a reset, or a password that's
 * easy to guess. "Later" hides it on this device for good (for that reason).
 */
function passwordPrompt() {
  if (!state.weakPin || !state.sid) return null;
  const reason = PROMPTS[state.weakReason] ? state.weakReason : state.secretKind !== 'password' ? 'pin' : 'guessable';
  if ((settings.passwordLater ?? []).includes(reason)) return null;
  return { reason, text: PROMPTS[reason] };
}

/** Later: not on this device again. */
export function passwordLater() {
  const prompt = passwordPrompt();
  if (prompt) save({ passwordLater: [...new Set([...(settings.passwordLater ?? []), prompt.reason])] });
  renderAccountBars();
}

export function renderDevices() {
  const list = $('devices-list');
  if (!list) return;
  const day = 86_400_000;
  const when = (t) => {
    const ago = Date.now() - t;
    return ago < 120_000 ? 'just now' : ago < day ? `today, ${formatTime(t)}` : new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: ago > 300 * day ? 'numeric' : undefined });
  };
  list.innerHTML = (state.devices ?? []).map((d) => `
    <li class="member device ${Date.now() - d.createdAt < day && !d.current ? 'new' : ''}">
      <span class="member-text">
        <span class="name">${esc(d.label)}${d.current ? ' <small>(this device)</small>' : ''}${Date.now() - d.createdAt < day && !d.current ? ' <span class="role-badge new-badge">new</span>' : ''}</span>
        <span class="sub">Signed in ${esc(when(d.createdAt))} · last used ${esc(when(d.lastSeen))}</span>
      </span>
      <span class="spacer"></span>
      ${d.current ? '' : `<button type="button" class="btn small" data-action="revoke-device" data-id="${esc(d.id)}">Sign out</button>`}
    </li>`).join('') || '<li class="muted">Just this one.</li>';
}

/** The "new sign-in" alert and the "Choose a password" prompt, above the chat. */
export function renderAccountBars() {
  const alert = state.signinAlert;
  const bar = $('signin-bar');
  bar.hidden = !alert;
  if (alert) {
    bar.innerHTML = `<span>🔐 <b>New sign-in</b> to your account: ${esc(alert.label)}, just now. Not you?</span><span class="spacer"></span>
      <button class="btn small danger" type="button" data-action="signout-others">Sign out everywhere else</button>
      <button class="btn small" type="button" data-action="dismiss-signin">It was me</button>`;
  }
  const prompt = passwordPrompt();
  const ask = $('weak-bar');
  ask.hidden = !prompt;
  if (prompt) {
    ask.innerHTML = `<span class="prompt-words">🔑 <b>Choose a password</b> <span class="prompt-text">${esc(prompt.text)}</span></span>
      <button class="btn small primary" type="button" data-action="change-pin">Choose a password</button>
      <button class="btn small" type="button" data-action="password-later">Later</button>`;
  }
  const change = $('change-pin-btn');
  if (change) change.textContent = passwordAction();
}

// ── Add to Home Screen ────────────────────────────────────────────────────
// Chrome/Edge/Android: their own install prompt. iPhone/iPad (and Mac
// Safari): no prompt exists, so we show the two taps — and since a
// home-screen app has its own storage, the page's URL first gets a one-time
// sign-in pass (15 min) so the new icon opens already signed in.

let installPrompt = null;
addEventListener('beforeinstallprompt', (e) => {
  e.preventDefault();
  installPrompt = e;
  renderHomeBar();
});
addEventListener('appinstalled', () => {
  installPrompt = null;
  renderHomeBar();
});

const isStandalone = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
const isIos = () => /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const isMacSafari = () => /Macintosh/.test(navigator.userAgent) && /Safari\//.test(navigator.userAgent) && !/Chrome|Chromium|Edg\//.test(navigator.userAgent) && navigator.maxTouchPoints <= 1;
export const canAddToHome = () => !isStandalone() && (Boolean(installPrompt) || isIos() || isMacSafari());

export function renderHomeBar() {
  let dismissed = 0;
  try { dismissed = Number(localStorage.getItem('yap.homeDismissed')) || 0; } catch {}
  // Phones and tablets only; asked again a month after "not now".
  $('home-bar').hidden = !(state.status === 'online' && canAddToHome() && (isIos() || (installPrompt && coarsePointer)) && Date.now() - dismissed > 30 * 86_400_000);
}

function handoffPass() {
  return new Promise((resolve) => {
    state.handoffWaiter = resolve;
    conn?.send({ t: 'device.handoff' });
    setTimeout(() => resolve(null), 4000);
  });
}

export async function addToHomeScreen() {
  if (installPrompt) {
    const prompt = installPrompt;
    installPrompt = null;
    await prompt.prompt();
    return renderHomeBar();
  }
  // Bake a one-time pass into the address the shortcut will open — and copy
  // the same link, in case the Home Screen app opens without it. (Copying
  // with a promise keeps it inside this tap, which Safari insists on.)
  const link = handoffPass().then((pass) => {
    const params = new URLSearchParams({ key: settings.key });
    if (pass) params.set('handoff', pass);
    if (settings.name) params.set('name', settings.name);
    return { pass, url: `${location.origin}/#${params}` };
  });
  try {
    navigator.clipboard.write([new ClipboardItem({ 'text/plain': link.then(({ url }) => new Blob([url], { type: 'text/plain' })) })]).catch(() => {});
  } catch {}
  const { pass, url } = await link;
  // The address keeps the pass while you add the icon (people tap OK first,
  // then Share); the next load reads it and clears it.
  history.replaceState(null, '', url);
  const share = '<svg class="home-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12"/><path d="m7 8 5-5 5 5"/><path d="M5 12v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7"/></svg>';
  const plus = '<svg class="home-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="4"/><path d="M12 8v8M8 12h8"/></svg>';
  const ipad = /iPad/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
  $('home-steps').innerHTML = isMacSafari()
    ? `<li>In the menu bar, choose <b>File › Add to Dock…</b></li><li>Click <b>Add</b>. Yap gets its own window and Dock icon.</li>`
    : `<li>Tap the <b>Share</b> button ${share} ${ipad ? 'at the top right' : 'in the toolbar (bottom of the screen, or next to the address bar)'}.</li>
       <li>Scroll down and tap <b>Add to Home Screen</b> ${plus}</li>
       <li>Tap <b>Add</b>. Yap's icon appears on your Home Screen.</li>`;
  $('home-note').textContent = pass
    ? 'The new icon opens Yap already signed in (within 15 minutes). We also copied a sign-in link: if the icon asks you to sign in, tap “Paste the sign-in link”.'
    : 'The new icon will ask for your name and password once (home-screen apps keep their own sign-in).';
  $('home-dialog').showModal();
}
