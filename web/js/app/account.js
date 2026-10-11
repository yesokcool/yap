// Signing up and in, your PIN or password, your devices, sign-in alerts,
// starting over from the waiting room, and Add to Home Screen.

import { settings, save } from '../settings.js';
import { icon } from '../icons.js';
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
    error = `Someone already has the name ${tried.signup.name}. If that's you, sign in with your PIN.`;
  }
  // A wrong PIN was a dead end: say who can help.
  if (msg.code === 'bad_login' || msg.code === 'locked') error = `${error} Forgot your PIN? The owner or a mod can reset it.`;
  showWelcome({ needKey: false, error, mode, ...(typed ? { name: typed } : {}) });
}

export function showWelcome({ needKey, error = '', mode, name = settings.name }) {
  const dialog = $('welcome-dialog');
  const form = $('welcome-form');
  // Signed out: nothing of the app behind the form (no "…" user, no empty lists).
  if (!state.sid) document.documentElement.classList.add('signed-out');
  form.name.value = name;
  form.pin.value = '';
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
  form.pin.autocomplete = signup ? 'new-password' : 'current-password';
  // The hint follows "Use a password" (it kept saying PIN).
  const password = form.dataset.secret === 'password';
  $('welcome-hint').textContent = signup
    ? (password
      ? 'Pick a name friends will see and a password of 10 or more characters. It’s how you sign in on your other devices.'
      : 'Pick a name friends will see and a 6-digit PIN. The PIN is how you sign in on your other devices.')
    : `Sign in with your name and ${password ? 'password' : 'PIN'} — same you, same messages, on any device.`;
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
    const pin = form.pin.value.trim();
    const error = (text) => {
      $('welcome-error').textContent = text;
      $('welcome-error').hidden = false;
    };
    if (!name) return;
    const password = form.dataset.secret === 'password';
    if (password ? pin.length < 10 : !/^\d{6}$/.test(pin)) return error(password ? 'Passwords are at least 10 characters.' : 'Your PIN is exactly 6 digits.');
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
    dialog.close();
    renderUserPanel();
    if (conn?.open && !state.sid) sendHello();
    else if (conn?.open) location.reload(); // switching accounts mid-session: start clean
    start();
  });
}

export function showPinDialog() {
  const form = $('pin-form');
  form.reset();
  $('pin-error').hidden = true;
  $('pin-current-field').hidden = !state.hasPin;
  form.current.required = Boolean(state.hasPin);
  $('pin-current-label').textContent = state.secretKind === 'password' ? 'Current password' : 'Current PIN';
  setSecretMode(form, 'pin');
  $('pin-title').textContent = state.hasPin ? 'Change your PIN or password' : 'Set a PIN';
  $('pin-hint').textContent = 'Six digits. You sign in on new devices with your name and this PIN.';
  $('pin-dialog').showModal();
}

export function bindPinDialog() {
  const form = $('pin-form');
  form.addEventListener('submit', (e) => {
    if (e.submitter?.value === 'cancel') return; // (Cancel closes it; it used to say "Exactly 6 digits." and stay)
    e.preventDefault();
    const fail = (text) => {
      $('pin-error').textContent = text;
      $('pin-error').hidden = false;
    };
    // (Our own words, not the browser's "Please match the requested format": the form is novalidate.)
    const password = form.dataset.secret === 'password';
    if (form.current.required && !form.current.value) return fail(`Type your current ${state.secretKind === 'password' ? 'password' : 'PIN'} first.`);
    if (password ? form.pin.value.length < 10 : !/^\d{6}$/.test(form.pin.value)) return fail(password ? 'At least 10 characters — a short phrase works well.' : 'Your new PIN is exactly 6 digits.');
    if (form.pin.value !== form.again.value) return fail("Those two don't match.");
    conn?.send({ t: 'account.pin', pin: form.pin.value, current: form.current.value || undefined });
  });
}

// ── Account protection: PIN or password, devices, sign-in alerts ──────────

/** Switch a form's secret field between a 6-digit PIN and a password. */
export function setSecretMode(form, mode) {
  form.dataset.secret = mode;
  const password = mode === 'password';
  for (const input of [form.pin, form.again].filter(Boolean)) {
    input.value = '';
    if (password) {
      input.removeAttribute('pattern');
      input.removeAttribute('maxlength');
      input.removeAttribute('inputmode');
      input.placeholder = '';
    } else {
      input.setAttribute('pattern', '\\d{6}');
      input.setAttribute('maxlength', '6');
      input.setAttribute('inputmode', 'numeric');
      if (form.id === 'welcome-form') input.placeholder = '••••••';
    }
  }
  const label = form.querySelector('.secret-label');
  const isNew = form.id === 'pin-form';
  label.innerHTML = password ? `${isNew ? 'New password' : 'Password'} <small class="muted">— 10+ characters</small>` : `${isNew ? 'New PIN' : 'PIN'} <small class="muted">— 6 digits</small>`;
  form.querySelector('[data-action="secret-mode"]').textContent = password ? 'Use a PIN' : isNew ? 'Use a password instead' : 'Use a password';
  if ($('pin-save') && isNew) $('pin-save').textContent = password ? 'Save password' : 'Save PIN';
  if (form.id === 'welcome-form') syncWelcomeMode();
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

/** The "new sign-in" alert and the weak-PIN nudge, above the chat. */
export function renderAccountBars() {
  const alert = state.signinAlert;
  const bar = $('signin-bar');
  bar.hidden = !alert;
  if (alert) {
    bar.innerHTML = `<span>🔐 <b>New sign-in</b> to your account: ${esc(alert.label)}, just now. Not you?</span><span class="spacer"></span>
      <button class="btn small danger" type="button" data-action="signout-others">Sign out everywhere else</button>
      <button class="btn small" type="button" data-action="dismiss-signin">It was me</button>`;
  }
  const weak = $('weak-bar');
  weak.hidden = !state.weakPin || state.weakDismissed || !state.sid;
  if (!weak.hidden) {
    weak.innerHTML = `<span>🔑 Your PIN is easy to guess for someone who knows you.</span><span class="spacer"></span>
      <button class="btn small primary" type="button" data-action="change-pin">Change it</button>
      <button class="icon-btn tiny" type="button" data-action="dismiss-weak" aria-label="Not now" title="Not now">${icon('x')}</button>`;
  }
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
    : 'The new icon will ask for your name and PIN once (home-screen apps keep their own sign-in).';
  $('home-dialog').showModal();
}
