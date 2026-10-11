//! The map (web/js/space-map.js): rooms and their walls, the pitch and its
//! nets, the fire, fountain and pond, the race track, the floor piano and drum
//! pads, the Gallery's frames, decorations and seats, and what's where.

use super::*;

/// A rect in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct R {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

pub(super) fn in_rect(x: f32, y: f32, r: &R) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

pub(super) struct Room {
    pub(super) name: &'static str,
    /// Radar name on small radars.
    pub(super) short: Option<&'static str>,
    pub(super) r: R,
    pub(super) floor: u32,
    pub(super) doors: &'static [&'static str],
    /// No walls (the Garden): a dashed edge shows where it ends.
    pub(super) open: bool,
    /// Where the floor lettering goes when the middle is taken.
    pub(super) label: Option<(f32, f32)>,
}

pub(super) const TOP_Y: f32 = 100.0;
pub(super) const BOTTOM_Y: f32 = 1044.0;
pub(super) const ROW_H: f32 = 456.0;
/// Top row, the Gallery hall, bottom row. Everywhere that isn't a room is the Plaza.
pub(super) const ROOMS: [Room; 9] = [
    Room { name: "Lounge", short: None, r: R { x: 100.0, y: TOP_Y, w: 480.0, h: ROW_H }, floor: 0x7b5ea7, doors: &["bottom"], open: false, label: Some((340.0, 372.0)) },
    Room { name: "Cinema", short: None, r: R { x: 660.0, y: TOP_Y, w: 560.0, h: ROW_H }, floor: 0x4a2430, doors: &["bottom"], open: false, label: Some((940.0, 512.0)) },
    Room { name: "Music", short: None, r: R { x: 1300.0, y: TOP_Y, w: 440.0, h: ROW_H }, floor: 0x1f5f5b, doors: &["bottom"], open: false, label: Some((1520.0, 512.0)) },
    Room { name: "Arcade", short: None, r: R { x: 1820.0, y: TOP_Y, w: 480.0, h: ROW_H }, floor: 0x2e5a88, doors: &["bottom"], open: false, label: None },
    Room { name: "Gallery", short: None, r: R { x: 720.0, y: 636.0, w: 960.0, h: 328.0 }, floor: 0x6e5442, doors: &["left", "right"], open: false, label: Some((1200.0, 805.0)) },
    Room { name: "Garden", short: None, r: R { x: 100.0, y: BOTTOM_Y, w: 480.0, h: ROW_H }, floor: 0x4f8a3c, doors: &["top"], open: true, label: Some((340.0, 1466.0)) },
    Room { name: "Pond", short: None, r: R { x: 660.0, y: BOTTOM_Y, w: 420.0, h: ROW_H }, floor: 0x5b8a46, doors: &["top"], open: false, label: Some((870.0, 1466.0)) },
    Room { name: "Pitch", short: None, r: R { x: 1160.0, y: BOTTOM_Y, w: 660.0, h: ROW_H }, floor: 0x3d8a46, doors: &["top"], open: false, label: Some((1490.0, 1466.0)) },
    Room { name: "Campfire", short: Some("Camp"), r: R { x: 1900.0, y: BOTTOM_Y, w: 400.0, h: ROW_H }, floor: 0x8a5a32, doors: &["top"], open: false, label: Some((2100.0, 1466.0)) },
];

pub(super) const MUSIC_ZONE: i32 = 2;
pub(super) const GALLERY_ZONE: i32 = 4;
pub(super) const GALLERY: R = ROOMS[4].r;
pub(super) fn walls_for(room: &Room, out: &mut Vec<R>) {
    if room.open {
        return;
    }
    let R { x, y, w, h } = room.r;
    let mid = |a: f32, len: f32| a + len / 2.0 - DOOR / 2.0;
    let mut side = |s: &str, r: R| {
        if !room.doors.contains(&s) {
            out.push(r);
            return;
        }
        // A wall with a doorway in the middle.
        if s == "top" || s == "bottom" {
            out.push(R { x: r.x, y: r.y, w: mid(x, w) - r.x, h: r.h });
            out.push(R { x: mid(x, w) + DOOR, y: r.y, w: r.x + r.w - (mid(x, w) + DOOR), h: r.h });
        } else {
            out.push(R { x: r.x, y: r.y, w: r.w, h: mid(y, h) - r.y });
            out.push(R { x: r.x, y: mid(y, h) + DOOR, w: r.w, h: r.y + r.h - (mid(y, h) + DOOR) });
        }
    };
    side("top", R { x, y, w, h: T });
    side("bottom", R { x, y: y + h - T, w, h: T });
    side("left", R { x, y, w: T, h });
    side("right", R { x: x + w - T, y, w: T, h });
}

/// Where a room's name signs hang: on the wall beside its door (on a post
/// for the open Garden); a hall with doors at both ends gets one at each end.
pub(super) fn plaque_spots(room: &Room) -> Vec<(f32, f32)> {
    let r = room.r;
    let door = room.doors[0];
    if door == "left" || door == "right" {
        return vec![(r.x + 90.0, r.y + T / 2.0), (r.x + r.w - 90.0, r.y + T / 2.0)];
    }
    let y = if door == "top" { r.y + T / 2.0 } else { r.y + r.h - T / 2.0 };
    vec![(if room.open { r.x + r.w / 2.0 } else { r.x + r.w / 2.0 + DOOR / 2.0 + 60.0 }, y)]
}

pub(super) static WALLS: LazyLock<Vec<R>> = LazyLock::new(|| {
    let mut out = vec![R { x: 0.0, y: 0.0, w: WORLD_W, h: 12.0 }, R { x: 0.0, y: WORLD_H - 12.0, w: WORLD_W, h: 12.0 }, R { x: 0.0, y: 0.0, w: 12.0, h: WORLD_H }, R { x: WORLD_W - 12.0, y: 0.0, w: 12.0, h: WORLD_H }];
    for room in &ROOMS {
        walls_for(room, &mut out);
    }
    out
});
// The Pitch: a goal at each end, its posts solid.
pub(super) const PITCH: R = ROOMS[7].r;
pub(super) const MID: (f32, f32) = (PITCH.x + PITCH.w / 2.0, PITCH.y + PITCH.h / 2.0);
pub(super) const NET_HALF: f32 = 80.0;
pub(super) const NET_DEPTH: f32 = 46.0;
/// Inner face of each end wall of the Pitch (0 = left / Blue's, 1 = right / Red's).
pub(super) const NET_X: [f32; 2] = [PITCH.x + T, PITCH.x + PITCH.w - T];
/// The goal posts: solid (players and the ball bounce off them).
pub(super) static POSTS: LazyLock<Vec<R>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for (side, wx) in NET_X.iter().enumerate() {
        let x = if side == 1 { wx - NET_DEPTH } else { *wx };
        out.push(R { x, y: MID.1 - NET_HALF - 8.0, w: NET_DEPTH, h: 8.0 });
        out.push(R { x, y: MID.1 + NET_HALF, w: NET_DEPTH, h: 8.0 });
    }
    out
});
pub(super) static SOLID: LazyLock<Vec<R>> = LazyLock::new(|| WALLS.iter().chain(POSTS.iter()).copied().collect());
pub(super) fn blocked(x: f32, y: f32, r: f32) -> bool {
    SOLID.iter().any(|w| x + r > w.x && x - r < w.x + w.w && y + r > w.y && y - r < w.y + w.h)
}
// Things that happen to you: fire sets you alight, water soaks you.
pub(super) const FIRE: (f32, f32, f32) = (2100.0, 1272.0, 32.0);
/// The water inside the Garden's basin.
pub(super) const FOUNTAIN: (f32, f32, f32) = (340.0, 1272.0, 78.0);
/// The Pond: centre and radii.
pub(super) const POND: (f32, f32, f32, f32) = (870.0, 1300.0, 160.0, 160.0);
pub(super) const DOCK: [R; 2] = [R { x: 845.0, y: 1066.0, w: 50.0, h: 194.0 }, R { x: 815.0, y: 1260.0, w: 110.0, h: 60.0 }];
pub(super) const TAG_SPOT: (f32, f32, f32) = (2080.0, 800.0, 44.0);
/// The lap records board: on the Plaza where newcomers appear, a few steps
/// from the start line (web/js/space-map.js › LAP_BOARD; positions are shared).
pub(super) const LAP_BOARD: (f32, f32) = (440.0, 680.0);
/// The race track: the lane between the outer wall and the rooms, run clockwise.
pub(super) const LANE: f32 = 100.0;
pub(super) const START_Y: f32 = 800.0;
pub(super) const CORNERS: [R; 4] = [
    R { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
    R { x: WORLD_W - 100.0, y: 0.0, w: 100.0, h: 100.0 },
    R { x: WORLD_W - 100.0, y: WORLD_H - 100.0, w: 100.0, h: 100.0 },
    R { x: 0.0, y: WORLD_H - 100.0, w: 100.0, h: 100.0 },
];

pub(super) fn on_track(x: f32, y: f32) -> bool {
    x < LANE || x > WORLD_W - LANE || y < LANE || y > WORLD_H - LANE
}

/// The Music room's floor piano (C4–G5, black keys too) and four drum pads.
pub(super) const PIANO: (f32, f32) = (1328.0, 150.0);
pub(super) const KEY_W: f32 = 32.0;
pub(super) const KEY_H: f32 = 150.0;
pub(super) const BLACK_W: f32 = 20.0;
pub(super) const BLACK_H: f32 = 92.0;
pub(super) const WHITE: [u8; 12] = [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79];
/// [after white key i, midi].
pub(super) const BLACK: [(usize, u8); 8] = [(0, 61), (1, 63), (3, 66), (4, 68), (5, 70), (7, 73), (8, 75), (10, 78)];
pub(super) const PAD_R: f32 = 32.0;
pub(super) const PADS: [(f32, f32, &str, u32); 4] = [(1400.0, 420.0, "kick", 0xe5484d), (1480.0, 420.0, "snare", 0xffd166), (1560.0, 420.0, "tom", 0x30a46c), (1640.0, 420.0, "hat", 0x7afcff)];
/// The Gallery: twelve frames to a wall (six top, six bottom), newest first.
pub(super) const ART_W: f32 = 112.0;
pub(super) const ART_H: f32 = 70.0;
pub(super) const ART_BORDER: f32 = 5.0;
pub(super) const ART_PLATE: f32 = 13.0;
pub(super) const PER_WALL: usize = 12;
pub(super) struct Frame {
    pub(super) i: usize,
    pub(super) row: usize,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    /// Where you stand to look at it (in front of it, the aisle side).
    pub(super) look: R,
}

pub(super) static FRAMES: LazyLock<Vec<Frame>> = LazyLock::new(|| {
    let fw = ART_W + ART_BORDER * 2.0;
    let fh = ART_H + ART_BORDER * 2.0;
    let gap = 24.0;
    let x0 = GALLERY.x + T + (GALLERY.w - T * 2.0 - (fw * 6.0 + gap * 5.0)) / 2.0;
    let top_y = GALLERY.y + T + 6.0;
    let bottom_y = GALLERY.y + GALLERY.h - T - 6.0 - ART_PLATE - fh;
    (0..PER_WALL)
        .map(|i| {
            let row = if i < 6 { 0 } else { 1 };
            let x = x0 + (i % 6) as f32 * (fw + gap);
            let y = if row == 1 { bottom_y } else { top_y };
            let look = if row == 1 { R { x: x - 8.0, y: y - 82.0, w: fw + 16.0, h: 82.0 } } else { R { x: x - 8.0, y: y + fh + ART_PLATE, w: fw + 16.0, h: 82.0 } };
            Frame { i, row, x, y, w: fw, h: fh, look }
        })
        .collect()
});
/// The frame you're standing in front of (closest first).
pub(super) fn frame_at(x: f32, y: f32) -> Option<&'static Frame> {
    FRAMES.iter().filter(|f| in_rect(x, y, &f.look)).min_by(|a, b| {
        let d = |f: &Frame| (x - (f.x + f.w / 2.0)).hypot(y - (f.y + f.h / 2.0));
        d(a).total_cmp(&d(b))
    })
}

pub(super) type Decor = (&'static str, f32, f32, Option<f32>);
/// Decorations (not solid): kind, x, y, facing (seats) or side (nets).
pub(super) static DECOR: LazyLock<Vec<Decor>> = LazyLock::new(|| {
    let up = Some(-FRAC_PI_2);
    let mut d: Vec<Decor> = vec![
        // Lounge
        ("shelf", 340.0, 140.0, None),
        ("sofa", 260.0, 300.0, None),
        ("sofa", 440.0, 300.0, None),
        ("rug", 340.0, 440.0, None),
        ("ctable", 340.0, 438.0, None),
        ("plant", 150.0, 170.0, None),
        ("plant", 530.0, 170.0, None),
        ("plant", 150.0, 500.0, None),
        ("lamp", 530.0, 500.0, None),
        // Cinema: a screen up front, rows of couches facing it.
        ("screen", 940.0, 160.0, None),
        ("couch", 840.0, 320.0, up),
        ("couch", 1040.0, 320.0, up),
        ("couch", 840.0, 430.0, up),
        ("couch", 1040.0, 430.0, up),
        ("popcorn", 1155.0, 495.0, None),
        ("plant", 720.0, 495.0, None),
        // Music: walk on the keys and pads.
        ("piano", PIANO.0, PIANO.1, None),
        ("pads", 0.0, 0.0, None),
        ("mic", 1520.0, 345.0, None),
        ("speaker", 1365.0, 488.0, None),
        ("speaker", 1675.0, 488.0, None),
    ];
    // Arcade: cabinets with stools, a pool table, a claw machine.
    for x in [1880.0, 1970.0, 2060.0, 2150.0, 2240.0] {
        d.push(("cabinet", x, 190.0, None));
        d.push(("stool", x, 282.0, up));
    }
    d.extend([
        ("pool", 2060.0, 440.0, None),
        ("claw", 1885.0, 482.0, None),
        ("plant", 2245.0, 495.0, None),
        // Gallery: the frames, and benches down the middle facing the top wall.
        ("gallery", 0.0, 0.0, None),
        ("bench", 1000.0, 801.0, up),
        ("bench", 1400.0, 801.0, up),
        // Garden: a fountain with benches, trees, flowers.
        ("tree", 185.0, 1118.0, None),
        ("tree", 495.0, 1118.0, None),
        ("tree", 185.0, 1425.0, None),
        ("tree", 495.0, 1425.0, None),
        ("fountain", FOUNTAIN.0, FOUNTAIN.1, None),
        ("bench", 340.0, 1128.0, Some(FRAC_PI_2)),
        ("bench", 340.0, 1418.0, up),
        ("flowers", 170.0, 1272.0, None),
        ("flowers", 510.0, 1272.0, None),
        // Pond: water, lily pads and ducks, a dock out to the middle.
        ("pond", POND.0, POND.1, None),
        ("reeds", 715.0, 1110.0, None),
        ("reeds", 1025.0, 1115.0, None),
        ("reeds", 1030.0, 1440.0, None),
        ("dock", 870.0, 1300.0, Some(FRAC_PI_2)),
        // Pitch
        ("net", NET_X[0], MID.1, Some(0.0)),
        ("net", NET_X[1], MID.1, Some(1.0)),
        ("scoreboard", 1285.0, 1100.0, None),
        // Campfire
        ("tent", 2225.0, 1110.0, None),
        ("rock", 1960.0, 1455.0, None),
        ("rock", 2245.0, 1455.0, None),
        ("fire", FIRE.0, FIRE.1, None),
        ("log", 1988.0, FIRE.1, None),
        ("log", 2212.0, FIRE.1, None),
        ("log", FIRE.0, 1142.0, None),
        ("log", FIRE.0, 1402.0, None),
        // Plaza
        ("lamp", 610.0, 600.0, None),
        ("lamp", 1790.0, 1000.0, None),
        ("tagspot", TAG_SPOT.0, TAG_SPOT.1, None),
        ("laps", LAP_BOARD.0, LAP_BOARD.1, None),
    ]);
    d
});
/// Places to sit: spots along sofas, couches, benches and the dock; one per stool and log.
pub(super) static SEATS: LazyLock<Vec<(f32, f32, f32)>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for &(kind, x, y, extra) in DECOR.iter() {
        let d = extra.unwrap_or(FRAC_PI_2);
        let offsets: &[f32] = match kind {
            "sofa" => &[-45.0, 45.0],
            "couch" => &[-50.0, 0.0, 50.0],
            "bench" => &[-40.0, 40.0],
            "stool" => &[0.0],
            "dock" => &[-30.0, 30.0],
            "log" => {
                out.push((x, y, (FIRE.1 - y).atan2(FIRE.0 - x)));
                &[]
            }
            _ => &[],
        };
        for dx in offsets {
            out.push((x + dx, y, d));
        }
    }
    out
});
pub(super) fn seat_at(x: f32, y: f32) -> Option<(f32, f32, f32)> {
    SEATS.iter().copied().find(|s| (s.0 - x).abs() < 4.0 && (s.1 - y).abs() < 4.0)
}

/// Which room (voice zone) a spot is in: an index into ROOMS, or -1 for the Plaza.
pub(super) fn zone_of(x: f32, y: f32) -> i32 {
    ROOMS.iter().position(|r| in_rect(x, y, &r.r)).map(|i| i as i32).unwrap_or(-1)
}

pub(super) fn zone_name(i: i32) -> &'static str {
    if i >= 0 { ROOMS[i as usize].name } else { PLAZA }
}

pub(super) fn on_fire(x: f32, y: f32) -> bool {
    (x - FIRE.0).hypot(y - FIRE.1) < FIRE.2
}

pub(super) fn in_water(x: f32, y: f32) -> bool {
    let (px, py, rx, ry) = POND;
    (x - FOUNTAIN.0).hypot(y - FOUNTAIN.1) < FOUNTAIN.2 || (((x - px) / rx).powi(2) + ((y - py) / ry).powi(2) < 1.0 && !DOCK.iter().any(|r| in_rect(x, y, r)))
}

/// The key or pad under a spot.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hit {
    pub(super) id: String,
    pub(super) midi: Option<u8>,
    pub(super) drum: Option<&'static str>,
}

pub(super) fn instrument_at(x: f32, y: f32) -> Option<Hit> {
    let (px, py) = PIANO;
    if y >= py && y <= py + KEY_H && x >= px && x < px + KEY_W * WHITE.len() as f32 {
        if y <= py + BLACK_H {
            for (i, midi) in BLACK {
                if (x - (px + (i + 1) as f32 * KEY_W)).abs() <= BLACK_W / 2.0 {
                    return Some(Hit { id: format!("b{i}"), midi: Some(midi), drum: None });
                }
            }
        }
        let i = ((x - px) / KEY_W).floor() as usize;
        return Some(Hit { id: format!("w{i}"), midi: Some(WHITE[i]), drum: None });
    }
    PADS.iter().enumerate().find(|(_, p)| (x - p.0).hypot(y - p.1) < PAD_R).map(|(i, p)| Hit { id: format!("p{i}"), midi: None, drum: Some(p.2) })
}

/// Which goal mouth a ball centre is inside: 0 = left (Blue's), 1 = right (Red's), -1 = neither.
pub(super) fn net_at(x: f32, y: f32) -> i32 {
    if (y - MID.1).abs() >= NET_HALF || x < PITCH.x || x > PITCH.x + PITCH.w {
        return -1;
    }
    if x < NET_X[0] + NET_DEPTH - BALL_R {
        0
    } else if x > NET_X[1] - NET_DEPTH + BALL_R {
        1
    } else {
        -1
    }
}

#[cfg(test)]
mod tests {
    /// Positions are shared with the web (and the iPhone's web view): the lap
    /// records board stands where web/js/space-map.js puts it, as wide as
    /// space-art.js draws it.
    #[test]
    fn lap_board_matches_the_web() {
        let map = include_str!("../../../../web/js/space-map.js");
        assert!(map.contains(&format!("const LAP_BOARD = {{ x: {}, y: {} }};", super::LAP_BOARD.0, super::LAP_BOARD.1)), "web/js/space-map.js › LAP_BOARD moved");
        let art = include_str!("../../../../web/js/space-art.js");
        assert!(art.contains("box(ctx, x - 110, y - 46, 220, 92, EDGE, '#ffd166');"), "web/js/space-art.js › the lap board's size changed");
    }
}
