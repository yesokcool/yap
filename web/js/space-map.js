/*
 * Space's map: rooms (each its own voice zone), walls, furniture, seats and
 * the spots where something happens — the fire, water, the instruments, the
 * race track, the gallery's frames. Plain data and geometry; space.js runs
 * the world and space-art.js draws it. PROTOCOL.md › Space has the numbers
 * other clients need.
 *
 * Layout: a row of rooms along the top and one along the bottom, the Gallery
 * (a long hall) between them, the Plaza all round the hall (wings left and
 * right, corridors above and below), and the race track round the edge.
 */

export const WORLD = { w: 2400, h: 1600 };
export const RADIUS = 24; // a person
export const T = 22; // wall thickness
const DOOR = 140;
export const PLAZA = 'Plaza';
/** Where the Plaza's name goes (lettering and radar), and where newcomers appear. */
export const PLAZA_SPOT = { x: 430, y: 830 };
export const SPAWN = { x: 450, y: 900, spread: 200 };

const TOP = { y: 100, h: 456 };
const BOTTOM = { y: 1044, h: 456 };

/**
 * Rooms: name, rect, floor colour, which side(s) have a door; `label` = where
 * the big floor lettering goes when the middle is taken, `short` = radar name
 * on small screens. Everywhere that isn't a room is the Plaza.
 */
export const ROOMS = [
  { name: 'Lounge', x: 100, ...TOP, w: 480, floor: '#7b5ea7', door: 'bottom', label: [340, 372] },
  { name: 'Cinema', x: 660, ...TOP, w: 560, floor: '#4a2430', door: 'bottom', label: [940, 512] },
  { name: 'Music', x: 1300, ...TOP, w: 440, floor: '#1f5f5b', door: 'bottom', label: [1520, 512] },
  { name: 'Arcade', x: 1820, ...TOP, w: 480, floor: '#2e5a88', door: 'bottom' },
  { name: 'Gallery', x: 720, y: 636, w: 960, h: 328, floor: '#6e5442', door: ['left', 'right'], label: [1200, 805] },
  { name: 'Garden', x: 100, ...BOTTOM, w: 480, floor: '#4f8a3c', door: 'top', open: true, label: [340, 1466] },
  { name: 'Pond', x: 660, ...BOTTOM, w: 420, floor: '#5b8a46', door: 'top', label: [870, 1466] },
  { name: 'Pitch', x: 1160, ...BOTTOM, w: 660, floor: '#3d8a46', door: 'top', label: [1490, 1466] },
  { name: 'Campfire', x: 1900, ...BOTTOM, w: 400, floor: '#8a5a32', door: 'top', label: [2100, 1466], short: 'Camp' },
];
const roomNamed = (name) => ROOMS.find((r) => r.name === name);
const doorsOf = (room) => [room.door].flat();

function wallsFor(room) {
  if (room.open) return [];
  const { x, y, w, h } = room;
  const mid = (a, len) => a + len / 2 - DOOR / 2;
  const walls = [];
  const side = (s, rect) => {
    if (!doorsOf(room).includes(s)) return walls.push(rect);
    // wall with a doorway in the middle
    if (s === 'top' || s === 'bottom') {
      walls.push({ x: rect.x, y: rect.y, w: mid(x, w) - rect.x, h: rect.h }, { x: mid(x, w) + DOOR, y: rect.y, w: rect.x + rect.w - (mid(x, w) + DOOR), h: rect.h });
    } else {
      walls.push({ x: rect.x, y: rect.y, w: rect.w, h: mid(y, h) - rect.y }, { x: rect.x, y: mid(y, h) + DOOR, w: rect.w, h: rect.y + rect.h - (mid(y, h) + DOOR) });
    }
  };
  side('top', { x, y, w, h: T });
  side('bottom', { x, y: y + h - T, w, h: T });
  side('left', { x, y, w: T, h });
  side('right', { x: x + w - T, y, w: T, h });
  return walls;
}

/**
 * Where each room's name signs hang: on the wall beside its door, on a post
 * for the open Garden; a hall with doors at both ends gets one at each end of
 * its top wall.
 */
export function plaqueSpots(room) {
  const door = doorsOf(room)[0];
  if (door === 'left' || door === 'right') return [{ x: room.x + 90, y: room.y + T / 2 }, { x: room.x + room.w - 90, y: room.y + T / 2 }];
  const y = door === 'top' ? room.y + T / 2 : room.y + room.h - T / 2;
  return [{ x: room.open ? room.x + room.w / 2 : room.x + room.w / 2 + DOOR / 2 + 60, y }];
}

export const WALLS = [
  { x: 0, y: 0, w: WORLD.w, h: 12 }, { x: 0, y: WORLD.h - 12, w: WORLD.w, h: 12 },
  { x: 0, y: 0, w: 12, h: WORLD.h }, { x: WORLD.w - 12, y: 0, w: 12, h: WORLD.h },
  ...ROOMS.flatMap(wallsFor),
];

// The Pitch: a goal at each end, its posts solid (players and the ball bounce off them).
const PITCH = roomNamed('Pitch');
export const MID = { x: PITCH.x + PITCH.w / 2, y: PITCH.y + PITCH.h / 2 };
export const NET = { half: 80, depth: 46 };
export const NET_X = [PITCH.x + T, PITCH.x + PITCH.w - T]; // inner face of each end wall
export const POSTS = NET_X.flatMap((wx, side) => {
  const x = side ? wx - NET.depth : wx;
  return [{ x, y: MID.y - NET.half - 8, w: NET.depth, h: 8 }, { x, y: MID.y + NET.half, w: NET.depth, h: 8 }];
});
export const SOLID = [...WALLS, ...POSTS];

// Things that happen to you: fire sets you alight, water soaks you.
const FIRE = { x: 2100, y: 1272, r: 32 };
const FOUNTAIN = { x: 340, y: 1272, r: 78 }; // the water inside the basin
export const POND = { x: 870, y: 1300, rx: 160, ry: 160 };
export const DOCK = [{ x: 845, y: 1066, w: 50, h: 194 }, { x: 815, y: 1260, w: 110, h: 60 }];

export const TAG_SPOT = { x: 2080, y: 800, r: 44 };
// On the Plaza where newcomers appear (not by the lane's edge, where a phone's
// screen or the room window cut it off), still a few steps from the start line.
const LAP_BOARD = { x: 440, y: 680 };

/**
 * The race track: the lane between the outer wall and the rooms, run
 * clockwise. A lap = cross the start line on the left going up, pass the four
 * corners in order (top-left, top-right, bottom-right, bottom-left), cross it again.
 */
export const TRACK = { lane: 100, startY: 800 };
export const CORNERS = [
  { x: 0, y: 0, w: 100, h: 100 },
  { x: WORLD.w - 100, y: 0, w: 100, h: 100 },
  { x: WORLD.w - 100, y: WORLD.h - 100, w: 100, h: 100 },
  { x: 0, y: WORLD.h - 100, w: 100, h: 100 },
];
export const onTrack = (x, y) => x < TRACK.lane || x > WORLD.w - TRACK.lane || y < TRACK.lane || y > WORLD.h - TRACK.lane;

/** The Music room's floor piano (C4–G5, black keys too) and four drum pads. */
export const PIANO = {
  x: 1328, y: 150, w: 32, h: 150, bw: 20, bh: 92,
  white: [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79],
  black: [[0, 61], [1, 63], [3, 66], [4, 68], [5, 70], [7, 73], [8, 75], [10, 78]], // [after white key i, midi]
};
export const PADS = [
  { x: 1400, y: 420, r: 32, drum: 'kick', color: '#e5484d' },
  { x: 1480, y: 420, r: 32, drum: 'snare', color: '#ffd166' },
  { x: 1560, y: 420, r: 32, drum: 'tom', color: '#30a46c' },
  { x: 1640, y: 420, r: 32, drum: 'hat', color: '#7afcff' },
];

/**
 * The Gallery: twelve frames to a wall — six on the top wall, six on the
 * bottom one — newest first; more than twelve drawings and you page through
 * walls. Each frame: the drawing at 112×70 (a 640×400 page, crisp pixels)
 * in a 5-unit frame, a name plate under it. `look` = where you stand to look
 * at it (in front of it, the aisle side).
 */
export const GALLERY = roomNamed('Gallery');
export const ART = { w: 112, h: 70, border: 5, plate: 13, perWall: 12 };
export const FRAMES = (() => {
  const fw = ART.w + ART.border * 2;
  const fh = ART.h + ART.border * 2;
  const gap = 24;
  const x0 = GALLERY.x + T + (GALLERY.w - T * 2 - (fw * 6 + gap * 5)) / 2;
  const topY = GALLERY.y + T + 6;
  const bottomY = GALLERY.y + GALLERY.h - T - 6 - ART.plate - fh;
  return Array.from({ length: ART.perWall }, (_, i) => {
    const row = i < 6 ? 0 : 1;
    const x = x0 + (i % 6) * (fw + gap);
    const y = row ? bottomY : topY;
    const look = row ? { x: x - 8, y: y - 82, w: fw + 16, h: 82 } : { x: x - 8, y: y + fh + ART.plate, w: fw + 16, h: 82 };
    return { i, row, x, y, w: fw, h: fh, look };
  });
})();
/** The frame you're standing in front of (closest first), or null. */
export function frameAt(x, y) {
  let best = null;
  for (const f of FRAMES) {
    if (!inRect(x, y, f.look)) continue;
    const d = Math.hypot(x - (f.x + f.w / 2), y - (f.y + f.h / 2));
    if (!best || d < best.d) best = { f, d };
  }
  return best?.f ?? null;
}

const CABINETS = [1880, 1970, 2060, 2150, 2240];

// Decorations (not solid): [kind, x, y, facing or side]
export const DECOR = [
  // Lounge
  ['shelf', 340, 140], ['sofa', 260, 300], ['sofa', 440, 300], ['rug', 340, 440], ['ctable', 340, 438],
  ['plant', 150, 170], ['plant', 530, 170], ['plant', 150, 500], ['lamp', 530, 500],
  // Cinema: a screen up front, rows of couches facing it
  ['screen', 940, 160], ['couch', 840, 320, -Math.PI / 2], ['couch', 1040, 320, -Math.PI / 2],
  ['couch', 840, 430, -Math.PI / 2], ['couch', 1040, 430, -Math.PI / 2], ['popcorn', 1155, 495], ['plant', 720, 495],
  // Music: walk on the keys and pads
  ['piano', PIANO.x, PIANO.y], ['pads', 0, 0], ['mic', 1520, 345], ['speaker', 1365, 488], ['speaker', 1675, 488],
  // Arcade: cabinets with stools, a pool table, a claw machine
  ...CABINETS.flatMap((x) => [['cabinet', x, 190], ['stool', x, 282, -Math.PI / 2]]),
  ['pool', 2060, 440], ['claw', 1885, 482], ['plant', 2245, 495],
  // Gallery: the frames, and benches down the middle facing the top wall
  ['gallery', 0, 0], ['bench', 1000, 801, -Math.PI / 2], ['bench', 1400, 801, -Math.PI / 2],
  // Garden: a fountain with benches, trees, flowers
  ['tree', 185, 1118], ['tree', 495, 1118], ['tree', 185, 1425], ['tree', 495, 1425],
  ['fountain', FOUNTAIN.x, FOUNTAIN.y], ['bench', 340, 1128, Math.PI / 2], ['bench', 340, 1418, -Math.PI / 2],
  ['flowers', 170, 1272], ['flowers', 510, 1272],
  // Pond: water, lily pads and ducks, a dock out to the middle
  ['pond', POND.x, POND.y], ['reeds', 715, 1110], ['reeds', 1025, 1115], ['reeds', 1030, 1440], ['dock', 870, 1300, Math.PI / 2],
  // Pitch
  ['net', NET_X[0], MID.y, 0], ['net', NET_X[1], MID.y, 1], ['scoreboard', 1285, 1100],
  // Campfire
  ['tent', 2225, 1110], ['rock', 1960, 1455], ['rock', 2245, 1455], ['fire', FIRE.x, FIRE.y],
  ['log', 1988, FIRE.y], ['log', 2212, FIRE.y], ['log', FIRE.x, 1142], ['log', FIRE.x, 1402],
  // Plaza
  ['lamp', 610, 600], ['lamp', 1790, 1000], ['tagspot', TAG_SPOT.x, TAG_SPOT.y], ['laps', LAP_BOARD.x, LAP_BOARD.y],
];

/** Places to sit (press E nearby): spots along sofas, couches, benches and the dock; one per stool and log. */
const SEAT_OFFSETS = { sofa: [-45, 45], couch: [-50, 0, 50], bench: [-40, 40], stool: [0], dock: [-30, 30] };
export const SEATS = DECOR.flatMap(([kind, x, y, d = Math.PI / 2]) =>
  kind === 'log' ? [{ x, y, d: Math.atan2(FIRE.y - y, FIRE.x - x) }] : (SEAT_OFFSETS[kind] ?? []).map((dx) => ({ x: x + dx, y, d })),
);

export function inRect(x, y, r) {
  return x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
}
export const seatAt = (p) => SEATS.find((s) => Math.abs(s.x - p.x) < 4 && Math.abs(s.y - p.y) < 4);
/** Which room (voice zone) a spot is in: an index into ROOMS, or -1 for the Plaza. */
export const zoneOf = (x, y) => ROOMS.findIndex((r) => inRect(x, y, r));
export const zoneName = (i) => ROOMS[i]?.name ?? PLAZA;
export const MUSIC_ZONE = ROOMS.findIndex((r) => r.name === 'Music');
export const GALLERY_ZONE = ROOMS.findIndex((r) => r.name === 'Gallery');
export const blocked = (x, y, r = RADIUS) => SOLID.some((w) => x + r > w.x && x - r < w.x + w.w && y + r > w.y && y - r < w.y + w.h);
export const onFire = (x, y) => Math.hypot(x - FIRE.x, y - FIRE.y) < FIRE.r;
export const inWater = (x, y) =>
  Math.hypot(x - FOUNTAIN.x, y - FOUNTAIN.y) < FOUNTAIN.r ||
  (((x - POND.x) / POND.rx) ** 2 + ((y - POND.y) / POND.ry) ** 2 < 1 && !DOCK.some((r) => inRect(x, y, r)));

/** The key or pad under a spot: { id, midi } for a piano key, { id, drum } for a pad, or null. */
export function instrumentAt(x, y) {
  const p = PIANO;
  if (y >= p.y && y <= p.y + p.h && x >= p.x && x < p.x + p.w * p.white.length) {
    if (y <= p.y + p.bh) {
      for (const [i, midi] of p.black) if (Math.abs(x - (p.x + (i + 1) * p.w)) <= p.bw / 2) return { id: `b${i}`, midi };
    }
    const i = Math.floor((x - p.x) / p.w);
    return { id: `w${i}`, midi: p.white[i] };
  }
  for (const [i, pad] of PADS.entries()) if (Math.hypot(x - pad.x, y - pad.y) < pad.r) return { id: `p${i}`, drum: pad.drum };
  return null;
}

/** Which goal mouth a ball centre is inside: 0 = left (Blue's), 1 = right (Red's), -1 = neither. */
export const BALL_R = 14;
export function netAt(x, y) {
  if (Math.abs(y - MID.y) >= NET.half || x < PITCH.x || x > PITCH.x + PITCH.w) return -1;
  return x < NET_X[0] + NET.depth - BALL_R ? 0 : x > NET_X[1] - NET.depth + BALL_R ? 1 : -1;
}
