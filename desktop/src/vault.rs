//! Encrypted Personal space — byte-compatible with web/js/vault.js
//! (PROTOCOL.md › Encrypted Personal space).
//!
//!  space key K   32 random bytes, AES-256-GCM, made once
//!  wrap          PBKDF2-HMAC-SHA256(NFKC(passcode), salt, iter) → KEK;
//!                AES-GCM(KEK, iv, K, AAD "yap:wrap:v1") — stored on the server
//!  note          "e1:" + b64url(iv ‖ AES-GCM(K, iv, JSON {text, gif?, image?}, AAD "yap:note:v1"))
//!  picture       iv ‖ AES-GCM(K, iv, bytes, AAD "yap:blob:v1"), uploaded as opaque bytes
//!
//! The key is kept in the OS keychain (`keyring`), tagged with the wrap's
//! `ct` so a reset on another device is noticed.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

pub const ITERATIONS: u32 = 600_000;
const AAD_WRAP: &[u8] = b"yap:wrap:v1";
const AAD_NOTE: &[u8] = b"yap:note:v1";
const AAD_BLOB: &[u8] = b"yap:blob:v1";

pub type Key = [u8; 32];

/// The wrapped space key, as the server stores it (`ready.vault`, `vault.set`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Wrap {
    pub v: u32,
    pub iter: u32,
    pub salt: String,
    pub iv: String,
    pub ct: String,
}

/// A picture inside an encrypted note.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NoteImage {
    pub id: String,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub mime: String,
}

/// What an encrypted note says.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Note {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif: Option<crate::protocol::Gif>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<NoteImage>,
}

pub fn b64u(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Lenient like the web's: padding (if any) is ignored.
pub fn unb64u(text: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).map_err(|_| "bad base64".to_string())
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("system randomness");
    b
}

fn kek(passcode: &str, salt: &[u8], iterations: u32) -> Key {
    let normalized: String = passcode.nfkc().collect();
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(normalized.as_bytes(), salt, iterations, &mut out);
    out
}

fn seal(key: &Key, aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let iv: [u8; 12] = random();
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let ct = cipher.encrypt(Nonce::from_slice(&iv), Payload { msg: plain, aad }).expect("AES-GCM");
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ct);
    out
}

fn open(key: &Key, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < 12 + 16 {
        return Err("too short".into());
    }
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    cipher.decrypt(Nonce::from_slice(&sealed[..12]), Payload { msg: &sealed[12..], aad }).map_err(|_| "can't open it".to_string())
}

fn wrap_raw(raw: &Key, passcode: &str, iterations: u32) -> Wrap {
    let salt: [u8; 16] = random();
    let iv: [u8; 12] = random();
    let k = kek(passcode, &salt, iterations);
    let cipher = Aes256Gcm::new_from_slice(&k).expect("32-byte key");
    let ct = cipher.encrypt(Nonce::from_slice(&iv), Payload { msg: raw, aad: AAD_WRAP }).expect("AES-GCM");
    Wrap { v: 1, iter: iterations, salt: b64u(&salt), iv: b64u(&iv), ct: b64u(&ct) }
}

/// A brand-new space key, wrapped with `passcode`.
pub fn create(passcode: &str, iterations: u32) -> (Wrap, Key) {
    let raw: Key = random();
    (wrap_raw(&raw, passcode, iterations), raw)
}

/// Unwrap with the passcode; Err("wrong") if it doesn't fit.
pub fn unlock(wrap: &Wrap, passcode: &str) -> Result<Key, String> {
    let salt = unb64u(&wrap.salt)?;
    let iv = unb64u(&wrap.iv)?;
    let ct = unb64u(&wrap.ct)?;
    if iv.len() != 12 {
        return Err("bad wrap".into());
    }
    let k = kek(passcode, &salt, wrap.iter);
    let cipher = Aes256Gcm::new_from_slice(&k).expect("32-byte key");
    let raw = cipher.decrypt(Nonce::from_slice(&iv), Payload { msg: &ct, aad: AAD_WRAP }).map_err(|_| "wrong".to_string())?;
    raw.try_into().map_err(|_| "bad key".to_string())
}

/// Same key, new passcode (the notes don't change).
pub fn rewrap(wrap: &Wrap, old: &str, new: &str) -> Result<Wrap, String> {
    let raw = unlock(wrap, old)?;
    Ok(wrap_raw(&raw, new, wrap.iter.max(ITERATIONS)))
}

pub fn is_note(body: &str) -> bool {
    body.len() > 3 && body.starts_with("e1:") && body[3..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn encrypt_note(key: &Key, note: &Note) -> String {
    let json = serde_json::to_vec(note).expect("note JSON");
    format!("e1:{}", b64u(&seal(key, AAD_NOTE, &json)))
}

pub fn decrypt_note(key: &Key, body: &str) -> Result<Note, String> {
    let bytes = unb64u(body.get(3..).unwrap_or(""))?;
    let plain = open(key, AAD_NOTE, &bytes)?;
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

pub fn encrypt_blob(key: &Key, data: &[u8]) -> Vec<u8> {
    seal(key, AAD_BLOB, data)
}

pub fn decrypt_blob(key: &Key, data: &[u8]) -> Result<Vec<u8>, String> {
    open(key, AAD_BLOB, data)
}

/// Passcode rules for setting one (unlock takes anything).
pub fn passcode_problem(pass: &str, again: &str) -> Option<&'static str> {
    if pass.chars().count() < 8 {
        Some("Use at least 8 characters (a short phrase is easiest to remember).")
    } else if pass.chars().all(|c| c.is_ascii_digit()) && pass.chars().count() < 12 {
        Some("Numbers alone are easy to guess. Add some words.")
    } else if pass != again {
        Some("Those two don't match.")
    } else {
        None
    }
}

// ── This device's copy of the key (OS keychain) ─────────────────────────────

#[derive(Serialize, Deserialize)]
struct Stored {
    key: String,
    ct: String,
}

fn entry(server: &str, uid: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("Yap Personal space", &format!("{server} {uid}")).map_err(|e| e.to_string())
}

static NO_KEYCHAIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keep the key in memory only (headless test bots; screenshot runs): never
/// touch the OS keychain.
pub fn disable_keychain() {
    NO_KEYCHAIN.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn keychain_off() -> bool {
    NO_KEYCHAIN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Keychain calls on a plain thread (the Secret Service backend runs its own executor).
fn on_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    std::thread::Builder::new().name("yap-keychain".into()).spawn(f).ok()?.join().ok()
}

/// Remember K for this account, tagged with the wrap it came from. Err = no keychain here.
pub fn remember(server: &str, uid: &str, key: &Key, ct: &str) -> Result<(), String> {
    if keychain_off() {
        return Err("keychain off".into());
    }
    let secret = serde_json::to_string(&Stored { key: b64u(key), ct: ct.to_string() }).unwrap_or_default();
    let (server, uid) = (server.to_string(), uid.to_string());
    on_thread(move || entry(&server, &uid)?.set_password(&secret).map_err(|e| e.to_string())).unwrap_or_else(|| Err("keychain thread".into()))
}

/// The remembered K, if it belongs to this wrap (a reset elsewhere makes it stale: forgotten).
pub fn recall(server: &str, uid: &str, ct: &str) -> Option<Key> {
    if keychain_off() {
        return None;
    }
    let (s, u) = (server.to_string(), uid.to_string());
    let secret = on_thread(move || entry(&s, &u).ok()?.get_password().ok()).flatten()?;
    let stored: Stored = serde_json::from_str(&secret).ok()?;
    if stored.ct != ct {
        forget(server, uid);
        return None;
    }
    unb64u(&stored.key).ok()?.try_into().ok()
}

pub fn forget(server: &str, uid: &str) {
    if keychain_off() {
        return;
    }
    let (s, u) = (server.to_string(), uid.to_string());
    on_thread(move || {
        if let Ok(e) = entry(&s, &u) {
            let _ = e.delete_credential();
        }
    });
}

/// Sealed notes `(id, body)` as `personal.seal` batches that each fit one
/// WebSocket frame (PROTOCOL.md: the server refuses frames over 384 KB, and
/// more than 50 notes): at most `max_bytes` of JSON and `max_notes` notes per
/// batch, like web/js/vault.js › sealBatches (300 KB, 50). A note too big for
/// any frame comes back in the second list.
pub type SealNote = (String, String);
pub fn seal_batches(notes: Vec<SealNote>, max_bytes: usize, max_notes: usize) -> (Vec<Vec<SealNote>>, Vec<SealNote>) {
    let mut batches = Vec::new();
    let mut too_big = Vec::new();
    let mut batch: Vec<SealNote> = Vec::new();
    let mut bytes = 0;
    for note in notes {
        // `{"id":…,"body":…}` and a comma (ASCII: ids and base64url bodies).
        let size = serde_json::json!({ "id": note.0, "body": note.1 }).to_string().len() + 1;
        if size > max_bytes {
            too_big.push(note);
            continue;
        }
        if !batch.is_empty() && (bytes + size > max_bytes || batch.len() >= max_notes) {
            batches.push(std::mem::take(&mut batch));
            bytes = 0;
        }
        batch.push(note);
        bytes += size;
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    (batches, too_big)
}

#[cfg(test)]
mod tests {
    /// Batches by size and count, like the web's sealBatches (test/vault.test.mjs).
    #[test]
    fn seal_batches_fit_one_frame() {
        use super::seal_batches;
        let note = |i: usize, len: usize| (format!("m{i}"), "e".repeat(len));
        // 8 long notes (~40,000 characters sealed is ~53 KB) can't all go in one 300 KB frame.
        let (b, big) = seal_batches((0..8).map(|i| note(i, 53_000)).collect(), 300_000, 50);
        assert!(big.is_empty());
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![5, 3]);
        for batch in &b {
            assert!(serde_json::json!({ "t": "personal.seal", "notes": batch.iter().map(|(id, body)| serde_json::json!({ "id": id, "body": body })).collect::<Vec<_>>() }).to_string().len() <= 300_100);
        }
        // Small ones: at most 50 to a batch, in order.
        let (b, _) = seal_batches((0..120).map(|i| note(i, 10)).collect(), 300_000, 50);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![50, 50, 20]);
        assert_eq!(b[1][0].0, "m50");
        // One too big for any frame is set aside (counted as not sealed); the rest still go.
        let (b, big) = seal_batches(vec![note(0, 10), note(1, 400_000), note(2, 10)], 300_000, 50);
        assert_eq!(big.len(), 1);
        assert_eq!(big[0].0, "m1");
        assert_eq!(b, vec![vec![note(0, 10), note(2, 10)]]);
        assert_eq!(seal_batches(vec![], 300_000, 50), (vec![], vec![]));
    }

    use super::*;

    const FAST: u32 = 1_000;

    #[test]
    fn note_round_trip_and_wrong_key() {
        let (wrap, key) = create("purple tiger river", FAST);
        assert_eq!(unb64u(&wrap.salt).unwrap().len(), 16);
        assert_eq!(unb64u(&wrap.iv).unwrap().len(), 12);
        assert_eq!(unb64u(&wrap.ct).unwrap().len(), 48);
        let note = Note { text: "wifi at the cabin is on the fridge".into(), ..Default::default() };
        let body = encrypt_note(&key, &note);
        assert!(is_note(&body) && !body.contains("cabin"));
        assert_eq!(decrypt_note(&key, &body).unwrap(), note);
        assert_eq!(unlock(&wrap, "purple tiger river").unwrap(), key);
        assert!(unlock(&wrap, "purple tiger rivers").is_err());
        let (_, other) = create("another one entirely", FAST);
        assert!(decrypt_note(&other, &body).is_err());
    }

    #[test]
    fn rewrap_keeps_the_key_and_blobs_round_trip() {
        let (wrap, key) = create("purple tiger river", FAST);
        let next = rewrap(&wrap, "purple tiger river", "green ocean lamp").unwrap();
        assert!(unlock(&next, "purple tiger river").is_err());
        assert_eq!(unlock(&next, "green ocean lamp").unwrap(), key);
        assert!(next.iter >= ITERATIONS);
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 31) as u8).collect();
        let sealed = encrypt_blob(&key, &data);
        assert_eq!(sealed.len(), 12 + 5000 + 16);
        assert_eq!(decrypt_blob(&key, &sealed).unwrap(), data);
    }

    #[test]
    fn nfkc_and_passcode_rules() {
        // "ﬁ" (U+FB01) normalizes to "fi" under NFKC, like String.normalize('NFKC').
        let (wrap, _) = create("ﬁsh and chips", FAST);
        assert!(unlock(&wrap, "fish and chips").is_ok());
        assert!(passcode_problem("short", "short").is_some());
        assert!(passcode_problem("12345678901", "12345678901").is_some());
        assert!(passcode_problem("123456789012", "123456789012").is_none());
        assert!(passcode_problem("purple tiger", "purple tigre").is_some());
        assert!(passcode_problem("purple tiger", "purple tiger").is_none());
    }

    /// Cross-check the other way: YAP_VAULT_WRITE=<path> writes a fixture (600k
    /// iterations, like the web) for web/js/vault.js to open.
    #[test]
    fn writes_for_the_web() {
        let Ok(path) = std::env::var("YAP_VAULT_WRITE") else { return };
        let passcode = "grüne Ölampe ﬁsh 42";
        let t = std::time::Instant::now();
        let (wrap, key) = create(passcode, ITERATIONS);
        let made_ms = t.elapsed().as_millis();
        let t = std::time::Instant::now();
        assert_eq!(unlock(&wrap, passcode).unwrap(), key);
        let unlock_ms = t.elapsed().as_millis();
        let text = "from Rust: the spare key is under the blue pot 🔑";
        let gif = crate::protocol::Gif { provider: "klipy".into(), slug: "cat".into(), title: "Cat".into(), width: 10.0, height: 20.0, ..Default::default() };
        let note = encrypt_note(&key, &Note { text: text.into(), gif: Some(gif), image: Some(NoteImage { id: "a".repeat(32), width: 640.0, height: 480.0, mime: "image/jpeg".into() }) });
        let plain: Vec<u8> = (0..3000u32).map(|i| (i * 7 + 3) as u8).collect();
        let blob = encrypt_blob(&key, &plain);
        let out = serde_json::json!({ "wrap": wrap, "passcode": passcode, "note": note, "text": text, "blob": b64u(&blob), "plainBlob": b64u(&plain), "createMs": made_ms, "unlockMs": unlock_ms });
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }

    /// Cross-check with web/js/vault.js: YAP_VAULT_FIXTURE=<json from Node>.
    #[test]
    fn opens_what_the_web_wrote() {
        let Ok(path) = std::env::var("YAP_VAULT_FIXTURE") else { return };
        let f: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let wrap: Wrap = serde_json::from_value(f["wrap"].clone()).unwrap();
        let key = unlock(&wrap, f["passcode"].as_str().unwrap()).unwrap();
        let note = decrypt_note(&key, f["note"].as_str().unwrap()).unwrap();
        assert_eq!(note.text, f["text"].as_str().unwrap());
        let blob = unb64u(f["blob"].as_str().unwrap()).unwrap();
        assert_eq!(b64u(&decrypt_blob(&key, &blob).unwrap()), f["plainBlob"].as_str().unwrap());
    }
}
