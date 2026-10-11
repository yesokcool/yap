// Which Yap server the admin tools (backup, restore, import-emojis) talk to,
// and its admin key (the Worker's ADMIN_KEY secret).
//   Server:    YAP_URL, else "PUBLIC_URL" in your wrangler.jsonc's "vars".
//   Admin key: YAP_ADMIN_KEY, else (on a Mac) the Keychain item named by
//              YAP_KEYCHAIN_ADMIN (default "yap-admin-key"):
//              security add-generic-password -s yap-admin-key -a yap -w
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, keychain } from './lib.mjs';

export function serverUrl() {
  if (process.env.YAP_URL) return process.env.YAP_URL.replace(/\/+$/, '');
  try {
    const text = fs.readFileSync(path.join(ROOT, 'wrangler.jsonc'), 'utf8').replace(/^\s*\/\/.*$/gm, '');
    const url = /"PUBLIC_URL"\s*:\s*"([^"]+)"/.exec(text)?.[1];
    if (url) return url.replace(/\/+$/, '');
  } catch {
    // No wrangler.jsonc: fall through.
  }
  throw new Error('Which server? Set YAP_URL, or "PUBLIC_URL" in the "vars" of your wrangler.jsonc.');
}

export function adminKey() {
  if (process.env.YAP_ADMIN_KEY) return process.env.YAP_ADMIN_KEY;
  const item = process.env.YAP_KEYCHAIN_ADMIN || 'yap-admin-key';
  const fromKeychain = keychain(item);
  if (fromKeychain) return fromKeychain;
  throw new Error(`No admin key: set YAP_ADMIN_KEY${process.platform === 'darwin' ? `, or add the Keychain item "${item}"` : ''}.`);
}
