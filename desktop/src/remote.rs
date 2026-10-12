//! `yap --remote mute|unmute|toggle-mute|deafen|undeafen|toggle-deafen|status`:
//! mute and deafen the running app from a script, a Stream Deck or a
//! Wayland compositor's own shortcut (where Yap's global keys can't see the
//! keyboard). It talks to the app over a Unix socket that only you can reach:
//! a file next to the app's settings (or in `$XDG_RUNTIME_DIR` on Linux),
//! owner-only, refusing any other user even if they got to it. Nothing
//! listens on the network. macOS and Linux only; on Windows the global keys
//! see everything anyway.
//!
//! One line in ("mute\n"), one line back ("muted"), then it closes.
//! `YAP_REMOTE=<path>` moves the socket (both ends must agree).
#![cfg_attr(not(unix), allow(dead_code))]

use std::path::{Path, PathBuf};

/// What a remote can ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    Mute,
    Unmute,
    ToggleMute,
    Deafen,
    Undeafen,
    ToggleDeafen,
    /// Just say how things are.
    Status,
}

impl Remote {
    pub fn parse(s: &str) -> Option<Remote> {
        Some(match s.trim() {
            "mute" => Remote::Mute,
            "unmute" => Remote::Unmute,
            "toggle-mute" => Remote::ToggleMute,
            "deafen" => Remote::Deafen,
            "undeafen" => Remote::Undeafen,
            "toggle-deafen" => Remote::ToggleDeafen,
            "status" => Remote::Status,
            _ => return None,
        })
    }
}

/// The words `--help` lists.
pub const COMMANDS: &str = "mute, unmute, toggle-mute, deafen, undeafen, toggle-deafen, status";

/// (muted, deafened) after a command, by the Mute and Deafen buttons' own
/// rules: unmuting undeafens too (you can't talk while deafened), deafening
/// leaves mute as it was underneath, muting while deafened changes nothing.
pub fn next(cmd: Remote, muted: bool, deafened: bool) -> (bool, bool) {
    match cmd {
        Remote::Mute => (true, deafened),
        Remote::Unmute => (false, false),
        Remote::ToggleMute if deafened => (false, false),
        Remote::ToggleMute => (!muted, false),
        Remote::Deafen => (muted, true),
        Remote::Undeafen => (muted, false),
        Remote::ToggleDeafen => (muted, !deafened),
        Remote::Status => (muted, deafened),
    }
}

/// The answer: how things are now ("muted", "deafened", "unmuted"), and whether you're in a call.
pub fn reply(muted: bool, deafened: bool, in_call: bool) -> String {
    let state = if deafened {
        "deafened"
    } else if muted {
        "muted"
    } else {
        "unmuted"
    };
    format!("{state}, {}", if in_call { "in a call" } else { "not in a call" })
}

/// Where the app with this settings file listens: `YAP_REMOTE`, else
/// `yap-remote-<8 hex of the settings path's SHA-256>.sock` (so a second
/// identity with `--config` has its own) in `$XDG_RUNTIME_DIR` on Linux,
/// else next to the settings file.
pub fn socket_path(config: &Path) -> PathBuf {
    if let Some(p) = std::env::var_os("YAP_REMOTE").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    use sha2::Digest;
    let hash = sha2::Sha256::digest(config.to_string_lossy().as_bytes());
    let name = format!("yap-remote-{}.sock", hash.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>());
    let runtime = if cfg!(target_os = "linux") { std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir()) } else { None };
    runtime.unwrap_or_else(|| config.parent().map(Path::to_path_buf).unwrap_or_default()).join(name)
}

pub type Handler = std::sync::Arc<dyn Fn(Remote) -> String + Send + Sync>;

#[cfg(unix)]
pub use unix::{listen, send};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::Duration;

    /// A Unix socket path can't be longer than this (macOS's sun_path; Linux allows 108).
    const MAX_PATH: usize = 103;

    /// Listening; the socket file goes when this is dropped.
    pub struct Listener {
        path: PathBuf,
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn uid() -> u32 {
        // SAFETY: geteuid has no preconditions and can't fail.
        unsafe { libc::geteuid() }
    }

    /// Who's on the other end (their user id).
    fn peer_uid(s: &UnixStream) -> Option<u32> {
        use std::os::fd::AsRawFd;
        #[cfg(target_os = "linux")]
        {
            let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
            let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: SO_PEERCRED fills a ucred of the size given.
            let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
            (r == 0).then_some(cred.uid)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let (mut euid, mut egid) = (0, 0);
            // SAFETY: getpeereid writes the two ids.
            let r = unsafe { libc::getpeereid(s.as_raw_fd(), &mut euid, &mut egid) };
            (r == 0).then_some(euid)
        }
    }

    /// Start listening at `path` (see `socket_path`). Refuses a path that's
    /// too long, a place it can't make private, or another running Yap's.
    pub fn listen(path: &Path, handler: Handler) -> Result<Listener, String> {
        if path.as_os_str().len() > MAX_PATH {
            return Err(format!("{} is too long for a socket; set YAP_REMOTE to a shorter path", path.display()));
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            // Only ever replace a socket of our own that nothing answers on (a crash's leftover).
            if !meta.file_type().is_socket() || meta.uid() != uid() {
                return Err(format!("{} is in the way (not a socket of yours)", path.display()));
            }
            if UnixStream::connect(path).is_ok() {
                return Err("another Yap with these settings is already listening".into());
            }
            std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let listener = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // Owner only (connecting needs write permission on it).
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{}: {e}", path.display()))?;
        let me = uid();
        let _ = std::thread::Builder::new().name("yap-remote".into()).spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { continue };
                if peer_uid(&s) != Some(me) {
                    log::warn!("remote: refused a connection from another user");
                    continue;
                }
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let _ = BufReader::new((&s).take(64)).read_line(&mut line);
                let answer = match Remote::parse(&line) {
                    Some(cmd) => handler(cmd),
                    None => format!("error: unknown command {:?} (try {COMMANDS})", line.trim()),
                };
                let _ = writeln!(s, "{answer}");
            }
        });
        Ok(Listener { path: path.to_path_buf() })
    }

    /// Send one command to the app listening at `path`; its answer.
    pub fn send(path: &Path, cmd: &str) -> Result<String, String> {
        let mut s = UnixStream::connect(path).map_err(|e| format!("no Yap is listening at {} ({e}); is it running with these settings?", path.display()))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
        writeln!(s, "{cmd}").map_err(|e| e.to_string())?;
        let mut answer = String::new();
        BufReader::new(s).read_line(&mut answer).map_err(|e| e.to_string())?;
        Ok(answer.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_follow_the_buttons_rules() {
        assert_eq!(Remote::parse("mute\n"), Some(Remote::Mute));
        assert_eq!(Remote::parse("toggle-deafen"), Some(Remote::ToggleDeafen));
        assert_eq!(Remote::parse("shout"), None);
        assert_eq!(next(Remote::Mute, false, false), (true, false));
        assert_eq!(next(Remote::Unmute, true, true), (false, false), "unmuting undeafens");
        assert_eq!(next(Remote::ToggleMute, false, true), (false, false), "toggling mute while deafened: both off, like the button");
        assert_eq!(next(Remote::ToggleMute, true, false), (false, false));
        assert_eq!(next(Remote::Deafen, false, false), (false, true), "mute stays as it was underneath");
        assert_eq!(next(Remote::Undeafen, true, true), (true, false));
        assert_eq!(next(Remote::ToggleDeafen, false, true), (false, false));
        assert_eq!(next(Remote::Status, true, false), (true, false));
        assert_eq!(reply(true, false, true), "muted, in a call");
        assert_eq!(reply(true, true, false), "deafened, not in a call");
        assert_eq!(reply(false, false, true), "unmuted, in a call");
    }

    #[test]
    fn each_settings_file_has_its_own_socket() {
        let a = socket_path(Path::new("/home/ada/.config/yap/config.json"));
        let b = socket_path(Path::new("/tmp/other.json"));
        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("yap-remote-"));
    }

    /// End to end on a real socket in a private temporary folder: a command
    /// goes in and its answer comes back; junk gets an error; a second app
    /// can't take a live socket over; the file goes when the listener does.
    #[cfg(unix)]
    #[test]
    fn a_command_over_the_socket() {
        use std::sync::{Arc, Mutex};
        let dir = std::env::temp_dir().join(format!("yr-{}", crate::config::random_hex(4)));
        let path = dir.join("r.sock");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let listener = listen(&path, Arc::new(move |cmd| {
            s.lock().unwrap().push(cmd);
            reply(cmd == Remote::Mute, false, true)
        }))
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "owner only");
        assert_eq!(send(&path, "mute").unwrap(), "muted, in a call");
        assert!(send(&path, "sing").unwrap().starts_with("error: unknown command"));
        assert_eq!(*seen.lock().unwrap(), vec![Remote::Mute]);
        assert!(listen(&path, Arc::new(|_| String::new())).is_err(), "a live socket isn't taken over");
        drop(listener);
        assert!(!path.exists(), "gone with the listener");
        assert!(send(&path, "mute").is_err(), "nothing listening");
        // A file that isn't a socket is left alone; a leftover socket (a crash) is replaced.
        std::fs::write(&path, b"not a socket").unwrap();
        assert!(listen(&path, Arc::new(|_| String::new())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");
        std::fs::remove_file(&path).unwrap();
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists(), "a dead socket left behind");
        let again = listen(&path, Arc::new(|_| "back".into())).unwrap();
        assert_eq!(send(&path, "status").unwrap(), "back");
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
