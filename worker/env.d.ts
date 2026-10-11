// Secrets aren't in wrangler.jsonc, so `wrangler types` can't see them.
// Plain settings (PUBLIC_URL, MINECRAFT_SERVER, BOARD, …) are read through worker/settings.ts.
interface YapSecrets {
  YAP_KEY?: string;
  TURN_KEY_ID?: string;
  TURN_KEY_API_TOKEN?: string;
  /** Any TURN relay with a fixed login (e.g. ExpressTURN's free plan): comma-separated turn: URLs. */
  TURN_URLS?: string;
  TURN_USERNAME?: string;
  TURN_CREDENTIAL?: string;
  KLIPY_KEY?: string;
  ADMIN_KEY?: string;
  /** ntfy server for phone pushes (default https://ntfy.sh) and optional access token. */
  NTFY_SERVER?: string;
  NTFY_TOKEN?: string;
  /** Server secret mixed into password (and old PIN) hashes. Keep a copy somewhere safe: a restore needs the same one. */
  PIN_PEPPER?: string;
  /** EC P-256 private JWK for browser (Web Push) notifications. */
  VAPID_PRIVATE_JWK?: string;
  /** Local tests only: hangs may start a second from now, off the half-hour grid. Never set in production. */
  TEST_HANGS?: string;
  /** Local tests only: an OBS link keeps listening this many seconds (not 5 minutes) after its account left the call. Never set in production. */
  TEST_OBS_GRACE?: string;
  /** Local tests only: POST /api/admin/test-pin gives an account an old 6-digit PIN. Never set in production. */
  TEST_PINS?: string;
}
interface Env extends YapSecrets {}
declare namespace Cloudflare {
  interface Env extends YapSecrets {}
}
