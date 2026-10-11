// Bytes ↔ text, and comparing secrets: one copy for the whole Worker. No
// imports, so the modules Node tests load can use it (with `.ts` on the import).

/** base64url without padding (tokens, salts, keys, Web Push). */
export function b64urlEncode(bytes: Uint8Array): string {
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b); // no spread: fine for big buffers
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function b64urlDecode(text: string): Uint8Array {
  const s = atob(text.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((text.length + 3) % 4));
  return Uint8Array.from(s, (c) => c.charCodeAt(0));
}

export const hex = (bytes: Uint8Array): string => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');

/** Is `presented` exactly `actual`? In constant time; never true for an empty `actual`. */
export function sameSecret(presented: string, actual: string): boolean {
  const a = new TextEncoder().encode(presented);
  const b = new TextEncoder().encode(actual);
  return b.byteLength > 0 && a.byteLength === b.byteLength && crypto.subtle.timingSafeEqual(a, b);
}
