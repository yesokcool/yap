//! The mini-games over `space.game` (PROTOCOL.md › Space): the ball, tag,
//! fire and water, water balloons, laps, and the floor instruments.

use super::*;

pub(super) struct Ball {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) vx: f32,
    pub(super) vy: f32,
    pub(super) k: u64,
    pub(super) s: [u32; 2],
    pub(super) owner: Option<String>,
    /// Drawing offset that glides to 0 after a correction from the network.
    pub(super) ox: f32,
    pub(super) oy: f32,
    pub(super) sent_at: f64,
    pub(super) dirty: bool,
    pub(super) in_net: i32,
    pub(super) goal: Option<i32>,
    pub(super) reset_at: f64,
    pub(super) kick_at: f64,
}

impl Default for Ball {
    fn default() -> Self {
        Ball { x: MID.0, y: MID.1, vx: 0.0, vy: 0.0, k: 0, s: [0, 0], owner: None, ox: 0.0, oy: 0.0, sent_at: -1e9, dirty: false, in_net: -1, goal: None, reset_at: 0.0, kick_at: -1e9 }
    }
}

pub(super) struct Tag {
    pub(super) it: Option<String>,
    pub(super) by: Option<String>,
    pub(super) k: u64,
    pub(super) end_at: f64,
    pub(super) t: HashMap<String, u64>,
    pub(super) at: f64,
    pub(super) since: f64,
    pub(super) heard: f64,
    pub(super) beat: f64,
    pub(super) over: bool,
    pub(super) result_until: f64,
}

impl Default for Tag {
    fn default() -> Self {
        Tag { it: None, by: None, k: 0, end_at: 0.0, t: HashMap::new(), at: 0.0, since: 0.0, heard: 0.0, beat: 0.0, over: true, result_until: 0.0 }
    }
}

/// My own effects: until when I burn / drip; fire can't relight me before `safe`.
#[derive(Default, Clone, Copy)]
pub(super) struct MyFx {
    pub(super) fire: f64,
    pub(super) wet: f64,
    pub(super) safe: f64,
}

/// A water balloon in flight.
pub(super) struct Shot {
    pub(super) sid: String,
    pub(super) id: u32,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) vx: f32,
    pub(super) vy: f32,
    pub(super) left: f32,
}

/// My lap of the track: when it started (0 = not running), the next corner.
#[derive(Default)]
pub(super) struct Race {
    pub(super) start: f64,
    pub(super) cp: usize,
    pub(super) best: f64,
    pub(super) laps: u32,
    pub(super) last_y: Option<f32>,
    pub(super) off_at: f64,
}

#[derive(Clone)]
pub(super) enum BurstKind {
    Steam,
    Splash,
    Note(&'static str, Color32),
    /// Over whoever kicked the goal in.
    Confetti(String),
}

pub(super) struct Burst {
    pub(super) kind: BurstKind,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) at: f64,
}

impl SpaceUi {
    pub(super) fn on_game(&mut self, c: &mut Ctx, sid: &str, g: &str, d: &Value) {
        if sid == c.mine || !d.is_object() {
            return;
        }
        match g {
            "sync" => self.answer_sync(c),
            "ball" => self.on_ball(sid, d, c),
            "tag" => self.on_tag(sid, d, c),
            "fx" => self.on_fx(sid, d, c),
            "throw" => self.on_throw(sid, d),
            "lap" => self.on_lap(sid, d, c),
            _ => {}
        }
    }

    pub(super) fn answer_sync(&mut self, c: &mut Ctx) {
        if self.ball.owner.as_deref() == Some(c.mine.as_str()) {
            self.ball.dirty = true; // goes out on the next tick
        }
        if !self.tag.over && self.tag.it.as_deref() == Some(short(&c.mine).as_str()) {
            self.send_tag(c);
        }
        self.announce_mine(c);
    }

    /// Our own effects, splash count and best lap, for whoever just arrived.
    pub(super) fn announce_mine(&mut self, c: &mut Ctx) {
        if self.my_fx.fire > c.now || self.my_fx.wet > c.now || self.my_splashes > 0 {
            self.send_fx(c, None);
        }
        if self.race.best > 0.0 {
            c.game("lap", json!({ "best": self.race.best.round() as i64, "n": self.race.laps }));
        }
    }

    /// Someone left Space: whatever they owned goes to the lowest sid still here.
    pub(super) fn on_leave(&mut self, c: &mut Ctx, sid: &str, last: Option<(f32, f32)>) {
        self.shown.remove(sid);
        self.vol.remove(sid);
        self.fx.remove(sid);
        self.laps.remove(sid);
        self.splashes.remove(sid);
        self.inst.remove(sid);
        self.drip_from.remove(sid);
        if self.ball.owner.as_deref() == Some(sid) {
            self.ball.owner = None;
            if self.lowest_sid(c) == c.mine {
                self.ball.owner = Some(c.mine.clone());
                self.ball.dirty = true;
            }
        }
        if !self.tag.over && self.tag.it.as_deref() == Some(short(sid).as_str()) {
            self.reassign_it(c, last);
        }
    }

    pub(super) fn step_ball(&mut self, dt: f32, c: &mut Ctx) {
        self.kick(c.now, &c.mine);
        let moving = self.roll_ball(dt);
        let settle = (-dt * 10.0).exp();
        let b = &mut self.ball;
        b.ox *= settle;
        b.oy *= settle;
        let net = net_at(b.x, b.y);
        let entered = net >= 0 && b.in_net < 0;
        b.in_net = net;
        if b.owner.as_deref() != Some(c.mine.as_str()) {
            return;
        }
        if entered {
            // Into Red's net (right) scores for Blue, and the other way round.
            b.s[(1 - net) as usize] += 1;
            b.vx = 0.0;
            b.vy = 0.0;
            b.goal = Some(net);
            b.reset_at = c.now + 1800.0;
            b.dirty = true;
            self.goal_flash(net, c.now);
        }
        let b = &mut self.ball;
        if b.reset_at > 0.0 && c.now >= b.reset_at {
            b.x = MID.0;
            b.y = MID.1;
            b.vx = 0.0;
            b.vy = 0.0;
            b.reset_at = 0.0;
            b.in_net = -1;
            b.dirty = true;
        }
        if moving == Some(false) {
            b.dirty = true;
        }
        if (b.dirty && c.now - b.sent_at >= 70.0) || (moving == Some(true) && c.now - b.sent_at >= 100.0) {
            self.send_ball(c);
        }
    }

    /// Walking into the ball kicks it: harder the faster you're going into it.
    pub(super) fn kick(&mut self, now: f64, mine: &str) {
        let Some((mx, my, md)) = self.me else { return };
        let (vx, vy) = self.vel;
        let b = &mut self.ball;
        if now - b.kick_at < 150.0 || seat_at(mx, my).is_some() {
            return;
        }
        let (dx, dy) = (b.x - mx, b.y - my);
        let dist = dx.hypot(dy);
        if dist >= RADIUS + BALL_R {
            return;
        }
        let (nx, ny) = if dist > 0.5 { (dx / dist, dy / dist) } else { (md.cos(), md.sin()) };
        let into = (vx * nx + vy * ny).max(0.0);
        let power = (150.0 + into * 1.9 + b.vx.hypot(b.vy) * 0.3).min(KICK_MAX);
        b.vx = nx * power;
        b.vy = ny * power;
        let (px, py) = (mx + nx * (RADIUS + BALL_R + 1.0), my + ny * (RADIUS + BALL_R + 1.0));
        if !blocked(px, py, BALL_R) {
            b.ox += b.x - px;
            b.oy += b.y - py;
            b.x = px;
            b.y = py;
        }
        b.k += 1;
        b.owner = Some(mine.to_string());
        b.kick_at = now;
        b.dirty = true;
        b.reset_at = 0.0;
    }

    /// Physics: friction and bouncy walls. Some(true) = moving, Some(false) = just stopped.
    pub(super) fn roll_ball(&mut self, dt: f32) -> Option<bool> {
        let b = &mut self.ball;
        if b.vx == 0.0 && b.vy == 0.0 {
            return None;
        }
        let steps = ((b.vx.hypot(b.vy) * dt) / 8.0).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            // Bounce only when moving into a wall, so a ball overlapping one can get out.
            let free = !blocked(b.x, b.y, BALL_R);
            let nx = b.x + b.vx * h;
            if free && blocked(nx, b.y, BALL_R) {
                b.vx *= -BOUNCE;
            } else {
                b.x = nx;
            }
            let ny = b.y + b.vy * h;
            if free && blocked(b.x, ny, BALL_R) {
                b.vy *= -BOUNCE;
            } else {
                b.y = ny;
            }
        }
        b.x = b.x.clamp(BALL_R, WORLD_W - BALL_R);
        b.y = b.y.clamp(BALL_R, WORLD_H - BALL_R);
        let f = (-FRICTION * dt).exp();
        b.vx *= f;
        b.vy *= f;
        if b.vx.hypot(b.vy) < 12.0 {
            b.vx = 0.0;
            b.vy = 0.0;
            return Some(false);
        }
        Some(true)
    }

    pub(super) fn send_ball(&mut self, c: &mut Ctx) {
        let b = &mut self.ball;
        b.sent_at = c.now;
        b.dirty = false;
        let mut d = json!({ "x": b.x.round() as i64, "y": b.y.round() as i64, "vx": b.vx.round() as i64, "vy": b.vy.round() as i64, "k": b.k, "s": b.s });
        if let Some(goal) = b.goal.take() {
            d["goal"] = json!(goal);
        }
        c.game("ball", d);
    }

    pub(super) fn on_ball(&mut self, sid: &str, d: &Value, c: &mut Ctx) {
        let k = num(&d["k"], 0.0, 1e9).floor() as u64;
        let owner = self.ball.owner.clone();
        // Same k from someone else: two kicks at once (lower sid wins) or an adoption after the owner left.
        if k < self.ball.k || (k == self.ball.k && owner.as_deref() != Some(sid) && self.here(c, owner.as_deref()) && owner.as_deref().is_some_and(|o| sid > o)) {
            return;
        }
        let b = &mut self.ball;
        let x = num(&d["x"], BALL_R as f64, (WORLD_W - BALL_R) as f64) as f32;
        let y = num(&d["y"], BALL_R as f64, (WORLD_H - BALL_R) as f64) as f32;
        // Keep drawing it where it was and glide over, rather than jump.
        let (rx, ry) = (b.x + b.ox, b.y + b.oy);
        b.x = x;
        b.y = y;
        b.vx = num(&d["vx"], -KICK_MAX as f64, KICK_MAX as f64) as f32;
        b.vy = num(&d["vy"], -KICK_MAX as f64, KICK_MAX as f64) as f32;
        b.k = k;
        b.owner = Some(sid.to_string());
        b.reset_at = 0.0;
        b.in_net = net_at(x, y);
        let far = (rx - x).hypot(ry - y) > 200.0;
        b.ox = if far { 0.0 } else { rx - x };
        b.oy = if far { 0.0 } else { ry - y };
        if let Some(s) = d["s"].as_array().filter(|s| s.len() >= 2) {
            b.s = [num(&s[0], 0.0, 999.0) as u32, num(&s[1], 0.0, 999.0) as u32];
        }
        if let Some(goal) = d["goal"].as_i64().filter(|g| *g == 0 || *g == 1) {
            self.goal_flash(goal as i32, c.now);
        }
    }

    pub(super) fn goal_flash(&mut self, net: i32, now: f64) {
        let [blue, red] = self.ball.s;
        self.flash_text(format!("⚽ GOAL for {}! Blue {blue} – {red} Red", if net == 1 { "Blue" } else { "Red" }), 3500.0, now);
        // Confetti over whoever kicked it in.
        if let Some(o) = self.ball.owner.clone() {
            self.burst(BurstKind::Confetti(o), 0.0, 0.0, now);
        }
    }

    pub(super) fn start_tag(&mut self, c: &mut Ctx) {
        if self.me.is_none() || !self.tag.over {
            return;
        }
        if c.m.space.players.is_empty() {
            return self.flash_text("Tag needs someone else on Space.", 3000.0, c.now);
        }
        let k = (self.tag.k + 1).max(unix_secs());
        self.tag = Tag { it: Some(short(&c.mine)), by: None, k, end_at: c.now + TAG_MS, t: HashMap::new(), at: c.now, since: c.now, heard: c.now, beat: 0.0, over: false, result_until: 0.0 };
        self.send_tag(c);
        self.flash_text("You're it! Touch someone to pass it on.", 2500.0, c.now);
    }

    pub(super) fn step_tag(&mut self, c: &mut Ctx) {
        if self.tag.over {
            return;
        }
        if c.now >= self.tag.end_at {
            return self.end_tag(c);
        }
        let mine8 = short(&c.mine);
        if self.tag.it.as_deref() != Some(mine8.as_str()) {
            if c.now - self.tag.heard > TAG_SILENT_MS {
                self.reassign_it(c, None);
            }
            return;
        }
        if c.now - self.tag.beat >= TAG_BEAT_MS {
            self.send_tag(c);
        }
        let Some((mx, my, _)) = self.me else { return };
        if c.now - self.tag.since < 600.0 {
            return;
        }
        let candidates: Vec<String> = c.m.space.players.keys().cloned().collect();
        for sid in candidates {
            let Some((px, py)) = self.pos_of(c, &sid) else { continue };
            if (px - mx).hypot(py - my) > RADIUS * 2.0 + 4.0 {
                continue;
            }
            let target = c.m.space.players.get(&sid).map(|p| (p.x, p.y)).unwrap_or((px, py));
            if seat_at(target.0, target.1).is_some() {
                continue; // sitting is safe
            }
            if self.tag.by.as_deref() == Some(short(&sid).as_str()) && c.now - self.tag.since < TAG_GRACE_MS {
                continue; // no tag-backs
            }
            self.fold_tag_time(c.now);
            self.tag.it = Some(short(&sid));
            self.tag.by = Some(mine8.clone());
            self.tag.k += 1;
            self.tag.since = c.now;
            self.tag.heard = c.now;
            self.send_tag(c);
            let name = c.name(Some(&sid)).unwrap_or_else(|| "them".into());
            self.flash_text(format!("You tagged {name}!"), 2000.0, c.now);
            return;
        }
    }

    /// Add the running "it" time into the table.
    pub(super) fn fold_tag_time(&mut self, now: f64) {
        let tg = &mut self.tag;
        if let Some(it) = tg.it.clone().filter(|_| !tg.over) {
            let add = (now.min(tg.end_at) - tg.at).max(0.0) / 100.0;
            let e = tg.t.entry(it).or_insert(0);
            *e = (*e as f64 + add).round() as u64;
        }
        tg.at = now;
    }

    pub(super) fn send_tag(&mut self, c: &mut Ctx) {
        self.fold_tag_time(c.now);
        self.tag.beat = c.now;
        // Stay well under the relay's 512 bytes: drop people who've left if the table grows.
        if self.tag.t.len() > 16 {
            let here: HashSet<String> = std::iter::once(short(&c.mine)).chain(c.m.space.players.keys().map(|s| short(s))).collect();
            self.tag.t.retain(|id, _| here.contains(id));
        }
        let tg = &self.tag;
        let left = if tg.over { 0.0 } else { (tg.end_at - c.now).max(0.0).round() };
        let mut d = json!({ "it": tg.it, "k": tg.k, "left": left as i64, "t": tg.t });
        if let Some(by) = &tg.by {
            d["by"] = json!(by);
        }
        c.game("tag", d);
    }

    pub(super) fn end_tag(&mut self, c: &mut Ctx) {
        self.fold_tag_time(c.now);
        self.tag.over = true;
        self.tag.result_until = c.now + 9000.0;
        if self.tag.it.as_deref() == Some(short(&c.mine).as_str()) {
            self.send_tag(c); // the final scores
        }
    }

    /// "It" left or went silent: the lowest sid still here hands it to whoever was nearest.
    pub(super) fn reassign_it(&mut self, c: &mut Ctx, gone: Option<(f32, f32)>) {
        let it = self.tag.it.clone().unwrap_or_default();
        let mut ids: Vec<String> = std::iter::once(c.mine.clone()).chain(c.m.space.players.keys().cloned()).filter(|s| short(s) != it).collect();
        if ids.len() < 2 {
            self.end_tag(c);
            return self.flash_text("Everyone else left: tag is over.", 3000.0, c.now);
        }
        self.tag.heard = c.now;
        ids.sort();
        if ids[0] != c.mine {
            return;
        }
        let holder = c.m.space.players.keys().find(|s| short(s) == it).and_then(|s| self.pos_of(c, s));
        let from = gone.or(holder).or(self.me.map(|(x, y, _)| (x, y))).unwrap_or(MID);
        let next = ids
            .iter()
            .filter_map(|s| self.pos_of(c, s).map(|p| (s.clone(), (p.0 - from.0).hypot(p.1 - from.1))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s);
        let Some(next) = next else { return };
        self.fold_tag_time(c.now);
        self.tag.it = Some(short(&next));
        self.tag.by = None;
        self.tag.k += 1;
        self.tag.since = c.now;
        self.send_tag(c);
        if next == c.mine {
            self.flash_text("You're it now! Touch someone to pass it on.", 2500.0, c.now);
        }
    }

    pub(super) fn on_tag(&mut self, sid: &str, d: &Value, c: &mut Ctx) {
        let k = num(&d["k"], 0.0, 1e12).floor() as u64;
        let it = short(d["it"].as_str().unwrap_or(""));
        if !short_ok(&it) || k < self.tag.k {
            return;
        }
        let left = num(&d["left"], 0.0, TAG_MS);
        if k == self.tag.k {
            if self.tag.over && left > 0.0 {
                return; // a late beat from a finished round
            }
            let cur = self.tag.it.clone().unwrap_or_default();
            if short(sid) != cur && short(sid) > cur {
                return; // two rounds started at once: lower sid wins
            }
        }
        let mut t = HashMap::new();
        if let Some(obj) = d["t"].as_object() {
            for (id, v) in obj.iter().take(40) {
                if short_ok(id) {
                    t.insert(id.clone(), num(v, 0.0, 1e6).floor() as u64);
                }
            }
        }
        let fresh = self.tag.over;
        let changed = Some(&it) != self.tag.it.as_ref() || fresh;
        self.tag.it = Some(it.clone());
        self.tag.by = d["by"].as_str().map(short).filter(|b| short_ok(b));
        self.tag.k = k;
        self.tag.t = t;
        self.tag.at = c.now;
        self.tag.end_at = c.now + left;
        self.tag.heard = c.now;
        if changed {
            self.tag.since = c.now;
        }
        if left <= 0.0 {
            self.tag.over = true;
            self.tag.result_until = c.now + 9000.0;
            return;
        }
        self.tag.over = false;
        self.tag.result_until = 0.0;
        if changed && it == short(&c.mine) {
            let by = self.sid_of(c, self.tag.by.clone().as_deref()).and_then(|s| c.name(Some(&s)));
            let text = match by {
                Some(b) => format!("{b} tagged you: you're it!"),
                None => "You're it! Touch someone to pass it on.".into(),
            };
            self.flash_text(text, 2500.0, c.now);
        } else if fresh {
            let who = self.sid_of(c, Some(&it)).and_then(|s| c.name(Some(&s))).unwrap_or_else(|| "Someone".into());
            self.flash_text(format!("{who} is it: run!"), 2500.0, c.now);
        }
    }

    /// Everyone here with their seconds as "it", least first.
    pub(super) fn tag_rows(&self, c: &Ctx) -> Vec<(String, u64)> {
        let mut rows: Vec<(String, u64)> = std::iter::once(c.mine.clone())
            .chain(c.m.space.players.keys().cloned())
            .filter_map(|sid| {
                let id = short(&sid);
                let mut ds = *self.tag.t.get(&id).unwrap_or(&0) as f64;
                if !self.tag.over && self.tag.it.as_deref() == Some(id.as_str()) {
                    ds += (c.now.min(self.tag.end_at) - self.tag.at).max(0.0) / 100.0;
                }
                c.name(Some(&sid)).map(|n| (n, (ds / 10.0).floor() as u64))
            })
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        rows
    }

    pub(super) fn step_effects(&mut self, c: &mut Ctx) {
        let Some((x, y, _)) = self.me else { return };
        if in_water(x, y) {
            self.soak(c.now, false);
        } else if on_fire(x, y) && seat_at(x, y).is_none() {
            self.ignite(c.now);
        }
        // Standing in it keeps it going: tell the others when that drifts by more than 1.5 s.
        let now = c.now;
        let drift = |a: f64, b: f64| (a > now) != (b > now) || (a - b).abs() > 1500.0;
        if drift(self.my_fx.fire, self.fx_sent.0) || drift(self.my_fx.wet, self.fx_sent.1) {
            self.send_fx(c, None);
        }
    }

    /// Water: you drip for a while, and any fire on you goes out.
    pub(super) fn soak(&mut self, now: f64, quiet: bool) {
        let Some((x, y, _)) = self.me else { return };
        if self.my_fx.fire > now {
            self.my_fx.fire = 0.0;
            self.burst(BurstKind::Steam, x, y, now);
            if !quiet {
                self.flash_text("Pssshh. The water put you out.", 2500.0, now);
            }
        }
        self.my_fx.wet = now + WET_MS;
    }

    /// Fire: you burn for a while. If you're wet, it dries you off instead (and you're safe a moment).
    pub(super) fn ignite(&mut self, now: f64) {
        let Some((x, y, _)) = self.me else { return };
        if now < self.my_fx.safe {
            return;
        }
        if self.my_fx.wet > now {
            self.my_fx.wet = 0.0;
            self.my_fx.safe = now + 1200.0;
            self.burst(BurstKind::Steam, x, y, now);
            return self.flash_text("Sizzle! The fire dried you off.", 2500.0, now);
        }
        if self.my_fx.fire <= now {
            self.flash_text("🔥 You're on fire! Water puts it out.", 2500.0, now);
        }
        self.my_fx.fire = now + FIRE_MS;
    }

    /// Announce my effects (`by_hit`: the balloon that just hit me).
    pub(super) fn send_fx(&mut self, c: &mut Ctx, by_hit: Option<(String, u32)>) {
        let now = c.now;
        self.fx_sent = (self.my_fx.fire, self.my_fx.wet);
        let left = |until: f64| (until - now).clamp(0.0, FX_MAX).round() as i64;
        let mut d = json!({ "fire": left(self.my_fx.fire), "wet": left(self.my_fx.wet) });
        if self.my_splashes > 0 {
            d["n"] = json!(self.my_splashes);
        }
        if let Some((by, hit)) = by_hit {
            d["by"] = json!(by);
            d["hit"] = json!(hit);
        }
        c.game("fx", d);
    }

    pub(super) fn on_fx(&mut self, sid: &str, d: &Value, c: &mut Ctx) {
        let now = c.now;
        let prev = self.fx.get(sid).copied().unwrap_or((0.0, 0.0));
        let fire = num(&d["fire"], 0.0, FX_MAX);
        let wet = num(&d["wet"], 0.0, FX_MAX);
        let next = (if fire > 0.0 { now + fire } else { 0.0 }, if wet > 0.0 { now + wet } else { 0.0 });
        let at = self.shown.get(sid).copied();
        // Fire meeting water (either way round) puffs steam.
        if let Some((x, y)) = at
            && ((prev.0 > now && next.0 == 0.0) || (prev.1 > now + 400.0 && next.1 == 0.0))
        {
            self.burst(BurstKind::Steam, x, y, now);
        }
        self.fx.insert(sid.to_string(), next);
        if !d["n"].is_null() {
            self.splashes.insert(sid.to_string(), num(&d["n"], 0.0, 99_999.0).floor() as u32);
        }
        // A water balloon hit them: pop it, and credit whoever threw it.
        let by = short(d["by"].as_str().unwrap_or(""));
        let hit = d["hit"].as_f64().filter(|h| h.fract() == 0.0 && *h >= 0.0);
        if short_ok(&by)
            && let Some(hit) = hit
        {
            let hit = hit as u32;
            self.shots.retain(|s| !(short(&s.sid) == by && s.id == hit));
            if let Some((x, y)) = at {
                self.burst(BurstKind::Splash, x, y, now);
            }
            let thrower = self.sid_of(c, Some(&by));
            let mine = thrower.as_deref() == Some(c.mine.as_str());
            if mine {
                self.my_splashes += 1;
            } else if let Some(t) = &thrower {
                *self.splashes.entry(t.clone()).or_insert(0) += 1;
            }
            let target = c.name(Some(sid)).unwrap_or_else(|| "someone".into());
            let text = if mine {
                format!("💦 You splashed {target}! ({})", self.my_splashes)
            } else {
                format!("💦 {} splashed {target}", thrower.and_then(|t| c.name(Some(&t))).unwrap_or_else(|| "Someone".into()))
            };
            self.flash_text(text, 2500.0, now);
            self.splash_board = now + 5000.0;
        }
    }

    /// (fire, wet) until, for me or someone on Space.
    pub(super) fn fx_of(&self, sid: &str, mine: &str) -> (f64, f64) {
        if sid == mine { (self.my_fx.fire, self.my_fx.wet) } else { self.fx.get(sid).copied().unwrap_or((0.0, 0.0)) }
    }

    /// Wet people leave puddles behind them; old ones dry up.
    pub(super) fn step_puddles(&mut self, c: &Ctx) {
        let now = c.now;
        let mut wet: Vec<(String, f32, f32)> = Vec::new();
        if let Some((x, y, _)) = self.me
            && self.my_fx.wet > now
        {
            wet.push((c.mine.clone(), x, y));
        }
        for (sid, (x, y)) in &self.shown {
            if self.fx.get(sid).is_some_and(|f| f.1 > now) {
                wet.push((sid.clone(), *x, *y));
            }
        }
        for (sid, x, y) in wet {
            if self.drip_from.get(&sid).is_some_and(|l| (x - l.0).hypot(y - l.1) < 34.0) {
                continue;
            }
            self.drip_from.insert(sid, (x, y));
            self.puddles.push((x + (rand01() - 0.5) * 10.0, y + RADIUS - 2.0, now));
        }
        while self.puddles.first().is_some_and(|p| now - p.2 > 4500.0) || self.puddles.len() > 160 {
            self.puddles.remove(0);
        }
    }

    pub(super) fn throw(&mut self, c: &mut Ctx) {
        let Some((mx, my, md)) = self.me else { return };
        if c.now - self.last_throw < SHOT_COOLDOWN {
            return;
        }
        self.last_throw = c.now;
        self.shot_id = (self.shot_id + 1) % 1000;
        let a = (md * 100.0).round() / 100.0;
        let x = (mx + a.cos() * (RADIUS + 12.0)).round();
        let y = (my + a.sin() * (RADIUS + 12.0)).round();
        self.launch(c.mine.clone(), self.shot_id, x, y, a);
        c.game("throw", json!({ "x": x as i64, "y": y as i64, "a": a, "id": self.shot_id }));
    }

    pub(super) fn launch(&mut self, sid: String, id: u32, x: f32, y: f32, a: f32) {
        self.shots.push(Shot { sid, id, x, y, vx: a.cos() * SHOT_SPEED, vy: a.sin() * SHOT_SPEED, left: SHOT_RANGE });
        if self.shots.len() > 40 {
            self.shots.remove(0);
        }
    }

    pub(super) fn on_throw(&mut self, sid: &str, d: &Value) {
        let id = num(&d["id"], 0.0, 999.0).floor() as u32;
        let Some(a) = d["a"].as_f64().filter(|a| a.is_finite()) else { return };
        self.launch(sid.to_string(), id, num(&d["x"], 0.0, WORLD_W as f64) as f32, num(&d["y"], 0.0, WORLD_H as f64) as f32, a as f32);
    }

    pub(super) fn step_shots(&mut self, dt: f32, c: &mut Ctx) {
        let now = c.now;
        let mut popped: Vec<(Shot, bool)> = Vec::new();
        let me = self.me;
        let mine = c.mine.clone();
        self.shots.retain_mut(|s| {
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.left -= SHOT_SPEED * dt;
            let hit_me = me.is_some_and(|(x, y, _)| s.sid != mine && (s.x - x).hypot(s.y - y) < RADIUS + SHOT_R);
            if !hit_me && s.left > 0.0 && !blocked(s.x, s.y, SHOT_R) {
                return true;
            }
            popped.push((Shot { sid: s.sid.clone(), id: s.id, x: s.x, y: s.y, vx: 0.0, vy: 0.0, left: 0.0 }, hit_me));
            false
        });
        for (s, hit_me) in popped {
            self.burst(BurstKind::Splash, s.x, s.y, now);
            self.puddles.push((s.x, s.y + 8.0, now));
            if hit_me {
                self.hit_by(&s, c);
            }
        }
    }

    pub(super) fn hit_by(&mut self, s: &Shot, c: &mut Ctx) {
        self.soak(c.now, true);
        *self.splashes.entry(s.sid.clone()).or_insert(0) += 1;
        self.splash_board = c.now + 5000.0;
        let who = c.name(Some(&s.sid)).unwrap_or_else(|| "Someone".into());
        self.flash_text(format!("💦 {who} splashed you!"), 2500.0, c.now);
        self.send_fx(c, Some((short(&s.sid), s.id)));
    }

    pub(super) fn step_race(&mut self, c: &mut Ctx) {
        let Some((x, y, _)) = self.me else { return };
        let now = c.now;
        let y0 = self.race.last_y;
        self.race.last_y = Some(y);
        if self.race.start > 0.0 {
            if on_track(x, y) {
                self.race.off_at = 0.0;
            } else if self.race.off_at == 0.0 {
                self.race.off_at = now;
            }
            if (self.race.off_at > 0.0 && now - self.race.off_at > 4000.0) || now - self.race.start > LAP_MAX_MS {
                self.race.start = 0.0;
            } else if self.race.cp < CORNERS.len() && in_rect(x, y, &CORNERS[self.race.cp]) {
                self.race.cp += 1;
            }
        }
        // The start/finish line runs across the lane on the left; cross it going up.
        let Some(y0) = y0 else { return };
        if x >= LANE || !(y0 >= START_Y && y < START_Y) {
            return;
        }
        if self.race.start > 0.0 && self.race.cp == CORNERS.len() {
            let ms = (now - self.race.start).round();
            let record = self.lap_rows(c).first().map(|r| r.1);
            self.race.laps += 1;
            let best = self.race.best == 0.0 || ms < self.race.best;
            if best {
                self.race.best = ms;
            }
            c.game("lap", json!({ "ms": ms as i64, "best": self.race.best as i64, "n": self.race.laps }));
            let fastest = record.is_none_or(|r| ms < r);
            let note = if fastest { ", the fastest here!" } else if best { ", your best!" } else { "" };
            self.flash_text(format!("🏁 Lap {}: {}{note}", self.race.laps, secs(ms)), 3500.0, now);
        } else if self.race.start == 0.0 {
            self.flash_text("🏁 Go! Run the lane clockwise, round all four corners.", 3000.0, now);
        }
        self.race.start = now;
        self.race.cp = 0;
        self.race.off_at = 0.0;
    }

    pub(super) fn on_lap(&mut self, sid: &str, d: &Value, c: &mut Ctx) {
        let best = num(&d["best"], 0.0, LAP_MAX_MS).round();
        if best <= 0.0 {
            return;
        }
        let record = self.lap_rows(c).first().map(|r| r.1);
        self.laps.insert(sid.to_string(), (best, num(&d["n"], 0.0, 99_999.0).floor() as u32));
        let ms = num(&d["ms"], 0.0, LAP_MAX_MS).round();
        if ms > 0.0 {
            let who = c.name(Some(sid)).unwrap_or_else(|| "Someone".into());
            let fastest = if record.is_none_or(|r| ms < r) { ", the fastest here!" } else { "" };
            self.flash_text(format!("🏁 {who} ran a lap in {}{fastest}", secs(ms)), 3000.0, c.now);
        }
    }

    /// Best laps of everyone here, fastest first: (name, ms).
    pub(super) fn lap_rows(&self, c: &Ctx) -> Vec<(String, f64)> {
        let mut rows: Vec<(String, f64)> = Vec::new();
        if self.race.best > 0.0 {
            rows.push((c.mine.clone(), self.race.best));
        }
        for (sid, (best, _)) in &self.laps {
            if c.m.space.players.contains_key(sid) {
                rows.push((sid.clone(), *best));
            }
        }
        let mut rows: Vec<(String, f64)> = rows.into_iter().filter_map(|(sid, ms)| c.name(Some(&sid)).map(|n| (n, ms))).collect();
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        rows
    }

    pub(super) fn step_instruments(&mut self, c: &Ctx) {
        let Some((mx, my, _)) = self.me else { return };
        let hear = zone_of(mx, my) == MUSIC_ZONE && !c.m.config.deafened && c.m.config.sounds;
        let mut people: Vec<(String, f32, f32)> = vec![(c.mine.clone(), mx, my)];
        people.extend(self.shown.iter().map(|(s, (x, y))| (s.clone(), *x, *y)));
        for (sid, x, y) in people {
            let hit = instrument_at(x, y);
            let id = hit.as_ref().map(|h| h.id.clone());
            if self.inst.get(&sid).cloned().flatten() == id {
                continue;
            }
            self.inst.insert(sid, id);
            let Some(hit) = hit else { continue };
            if hear {
                let clip = match (hit.drum, hit.midi) {
                    (Some(d), _) => crate::audio::space_sound::pad(d),
                    (None, Some(m)) => crate::audio::space_sound::piano(m),
                    _ => None,
                };
                self.to_play.extend(clip);
            }
            let (glyph, color) = match (hit.drum, hit.midi) {
                (Some(_), _) => ("✦", rgb(0xffd166)),
                (None, Some(midi)) => (if midi % 2 == 1 { "♫" } else { "♪" }, hsl((midi % 12) as f32 * 30.0, 0.9, 0.7)),
                _ => ("♪", Color32::WHITE),
            };
            self.burst(BurstKind::Note(glyph, color), x, y - RADIUS - 24.0, c.now);
        }
    }
}
