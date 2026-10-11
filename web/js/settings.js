// Per-device preferences in localStorage. Everything degrades to defaults
// if storage is unavailable (private windows, blocked site data).

const KEY = 'yap.settings';

export const COLORS = ['#5b6cff', '#e5484d', '#f76b15', '#f5a524', '#30a46c', '#12a594', '#0090ff', '#8e4ec6', '#d6409f', '#8d8d8d'];

const DEFAULTS = {
  /** Your account id, from the server (ready/pending). Empty until this device signs in. */
  uid: '',
  /** Device token from signing in (name + password). Empty until this device signs in. */
  token: '',
  /** "Choose a password" prompts answered with Later on this device, for good (ready.weakReason: pin | temporary | guessable). */
  passwordLater: [],
  name: '',
  color: '',
  key: '',
  inputId: '',
  outputId: '',
  mode: 'vad', // vad | ptt | open
  vadThreshold: -50, // dBFS
  pttKey: 'Backquote',
  bitrate: 128000,
  music: false,
  noiseSuppression: true,
  rnnoise: false, // RNNoise (WebAssembly) instead of the browser's noise suppression
  relayOnly: false, // voice only through the TURN relay, so people in a call never see your IP
  shareSurface: 'window', // screen sharing: window | monitor | browser
  shareQuality: 'smooth', // smooth (1080p60) | sharp (1440p30) | max (4K60)
  shareSound: true, // screen sharing: send the share's sound (a tab's, or the whole screen's on Windows)
  echoCancellation: true,
  autoGain: true,
  sounds: true,
  soundboardVolume: 0.5, // 0..1 of each clip's measured (already capped) loudness
  soundboardMuted: false,
  notifications: false, // superseded by desktopNotify (kept so old settings still read)
  desktopNotify: '', // all | mentions | off; '' = derive from `notifications`
  muted: false,
  deafened: false,
  volumes: {}, // uid -> 0..2 (their voice)
  screenVolumes: {}, // uid -> 0..1 (the sound of their screen share)
  channel: 'general',
  lastRead: {}, // channelId -> ts
  theme: 'dark', // dark | light | system
};

function load() {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || '{}') };
  } catch {
    return { ...DEFAULTS };
  }
}

export const settings = load();

// Before the 3-way choice there was one switch: on = everything, off = mentions only.
if (!settings.desktopNotify) settings.desktopNotify = settings.notifications ? 'all' : 'mentions';
if (!settings.color) settings.color = COLORS[Math.floor(Math.random() * COLORS.length)];
// One look since the Apple and Arch skins went (2026-10): a skin saved before then
// is dropped, so everyone is on Windows 98 with the light or dark they picked.
delete settings.skin;

/**
 * Same-origin API call with the invite key in a header (never in the URL,
 * where it would end up in logs and history).
 */
export function api(path, init = {}, key = settings.key) {
  return fetch(path, { ...init, headers: { ...init.headers, 'X-Yap-Key': key } });
}

/**
 * POST bytes to an upload route (/api/image, /api/emoji, …) with `params` in
 * the query (sid, ut, …) → its JSON. Throws the server's message if it says no.
 */
export async function upload(path, body, params, type = body.type || 'application/octet-stream') {
  const res = await api(`${path}?${new URLSearchParams(params)}`, { method: 'POST', body, headers: { 'Content-Type': type } });
  const json = await res.json().catch(() => ({ ok: false, message: `Upload failed (HTTP ${res.status}).` }));
  if (!json.ok) throw new Error(json.message ?? 'Upload failed.');
  return json;
}

/** WebSocket subprotocols carrying the key: ["yap", "key.<key>"] (only letters, digits, - and _ fit in one). */
export const keyProtocols = (key = settings.key) => (/^[A-Za-z0-9_-]+$/.test(key) ? ['yap', `key.${key}`] : ['yap']);

/** The socket's address. The key rides in the subprotocol, or (when it can't) in ?key=. */
export const socketUrl = (key = settings.key) =>
  `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws${keyProtocols(key).length > 1 ? '' : `?key=${encodeURIComponent(key)}`}`;

export function save(patch = {}) {
  Object.assign(settings, patch);
  try {
    localStorage.setItem(KEY, JSON.stringify(settings));
  } catch {}
}
save();
