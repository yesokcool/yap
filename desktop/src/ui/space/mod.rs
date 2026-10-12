//! Space: the top-down world (web/js/space.js + space-map.js + space-art.js,
//! PROTOCOL.md › Space) inside the app. Walk with WASD / arrows (Shift runs)
//! or click where to go, sit with E, emote with 1–6, start tag with T (or the
//! TAG spot), throw a water balloon with F. Rooms are voice zones ("This room
//! only" by default; "Nearby is louder"; "Everyone"), eased so doorways
//! crossfade.
//!
//! Mini-games ride on `space.game`, never stored: a football on the Pitch (the
//! last kicker owns and broadcasts it), tag (whoever is "it" owns the round),
//! fire and water (`fx`: you announce your own), water balloons (`throw`; the
//! person hit decides) and race laps round the edge (`lap`; each runner times
//! their own). The floor piano and drum pads need no messages: everyone sees
//! where everyone stands. The Gallery hangs Draw's saved drawings, twelve to a
//! wall (`draw.gallery`, then each `drawing`), with a big viewer so GIFs play.
//! The numbers and rules mirror the web's, so both clients play the same game.
//!
//! Here: the state (`SpaceUi`), each frame (`show`), the camera and the voice
//! zones. The map is in `map`, the mini-games in `games`, the Gallery in
//! `gallery`, drawing the world in `art`, the radar and panels in `hud`.

use super::{Act, App, theme, theme::Pal};
use crate::core::{Cmd, Model};
use crate::audio::space_sound::voice_pan;
use crate::protocol::SavedDrawing;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::sync::{Arc, LazyLock};

mod art;
mod gallery;
mod games;
mod hud;
mod map;

use art::*;
use gallery::*;
use games::*;
use hud::*;
use map::*;

pub const WORLD_W: f32 = 2400.0;
pub const WORLD_H: f32 = 1600.0;
const SPEED: f32 = 280.0;
const RADIUS: f32 = 24.0;
const SEND_EVERY_MS: f64 = 80.0;
const T: f32 = 22.0;
const DOOR: f32 = 140.0;
const EMOTE_SECS: f32 = 3.0;
pub const EMOTES: [&str; 6] = ["👋", "😂", "❤️", "👍", "🎉", "💤"];
const LOOKS: [&str; 24] = ["🙂", "😎", "🤠", "🤖", "👻", "👽", "🐸", "🐱", "🐶", "🦊", "🐼", "🐧", "🦄", "🐙", "🐢", "🦆", "🐝", "🦖", "🍄", "🌵", "🎃", "🧙", "🥷", "🧑‍🚀"];
const PLAZA: &str = "Plaza";
/// Where the Plaza's name goes (lettering and radar).
const PLAZA_SPOT: (f32, f32) = (430.0, 830.0);
/// Where newcomers appear: x, y, spread.
const SPAWN: (f32, f32, f32) = (450.0, 900.0, 200.0);

// Mini-games.
const BALL_R: f32 = 14.0;
const FRICTION: f32 = 1.3;
const BOUNCE: f32 = 0.72;
const KICK_MAX: f32 = 1100.0;
const TAG_MS: f64 = 90_000.0;
const TAG_GRACE_MS: f64 = 2500.0;
const TAG_BEAT_MS: f64 = 3000.0;
const TAG_SILENT_MS: f64 = 10_000.0;
/// How long you burn after leaving the fire / drip after leaving the water.
const FIRE_MS: f64 = 5000.0;
const WET_MS: f64 = 6000.0;
const FX_MAX: f64 = 10_000.0;
const SHOT_SPEED: f32 = 650.0;
const SHOT_RANGE: f32 = 520.0;
const SHOT_COOLDOWN: f64 = 700.0;
const SHOT_R: f32 = 8.0;
const LAP_MAX_MS: f64 = 180_000.0;
/// Stand in front of a drawing this long and it opens up.
const LOOK_MS: f64 = 450.0;

const EDGE: u32 = 0x0d0b1e;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0).round() as u8)
}

/// `hsl(h s% l%)` (degrees, 0–1, 0–1).
fn hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let f = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(r), f(g), f(b))
}

// ── The map (web/js/space-map.js) ────────────────────────────────────────

/// Games name people by the first 8 characters of their session id.
fn short(sid: &str) -> String {
    sid.chars().take(8).collect()
}

fn short_ok(id: &str) -> bool {
    id.len() == 8 && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f' | '-'))
}

fn num(v: &Value, lo: f64, hi: f64) -> f64 {
    v.as_f64().filter(|x| x.is_finite()).unwrap_or(0.0).clamp(lo, hi)
}

fn secs(ms: f64) -> String {
    format!("{:.1} s", ms / 1000.0)
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A saved drawing worth hanging (a media id we'd fetch).
fn hangable(d: &SavedDrawing) -> bool {
    (16..=64).contains(&d.media.len()) && d.media.chars().all(|c| c.is_ascii_hexdigit())
}

// ── State ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VoiceMode {
    /// "This room only" (the default): your zone at 100 %, the rest of Space at 0 %.
    #[default]
    Room,
    Near,
    Everyone,
}

pub struct SpaceUi {
    me: Option<(f32, f32, f32)>,
    /// Others, drawn where they are now (gliding toward the server's spot), and their voice level.
    shown: HashMap<String, (f32, f32)>,
    vol: HashMap<String, f32>,
    /// …and where their voice comes from (−1 left … 1 right).
    pan: HashMap<String, f32>,
    target: Option<Pos2>,
    vel: (f32, f32),
    last_sent: f64,
    last: f64,
    pub voice: VoiceMode,
    looks_open: bool,
    ball: Ball,
    tag: Tag,
    flash: Option<(String, f64)>,
    on_spot: bool,
    state_seq: u64,
    known: HashSet<String>,
    // Fire and water.
    my_fx: MyFx,
    fx_sent: (f64, f64),
    fx: HashMap<String, (f64, f64)>,
    // Water balloons.
    shots: Vec<Shot>,
    shot_id: u32,
    last_throw: f64,
    my_splashes: u32,
    splashes: HashMap<String, u32>,
    splash_board: f64,
    // Race laps.
    race: Race,
    laps: HashMap<String, (f64, u32)>,
    // Instruments: the key or pad each person stands on; clips to play this frame.
    inst: HashMap<String, Option<String>>,
    to_play: Vec<Arc<Vec<f32>>>,
    bursts: Vec<Burst>,
    puddles: Vec<(f32, f32, f64)>,
    drip_from: HashMap<String, (f32, f32)>,
    // The Gallery: which wall you're looking at, the viewer (a drawing id), dwelling.
    page: usize,
    viewing: Option<String>,
    view_anchor: Option<(f32, f32)>,
    dwell: (i32, f64),
    dismissed: i32,
    gallery_ids: HashSet<String>,
    gallery_synced: bool,
}

impl Default for SpaceUi {
    fn default() -> Self {
        SpaceUi {
            me: None,
            shown: HashMap::new(),
            vol: HashMap::new(),
            pan: HashMap::new(),
            target: None,
            vel: (0.0, 0.0),
            last_sent: -1e9,
            last: 0.0,
            voice: VoiceMode::Room,
            looks_open: false,
            ball: Ball::default(),
            tag: Tag::default(),
            flash: None,
            on_spot: false,
            state_seq: 0,
            known: HashSet::new(),
            my_fx: MyFx::default(),
            fx_sent: (0.0, 0.0),
            fx: HashMap::new(),
            shots: Vec::new(),
            shot_id: 0,
            last_throw: -1e9,
            my_splashes: 0,
            splashes: HashMap::new(),
            splash_board: 0.0,
            race: Race::default(),
            laps: HashMap::new(),
            inst: HashMap::new(),
            to_play: Vec::new(),
            bursts: Vec::new(),
            puddles: Vec::new(),
            drip_from: HashMap::new(),
            page: 0,
            viewing: None,
            view_anchor: None,
            dwell: (-1, 0.0),
            dismissed: -1,
            gallery_ids: HashSet::new(),
            gallery_synced: false,
        }
    }
}

impl SpaceUi {
    /// Leaving Space: forget my spot (the server keeps it) and the games (the others take over).
    pub fn reset(&mut self) {
        let voice = self.voice;
        *self = SpaceUi { voice, ..Default::default() };
    }
}

/// What a frame of Space needs from outside.
struct Ctx<'a> {
    m: &'a Model,
    mine: String,
    now: f64,
    acts: &'a mut Vec<Act>,
}

impl Ctx<'_> {
    fn game(&mut self, g: &str, d: Value) {
        self.acts.push(Act::Cmd(Cmd::SpaceGame { g: g.into(), d }));
    }

    fn name(&self, sid: Option<&str>) -> Option<String> {
        let sid = sid?;
        if sid == self.mine {
            return Some(self.m.config.name.clone());
        }
        let s = self.m.sessions.get(sid)?;
        self.m.users.get(&s.uid).map(|u| u.name.clone())
    }
}

impl SpaceUi {
    fn here(&self, c: &Ctx, sid: Option<&str>) -> bool {
        sid.is_some_and(|s| s == c.mine || c.m.space.players.contains_key(s))
    }

    fn lowest_sid(&self, c: &Ctx) -> String {
        std::iter::once(c.mine.clone()).chain(c.m.space.players.keys().cloned()).min().unwrap_or_default()
    }

    /// The full sid (me or someone on Space) behind a game's 8-character id.
    fn sid_of(&self, c: &Ctx, id: Option<&str>) -> Option<String> {
        let id = id?;
        if short(&c.mine) == id {
            return Some(c.mine.clone());
        }
        c.m.space.players.keys().find(|s| short(s) == id).cloned()
    }

    fn pos_of(&self, c: &Ctx, sid: &str) -> Option<(f32, f32)> {
        if sid == c.mine {
            return self.me.map(|(x, y, _)| (x, y));
        }
        self.shown.get(sid).copied().or_else(|| c.m.space.players.get(sid).map(|p| (p.x, p.y)))
    }

    fn flash_text(&mut self, text: impl Into<String>, ms: f64, now: f64) {
        self.flash = Some((text.into(), now + ms));
    }

    fn burst(&mut self, kind: BurstKind, x: f32, y: f32, now: f64) {
        self.bursts.push(Burst { kind, x, y, at: now });
        if self.bursts.len() > 60 {
            self.bursts.remove(0);
        }
    }

    // ── Incoming ─────────────────────────────────────────────────────────

    // ── Ball ─────────────────────────────────────────────────────────────
    // Everyone rolls the ball forward locally; the last kicker's copy is the
    // truth and goes out ~10×/s while it moves, plus once when it stops.

    // ── Tag ──────────────────────────────────────────────────────────────
    // Whoever is "it" owns the round: they pass it on by touching someone and
    // re-announce it every few seconds. `t` = deciseconds each person has been
    // it; `k` goes up with every change, so the newest state wins.

    // ── Fire and water ───────────────────────────────────────────────────
    // You're the authority on yourself: your client sees you walk into the
    // fire or the water and says so (`fx`); everyone draws it until it runs out.

    // ── Water balloons ───────────────────────────────────────────────────
    // The thrower says where it starts and which way it flies; everyone flies
    // it the same way. If it hits you, your client soaks you and says who did it.

    // ── Race laps ────────────────────────────────────────────────────────
    // Each runner times their own laps and announces them; the board shows
    // the best lap of everyone here.

    // ── Instruments ──────────────────────────────────────────────────────
    // Nothing is sent: everyone sees where everyone stands, so each client
    // plays the key or pad someone steps onto, if it's in the Music room too.

    // ── Gallery ──────────────────────────────────────────────────────────
    // Paging is yours alone: nothing is sent, the walls just show another twelve.

    /// Per-person volume for Space voice, and where it comes from: in "Nearby
    /// is louder" and inside your own room, left or right by where they stand
    /// on screen (`voice_pan`). Both eased, so walking through a door
    /// crossfades and someone walking past moves across smoothly.
    fn step_voice(&mut self, c: &mut Ctx, dt: f32) {
        let Some((mx, my, _)) = self.me else { return };
        let my_zone = zone_of(mx, my);
        let ease = (dt * 8.0).min(1.0);
        let mut changes = Vec::new();
        for (sid, pos) in &c.m.space.players {
            let (x, y) = self.shown.get(sid).copied().unwrap_or((pos.x, pos.y));
            let (dx, dy) = (x - mx, y - my);
            let (target, pan_to) = match self.voice {
                VoiceMode::Everyone => (1.0, 0.0),
                VoiceMode::Near => {
                    let dist = dx.hypot(dy);
                    let v = if dist < 260.0 {
                        1.0
                    } else if dist > 1100.0 {
                        0.12
                    } else {
                        1.0 - (dist - 260.0) / 840.0 * 0.88
                    };
                    (v, voice_pan(dx, dy))
                }
                VoiceMode::Room => {
                    if zone_of(pos.x, pos.y) == my_zone {
                        (1.0, voice_pan(dx, dy))
                    } else {
                        (0.0, 0.0)
                    }
                }
            };
            let prev = self.vol.get(sid).copied();
            let mut v = match prev {
                None => target,
                Some(v) => v + (target - v) * ease,
            };
            if (target - v).abs() < 0.03 {
                v = target;
            }
            self.vol.insert(sid.clone(), v);
            let prev_pan = self.pan.get(sid).copied();
            let mut pan = match prev_pan {
                None => pan_to,
                Some(p) => p + (pan_to - p) * ease,
            };
            if (pan_to - pan).abs() < 0.02 {
                pan = pan_to;
            }
            self.pan.insert(sid.clone(), pan);
            if prev.is_none_or(|p| (p - v).abs() > 0.004) || prev_pan.is_none_or(|p| (p - pan).abs() > 0.004) {
                changes.push((sid.clone(), v, pan));
            }
        }
        if !changes.is_empty() {
            c.acts.push(Act::Cmd(Cmd::Proximity(changes)));
        }
    }

    /// E: sit on the nearest free seat (within reach), or stand up.
    fn toggle_sit(&mut self, c: &mut Ctx) {
        let Some((x, y, d)) = self.me else { return };
        if seat_at(x, y).is_some() {
            let ny = if blocked(x, y + 40.0, RADIUS) { y - 40.0 } else { y + 40.0 };
            self.me = Some((x, ny, d));
            c.acts.push(Act::Cmd(Cmd::SpaceMove { x, y: ny, d }));
            self.last_sent = c.now;
            return;
        }
        let taken = |s: &(f32, f32, f32)| c.m.space.players.values().any(|p| (p.x - s.0).abs() < 4.0 && (p.y - s.1).abs() < 4.0);
        let best = SEATS.iter().copied().filter(|s| !taken(s)).min_by(|a, b| (a.0 - x).hypot(a.1 - y).total_cmp(&(b.0 - x).hypot(b.1 - y)));
        if let Some(s) = best.filter(|s| (s.0 - x).hypot(s.1 - y) <= 110.0) {
            self.me = Some(s);
            self.target = None;
            c.acts.push(Act::Cmd(Cmd::SpaceMove { x: s.0, y: s.1, d: s.2 }));
            self.last_sent = c.now;
        }
    }
}

/// Debug builds: YAP_DEBUG_SPACE_AT=x,y puts you there when you walk in (screenshots).
fn debug_spawn() -> Option<(f32, f32)> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("YAP_DEBUG_SPACE_AT").ok()?;
    let (x, y) = v.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

pub fn show(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let now_s = ui.input(|i| i.time);
    let now = now_s * 1000.0;
    let mine = m.sid.clone().unwrap_or_default();
    let modal_open = app.modal.is_some();
    let list = gallery(m);
    let st = &mut app.space;
    let mut c = Ctx { m, mine: mine.clone(), now, acts };
    // Spawn where I stood last time (or the plaza's left wing) once the server's state is in.
    if st.me.is_none() && m.space.got_state {
        let (mut x, mut y, d) = match (&m.space.me, debug_spawn()) {
            (_, Some((x, y))) => (x, y, FRAC_PI_2),
            (Some(me), None) => (me.x, me.y, me.d),
            (None, None) => (SPAWN.0 + (rand01() - 0.5) * SPAWN.2, SPAWN.1 + (rand01() - 0.5) * SPAWN.2 / 3.0, FRAC_PI_2),
        };
        if blocked(x, y, RADIUS) {
            (x, y) = (SPAWN.0, SPAWN.1);
        }
        st.me = Some((x, y, d));
        c.acts.push(Act::Cmd(Cmd::SpaceMove { x, y, d }));
        st.last_sent = now;
    }
    // A fresh space.state: catch up on the games (whoever owns one answers), and announce ours.
    if m.space.state_seq != st.state_seq {
        st.state_seq = m.space.state_seq;
        c.game("sync", json!({}));
        st.announce_mine(&mut c);
    }
    // People who left Space.
    let gone: Vec<String> = st.known.iter().filter(|s| !m.space.players.contains_key(*s)).cloned().collect();
    for sid in gone {
        let last = st.shown.get(&sid).copied();
        st.known.remove(&sid);
        st.on_leave(&mut c, &sid, last);
    }
    st.known.extend(m.space.players.keys().cloned());
    // Mini-game updates from the others.
    for (sid, g, d) in &m.space.games {
        st.on_game(&mut c, sid, g, d);
    }
    if !m.space.games.is_empty() {
        c.acts.push(Act::ClearSpaceGames(m.space.games.len()));
    }
    // The Gallery: someone just hung a drawing (a fresh list arrives quietly).
    if !m.space.gallery_loaded {
        st.gallery_synced = false;
    } else if !st.gallery_synced {
        st.gallery_synced = true;
        st.gallery_ids = list.iter().map(|d| d.id.clone()).collect();
    } else if let Some(new) = list.iter().find(|d| !st.gallery_ids.contains(&d.id)) {
        let who = m.users.get(&new.uid).map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into());
        st.flash_text(format!("🖼 {who} hung a new drawing in the Gallery"), 3000.0, now);
        st.gallery_ids.extend(list.iter().map(|d| d.id.clone()));
    }
    let dt = ((now_s - st.last) as f32).clamp(0.0, 0.05);
    st.last = now_s;
    let typing = ui.ctx().egui_wants_keyboard_input();
    let hud_h = 40.0;
    let area = ui.available_rect_before_wrap();
    let view = Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - hud_h));
    let resp = ui.interact(view, ui.id().with("space-world"), Sense::click());
    // Keys (not while typing somewhere).
    let (mut dx, mut dy, run) = (0.0f32, 0.0f32, ui.input(|i| i.modifiers.shift));
    let viewing = st.viewing.is_some();
    if !typing && !modal_open {
        ui.input(|i| {
            if i.key_down(egui::Key::A) || (!viewing && i.key_down(egui::Key::ArrowLeft)) {
                dx -= 1.0;
            }
            if i.key_down(egui::Key::D) || (!viewing && i.key_down(egui::Key::ArrowRight)) {
                dx += 1.0;
            }
            if i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp) {
                dy -= 1.0;
            }
            if i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown) {
                dy += 1.0;
            }
        });
        if viewing {
            // The viewer: ← → page through every drawing, Esc closes.
            let (left, right, esc) = ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::Escape)));
            if esc {
                st.close_viewer(true);
            } else if left || right {
                let at = st.viewing_index(&list).unwrap_or(0) as i64;
                st.show_drawing(&list, at + if left { -1 } else { 1 });
            }
        }
        for (k, e) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6].into_iter().zip(EMOTES) {
            if ui.input(|i| i.key_pressed(k)) {
                c.acts.push(Act::Cmd(Cmd::SpaceEmote(e.into())));
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::E)) {
            st.look_or_sit(&mut c, &list);
        }
        if ui.input(|i| i.key_pressed(egui::Key::T)) {
            st.start_tag(&mut c);
        }
        if ui.input(|i| i.key_pressed(egui::Key::F) && !i.modifiers.command) {
            st.throw(&mut c);
        }
    }
    if dx != 0.0 || dy != 0.0 {
        st.target = None;
    }
    let cam = st.me.map(|(x, y, _)| camera(x, y, view)).unwrap_or(pos2(0.0, 0.0));
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        st.target = Some(pos2(cam.x + pos.x - view.left(), cam.y + pos.y - view.top()));
    }
    // Me.
    st.vel = (0.0, 0.0);
    if let Some((mut x, mut y, _)) = st.me {
        if dx == 0.0 && dy == 0.0
            && let Some(t) = st.target
        {
            let (tx, ty) = (t.x - x, t.y - y);
            let dist = tx.hypot(ty);
            if dist < 8.0 {
                st.target = None;
            } else {
                dx = tx / dist;
                dy = ty / dist;
            }
        }
        if dx != 0.0 || dy != 0.0 {
            let len = dx.hypot(dy);
            // "It" gets a hair more speed, so a chase can end.
            let it = !st.tag.over && st.tag.it.as_deref() == Some(short(&mine).as_str());
            let speed = SPEED * if run { 1.7 } else { 1.0 } * if it { 1.08 } else { 1.0 };
            st.vel = (dx / len * speed, dy / len * speed);
            let (nx, ny) = (x + st.vel.0 * dt, y + st.vel.1 * dt);
            let before = (x, y);
            if !blocked(nx, y, RADIUS) {
                x = nx;
            }
            if !blocked(x, ny, RADIUS) {
                y = ny;
            }
            let d = dy.atan2(dx);
            if st.target.is_some() && (x, y) == before {
                st.target = None; // walked into a wall
            }
            st.me = Some((x, y, d));
            if now - st.last_sent >= SEND_EVERY_MS {
                st.last_sent = now;
                c.acts.push(Act::Cmd(Cmd::SpaceMove { x, y, d }));
            }
        }
        // Stepping onto the TAG spot starts a round (you're it).
        let on_spot = (x - TAG_SPOT.0).hypot(y - TAG_SPOT.1) < TAG_SPOT.2;
        if on_spot && !st.on_spot {
            st.start_tag(&mut c);
        }
        st.on_spot = on_spot;
    }
    // Others glide toward where they are.
    for (sid, pos) in &m.space.players {
        let e = st.shown.entry(sid.clone()).or_insert((pos.x, pos.y));
        let k = (dt * 12.0).min(1.0);
        e.0 += (pos.x - e.0) * k;
        e.1 += (pos.y - e.1) * k;
    }
    st.shown.retain(|sid, _| m.space.players.contains_key(sid));
    st.step_ball(dt, &mut c);
    st.step_shots(dt, &mut c);
    st.step_effects(&mut c);
    st.step_race(&mut c);
    st.step_tag(&mut c);
    st.step_voice(&mut c, dt);
    st.step_instruments(&c);
    st.step_puddles(&c);
    st.step_gallery(&c, &list);
    // The instruments' notes (over the call, or on their own little stream).
    for clip in st.to_play.drain(..) {
        app.shared.audio.play_clip(clip, crate::audio::space_sound::GAIN, crate::audio::clips::Bus::Fx, &m.config.output_device);
    }
    let st = &mut app.space;
    // Draw.
    let painter = ui.painter().with_clip_rect(view);
    painter.rect_filled(view, 0.0, rgb(0x14122a));
    let Some((mx, my, md)) = st.me else {
        painter.text(view.center(), Align2::CENTER_CENTER, "Walking in…", theme::font(14.0), Color32::from_white_alpha(180));
        hud(app, ui, m, p, area, hud_h, acts, now);
        ui.ctx().request_repaint();
        return;
    };
    let cam = camera(mx, my, view);
    let pen = Pen { p: painter.clone(), origin: pos2(view.left() - cam.x, view.top() - cam.y) };
    let clock = deco_clock(now);
    let focus = match st.viewing_index(&list) {
        Some(i) if i / PER_WALL == st.page => (i % PER_WALL) as i32,
        _ => st.dwell.0,
    };
    let wall = Wall { items: list.iter().skip(st.page * PER_WALL).take(PER_WALL).copied().collect(), focus, page: st.page, pages: pages(list.len()), total: list.len() };
    let laps = st.lap_rows(&c);
    let env = Env { m, images: &app.images, now, clock, tag_on: !st.tag.over, score: st.ball.s, keys_down: st.inst.values().flatten().cloned().collect(), laps, wall };
    world(&pen, &env);
    for &(x, y, at) in &st.puddles {
        let age = (now - at).max(0.0);
        if age < 4500.0 {
            puddle(&pen, x, y, 0.5 * ((4500.0 - age) / 1500.0).min(1.0) as f32);
        }
    }
    ball(&pen, &st.ball);
    // Everyone, back to front.
    let mut everyone: Vec<(String, f32, f32, f32)> = m
        .space
        .players
        .iter()
        .map(|(sid, pos)| {
            let (x, y) = st.shown.get(sid).copied().unwrap_or((pos.x, pos.y));
            (sid.clone(), x, y, pos.d)
        })
        .collect();
    everyone.push((mine.clone(), mx, my, md));
    everyone.sort_by(|a, b| a.2.total_cmp(&b.2));
    let it8 = (!st.tag.over).then(|| st.tag.it.clone()).flatten();
    for (sid, x, y, d) in &everyone {
        let target = m.space.players.get(sid).map(|p| (p.x, p.y)).unwrap_or((*x, *y));
        let it = it8.as_deref() == Some(short(sid).as_str());
        let (fire, wet) = st.fx_of(sid, &mine);
        let look = Look { me: *sid == mine, sitting: seat_at(target.0, target.1).is_some(), it, burning: fire > now, wet: wet > now };
        player(&app.images, &pen, m, sid, (*x, *y), *d, look, now, clock);
    }
    for (sid, x, y, _) in &everyone {
        emote(&painter, m, sid, pen.at(*x, *y));
    }
    for s in &st.shots {
        balloon(&pen, s.x, s.y, ((1.0 - s.left / SHOT_RANGE) * PI).sin() * 26.0 + 6.0);
    }
    let spot = |sid: &str| if sid == mine { Some((mx, my)) } else { st.shown.get(sid).copied() };
    st.bursts.retain(|b| {
        let at = match &b.kind {
            BurstKind::Confetti(sid) => spot(sid),
            _ => Some((b.x, b.y)),
        };
        at.is_some_and(|(x, y)| burst(&pen, b, x, y, (now - b.at).max(0.0) as f32))
    });
    if let Some(t) = st.target {
        dashed_circle(&painter, pen.at(t.x, t.y), 14.0, 6.0, 6.0, Stroke::new(1.5, Color32::from_white_alpha(150)));
    }
    radar(&painter, m, st, (mx, my, md), cam, view, it8.as_deref(), now);
    game_line(st, &c, &painter, view);
    room_panel(st, ui, m, p, view, (mx, my), &list, c.acts, now);
    viewer(st, ui, m, p, view, &list, &app.images, app.h12);
    hud(app, ui, m, p, area, hud_h, acts, now);
    ui.ctx().request_repaint();
}

fn rand01() -> f32 {
    let b = crate::config::random_hex(4);
    u16::from_str_radix(&b, 16).unwrap_or(0) as f32 / 65535.0
}

/// Decorative motion: stepped (4 fps), like the rest of Windows 98.
fn deco_clock(now: f64) -> f64 {
    (now / 250.0).floor() * 250.0
}

fn camera(x: f32, y: f32, view: Rect) -> Pos2 {
    let (w, h) = (view.width(), view.height());
    let mut cx = (x - w / 2.0).clamp(0.0, (WORLD_W - w).max(0.0));
    let mut cy = (y - h / 2.0).clamp(0.0, (WORLD_H - h).max(0.0));
    if w > WORLD_W {
        cx = (WORLD_W - w) / 2.0;
    }
    if h > WORLD_H {
        cy = (WORLD_H - h) / 2.0;
    }
    pos2(cx, cy)
}

// ── Drawing (web/js/space-art.js) ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_matches_the_web() {
        assert_eq!(ROOMS.len(), 9);
        assert_eq!(MID, (1490.0, 1272.0));
        assert_eq!(NET_X, [1182.0, 1798.0]);
        assert_eq!(POSTS.len(), 4);
        assert!(!blocked(MID.0, MID.1, BALL_R));
        assert!(blocked(NET_X[0] + 10.0, MID.1 - NET_HALF - 4.0, 2.0), "the top-left post");
        // Seats: 2 per sofa (2), 3 per couch (4), 2 per bench (4), the dock's 2, 1 per stool (5) and log (4).
        assert_eq!(SEATS.len(), 2 * 2 + 4 * 3 + 4 * 2 + 2 + 5 + 4);
        assert_eq!(zone_of(430.0, 830.0), -1, "the Plaza's left wing");
        assert_eq!(zone_name(zone_of(1200.0, 800.0)), "Gallery");
        assert_eq!(zone_name(zone_of(1490.0, 1272.0)), "Pitch");
        assert_eq!(zone_name(zone_of(1520.0, 300.0)), "Music");
        assert_eq!(zone_name(zone_of(870.0, 1300.0)), "Pond");
        assert_eq!(zone_name(zone_of(2100.0, 1272.0)), "Campfire");
        assert_eq!(net_at(NET_X[0] + 5.0, MID.1), 0);
        assert_eq!(net_at(NET_X[1] - 5.0, MID.1), 1);
        assert_eq!(net_at(MID.0, MID.1), -1);
        assert!(short_ok("1a2b3c4d") && !short_ok("1A2B3C4D") && !short_ok("abc"));
        // The Gallery's doors: open in the middle of each end wall.
        assert!(!blocked(GALLERY.x + 5.0, GALLERY.y + GALLERY.h / 2.0, RADIUS));
        assert!(blocked(GALLERY.x + 5.0, GALLERY.y + 40.0, RADIUS));
        assert_eq!(plaque_spots(&ROOMS[4]).len(), 2);
        // Twelve frames: six top at y 664, six bottom at y 843, x = 774 + 146·i, 122 × 80.
        assert_eq!(FRAMES.len(), 12);
        for (i, f) in FRAMES.iter().enumerate() {
            assert_eq!((f.x, f.w, f.h), (774.0 + 146.0 * (i % 6) as f32, 122.0, 80.0));
            assert_eq!(f.y, if i < 6 { 664.0 } else { 843.0 });
        }
        assert_eq!(frame_at(835.0, 790.0).map(|f| f.i), Some(0));
        assert_eq!(frame_at(835.0, 820.0).map(|f| f.i), Some(6));
        assert!(frame_at(1200.0, 700.0).is_none());
    }

    #[test]
    fn fire_water_track_instruments() {
        assert!(on_fire(2100.0, 1272.0) && !on_fire(2140.0, 1272.0));
        assert!(in_water(340.0, 1272.0), "the fountain");
        assert!(in_water(800.0, 1330.0), "wading in the pond");
        assert!(!in_water(870.0, 1200.0), "on the dock");
        assert!(!in_water(870.0, 1290.0), "the end of the dock");
        assert!(on_track(50.0, 800.0) && on_track(1200.0, 1550.0) && !on_track(430.0, 830.0));
        assert_eq!(instrument_at(1330.0, 280.0).map(|h| (h.id, h.midi)), Some(("w0".into(), Some(60))));
        assert_eq!(instrument_at(1360.0, 200.0).map(|h| (h.id, h.midi)), Some(("b0".into(), Some(61))));
        assert_eq!(instrument_at(1360.0, 280.0).map(|h| h.midi), Some(Some(62)), "below the black keys");
        assert_eq!(instrument_at(1700.0, 200.0).map(|h| h.midi), Some(Some(79)));
        assert_eq!(instrument_at(1480.0, 420.0).and_then(|h| h.drum), Some("snare"));
        assert!(instrument_at(1520.0, 345.0).is_none());
        assert_eq!(hsl(0.0, 0.9, 0.7), Color32::from_rgb(247, 110, 110));
    }
}
