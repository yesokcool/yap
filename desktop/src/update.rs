//! Signed click-to-update.
//!
//! `<server>/downloads/latest.json` lists one file per platform with its size,
//! SHA-256 and an Ed25519 signature over the raw bytes. We check at launch and
//! every 6 h; installing happens only when the person clicks: download next to
//! the executable, verify size + hash + signature against the key built into
//! this binary, swap the executable (`self_replace`), relaunch with the same
//! arguments. Debug builds accept `YAP_UPDATE_URL` / `YAP_UPDATE_PUBKEY` so the
//! whole path can be tested against a local server; release builds ignore them.
//!
//! The key is a build setting: `YAP_UPDATE_PUBKEY=<base64> cargo build`
//! (`scripts/build-cross.sh` also reads `update-pubkey.txt`). A build without
//! one never checks for updates and has no install button; new versions come
//! from its server's `/download` page, or from building it again.

use base64::Engine;
use serde::Deserialize;
use sha2::Digest;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The Ed25519 public key (raw 32 bytes, base64) this build trusts for
/// updates, from `YAP_UPDATE_PUBKEY` at build time. `None`: no self-update.
const BUILD_KEY: Option<&str> = option_env!("YAP_UPDATE_PUBKEY");

/// Shown where updates live (Settings › About, the Updates dialog) when the
/// build doesn't update itself.
pub const OFF_TEXT: &str = "This build doesn't update itself. Get new versions from your server's download page, or build Yap again.";

/// Set once an update is installed; `main` relaunches after the window closes.
pub static RELAUNCH: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize, Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub sig: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub files: HashMap<String, FileEntry>,
}

/// An update for this platform that is newer than us.
#[derive(Clone, Debug)]
pub struct Available {
    pub version: String,
    pub date: String,
    pub notes: Vec<String>,
    pub url: String,
    pub file: FileEntry,
}

/// The manifest key for this build.
pub fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        _ => "unknown",
    }
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn debug_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) { std::env::var(name).ok().filter(|v| !v.is_empty()) } else { None }
}

pub fn manifest_url(server: &str) -> String {
    debug_env("YAP_UPDATE_URL").unwrap_or_else(|| format!("{server}/downloads/latest.json"))
}

/// The run-time override (debug builds) wins over the built-in key; blank = none.
fn pick_key(built: Option<&str>, runtime: Option<String>) -> Option<String> {
    runtime.or_else(|| built.map(str::to_string)).map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

fn key_b64() -> Option<String> {
    pick_key(BUILD_KEY, debug_env("YAP_UPDATE_PUBKEY"))
}

/// Does this build update itself? Only with an update key (built in, or a
/// debug build's run-time `YAP_UPDATE_PUBKEY`).
pub fn enabled() -> bool {
    key_b64().is_some()
}

/// One line for the log: is self-update on, and with which key.
pub fn summary() -> String {
    match key_b64() {
        Some(k) => match decode_key(&k) {
            Ok(_) => format!("self-update on (key {}…)", k.chars().take(8).collect::<String>()),
            Err(e) => format!("self-update on, but {e}: every update will be refused"),
        },
        None => "self-update off: built without an update key (new versions come from the server's /download page)".into(),
    }
}

fn decode_key(b64: &str) -> Result<Vec<u8>, String> {
    let key = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| format!("bad update key: {e}"))?;
    if key.len() != 32 {
        return Err("bad update key length".into());
    }
    Ok(key)
}

fn public_key() -> Result<Vec<u8>, String> {
    decode_key(&key_b64().ok_or("This build doesn't update itself (no update key).")?)
}

/// `a` > `b` as semver (pre-release versions sort before their release).
pub fn newer(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, Option<String>) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (v, None),
        };
        let mut nums: Vec<u64> = core.split('.').map(|x| x.parse().unwrap_or(0)).collect();
        nums.resize(3, 0);
        (nums, pre)
    }
    let (an, ap) = parse(a);
    let (bn, bp) = parse(b);
    match an.cmp(&bn) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (ap, bp) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(concat!("yap-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Resolve the manifest's file path against the manifest URL's origin.
fn file_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let origin_end = manifest_url.find("://").map(|i| i + 3).and_then(|s| manifest_url[s..].find('/').map(|j| s + j)).unwrap_or(manifest_url.len());
    let origin = &manifest_url[..origin_end];
    if path.starts_with('/') {
        format!("{origin}{path}")
    } else {
        let dir = manifest_url.rsplit_once('/').map(|(d, _)| d).unwrap_or(origin);
        format!("{dir}/{path}")
    }
}

/// Fetch the manifest; `Some` if it offers a newer version for this platform.
/// Never asks the server when this build doesn't update itself.
pub fn check(server: &str) -> Result<Option<Available>, String> {
    if !enabled() {
        return Ok(None);
    }
    let url = manifest_url(server);
    let mut res = agent(Duration::from_secs(15)).get(&url).call().map_err(|e| format!("Can't check for updates: {e}"))?;
    if res.status() == 404 {
        return Ok(None); // nothing published yet
    }
    if res.status() != 200 {
        return Err(format!("Update check failed (HTTP {}).", res.status()));
    }
    let m: Manifest = res.body_mut().read_json().map_err(|e| format!("Update manifest unreadable: {e}"))?;
    if !newer(&m.version, current_version()) {
        return Ok(None);
    }
    let Some(file) = m.files.get(platform()).cloned() else { return Ok(None) };
    Ok(Some(Available { url: file_url(&url, &file.path), version: m.version, date: m.date, notes: m.notes, file }))
}

/// Check bytes against the manifest entry: size, SHA-256, Ed25519 signature.
pub fn verify(bytes: &[u8], file: &FileEntry) -> Result<(), String> {
    verify_with(bytes, file, &public_key()?)
}

fn verify_with(bytes: &[u8], file: &FileEntry, key: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != file.size {
        return Err(format!("Update rejected: size {} ≠ {}.", bytes.len(), file.size));
    }
    let hash: String = sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if !hash.eq_ignore_ascii_case(file.sha256.trim()) {
        return Err("Update rejected: SHA-256 doesn't match.".into());
    }
    let sig = base64::engine::general_purpose::STANDARD.decode(file.sig.trim()).map_err(|_| "Update rejected: bad signature encoding.".to_string())?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(bytes, &sig)
        .map_err(|_| "Update rejected: the signature doesn't match this build's update key.".to_string())
}

/// Where our executable lives, as it was at startup. Remembered on first use
/// (`cleanup()` runs first thing in main), because after the swap Linux's
/// /proc/self/exe names the replaced file ("yap (deleted)").
fn exe_path() -> Result<PathBuf, String> {
    static EXE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    EXE.get_or_init(|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.canonicalize().unwrap_or(exe);
        // Started from an already-replaced file: drop Linux's marker.
        let s = exe.to_string_lossy();
        Ok(match s.strip_suffix(" (deleted)") {
            Some(clean) => PathBuf::from(clean),
            None => exe,
        })
    })
    .clone()
}

/// Download, verify and swap in the new executable. `progress(done, total)`.
pub fn install(update: &Available, progress: impl Fn(u64, u64)) -> Result<PathBuf, String> {
    let exe = exe_path()?;
    let dir = exe.parent().ok_or("Can't find Yap's folder.")?.to_path_buf();
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("yap").to_string();
    let tmp = dir.join(format!(".{name}.update-{}.tmp", crate::config::random_hex(4)));
    let result = (|| -> Result<(), String> {
        let mut res = agent(Duration::from_secs(600)).get(&update.url).call().map_err(|e| format!("Download failed: {e}"))?;
        if res.status() != 200 {
            return Err(format!("Download failed (HTTP {}).", res.status()));
        }
        let mut reader = res.body_mut().as_reader();
        let mut bytes = Vec::with_capacity(update.file.size as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() as u64 > update.file.size + 1024 * 1024 {
                return Err("Update rejected: bigger than announced.".into());
            }
            progress(bytes.len() as u64, update.file.size);
        }
        verify(&bytes, &update.file)?;
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("Can't write next to Yap ({}): {e}", dir.display()))?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        // Keep the old one around until the next start (Windows can't delete a running exe).
        let old = old_path(&exe);
        let _ = std::fs::remove_file(&old);
        if cfg!(windows) {
            std::fs::rename(&exe, &old).map_err(|e| format!("Can't move the old version aside: {e}"))?;
            if let Err(e) = std::fs::rename(&tmp, &exe) {
                let _ = std::fs::rename(&old, &exe);
                return Err(format!("Can't put the new version in place: {e}"));
            }
        } else {
            let _ = std::fs::hard_link(&exe, &old);
            self_replace::self_replace(&tmp).map_err(|e| format!("Can't put the new version in place: {e}"))?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&tmp);
    result.map(|_| exe)
}

fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Remove what a previous update left behind (`yap.old`, stray temp files).
pub fn cleanup() {
    let Ok(exe) = exe_path() else { return };
    let _ = std::fs::remove_file(old_path(&exe));
    if let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) {
        let prefix = format!(".{name}.update-");
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Start the (new) executable with our arguments, then exit.
pub fn relaunch() -> ! {
    let exe = exe_path().unwrap_or_else(|_| PathBuf::from("yap"));
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let Err(e) = std::process::Command::new(&exe).args(&args).spawn() {
        eprintln!("yap: couldn't restart after updating ({e}); start it again yourself.");
    }
    std::process::exit(0);
}

pub fn mark_relaunch() {
    RELAUNCH.store(true, Ordering::SeqCst);
}

pub fn should_relaunch() -> bool {
    RELAUNCH.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(newer("0.2.0", "0.2.0-beta.1"));
        assert!(!newer("0.2.0-beta.1", "0.2.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
        assert!(newer("v0.10.0", "0.9.0"));
    }

    #[test]
    fn urls() {
        assert_eq!(file_url("https://yap.example/downloads/latest.json", "/downloads/yap.exe"), "https://yap.example/downloads/yap.exe");
        assert_eq!(file_url("http://localhost:8799/latest.json", "yap-new"), "http://localhost:8799/yap-new");
        assert_eq!(file_url("http://h/x/latest.json", "https://cdn/y"), "https://cdn/y");
    }

    fn test_key() -> ring::signature::Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    #[test]
    fn verifies_signatures() {
        use ring::signature::KeyPair;
        let (kp, other) = (test_key(), test_key());
        let data = b"new yap binary";
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let hash: String = sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
        let entry = FileEntry { path: "/x".into(), size: data.len() as u64, sha256: hash, sig: b64(kp.sign(data).as_ref()) };
        let key = kp.public_key().as_ref().to_vec();
        // Good against its own key, refused against another one…
        assert!(verify_with(data, &entry, &key).is_ok());
        assert!(verify_with(data, &entry, other.public_key().as_ref()).unwrap_err().contains("signature"));
        let mut wrong = entry.clone();
        wrong.sha256 = "00".repeat(32);
        assert!(verify_with(data, &wrong, &key).unwrap_err().contains("SHA-256"));
        let mut short = entry.clone();
        short.size += 1;
        assert!(verify_with(data, &short, &key).unwrap_err().contains("size"));
        // …and never accepted by this build unless it was built with that key.
        assert!(verify(data, &entry).is_err());
        // A key built in must be a raw 32-byte Ed25519 key.
        if let Some(k) = BUILD_KEY.filter(|k| !k.trim().is_empty()) {
            assert_eq!(decode_key(k).unwrap().len(), 32);
        }
    }

    #[test]
    fn update_key_is_a_build_setting() {
        // No key built in and none at run time: no self-update.
        assert_eq!(pick_key(None, None), None);
        assert_eq!(pick_key(Some("  "), None), None);
        // The built-in key, or a debug run's override over it.
        assert_eq!(pick_key(Some("built="), None).as_deref(), Some("built="));
        assert_eq!(pick_key(Some("built="), Some("runtime=".into())).as_deref(), Some("runtime="));
        assert_eq!(pick_key(None, Some(" runtime= ".into())).as_deref(), Some("runtime="));
        assert!(decode_key("not base64!").is_err());
        assert!(decode_key("AAAA").unwrap_err().contains("length"));
        assert_eq!(enabled(), key_b64().is_some());
        if !enabled() {
            // What a keyless build says, and that it never asks the server.
            assert!(summary().starts_with("self-update off"));
            assert!(matches!(check("http://127.0.0.1:9"), Ok(None)));
        }
    }
}
