/*
 * Space's drawing: floors, the race track, walls and door signs, furniture,
 * and the effects on people (fire, dripping water, puddles, balloons, bursts).
 * Pure canvas functions; space.js decides what to draw where. Decorative
 * motion uses clock(): stepped (4 fps), because Windows 98 never animates smoothly.
 */
import {
  WORLD, RADIUS, T, PLAZA, PLAZA_SPOT, ROOMS, WALLS, MID, NET, NET_X, POSTS, TAG_SPOT, TRACK, CORNERS, PIANO, PADS, DOCK, POND, BALL_R,
  GALLERY, FRAMES, ART, plaqueSpots,
} from './space-map.js';

export const FONT = 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif';
/** Time for decorative motion, stepped (4 fps). */
export const clock = () => Math.floor(performance.now() / 250) * 250;

const EDGE = '#0d0b1e';

function box(ctx, bx, by, bw, bh, fill, edge = EDGE) {
  ctx.fillStyle = fill;
  ctx.fillRect(bx, by, bw, bh);
  ctx.strokeStyle = edge;
  ctx.lineWidth = 3;
  ctx.strokeRect(bx, by, bw, bh);
}

function disc(ctx, cx, cy, r, fill) {
  ctx.fillStyle = fill;
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.fill();
}

// ── Floors, track, walls ───────────────────────────────────────────────────

export function drawFloor(ctx) {
  // Plaza: checkerboard tiles.
  for (let y = 0; y < WORLD.h; y += 80) {
    for (let x = 0; x < WORLD.w; x += 80) {
      ctx.fillStyle = (x / 80 + y / 80) % 2 ? '#2b2850' : '#312d5c';
      ctx.fillRect(x, y, 80, 80);
    }
  }
  drawTrack(ctx);
  ctx.fillStyle = 'rgba(255,255,255,0.08)';
  ctx.font = `800 44px ${FONT}`;
  ctx.textAlign = 'center';
  ctx.fillText(PLAZA.toUpperCase(), PLAZA_SPOT.x, PLAZA_SPOT.y + 15);
  for (const r of ROOMS) {
    ctx.fillStyle = r.floor;
    ctx.fillRect(r.x, r.y, r.w, r.h);
    if (r.name === 'Garden' || r.name === 'Pond') {
      ctx.fillStyle = 'rgba(255,255,255,0.05)';
      for (let i = 0; i < 50; i++) ctx.fillRect(r.x + ((i * 137) % r.w), r.y + ((i * 89) % r.h), 6, 10);
    }
    if (r.open) {
      // No walls, so a low dashed edge shows where the room (and its voice) ends.
      ctx.strokeStyle = 'rgba(255,255,255,0.28)';
      ctx.lineWidth = 3;
      ctx.setLineDash([12, 10]);
      ctx.strokeRect(r.x + 2, r.y + 2, r.w - 4, r.h - 4);
      ctx.setLineDash([]);
    }
    if (r.name === 'Pitch') drawPitch(ctx, r);
    if (r.name === 'Music') {
      // a stage rug under the instruments
      ctx.fillStyle = 'rgba(0,0,0,0.18)';
      ctx.fillRect(r.x + 40, r.y + 34, r.w - 80, r.h - 70);
    }
    if (r.name === 'Gallery') {
      // parquet
      ctx.strokeStyle = 'rgba(0,0,0,0.16)';
      ctx.lineWidth = 2;
      ctx.beginPath();
      for (let py = r.y + 30; py < r.y + r.h; py += 30) {
        ctx.moveTo(r.x, py);
        ctx.lineTo(r.x + r.w, py);
      }
      for (let px = r.x, i = 0; px < r.x + r.w; px += 90, i++) {
        for (let py = r.y + (i % 2) * 30; py < r.y + r.h; py += 60) {
          ctx.moveTo(px, py);
          ctx.lineTo(px, Math.min(py + 30, r.y + r.h));
        }
      }
      ctx.stroke();
    }
  }
}

/** Floor lettering, drawn after the furniture's floor parts but under people. */
export function drawRoomNames(ctx) {
  ctx.fillStyle = 'rgba(255,255,255,0.18)';
  ctx.textAlign = 'center';
  for (const r of ROOMS) {
    const [lx, ly] = r.label ?? [r.x + r.w / 2, r.y + r.h / 2];
    ctx.font = `800 ${r.w < 450 ? 38 : 44}px ${FONT}`;
    ctx.fillText(r.name.toUpperCase(), lx, ly + 14);
  }
}

/** The race track: a lane all the way round, run clockwise, start/finish on the left. */
function drawTrack(ctx) {
  const lane = TRACK.lane;
  ctx.fillStyle = '#7d3b2a';
  ctx.fillRect(0, 0, WORLD.w, lane);
  ctx.fillRect(0, WORLD.h - lane, WORLD.w, lane);
  ctx.fillRect(0, lane, lane, WORLD.h - lane * 2);
  ctx.fillRect(WORLD.w - lane, lane, lane, WORLD.h - lane * 2);
  // lane lines
  ctx.strokeStyle = 'rgba(255,255,255,0.35)';
  ctx.lineWidth = 2;
  ctx.setLineDash([16, 14]);
  ctx.strokeRect(56, 56, WORLD.w - 112, WORLD.h - 112);
  ctx.setLineDash([]);
  // chevrons pointing the way round
  ctx.strokeStyle = 'rgba(255,255,255,0.28)';
  ctx.lineWidth = 4;
  const chevron = (x, y, a) => {
    ctx.beginPath();
    ctx.moveTo(x + Math.cos(a + 2.5) * 12, y + Math.sin(a + 2.5) * 12);
    ctx.lineTo(x, y);
    ctx.lineTo(x + Math.cos(a - 2.5) * 12, y + Math.sin(a - 2.5) * 12);
    ctx.stroke();
  };
  for (let x = 400; x < WORLD.w - 200; x += 400) {
    chevron(x, 56, 0);
    chevron(x, WORLD.h - 56, Math.PI);
  }
  for (let y = 400; y < WORLD.h - 200; y += 400) {
    chevron(WORLD.w - 56, y, Math.PI / 2);
    if (Math.abs(y - TRACK.startY) > 60) chevron(56, y, -Math.PI / 2);
  }
  // corner flags (checkpoints, in order)
  CORNERS.forEach((c, i) => {
    const fx = c.x + c.w / 2;
    const fy = c.y + c.h / 2;
    ctx.fillStyle = '#ffd166';
    ctx.beginPath();
    ctx.moveTo(fx - 4, fy - 18);
    ctx.lineTo(fx + 14, fy - 12);
    ctx.lineTo(fx - 4, fy - 6);
    ctx.fill();
    ctx.fillStyle = '#e6e6e6';
    ctx.fillRect(fx - 6, fy - 20, 3, 30);
    ctx.font = `700 11px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.fillText(String(i + 1), fx + 4, fy + 22);
  });
  // start/finish: a chequered line across the left lane
  for (let i = 0; i < 11; i++) {
    for (let row = 0; row < 2; row++) {
      ctx.fillStyle = (i + row) % 2 ? '#111111' : '#f5f5f5';
      ctx.fillRect(12 + i * 8, TRACK.startY - 8 + row * 8, 8, 8);
    }
  }
  ctx.fillStyle = 'rgba(255,255,255,0.75)';
  ctx.font = `800 11px ${FONT}`;
  ctx.textAlign = 'center';
  ctx.fillText('FINISH', 56, TRACK.startY - 16);
  ctx.fillText('▲ RUN', 56, TRACK.startY + 26);
}

export function drawWalls(ctx) {
  // Walls: chunky with a highlight edge (retro bevel).
  for (const wall of WALLS) {
    ctx.fillStyle = EDGE;
    ctx.fillRect(wall.x, wall.y, wall.w, wall.h);
    ctx.fillStyle = 'rgba(255,255,255,0.14)';
    ctx.fillRect(wall.x, wall.y, wall.w, 3);
  }
  for (const r of ROOMS) for (const spot of plaqueSpots(r)) drawPlaque(ctx, r, spot.x, spot.y);
}

/** A room's name sign on its wall (on a post for the open Garden). */
function drawPlaque(ctx, r, x, doorY) {
  ctx.font = `700 13px ${FONT}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const w = ctx.measureText(r.name).width + 16;
  ctx.fillStyle = '#e8d9a8';
  ctx.fillRect(x - w / 2, doorY - 10, w, 20);
  ctx.strokeStyle = EDGE;
  ctx.lineWidth = 2;
  ctx.strokeRect(x - w / 2, doorY - 10, w, 20);
  ctx.fillStyle = '#2a1d0a';
  ctx.fillText(r.name, x, doorY + 1);
  ctx.textBaseline = 'alphabetic';
}

function drawPitch(ctx, r) {
  for (let i = 0; i * 60 < r.w; i++) {
    ctx.fillStyle = i % 2 ? '#3d8a46' : '#45964f';
    ctx.fillRect(r.x + i * 60, r.y, Math.min(60, r.w - i * 60), r.h);
  }
  ctx.strokeStyle = 'rgba(255,255,255,0.7)';
  ctx.lineWidth = 3;
  const left = NET_X[0] + NET.depth;
  const right = NET_X[1] - NET.depth;
  const top = r.y + T + 18;
  const bottom = r.y + r.h - T - 18;
  ctx.strokeRect(left, top, right - left, bottom - top);
  ctx.beginPath();
  ctx.moveTo(MID.x, top);
  ctx.lineTo(MID.x, bottom);
  ctx.moveTo(MID.x + 70, MID.y);
  ctx.arc(MID.x, MID.y, 70, 0, Math.PI * 2);
  ctx.stroke();
  ctx.strokeRect(left, MID.y - 130, 100, 260);
  ctx.strokeRect(right - 100, MID.y - 130, 100, 260);
  disc(ctx, MID.x, MID.y, 5, 'rgba(255,255,255,0.8)');
}

// ── Furniture ──────────────────────────────────────────────────────────────

/**
 * @param env {{ tagOn: boolean, score: number[], keysDown: Set<string>, laps: {name: string, ms: number}[], touch?: boolean }}
 */
export function drawDecor(ctx, kind, x, y, extra, env) {
  ctx.save();
  switch (kind) {
    case 'sofa':
      box(ctx, x - 80, y - 34, 160, 68, '#e45b8f');
      box(ctx, x - 80, y - 34, 160, 16, '#c94677');
      break;
    case 'rug':
      ctx.fillStyle = 'rgba(255,214,102,0.35)';
      ctx.beginPath();
      ctx.ellipse(x, y, 190, 90, 0, 0, Math.PI * 2);
      ctx.fill();
      break;
    case 'ctable':
      box(ctx, x - 55, y - 26, 110, 52, '#c79a5b');
      disc(ctx, x - 22, y, 8, '#ffffff');
      disc(ctx, x + 20, y - 6, 7, '#7afcff');
      break;
    case 'shelf':
      box(ctx, x - 150, y - 16, 300, 32, '#5a3a22');
      for (let i = 0; i < 18; i++) {
        ctx.fillStyle = ['#e45b8f', '#ffd166', '#7afcff', '#8aff80', '#ff9f45', '#b48cff'][i % 6];
        ctx.fillRect(x - 142 + i * 16, y - 11, 11, 22);
      }
      break;
    case 'plant':
      box(ctx, x - 18, y - 4, 36, 30, '#b4693e');
      disc(ctx, x, y - 16, 26, '#5cc46a');
      break;
    case 'lamp':
      disc(ctx, x, y, 80, 'rgba(255,220,140,0.07)');
      disc(ctx, x, y, 46, 'rgba(255,220,140,0.10)');
      disc(ctx, x, y, 14, '#3a3350');
      disc(ctx, x, y, 8, '#ffe9a8');
      break;
    case 'screen':
      drawScreen(ctx, x, y);
      break;
    case 'couch':
      // Facing the screen (up): the backrest is at the bottom.
      box(ctx, x - 80, y - 26, 160, 52, '#b8333f');
      box(ctx, x - 80, y + 16, 160, 16, '#8e2430');
      box(ctx, x - 88, y - 26, 12, 58, '#8e2430');
      box(ctx, x + 76, y - 26, 12, 58, '#8e2430');
      break;
    case 'popcorn':
      box(ctx, x - 34, y - 26, 68, 52, '#c0392b');
      ctx.fillStyle = '#ffffff';
      for (let i = 0; i < 4; i++) ctx.fillRect(x - 30 + i * 17, y - 22, 8, 44);
      for (let i = 0; i < 6; i++) disc(ctx, x - 25 + i * 10, y - 28 + (i % 2) * 4, 8, '#fff3b0');
      break;
    case 'piano':
      drawPiano(ctx, env.keysDown);
      break;
    case 'gallery':
      drawGallery(ctx, env.gallery);
      break;
    case 'pads':
      for (const [i, pad] of PADS.entries()) {
        const down = env.keysDown.has(`p${i}`);
        disc(ctx, pad.x, pad.y, pad.r + 4, EDGE);
        disc(ctx, pad.x, pad.y, pad.r, down ? '#ffffff' : pad.color);
        disc(ctx, pad.x, pad.y, pad.r * 0.55, down ? pad.color : 'rgba(255,255,255,0.25)');
        ctx.fillStyle = 'rgba(255,255,255,0.8)';
        ctx.font = `700 10px ${FONT}`;
        ctx.textAlign = 'center';
        ctx.fillText(pad.drum.toUpperCase(), pad.x, pad.y + pad.r + 16);
      }
      break;
    case 'mic':
      disc(ctx, x, y, 14, '#3a3350');
      ctx.strokeStyle = '#c0c0c0';
      ctx.lineWidth = 3;
      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.lineTo(x + 14, y - 22);
      ctx.stroke();
      disc(ctx, x + 16, y - 25, 7, '#d0d0d0');
      break;
    case 'speaker':
      box(ctx, x - 32, y - 40, 64, 80, '#1b1b22');
      disc(ctx, x, y - 14, 15, '#3a3a48');
      disc(ctx, x, y - 14, 6, '#111');
      disc(ctx, x, y + 22, 9, '#3a3a48');
      break;
    case 'cabinet':
      box(ctx, x - 36, y - 50, 72, 100, '#1f2a44');
      ctx.fillStyle = ['#ff6ad5', '#7afcff', '#fff07a', '#8aff80'][Math.floor(x / 90) % 4];
      ctx.fillRect(x - 24, y - 38, 48, 34);
      disc(ctx, x - 10, y + 22, 6, '#e5484d');
      disc(ctx, x + 12, y + 22, 6, '#ffd166');
      break;
    case 'stool':
      disc(ctx, x, y, 17, EDGE);
      disc(ctx, x, y, 14, '#c0392b');
      disc(ctx, x, y, 7, '#e74c3c');
      break;
    case 'pool':
      box(ctx, x - 125, y - 65, 250, 130, '#6b4423');
      ctx.fillStyle = '#1f7a4a';
      ctx.fillRect(x - 110, y - 50, 220, 100);
      for (const [px, py] of [[-110, -50], [0, -52], [110, -50], [-110, 50], [0, 52], [110, 50]]) disc(ctx, x + px, y + py, 9, EDGE);
      disc(ctx, x - 60, y, 8, '#ffffff');
      for (const [i, [px, py]] of [[30, 0], [44, -8], [44, 8], [58, -16], [58, 0], [58, 16]].entries()) disc(ctx, x + px, y + py, 7, ['#ffd166', '#e5484d', '#5b6cff', '#30a46c', EDGE, '#f76b15'][i]);
      break;
    case 'claw':
      box(ctx, x - 42, y - 46, 84, 92, '#d6409f');
      ctx.fillStyle = 'rgba(168,230,255,0.55)';
      ctx.fillRect(x - 32, y - 36, 64, 50);
      for (const [i, [px, py]] of [[-16, 4], [0, 8], [14, 2], [-4, -6]].entries()) disc(ctx, x + px, y + py, 7, ['#ffd166', '#8aff80', '#ff9f45', '#b48cff'][i]);
      ctx.strokeStyle = EDGE;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x + 6, y - 36);
      ctx.lineTo(x + 6, y - 18);
      ctx.stroke();
      break;
    case 'tree':
      ctx.fillStyle = 'rgba(0,0,0,0.25)';
      ctx.beginPath();
      ctx.ellipse(x + 10, y + 30, 60, 22, 0, 0, Math.PI * 2);
      ctx.fill();
      disc(ctx, x, y, 58, '#2f7a3a');
      disc(ctx, x - 14, y - 14, 34, '#3f9a4a');
      break;
    case 'fountain': {
      disc(ctx, x, y, 98, EDGE);
      disc(ctx, x, y, 95, '#8d99a6');
      disc(ctx, x, y, 82, '#3f8fd0');
      ripples(ctx, x, y, 24, 56);
      disc(ctx, x, y, 18, '#8d99a6');
      disc(ctx, x, y, 9, '#dff3ff');
      break;
    }
    case 'pond':
      drawPond(ctx);
      break;
    case 'dock':
      for (const r of DOCK) box(ctx, r.x, r.y, r.w, r.h, '#a0703f', '#4a2f17');
      ctx.strokeStyle = 'rgba(74,47,23,0.7)';
      ctx.lineWidth = 2;
      ctx.beginPath();
      for (let py = DOCK[0].y + 18; py < DOCK[1].y; py += 18) {
        ctx.moveTo(DOCK[0].x, py);
        ctx.lineTo(DOCK[0].x + DOCK[0].w, py);
      }
      for (let px = DOCK[1].x + 18; px < DOCK[1].x + DOCK[1].w; px += 18) {
        ctx.moveTo(px, DOCK[1].y);
        ctx.lineTo(px, DOCK[1].y + DOCK[1].h);
      }
      ctx.stroke();
      for (const [px, py] of [[DOCK[1].x, DOCK[1].y + DOCK[1].h], [DOCK[1].x + DOCK[1].w, DOCK[1].y + DOCK[1].h]]) disc(ctx, px, py, 6, '#4a2f17');
      break;
    case 'reeds':
      ctx.strokeStyle = '#3f7a2c';
      ctx.lineWidth = 3;
      for (let i = 0; i < 6; i++) {
        const rx = x + (i - 2.5) * 7;
        ctx.beginPath();
        ctx.moveTo(rx, y + 18);
        ctx.lineTo(rx + (i % 2 ? 4 : -4), y - 16 - (i % 3) * 6);
        ctx.stroke();
        disc(ctx, rx + (i % 2 ? 4 : -4), y - 18 - (i % 3) * 6, 4, '#7a4a22');
      }
      break;
    case 'bench':
      box(ctx, x - 58, y - 14, 116, 28, '#a0522d');
      // The backrest is on the side away from where you face.
      box(ctx, x - 58, extra > 0 ? y - 22 : y + 14, 116, 8, '#7a3d1f');
      break;
    case 'flowers':
      for (let i = 0; i < 7; i++) disc(ctx, x + Math.cos(i) * 40, y + Math.sin(i * 2) * 18, 7, ['#ff7eb6', '#ffd166', '#ffffff'][i % 3]);
      break;
    case 'net': {
      const nx = extra ? x - NET.depth : x;
      ctx.fillStyle = 'rgba(255,255,255,0.12)';
      ctx.fillRect(nx, y - NET.half, NET.depth, NET.half * 2);
      ctx.strokeStyle = 'rgba(255,255,255,0.35)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let gx = nx + 8; gx < nx + NET.depth; gx += 9) {
        ctx.moveTo(gx, y - NET.half);
        ctx.lineTo(gx, y + NET.half);
      }
      for (let gy = y - NET.half + 9; gy < y + NET.half; gy += 9) {
        ctx.moveTo(nx, gy);
        ctx.lineTo(nx + NET.depth, gy);
      }
      ctx.stroke();
      // Blue's goal on the left, Red's on the right.
      ctx.fillStyle = extra ? '#e5484d' : '#5b6cff';
      ctx.fillRect(extra ? nx : nx + NET.depth - 5, y - NET.half, 5, NET.half * 2);
      for (const post of POSTS.slice(extra * 2, extra * 2 + 2)) box(ctx, post.x, post.y, post.w, post.h, '#f5f5f5');
      break;
    }
    case 'scoreboard': {
      box(ctx, x - 100, y - 24, 200, 48, EDGE, '#3a3350');
      const [blue, red] = env.score;
      ctx.textAlign = 'center';
      ctx.font = `700 10px ${FONT}`;
      ctx.fillStyle = '#9aa3ff';
      ctx.fillText('BLUE', x - 62, y - 6);
      ctx.fillStyle = '#ff9a9e';
      ctx.fillText('RED', x + 62, y - 6);
      ctx.font = `800 24px ${FONT}`;
      ctx.fillStyle = '#ffffff';
      ctx.fillText(`${blue} : ${red}`, x, y + 9);
      break;
    }
    case 'tent':
      ctx.fillStyle = '#e07a2e';
      ctx.strokeStyle = EDGE;
      ctx.lineWidth = 3;
      ctx.beginPath();
      ctx.moveTo(x - 50, y + 36);
      ctx.lineTo(x + 50, y + 36);
      ctx.lineTo(x + 38, y - 36);
      ctx.lineTo(x - 38, y - 36);
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
      ctx.strokeStyle = '#a5521a';
      ctx.beginPath();
      ctx.moveTo(x, y - 36);
      ctx.lineTo(x, y + 36);
      ctx.stroke();
      ctx.fillStyle = '#3b2412';
      ctx.beginPath();
      ctx.moveTo(x - 16, y + 36);
      ctx.lineTo(x + 16, y + 36);
      ctx.lineTo(x, y + 6);
      ctx.fill();
      break;
    case 'rock':
      ctx.fillStyle = '#6f6a7d';
      ctx.beginPath();
      ctx.ellipse(x, y, 26, 18, 0.3, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = 'rgba(255,255,255,0.15)';
      ctx.beginPath();
      ctx.ellipse(x - 6, y - 6, 10, 6, 0.3, 0, Math.PI * 2);
      ctx.fill();
      break;
    case 'log':
      box(ctx, x - 40, y - 14, 80, 28, '#6b4423');
      break;
    case 'fire': {
      // the real fire: walk into it and you catch
      const t = performance.now() / 120;
      disc(ctx, x, y, 46, 'rgba(255,140,40,0.16)');
      for (let i = 0; i < 3; i++) {
        ctx.fillStyle = ['#ff6b35', '#ffb347', '#fff07a'][i];
        ctx.beginPath();
        ctx.ellipse(x, y - i * 10, 34 - i * 10 + Math.sin(t + i) * 4, 44 - i * 12 + Math.cos(t + i) * 4, 0, 0, Math.PI * 2);
        ctx.fill();
      }
      break;
    }
    case 'tagspot': {
      const color = env.tagOn ? '#ff7a7a' : '#ffd166';
      disc(ctx, x, y, TAG_SPOT.r, env.tagOn ? 'rgba(229,72,77,0.28)' : 'rgba(255,214,102,0.16)');
      ctx.strokeStyle = color;
      ctx.lineWidth = 3;
      ctx.setLineDash([8, 6]);
      ctx.beginPath();
      ctx.arc(x, y, TAG_SPOT.r, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = color;
      ctx.textAlign = 'center';
      ctx.font = `800 20px ${FONT}`;
      ctx.fillText('TAG', x, y + 7);
      ctx.font = `600 12px ${FONT}`;
      ctx.fillText(env.tagOn ? 'game on!' : 'step in to play', x, y + TAG_SPOT.r + 18);
      break;
    }
    case 'laps': {
      // The lap records board, on the Plaza.
      box(ctx, x - 110, y - 46, 220, 92, EDGE, '#ffd166');
      ctx.textAlign = 'center';
      ctx.font = `800 13px ${FONT}`;
      ctx.fillStyle = '#ffd166';
      ctx.fillText('🏁 LAP RECORDS', x, y - 26);
      ctx.font = `600 12px ${FONT}`;
      ctx.fillStyle = '#ffffff';
      if (!env.laps.length) {
        ctx.fillText('Run a lap of the track:', x, y - 4);
        ctx.fillText('start on the left, go clockwise', x, y + 14);
        ctx.fillStyle = 'rgba(255,255,255,0.6)';
        ctx.fillText(env.touch ? '(tap along the track to go round)' : '(hold Shift to run)', x, y + 32);
      } else {
        env.laps.slice(0, 3).forEach((row, i) => {
          const name = row.name.length > 14 ? `${row.name.slice(0, 13)}…` : row.name;
          ctx.textAlign = 'left';
          ctx.fillText(`${i + 1}. ${name}`, x - 96, y - 4 + i * 18);
          ctx.textAlign = 'right';
          ctx.fillText(`${(row.ms / 1000).toFixed(1)} s`, x + 96, y - 4 + i * 18);
        });
      }
      break;
    }
  }
  ctx.restore();
}

function ripples(ctx, x, y, from, span) {
  const t = clock();
  ctx.strokeStyle = '#ffffff';
  ctx.lineWidth = 2;
  for (let i = 0; i < 3; i++) {
    const r = from + ((t / 30 + i * (span / 3)) % span);
    ctx.globalAlpha = 0.5 * (1 - (r - from) / span);
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
}

/** The pond: water, lily pads, two ducks paddling round. */
function drawPond(ctx) {
  const { x, y, rx, ry } = POND;
  ctx.fillStyle = '#c9b77a'; // a sandy edge
  ctx.beginPath();
  ctx.ellipse(x, y, rx + 10, ry + 10, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#2f7fb8';
  ctx.beginPath();
  ctx.ellipse(x, y, rx, ry, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = 'rgba(255,255,255,0.08)';
  ctx.beginPath();
  ctx.ellipse(x - 30, y - 40, rx * 0.6, ry * 0.45, 0, 0, Math.PI * 2);
  ctx.fill();
  for (const [px, py, r] of [[x - 95, y - 50, 14], [x + 90, y + 40, 16], [x - 60, y + 95, 12], [x + 105, y - 85, 11], [x - 110, y + 30, 10]]) {
    ctx.fillStyle = '#4caf50';
    ctx.beginPath();
    ctx.moveTo(px, py);
    ctx.arc(px, py, r, 0.4, Math.PI * 2 - 0.1);
    ctx.closePath();
    ctx.fill();
  }
  disc(ctx, x - 60, y + 95, 4, '#ff9ecb');
  const t = clock() / 5000;
  for (const [phase, size] of [[0, 1], [Math.PI, 0.85]]) {
    const a = t * (size === 1 ? 1 : -1.3) + phase;
    const dx = x + Math.cos(a) * rx * 0.55;
    const dy = y + 40 + Math.sin(a) * ry * 0.5;
    const heading = a + (size === 1 ? Math.PI / 2 : -Math.PI / 2);
    ctx.save();
    ctx.translate(dx, dy);
    ctx.rotate(heading);
    ctx.scale(size, size);
    ctx.fillStyle = 'rgba(255,255,255,0.25)';
    ctx.beginPath();
    ctx.ellipse(-10, 0, 16, 9, 0, 0, Math.PI * 2);
    ctx.fill();
    disc(ctx, 0, 0, 10, '#fff4d6');
    disc(ctx, 9, 0, 6, '#2e7d32');
    ctx.fillStyle = '#ff9800';
    ctx.beginPath();
    ctx.moveTo(14, -2);
    ctx.lineTo(20, 0);
    ctx.lineTo(14, 2);
    ctx.fill();
    ctx.restore();
  }
}

/**
 * The Gallery's walls: twelve frames, the drawings in them with crisp pixels
 * (no smoothing — they're pixel art), a name plate under each, and
 * hard-edged beams of light from little lamps.
 * @param g {{ items: {media: string, uid: string, ts: number}[], image: (media: string) => HTMLImageElement|null,
 *             user: (uid: string) => {name: string, color: string}|null, focus: number, page: number, pages: number, total: number }}
 */
function drawGallery(ctx, g) {
  for (const f of FRAMES) {
    const cx = f.x + f.w / 2;
    const toward = f.row ? -1 : 1; // which way the light falls: into the aisle
    const wallY = f.row ? f.y + f.h + ART.plate + 6 : f.y - 6;
    const reach = f.y + (f.row ? -40 : f.h + ART.plate + 40);
    ctx.fillStyle = 'rgba(255,250,210,0.13)';
    ctx.beginPath();
    ctx.moveTo(cx - 24, wallY);
    ctx.lineTo(cx + 24, wallY);
    ctx.lineTo(f.x + f.w + 14, reach);
    ctx.lineTo(f.x - 14, reach);
    ctx.closePath();
    ctx.fill();
    // the little lamp on the wall
    ctx.fillStyle = '#16141f';
    ctx.fillRect(cx - 12, wallY - (toward > 0 ? 6 : 0), 24, 6);
    disc(ctx, cx, wallY + toward * 2, 3, '#ffe9a8');
  }
  const firstEmpty = g.items.length;
  for (const f of FRAMES) {
    const item = g.items[f.i];
    const ax = f.x + ART.border;
    const ay = f.y + ART.border;
    if (!item) {
      ctx.strokeStyle = 'rgba(255,255,255,0.22)';
      ctx.lineWidth = 2;
      ctx.setLineDash([6, 5]);
      ctx.strokeRect(f.x + 1, f.y + 1, f.w - 2, f.h - 2);
      ctx.setLineDash([]);
      if (f.i === firstEmpty) {
        ctx.fillStyle = 'rgba(255,255,255,0.55)';
        ctx.font = `600 10px ${FONT}`;
        ctx.textAlign = 'center';
        ctx.fillText(g.total ? 'more soon…' : 'Save a drawing', f.x + f.w / 2, f.y + f.h / 2 - 2);
        if (!g.total) ctx.fillText('in Draw ✏️', f.x + f.w / 2, f.y + f.h / 2 + 12);
      }
      continue;
    }
    // frame: dark wood with a thin gold lip, then the drawing itself
    ctx.fillStyle = '#2b2018';
    ctx.fillRect(f.x, f.y, f.w, f.h);
    ctx.strokeStyle = '#c9a227';
    ctx.lineWidth = 1;
    ctx.strokeRect(f.x + 2.5, f.y + 2.5, f.w - 5, f.h - 5);
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(ax, ay, ART.w, ART.h);
    const img = g.image(item.media);
    if (img) {
      ctx.save();
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, ax, ay, ART.w, ART.h);
      ctx.restore();
    }
    if (g.focus === f.i) {
      ctx.strokeStyle = '#ffd166';
      ctx.lineWidth = 3;
      ctx.strokeRect(f.x - 3, f.y - 3, f.w + 6, f.h + 6);
    }
    // name plate: the artist in their colour, and the date
    const who = g.user(item.uid);
    const py = f.y + f.h + 1;
    ctx.fillStyle = '#15131c';
    ctx.fillRect(f.x + 6, py, f.w - 12, ART.plate - 1);
    ctx.font = `700 9px ${FONT}`;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    const name = who?.name ?? 'Someone';
    ctx.fillStyle = /^#[0-9a-f]{3,8}$/i.test(who?.color ?? '') ? who.color : '#dddddd';
    ctx.fillText(name.length > 11 ? `${name.slice(0, 10)}…` : name, f.x + 9, py + ART.plate / 2);
    ctx.textAlign = 'right';
    ctx.fillStyle = '#9a96a8';
    ctx.font = `600 9px ${FONT}`;
    ctx.fillText(item.when ?? '', f.x + f.w - 9, py + ART.plate / 2);
    ctx.textBaseline = 'alphabetic';
  }
  // which wall you're looking at, at both ends of the aisle
  if (g.pages > 1) {
    ctx.fillStyle = 'rgba(255,255,255,0.7)';
    ctx.font = `700 11px ${FONT}`;
    ctx.textAlign = 'center';
    const midY = FRAMES[0].look.y + (FRAMES[6].y - FRAMES[0].look.y) / 2 + 4;
    for (const x of [GALLERY.x + T + 20, GALLERY.x + GALLERY.w - T - 20]) ctx.fillText(`${g.page + 1}/${g.pages}`, x, midY);
  }
}

/** The floor piano: white keys, black keys, a lit key under anyone standing on it. */
function drawPiano(ctx, down) {
  const p = PIANO;
  ctx.fillStyle = EDGE;
  ctx.fillRect(p.x - 6, p.y - 8, p.w * p.white.length + 12, p.h + 14);
  p.white.forEach((_, i) => {
    ctx.fillStyle = down.has(`w${i}`) ? '#ffe08a' : '#f4f1e8';
    ctx.fillRect(p.x + i * p.w + 1, p.y, p.w - 2, p.h);
  });
  for (const [i] of p.black) {
    const cx = p.x + (i + 1) * p.w;
    ctx.fillStyle = down.has(`b${i}`) ? '#8f7bff' : '#16141f';
    ctx.fillRect(cx - p.bw / 2, p.y, p.bw, p.bh);
  }
  ctx.fillStyle = 'rgba(255,255,255,0.75)';
  ctx.font = `600 12px ${FONT}`;
  ctx.textAlign = 'center';
  ctx.fillText('♪ walk on the keys and the drums ♪', p.x + (p.w * p.white.length) / 2, p.y + p.h + 26);
}

/** The cinema screen: a new shot every couple of seconds, and the projector's beam. */
function drawScreen(ctx, x, y) {
  const half = 230;
  ctx.fillStyle = 'rgba(255,255,255,0.05)';
  ctx.beginPath();
  ctx.moveTo(x - half, y + 30);
  ctx.lineTo(x + half, y + 30);
  ctx.lineTo(x + 30, y + 330);
  ctx.lineTo(x - 30, y + 330);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = EDGE;
  ctx.fillRect(x - half - 12, y - 34, half * 2 + 24, 68);
  const sx = x - half;
  const sy = y - 26;
  const sw = half * 2;
  const sh = 52;
  ctx.save();
  ctx.beginPath();
  ctx.rect(sx, sy, sw, sh);
  ctx.clip();
  const shot = Math.floor(performance.now() / 2500) % 4;
  if (shot === 0) {
    const sky = ctx.createLinearGradient(0, sy, 0, sy + sh);
    sky.addColorStop(0, '#ff9f45');
    sky.addColorStop(1, '#8e4ec6');
    ctx.fillStyle = sky;
    ctx.fillRect(sx, sy, sw, sh);
    disc(ctx, x + 100, sy + sh - 6, 22, '#ffe07a');
    ctx.fillStyle = '#2b1d3a';
    ctx.beginPath();
    ctx.moveTo(sx, sy + sh);
    for (let i = 0; i <= 8; i++) ctx.lineTo(sx + (i * sw) / 8, sy + sh - 10 - (i % 2) * 12);
    ctx.lineTo(sx + sw, sy + sh);
    ctx.fill();
  } else if (shot === 1) {
    ctx.fillStyle = '#0b1030';
    ctx.fillRect(sx, sy, sw, sh);
    ctx.fillStyle = '#ffffff';
    for (let i = 0; i < 34; i++) ctx.fillRect(sx + ((i * 97) % sw), sy + ((i * 37) % sh), 2, 2);
    disc(ctx, x - 120, y, 18, '#ff7eb6');
  } else if (shot === 2) {
    ctx.fillStyle = '#2a8fd6';
    ctx.fillRect(sx, sy, sw, sh);
    ctx.strokeStyle = '#bfe6ff';
    ctx.lineWidth = 2;
    for (let row = 0; row < 3; row++) {
      ctx.beginPath();
      for (let i = 0; i <= 24; i++) ctx.lineTo(sx + i * 20, sy + 14 + row * 14 + (i % 2) * 4);
      ctx.stroke();
    }
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.moveTo(x + 40, sy + 8);
    ctx.lineTo(x + 40, sy + 34);
    ctx.lineTo(x + 64, sy + 34);
    ctx.fill();
  } else {
    ctx.fillStyle = '#111111';
    ctx.fillRect(sx, sy, sw, sh);
    ctx.fillStyle = '#ffd166';
    ctx.font = `800 24px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.fillText('YAP PICTURES', x, y + 9);
  }
  ctx.restore();
}

// ── Things that move ───────────────────────────────────────────────────────

export function drawBall(ctx, x, y, spin) {
  ctx.save();
  ctx.fillStyle = 'rgba(0,0,0,0.3)';
  ctx.beginPath();
  ctx.ellipse(x + 3, y + BALL_R - 2, BALL_R, 5, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = '#ffffff';
  ctx.strokeStyle = EDGE;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.arc(x, y, BALL_R, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  // Patches turn as it rolls.
  for (let i = 0; i < 3; i++) {
    const a = spin + (i * Math.PI * 2) / 3;
    disc(ctx, x + Math.cos(a) * 7, y + Math.sin(a) * 7, 3.2, EDGE);
  }
  ctx.restore();
}

/** A water balloon in flight: its shadow on the ground, the balloon `lift` above it. */
export function drawBalloon(ctx, x, y, lift) {
  ctx.save();
  ctx.fillStyle = 'rgba(0,0,0,0.25)';
  ctx.beginPath();
  ctx.ellipse(x, y + 8, 8, 4, 0, 0, Math.PI * 2);
  ctx.fill();
  const by = y - lift;
  ctx.fillStyle = '#4fa3ff';
  ctx.strokeStyle = '#1f5d9e';
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.ellipse(x, by, 9, 11, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  disc(ctx, x - 3, by - 4, 2.5, 'rgba(255,255,255,0.8)');
  ctx.fillStyle = '#1f5d9e';
  ctx.beginPath();
  ctx.moveTo(x - 3, by + 13);
  ctx.lineTo(x + 3, by + 13);
  ctx.lineTo(x, by + 9);
  ctx.fill();
  ctx.restore();
}

export function drawPuddle(ctx, x, y, alpha) {
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.fillStyle = '#5fb8ff';
  ctx.beginPath();
  ctx.ellipse(x, y, 13, 6, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = 'rgba(255,255,255,0.5)';
  ctx.beginPath();
  ctx.ellipse(x - 4, y - 1, 4, 1.5, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

/** Someone on fire: a glow under them (drawn before the body). */
export function drawGlow(ctx, x, y) {
  disc(ctx, x, y, RADIUS + 10, 'rgba(255,110,30,0.28)');
}

/** Someone on fire: flickering flames over their head and embers floating up. */
export function drawFlames(ctx, x, y) {
  const t = clock();
  ctx.save();
  for (const [color, scale] of [['#ff5a1f', 1], ['#ffb347', 0.62], ['#fff07a', 0.3]]) {
    ctx.fillStyle = color;
    for (let i = 0; i < 5; i++) {
      const fx = x + (i - 2) * 9;
      const base = y - 8 - Math.abs(i - 2) * 4;
      const h = (26 - Math.abs(i - 2) * 6 + Math.sin(t / 70 + i * 1.9) * 6) * scale;
      const w = 7 * (0.6 + scale * 0.4);
      ctx.beginPath();
      ctx.moveTo(fx - w, base);
      ctx.quadraticCurveTo(fx - w, base - h * 0.6, fx + Math.sin(t / 90 + i) * 2, base - h - 8 * scale);
      ctx.quadraticCurveTo(fx + w, base - h * 0.6, fx + w, base);
      ctx.closePath();
      ctx.fill();
    }
  }
  for (let i = 0; i < 6; i++) {
    const ph = (t / 900 + i / 6) % 1;
    ctx.globalAlpha = 1 - ph;
    disc(ctx, x + Math.sin(i * 2.7 + ph * 5) * 18, y - RADIUS - 6 - ph * 48, 2.4, i % 2 ? '#ffb347' : '#ff7a1a');
  }
  ctx.restore();
}

/** Someone dripping: a wet sheen on them and drops falling off. */
export function drawWet(ctx, x, y) {
  const t = clock();
  ctx.save();
  disc(ctx, x, y, RADIUS, 'rgba(90,170,255,0.22)');
  ctx.strokeStyle = 'rgba(170,225,255,0.9)';
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.arc(x, y, RADIUS - 5, -2.7, -1.2);
  ctx.stroke();
  for (let i = 0; i < 4; i++) {
    const ph = (t / 650 + i * 0.27) % 1;
    const dx = x + (i - 1.5) * 13;
    const dy = y + RADIUS * 0.3 + ph * 34;
    ctx.globalAlpha = 1 - ph;
    ctx.fillStyle = '#6cc4ff';
    ctx.beginPath();
    ctx.moveTo(dx, dy - 6);
    ctx.quadraticCurveTo(dx + 4, dy, dx, dy + 3);
    ctx.quadraticCurveTo(dx - 4, dy, dx, dy - 6);
    ctx.fill();
  }
  ctx.restore();
}

const CONFETTI = ['#ff6ad5', '#7afcff', '#fff07a', '#8aff80', '#ff9f45', '#9aa3ff'];

/**
 * Short-lived bursts: steam (fire meets water), a splash, a note from an
 * instrument, confetti for a goal. `age` in ms; false once it's over.
 */
export function drawBurst(ctx, b, x, y, age) {
  const life = { steam: 900, splash: 500, note: 1200, confetti: 2400 }[b.kind] ?? 1000;
  if (age > life) return false;
  const ph = age / life;
  ctx.save();
  if (b.kind === 'steam') {
    for (let i = 0; i < 5; i++) {
      ctx.globalAlpha = (1 - ph) * 0.7;
      disc(ctx, x + Math.sin(i * 1.7) * 16 * (0.4 + ph), y - RADIUS - ph * 42 - i * 5, 7 + ph * 10, '#e8eef4');
    }
  } else if (b.kind === 'splash') {
    ctx.globalAlpha = 1 - ph;
    ctx.strokeStyle = '#8fd3ff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(x, y, 8 + ph * 24, 0, Math.PI * 2);
    ctx.stroke();
    for (let i = 0; i < 8; i++) {
      const a = (i * Math.PI) / 4;
      disc(ctx, x + Math.cos(a) * (6 + ph * 30), y + Math.sin(a) * (6 + ph * 30) - Math.sin(ph * Math.PI) * 10, 3, '#6cc4ff');
    }
  } else if (b.kind === 'note') {
    ctx.globalAlpha = 1 - ph;
    ctx.fillStyle = b.color ?? '#ffffff';
    ctx.font = `700 22px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.fillText(b.glyph ?? '♪', x + Math.sin(ph * 6) * 8, y - ph * 44);
  } else if (b.kind === 'confetti') {
    for (let i = 0; i < 16; i++) {
      const a = -Math.PI / 2 + ((i - 7.5) / 8) * 1.3;
      const v = 60 + (i % 4) * 18;
      ctx.globalAlpha = 1 - ph * ph;
      ctx.fillStyle = CONFETTI[i % CONFETTI.length];
      ctx.fillRect(x + Math.cos(a) * v * ph * 1.4, y - RADIUS - 10 + Math.sin(a) * v * ph + ph * ph * 120, 5, 8);
    }
  }
  ctx.restore();
  return true;
}
