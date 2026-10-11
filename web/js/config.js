// Per-server client config from /api/config (needs the invite key), fetched
// once and shared: the KLIPY key for GIFs and the sidebar's Links.
import { api } from './settings.js';

let pending = null;

export function loadConfig() {
  pending ??= api('/api/config')
    .then((res) => {
      if (!res.ok) throw new Error(`config: HTTP ${res.status}`);
      return res.json();
    })
    .catch((err) => {
      pending = null; // retry next time
      throw err;
    });
  return pending;
}
