// Places in the main pane: the board, Draw, Space, When's good, a shared
// screen and Watch together. Voice keeps going while one is open.

import { settings } from '../settings.js';
import { BoardCursors } from '../cursors.js';
import { DrawBoard } from '../draw.js';
import { SpaceWorld } from '../space.js';
import { WhenPlanner } from '../when.js';
import { WatchParty } from '../watch.js';
import { $, conn, isMod, state } from './state.js';
import { avatar, toast } from './ui.js';
import { shareDrawing, uploadDrawing } from './uploads.js';
import { screenView, stopWatchingScreen, voice } from './voice-ui.js';
import { renderChannels, renderHeader, renderLinks } from './sidebar.js';

export let boardCursors = null;
export let drawBoard = null;
export let spaceWorld = null;
export let whenPlanner = null;

// Watch together: one YouTube player that keeps going while you browse (web/js/watch.js).
export const watchParty = new WatchParty($('watch-view'), {
  send: (m) => conn?.send(m),
  mySid: () => state.sid,
  myUid: () => settings.uid,
  sessions: () => state.sessions,
  users: () => state.users,
  avatar: (user, cls) => avatar(user, cls),
  speaking: (sid) => state.speaking.has(sid),
  open: () => setView('watch'),
  toast: (text) => toast(text),
  changed: () => renderLinks(),
});

/** The board's name: its Places link, if this server has a board (SELF_HOSTING.md › A board). */
const boardName = () => (state.links ?? []).find((l) => l.view === 'board')?.name ?? 'Board';

export const VIEWS = {
  board: { get title() { return boardName(); }, icon: 'board', el: 'board-view' },
  draw: { title: 'Draw', icon: 'brush', el: 'draw-view' },
  space: { title: 'Space', icon: 'map', el: 'space-view' },
  when: { title: "When's good", icon: 'calendar', el: 'when-view' },
  screen: { title: 'Screen share', icon: 'monitor', el: 'screen-view' },
  watch: { title: 'Watch together', icon: 'tv', el: 'watch-view' },
};

export function setView(view) {
  if (state.view === view) return;
  const leaving = state.view;
  state.view = view;
  for (const [name, v] of Object.entries(VIEWS)) $(v.el).hidden = name !== view;
  $('messages').hidden = view !== 'chat';
  document.querySelector('.composer-wrap').hidden = view !== 'chat';
  if (leaving === 'board') boardCursors?.hide();
  if (leaving === 'space') spaceWorld?.close();
  if (leaving === 'screen' && !screenView?.poppedOut) stopWatchingScreen();
  conn?.send({ t: 'view', view: view === 'chat' ? null : view });
  if (view === 'board') ensureBoardFrame();
  if (view === 'draw') {
    drawBoard ??= new DrawBoard($('draw-view'), {
      send: (m) => conn?.send(m),
      upload: uploadDrawing,
      share: shareDrawing,
      who: (uid) => state.users.get(uid) ?? null,
      whoSid: (sid) => state.users.get(state.sessions.get(sid)?.uid) ?? null,
      me: () => settings.uid,
      toast,
    });
    drawBoard.open();
  }
  if (view === 'space') {
    spaceWorld ??= new SpaceWorld($('space-view'), {
      send: (m) => conn?.send(m),
      mySid: () => state.sid,
      who: (sid) => state.users.get(state.sessions.get(sid)?.uid) ?? null,
      proximity: (sid, f, pan) => voice.setProximity(sid, f, pan ?? null),
      setLook: (look) => conn?.send({ t: 'profile', look }),
      inCall: (sid) => {
        const v = state.sessions.get(sid)?.voice;
        return state.voiceChannel && v?.channelId === state.voiceChannel ? v : null;
      },
      user: (uid) => state.users.get(uid) ?? null,
    });
    spaceWorld.open();
  }
  if (view === 'when') {
    whenPlanner ??= new WhenPlanner($('when-view'), {
      send: (m) => conn?.send(m),
      me: () => settings.uid,
      users: () => state.users,
      hangs: () => [...state.hangs.values()].sort((a, b) => a.at - b.at),
      canCancel: (h) => h.uid === settings.uid || isMod(),
      voiceChannels: () => [...state.channels.values()].filter((c) => c.kind === 'voice').sort((a, b) => a.position - b.position),
    });
    whenPlanner.open();
  }
  watchParty.setView(view); // the video moves between the Watch view and the mini player
  $('app').classList.remove('sidebar-open');
  renderHeader();
  renderChannels();
  renderLinks();
}

function ensureBoardFrame() {
  if ($('board-frame')) return;
  // Same origin as Yap: pre-fill the board's own "your name" and theme (the
  // keys a board can read: gk-name, gk-theme, from the first board Yap embedded).
  try {
    if (!localStorage.getItem('gk-name') && settings.name) localStorage.setItem('gk-name', settings.name);

  } catch {}
  const frame = document.createElement('iframe');
  frame.id = 'board-frame';
  frame.src = '/board';
  frame.title = boardName();
  $('board-view').append(frame);
  boardCursors = new BoardCursors(frame, {
    send: (a) => conn?.send({ t: 'cursor', view: 'board', a }),
    who: (sid) => {
      const s = state.sessions.get(sid);
      const u = s && state.users.get(s.uid);
      return u ? { name: u.name, color: u.color } : null;
    },
  });
}
