// Release builds on Windows are GUI apps (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Yap desktop: a native, lightweight client for Yap (text + voice).
//! `yap` opens the window; `yap --headless …` runs without one (tests).

mod audio;
mod config;
mod core;
mod drawing;
mod headless;
mod http;
mod klipy;
mod net;
mod notify;
mod picture;
mod protocol;
mod ptt;
mod screen;
mod ui;
mod update;
mod vault;
mod voice;

use config::InputMode;

const HELP: &str = "\
yap — native desktop client for Yap

USAGE:
  yap [--config FILE] [--server URL|INVITE] [--key KEY]
  yap --headless --server URL [options]

OPTIONS (window):
  --config FILE        Use this settings file instead of the default one
  --server URL         Server or invite link (https://host/#key=K)
  --key KEY            Invite key

OPTIONS (headless test mode):
  --headless           No window, no audio devices; logs events
  --name NAME          Display name (default Bot)
  --color #RRGGBB      Name color
  --say TEXT           Send a message once connected (repeatable: all at once, so
                       more than 10 meet the server's limit; SEND retry lines)
  --reply-to-missing TEXT  Send TEXT as a reply to a message that doesn't exist: the
                       server refuses it (SEND refused / RESTORED lines)
  --channel NAME       Text channel for --say (default: first)
  --join-voice NAME    Join this voice channel (id or name)
  --input-wav FILE     Use a WAV file (looped) as the microphone
  --record-wav FILE    Record what we hear (48 kHz stereo WAV)
  --bitrate BPS        Opus bitrate (default 128000)
  --mode open|vad|ptt  Transmit mode (default open)
  --denoise            Enable RNNoise (default off in headless)
  --expect TEXT        Report when a message containing TEXT arrives
  --mute-after SECS    Toggle mute after SECS (tests mute propagation)
  --drop-socket-at S   Drop the socket S seconds in (comma-separated for more);
                       the call is kept across it (RESUME / GHOST / CHIME lines)
  --hear-yourself WAV  Once in voice, run Settings › Voice › Hear yourself and save
                       what came out of the codec (HEAR lines)
  --approve-all        Let everyone in from the waiting room (test doorman)
  --pin PIN            Sign in with --name and this 6-digit PIN (prints TOKEN)
  --signup             With --pin: create the account (lands in the waiting room)
  --react EMOJI        With --react-to: react with this emoji
  --react-to TEXT      ...to every incoming message containing TEXT
  --ping TEXT          Ping everyone once connected
  --send-gif QUERY     Search KLIPY, send the first GIF, report the share
  --edit-to TEXT       Edit our first message (as echoed) to TEXT
  --send-image FILE    Send a picture (shrunk/re-encoded to fit 1.9 MB)
  --add-link NAME=URL  Add a sidebar Place (https only); logs LINKS updates
  --make-invite LABEL  Make a personal invite link (prints INVITE <label> <link>)
  --channel-bitrate N  On joining voice, set the channel's quality (0 = personal)
  --reply-to TEXT      Reply to the first message from someone else containing TEXT
  --reply-body TEXT    ...with this (default: got it)
  --vault PASSCODE     Personal space: encrypt it (sealing earlier notes) or unlock it;
                       --say/--send-image then go there, encrypted; notes log as NOTE
  --search Q           Search everywhere (--search-here: this channel); prints SEARCH
  --jump-oldest        ...then jump to the oldest result (loads older pages); JUMPED
  --pin-msg TEXT       Pin the first message containing TEXT; prints PINNED / PINS
  --draw               Open the drawing board, draw a line (top layer), a box and a word, save the GIF (DRAW)
  --space              Walk into Space: move, wave, kick, throw a balloon, catch fire, run a lap (SPACE)
  --poll Q|A|B         Post a poll (POLL lines log polls and votes; --say '/roll 2d6' rolls dice)
  --vote N             Vote for option N in the first poll someone else posts
  --away TEXT          Set an away message (AWAY lines: auto-replies we get)
  --hang TITLE         Plan a hang (--hang-in SECS: that far ahead; default 1 h) (HANG)
  --sound-file WAV     Upload a soundboard clip (--sound-name NAME) (SOUND)
  --play-sound NAME    In voice, play the soundboard clip called NAME once
  --chime ID           Set your join chime (a built-in id)
  --shelf              Ask for the channel's pictures and GIFs (SHELF)
  --unread             Ask what's new everywhere (UNREAD)
  --last-read CH=MS    Start as if channel CH was read up to MS (repeatable; the
                       counts on ready then say what came since: UNREAD, MENTIONS)
  --save-msg TEXT      Save the first message containing TEXT to the Personal space
  --doodle-on TEXT     React with a (generated) doodle to the first message containing TEXT
  --quiet              Snooze notifications for an hour (QUIET)
  --share-screen [Q]   Once in voice, share a synthetic test pattern (never the real
                       screen) at quality Q: smooth (default), sharp or max (SCREEN)
  --share-sound        ...with a test tone as the share's sound
  --watch-screen       Watch the first person sharing in our voice channel (SCREEN
                       frames lines; the SUMMARY says how many pictures decoded, and their size)
  --delete-after SECS  Delete our last picture (else last message) SECS in (encrypted: with its blob)
  --token TOKEN        Sign in with a device token (one of --pin / --token is needed)
  --duration SECS      Exit after SECS (default: run until killed)
  --list-devices       Print audio devices and exit
  --update-check       With --server: is a newer signed release available?
                       (prints UPDATE off when built without an update key)
  --update-install     ...and install it (verify, swap, relaunch with the same args)
";

/// Release builds on Windows have no console; reuse the parent's (cmd/PowerShell)
/// so `--headless`, `--help` and `--list-devices` print somewhere.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call; failing (no parent console) is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    if std::env::args().skip(1).any(|a| matches!(a.as_str(), "--headless" | "-h" | "--help" | "-V" | "--version" | "--list-devices")) {
        attach_console();
    }
    let mut args = std::env::args().skip(1).peekable();
    let mut headless = false;
    let mut h = headless::Args::default();
    let mut config_path: Option<std::path::PathBuf> = None;
    let mut server: Option<String> = None;
    let mut key: Option<String> = None;
    let mut list_devices = false;
    let mut update_mode: Option<bool> = None;
    let need = |args: &mut std::iter::Peekable<std::iter::Skip<std::env::Args>>, flag: &str| -> String {
        args.next().unwrap_or_else(|| {
            eprintln!("{flag} needs a value");
            std::process::exit(2)
        })
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return;
            }
            "-V" | "--version" => {
                println!("yap {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--headless" => headless = true,
            "--config" => config_path = Some(need(&mut args, "--config").into()),
            "--server" => server = Some(need(&mut args, "--server")),
            "--key" => key = Some(need(&mut args, "--key")),
            "--name" => h.name = need(&mut args, "--name"),
            "--color" => h.color = Some(need(&mut args, "--color")),
            "--say" => h.say.push(need(&mut args, "--say")),
            "--reply-to-missing" => h.reply_to_missing = Some(need(&mut args, "--reply-to-missing")),
            "--channel" => h.channel = Some(need(&mut args, "--channel")),
            "--join-voice" => h.join_voice = Some(need(&mut args, "--join-voice")),
            "--input-wav" => h.input_wav = Some(need(&mut args, "--input-wav")),
            "--record-wav" => h.record_wav = Some(need(&mut args, "--record-wav")),
            "--bitrate" => h.bitrate = need(&mut args, "--bitrate").parse().ok(),
            "--duration" => h.duration = need(&mut args, "--duration").parse().ok(),
            "--mute-after" => h.mute_after = need(&mut args, "--mute-after").parse().ok(),
            "--drop-socket-at" => h.drop_socket_at = need(&mut args, "--drop-socket-at").split(',').filter_map(|s| s.trim().parse().ok()).collect(),
            "--hear-yourself" => h.hear_yourself = Some(need(&mut args, "--hear-yourself")),
            "--expect" => h.expect = Some(need(&mut args, "--expect")),
            "--denoise" => h.denoise = true,
            "--approve-all" => h.approve_all = true,
            "--pin" => h.pin = Some(need(&mut args, "--pin")),
            "--signup" => h.signup = true,
            "--token" => h.token = Some(need(&mut args, "--token")),
            "--speakers" => h.speakers = true,
            "--react" => h.react = Some(need(&mut args, "--react")),
            "--react-to" => h.react_to = Some(need(&mut args, "--react-to")),
            "--ping" => h.ping = Some(need(&mut args, "--ping")),
            "--send-gif" => h.send_gif = Some(need(&mut args, "--send-gif")),
            "--edit-to" => h.edit_to = Some(need(&mut args, "--edit-to")),
            "--send-image" => h.send_image = Some(need(&mut args, "--send-image")),
            "--add-link" => h.add_link = Some(need(&mut args, "--add-link")),
            "--make-invite" => h.make_invite = Some(need(&mut args, "--make-invite")),
            "--reply-to" => h.reply_to = Some(need(&mut args, "--reply-to")),
            "--reply-body" => h.reply_body = Some(need(&mut args, "--reply-body")),
            "--vault" => h.vault = Some(need(&mut args, "--vault")),
            "--search" => h.search = Some(need(&mut args, "--search")),
            "--search-here" => h.search_here = true,
            "--jump-oldest" => h.jump_oldest = true,
            "--pin-msg" => h.pin_msg = Some(need(&mut args, "--pin-msg")),
            "--draw" => h.draw = true,
            "--space" => h.space = true,
            "--poll" => h.poll = Some(need(&mut args, "--poll")),
            "--vote" => h.vote = need(&mut args, "--vote").parse().ok(),
            "--away" => h.away = Some(need(&mut args, "--away")),
            "--hang" => h.hang = Some(need(&mut args, "--hang")),
            "--hang-in" => h.hang_in = need(&mut args, "--hang-in").parse().ok(),
            "--sound-file" => h.sound_file = Some(need(&mut args, "--sound-file")),
            "--sound-name" => h.sound_name = Some(need(&mut args, "--sound-name")),
            "--play-sound" => h.play_sound = Some(need(&mut args, "--play-sound")),
            "--chime" => h.chime = Some(need(&mut args, "--chime")),
            "--shelf" => h.shelf = true,
            "--unread" => h.unread = true,
            "--last-read" => {
                let v = need(&mut args, "--last-read");
                if let Some((ch, ms)) = v.split_once('=')
                    && let Ok(ms) = ms.parse()
                {
                    h.last_read.push((ch.to_string(), ms));
                }
            }
            "--save-msg" => h.save_msg = Some(need(&mut args, "--save-msg")),
            "--doodle-on" => h.doodle_on = Some(need(&mut args, "--doodle-on")),
            "--quiet" => h.quiet = true,
            "--share-screen" => {
                // An optional quality after it (smooth / sharp / max).
                h.share_screen = Some(args.next_if(|a| crate::screen::Quality::parse(a).is_some()).unwrap_or_else(|| "smooth".into()));
            }
            "--share-sound" => h.share_sound = true,
            "--watch-screen" => h.watch_screen = true,
            "--delete-after" => h.delete_after = need(&mut args, "--delete-after").parse().ok(),
            "--channel-bitrate" => h.channel_bitrate = need(&mut args, "--channel-bitrate").parse().ok(),
            "--update-check" => update_mode = Some(false),
            "--update-install" => update_mode = Some(true),
            "--mode" => {
                h.mode = Some(match need(&mut args, "--mode").as_str() {
                    "vad" => InputMode::Vad,
                    "ptt" => InputMode::Ptt,
                    _ => InputMode::Open,
                })
            }
            "--list-devices" => list_devices = true,
            other => {
                eprintln!("unknown argument {other:?} (try --help)");
                std::process::exit(2);
            }
        }
    }

    update::cleanup();
    let default_filter = "warn,yap=info,rtc=error,webrtc=error";
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter)).format_timestamp_millis().init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    log::info!("yap {} ({}): {}", update::current_version(), update::platform(), update::summary());

    if list_devices {
        let (inputs, outputs) = audio::list_devices();
        println!("Inputs:");
        for d in inputs {
            println!("  {d}");
        }
        println!("Outputs:");
        for d in outputs {
            println!("  {d}");
        }
        return;
    }

    if let Some(install) = update_mode {
        let (srv, _) = config::parse_invite(server.as_deref().unwrap_or(""));
        std::process::exit(update_cli(&srv, install));
    }

    if headless {
        h.server = server.unwrap_or_default();
        h.key = key;
        std::process::exit(headless::run(h));
    }

    let path = config_path.unwrap_or_else(config::Config::default_path);
    let mut cfg = config::Config::load(&path);
    if let Some(s) = server {
        let (srv, k) = config::parse_invite(&s);
        cfg.server = srv;
        if let Some(k) = k {
            cfg.key = k;
        }
    }
    if let Some(k) = key {
        cfg.key = k;
    }
    if let Err(e) = ui::run(cfg, path) {
        eprintln!("yap: {e}");
        std::process::exit(1);
    }
    if update::should_relaunch() {
        update::relaunch();
    }
}

/// `--update-check` / `--update-install` (scripts and tests).
fn update_cli(server: &str, install: bool) -> i32 {
    if !update::enabled() {
        println!("yap {} ({}): UPDATE off: this build has no update key; get new versions from {server}/download or build it again", update::current_version(), update::platform());
        return i32::from(install);
    }
    println!("yap {} ({}), checking {}", update::current_version(), update::platform(), update::manifest_url(server));
    match update::check(server) {
        Ok(None) => {
            println!("UPDATE none");
            0
        }
        Ok(Some(a)) => {
            println!("UPDATE available {} ({} bytes) {}", a.version, a.file.size, a.url);
            if !install {
                return 0;
            }
            match update::install(&a, |_, _| {}) {
                Ok(exe) => {
                    println!("UPDATE installed {} at {}; relaunching", a.version, exe.display());
                    update::relaunch();
                }
                Err(e) => {
                    println!("UPDATE failed: {e}");
                    1
                }
            }
        }
        Err(e) => {
            println!("UPDATE check failed: {e}");
            1
        }
    }
}
