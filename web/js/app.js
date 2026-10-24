// Yap's web client starts here: boot, what the server sends (handle) and
// every click (onClick). The page's parts are in js/app/; the widgets they
// use (voice.js, draw.js, space.js, watch.js…) are next to this file.

import { Connection } from './conn.js';
import { settings, save, api, keyProtocols, socketUrl } from './settings.js';
import { sounds, playChime, makeClip } from './sounds.js';
import { icon, hydrateIcons } from './icons.js';
import { esc } from './format.js';
import { loadStandardEmojis } from './emoji.js';
import { loadConfig } from './config.js';
import { confirmSheet, bindConfirmSheet } from './sheet.js';
import { forgetKey } from './vault.js';
import { $, TYPING_MS, coarsePointer, conn, findMessage, firstTextChannel, isMod, noHover, setConn, state } from './app/state.js';
import { closePopover, copyByHand, hideGate, openedBy, showGate, toast } from './app/ui.js';
import { applyTheme } from './app/look.js';
import { postSound } from './app/uploads.js';
import { bindUpdates, onBuild, reloadForUpdate, restoreAfterUpdate } from './app/updates.js';
import { syncBrowserPush } from './app/push.js';
import { VIEWS, boardCursors, drawBoard, setView, spaceWorld, watchParty, whenPlanner } from './app/places.js';
import { adoptResumed, bindShareDialog, onVoiceRowWheel, setVolumeFor, changeScreenShare, deferLeaveChime, dismissRejoin, dropGhost, holdGhost, joinVoice, leaveVoice, onSound, onVoice, openVoiceMore, rejoinVoice, renderSharePanel, renderVoicePanel, stopWatchingScreen, toggleDeafen, toggleMute, toggleScreenShare, voice, watchScreen } from './app/voice-ui.js';
import { bindChannelDialog, bindLinkDialog, openChannelDialog, openLinkDialog, renderChannels, renderConn, renderHeader, renderLinks, renderRequests, renderUserPanel, showVersion, startMinecraft } from './app/sidebar.js';
import { renderJumpPill, renderMessages, renderNewPill, renderTyping, tapMessage } from './app/messages.js';
import { addLocalNote, isQuietNow, jumpTo, loadLatest, loadOlder, markRead, onDeleted, onHistory, onMessage, onPing, onSendError, onSendsLost, openAndJump, openChannel, queueSend, rememberSince, sendOutbox, updateTitle, welcomeIn } from './app/chat.js';
import { askDelete, bindComposer, cancelEdit, cancelReply, clearStaged, deleteMessage, emojiPicker, gifPicker, openReactPicker, reactPicker, renderStaged, saveToPersonal, shrinkStaged, startEdit, startReply, toggleEmojis, toggleGifs } from './app/composer.js';
import { openMemberMenu, openPeople, renderMembers } from './app/people.js';
import { addToHomeScreen, bindPinDialog, bindWelcome, isStandalone, onHelloError, passwordLater, renderAccountBars, renderDevices, renderHomeBar, showPinDialog, showWelcome, startOver, toggleReveal } from './app/account.js';
import { askUnlock, bindVaultDialog, forgetSealedImages, showVaultDialog, syncVault } from './app/personal.js';
import { bindSearch, onSearchResults, openSearch, openSwitcher, renderResultList, runSearch } from './app/search.js';
import { doodlePad, markAllRead, onUnreadCounts, openCatchup, openDoodle, openPicture, openShelf, openSoundboard, renderHangs, renderShelf, requestUnreadCounts, sendQuiet, setAway, soundboard, viewDoodle } from './app/extras.js';
import { bindCallReport, callNote, callSettingsChanged, copyCallReport } from './app/call-report-ui.js';
import { HIGH_BITRATE, bindKey, bindSettings, bindingKey, clearPushToMute, copyInvite, copyMainLink, hearYourself, onMainKey, openSettings, renderAvatarSettings, renderAwaySettings, renderChimeSettings, renderInvites, renderModLog, renderNotifySettings, renderObsLink, renderStorage, showingAll, startMicTest } from './app/settings-ui.js';

// Console/debug handle (also used by test/e2e.test.mjs and the voice interop tests).
globalThis.yap = {
  state,
  voice,
  settings,
  ui: {
    get draw() { return drawBoard; },
    get space() { return spaceWorld; },
    get watch() { return watchParty; },
    /** For tests: talk to the server directly. (A `send` goes through the outbox like any other, so answers stay in order.) */
    send: (m) => (m?.t === 'send' ? queueSend({ ...m }) : conn?.send(m)),
    /** For tests: drop the socket as if the network had (it reconnects by itself). */
    dropSocket: () => conn?.drop(),
    /** For tests: 'dark' | 'light' | 'system', as if picked in Settings › Look. */
    setTheme: (theme) => {
      save({ theme });
      applyTheme({ changed: true });
    },
  },
};

// ── Boot ──────────────────────────────────────────────────────────────────

async function boot() {
  restoreAfterUpdate();
  applyTheme();
  hydrateIcons();
  showVersion();
  readInviteFromUrl();
  bindUi();
  renderAll();
  const check = await checkKey(settings.key);
  // A wrong (or old: the owner reset the main link) key doesn't stop a device
  // that's signed in: the server lets its token in and hands it the new link
  // (PROTOCOL.md › Resetting the main link). Anyone else needs a working link.
  if (check && !check.ok && !settings.token) {
    return showWelcome({ needKey: true, error: settings.key ? "That invite link didn't work. Ask for a new one." : '' });
  }
  state.keyWrong = Boolean(check && !check.ok); // (the API waits for the right one: ready.key)
  // Signed in on this device (a token): connect. Otherwise sign up or in first.
  if (!settings.token) return showWelcome({ needKey: false });
  start();
}

/** Server config (Places links, GIFs, Minecraft). Retried on every reconnect until it loads. */
function ensureConfig() {
  if (state.config) return;
  loadConfig().then((config) => {
    state.config = config;
    renderLinks();
    renderNotifySettings(); // this server's ntfy
    startMinecraft();
    if (/^https:\/\//.test(config.source ?? '')) Object.assign($('source-link'), { href: config.source, hidden: false });
  }, () => {});
}

export function start() {
  if (conn) return;
  if (!state.keyWrong) ensureConfig(); // (else once `ready` has handed us the link)
  // The key rides in the WebSocket subprotocol (not the URL), unless it has odd characters.
  setConn(new Connection(socketUrl(), keyProtocols()));
  conn.addEventListener('open', sendHello);
  conn.addEventListener('message', (e) => handle(e.detail));
  conn.addEventListener('close', onDisconnect);
  conn.connect();
  // Closing the tab in a call is a real goodbye: say so, or others keep our
  // connection for a while in case we're only reconnecting.
  addEventListener('pagehide', () => {
    if (state.voiceChannel) conn?.send({ t: 'voice.leave' });
  });
}

/**
 * This server's main invite key, from `ready.key` or `key` (the owner reset
 * the link): it's this device's link from now on, for the API and the next
 * connection.
 */
export function useKey(key) {
  state.keyWrong = false;
  if (!key || key === settings.key) return;
  save({ key });
  if (conn) Object.assign(conn, { url: socketUrl(), protocols: keyProtocols() });
}

/** Sign up / sign in (from the welcome form), else this device's token. */
export function sendHello() {
  if (!conn?.open) return;
  const base = { t: 'hello', v: 1, client: 'web' };
  if (state.helloPayload) conn.send({ ...base, ...state.helloPayload });
  else if (settings.token) conn.send({ ...base, token: settings.token });
  else showWelcome({ needKey: false });
}

function readInviteFromUrl() {
  const hash = new URLSearchParams(location.hash.slice(1));
  const key = hash.get('key');
  if (key !== null) save({ key });
  // A home-screen shortcut (or link) made with "Add to Home Screen" carries a
  // one-time sign-in pass and your name. Never replaces a sign-in we already have.
  const handoff = hash.get('handoff');
  // Such a link in a plain browser tab could be anyone's (a friend can post a
  // link with their own pass): once in, ask whether that account is yours
  // (onReady). The Home Screen app and the iPhone/Mac app's own pages don't ask.
  const ask = Boolean(handoff) && hash.get('embed') !== '1' && !isStandalone();
  if (handoff && !settings.token) {
    save({ token: handoff });
    if (ask) state.linkSignIn = { was: settings.uid };
  }
  // Already signed in here: keep the pass in case this device's token turns
  // out to be signed out (then it signs us back in instead of asking).
  else if (handoff) state.spareHandoff = { pass: handoff, ask };
  const name = hash.get('name');
  if (name && !settings.name) save({ name });
  const place = hash.get('place');
  if (place) state.openPlace = place;
  // A browser push tapped while Yap was closed (sw.js): open that message once we're in.
  const channel = hash.get('channel');
  if (channel) state.openMessage = { channelId: channel, id: hash.get('message') };
  if (location.hash) history.replaceState(null, '', location.pathname + location.search);
}

/**
 * A link from “Add to Home Screen” (…#key=…&handoff=…&name=…): sign in with
 * its one-time pass. Returns false if it isn't one.
 */
export function applySignInLink(text) {
  let hash;
  try {
    hash = new URLSearchParams(new URL(String(text).trim()).hash.slice(1));
  } catch {
    return false;
  }
  const handoff = hash.get('handoff');
  if (!handoff) return false;
  save({ key: hash.get('key') ?? settings.key, token: handoff, ...(hash.get('name') ? { name: hash.get('name') } : {}) });
  $('welcome-dialog').close();
  if (conn) location.reload();
  else start();
  return true;
}

/** Accepts a full invite link or a bare key. */
export function parseKey(input) {
  const value = input.trim();
  try {
    const url = new URL(value);
    return new URLSearchParams(url.hash.slice(1)).get('key') ?? url.searchParams.get('key') ?? '';
  } catch {
    return value;
  }
}

export async function checkKey(key) {
  try {
    const res = await api('/api/check', {}, key);
    return await res.json();
  } catch {
    return null; // offline: let the socket retry loop handle it
  }
}

// ── Server messages ───────────────────────────────────────────────────────

function handle(msg) {
  switch (msg.t) {
    case 'ready': return onReady(msg);
    case 'build': return onBuild(msg.build);
    case 'history': return onHistory(msg);
    case 'message': return onMessage(msg);
    case 'deleted': return onDeleted(msg);
    case 'links':
      state.links = msg.links;
      return renderLinks();
    case 'key':
      useKey(msg.key);
      return onMainKey(msg.key);
    case 'invite.created': {
      $('invite-made').hidden = false;
      $('invite-link').value = `${location.origin}/#key=${encodeURIComponent(msg.key)}`;
      $('invite-label').value = '';
      $('invite-link').select();
      conn?.send({ t: 'invite.list' });
      return;
    }
    case 'obs.created':
      state.obsToken = msg.token;
      renderObsLink();
      return $('obs-link').select();
    case 'vault':
      return syncVault(msg.wrap);
    case 'when.all': return whenPlanner?.onAll(msg);
    case 'personal.plain': return state.plainWaiter?.(msg.notes);
    case 'search.results': return onSearchResults(msg);
    case 'pins':
      if (msg.channelId === state.pinsChannel) renderResultList($('pins-list'), msg.messages, { pins: true });
      return;
    case 'pinned': {
      const m = state.history.get(msg.channelId)?.messages.find((x) => x.id === msg.id);
      if (m) {
        if (msg.pinnedAt) m.pinnedAt = msg.pinnedAt;
        else delete m.pinnedAt;
      }
      if (msg.channelId === settings.channel) renderMessages();
      if ($('pins-dialog').open && state.pinsChannel === msg.channelId) conn?.send({ t: 'pins', channelId: msg.channelId });
      if (msg.pinnedAt && msg.by !== settings.uid && msg.channelId === settings.channel) toast(`${state.users.get(msg.by)?.name ?? 'Someone'} pinned a message.`);
      return;
    }
    case 'when': return whenPlanner?.onOne(msg);
    case 'poll': {
      const m = state.history.get(msg.channelId)?.messages.find((x) => x.id === msg.id);
      if (m?.poll) m.poll = { ...m.poll, votes: msg.votes };
      if (msg.channelId === settings.channel) renderMessages();
      return;
    }
    case 'sound': return onSound(msg);
    case 'sound.added':
      state.sounds.set(msg.sound.id, msg.sound);
      return soundboard?.render();
    case 'sound.deleted':
      state.sounds.delete(msg.id);
      return soundboard?.render();
    case 'away': {
      // Only to us: someone we just mentioned is away (AIM's auto-reply).
      const mine = (state.history.get(settings.channel)?.messages ?? []).filter((m) => m.uid === settings.uid).at(-1);
      state.awayNotes = [...state.awayNotes.filter((n) => n.uid !== msg.uid || n.after !== mine?.id), { ...msg, after: mine?.id ?? null, channelId: settings.channel }].slice(-20);
      return renderMessages({ keep: 'auto' });
    }
    case 'shelf': return renderShelf(msg);
    case 'unread.counts': return onUnreadCounts(msg.counts);
    case 'hang':
      state.hangs.set(msg.hang.id, msg.hang);
      return renderHangs();
    case 'hang.cancelled':
      state.hangs.delete(msg.id);
      if (state.hangNow?.id === msg.id) state.hangNow = null;
      return renderHangs();
    case 'hang.now': {
      const hang = state.hangs.get(msg.id);
      if (!hang) return;
      state.hangNow = hang;
      if (!isQuietNow(state.notify?.quiet)) sounds.ping();
      if (!state.browserPush && settings.desktopNotify !== 'off' && document.visibilityState !== 'visible' && !isQuietNow(state.notify?.quiet) && typeof Notification !== 'undefined' && Notification.permission === 'granted') {
        new Notification(`🗓 ${hang.title} is starting`, { body: hang.channelId ? `Meet in ${state.channels.get(hang.channelId)?.name ?? 'voice'}` : 'Hop on Yap!', tag: `hang-${hang.id}`, icon: '/icon.svg' });
      }
      return renderHangs();
    }
    case 'watch.state': return watchParty.onState(msg);
    case 'watching': {
      const session = state.sessions.get(msg.sid);
      if (session) session.watching = msg.on;
      watchParty.onAudience();
      return renderLinks();
    }
    case 'device.handoff':
      state.handoffWaiter?.(msg.token);
      state.handoffWaiter = null;
      return;
    case 'obs.revoked':
      state.obsToken = null;
      $('obs-made').hidden = true;
      return toast(msg.count ? `Turned off ${msg.count === 1 ? 'your OBS link' : `${msg.count} OBS links`}.` : 'You had no OBS links.');
    case 'invites':
      state.invites = msg.invites;
      return renderInvites();
    case 'modlog':
      state.modlog = msg.entries;
      return renderModLog();
    case 'stats':
      return renderStorage(msg);
    case 'edited': {
      if (msg.sealed) state.sealWaiter?.(msg.id);
      const m = state.history.get(msg.channelId)?.messages.find((x) => x.id === msg.id);
      if (m) Object.assign(m, { body: msg.body, editedAt: msg.editedAt || undefined });
      if (m && msg.sealed) {
        delete m.gif;
        delete m.image;
      }
      if (msg.channelId === settings.channel) renderMessages();
      return;
    }
    case 'typing': {
      if (!state.typing.has(msg.channelId)) state.typing.set(msg.channelId, new Map());
      state.typing.get(msg.channelId).set(msg.uid, Date.now() + TYPING_MS);
      renderTyping();
      setTimeout(renderTyping, TYPING_MS + 50);
      return;
    }
    case 'user':
      if (['owner', 'mod', 'member'].includes(msg.user.role ?? 'member')) state.users.set(msg.user.id, msg.user);
      else state.users.delete(msg.user.id); // banned / still waiting
      if (msg.user.id === settings.uid && $('settings-dialog').open) {
        renderAvatarSettings();
        renderAwaySettings();
        renderChimeSettings();
      }
      if (msg.user.id === settings.uid) spaceWorld?.refreshLook();
      // No longer the owner or a mod: the waiting room is theirs, not ours.
      if (msg.user.id === settings.uid && !isMod()) state.waiting.clear();
      return renderAll();
    case 'pending':
      if (msg.token) save({ token: msg.token });
      if (msg.uid) save({ uid: msg.uid });
      if (state.helloPayload) save({ name: state.helloPayload.signup?.name ?? state.helloPayload.login?.name ?? settings.name });
      state.helloPayload = null;
      state.status = 'waiting';
      renderConn();
      return showGate('Waiting to be let in', `Hang tight, ${settings.name}. Waiting for the owner or a mod to let you in. This opens by itself as soon as they do, so keep this tab open.`, { waiting: true });
    case 'join.request':
      // (Only the owner and mods get these.)
      if (!state.waiting.has(msg.user.id)) {
        sounds.mention();
        if (settings.desktopNotify !== 'off' && document.visibilityState !== 'visible' && typeof Notification !== 'undefined' && Notification.permission === 'granted') {
          new Notification(`🚪 ${msg.user.name} is waiting to join`, { body: `${msg.user.invitedBy ? `Invited by ${msg.user.invitedBy}. ` : ''}Open Yap to let them in or turn them away.`, tag: `join-${msg.user.id}`, icon: '/icon.svg' });
        }
      }
      state.waiting.set(msg.user.id, msg.user);
      renderMembers();
      return renderRequests();
    case 'reaction': {
      const h = state.history.get(msg.channelId);
      const m = h?.messages.find((x) => x.id === msg.messageId);
      if (!m) return;
      m.reactions ??= [];
      let r = m.reactions.find((x) => x.emoji === msg.emoji);
      if (!r) m.reactions.push((r = { emoji: msg.emoji, uids: [] }));
      r.uids = r.uids.filter((u) => u !== msg.uid);
      if (msg.on) r.uids.push(msg.uid);
      m.reactions = m.reactions.filter((x) => x.uids.length);
      if (msg.channelId === settings.channel) renderMessages();
      return;
    }
    case 'join.resolved': {
      // The newcomer gets their own let-in too, right after `ready`: they're welcomed instead.
      if (msg.uid === settings.uid) return;
      // The owner and mods knew them from the waiting room; everyone else meets them here (join.resolved.user).
      const who = state.waiting.get(msg.uid) ?? (msg.approved ? msg.user ?? state.users.get(msg.uid) : undefined);
      state.waiting.delete(msg.uid);
      renderRequests();
      renderMembers(); // the People sheet: out of "Waiting to join" (their `user` came first)
      if (!who) return;
      const by = msg.by === settings.uid ? 'you' : state.users.get(msg.by)?.name ?? (msg.by === 'admin' ? 'the owner' : 'someone');
      // Let in: a calm line in the first channel, never a popup. Turned away: just a word.
      if (msg.approved) {
        const invitedBy = who.invite?.by ?? who.invitedBy?.replace(/ \(.*\)$/, '');
        addLocalNote(firstTextChannel()?.id, `join:${who.id}`, `${icon('user-plus')}<span><b style="color:${esc(who.color ?? 'inherit')}">${esc(who.name)}</b> joined — ${invitedBy ? `invited by ${esc(invitedBy)}` : `let in by ${esc(by)}`}</span>`);
      } else if (msg.by !== settings.uid) toast(`${by[0].toUpperCase()}${by.slice(1)} turned away ${who.name}.`);
      return;
    }
    case 'view': {
      const session = state.sessions.get(msg.sid);
      if (session) session.view = msg.view;
      if (msg.view !== 'board') boardCursors?.remove(msg.sid);
      if (msg.view !== 'space') spaceWorld?.remove(msg.sid);
      return renderLinks();
    }
    case 'cursor':
      return boardCursors?.update(msg.sid, msg.a);
    case 'notify':
      state.notify = { mode: msg.mode, topic: msg.topic, quiet: msg.quiet ?? null };
      return renderNotifySettings();
    case 'ping': return onPing(msg);
    case 'draw.state': return drawBoard?.onState(msg);
    case 'draw.seg': return drawBoard?.onSeg(msg);
    case 'draw.stroke': return drawBoard?.onStroke(msg);
    case 'draw.undo': return drawBoard?.onUndo(msg);
    case 'draw.clear': return drawBoard?.onClear(msg);
    case 'draw.wipe': return drawBoard?.onWipe(msg);
    case 'drawing':
      spaceWorld?.onDrawing(msg);
      return drawBoard?.onDrawing(msg);
    case 'draw.gallery': return spaceWorld?.onGallery(msg);
    case 'space.state': return spaceWorld?.onState(msg);
    case 'space.pos': return spaceWorld?.onPos(msg);
    case 'space.emote': return spaceWorld?.onEmote(msg);
    case 'space.game': return spaceWorld?.onGame(msg);
    case 'account':
      if (msg.token) save({ token: msg.token });
      state.hasPin = msg.hasPin;
      state.secretKind = msg.kind ?? 'password';
      state.weakPin = false;
      state.weakReason = null;
      renderAccountBars();
      $('pin-dialog').close();
      // (The server signs out your other devices with a new password: say how many.)
      return toast(msg.signedOut
        ? `Password saved. ${msg.signedOut === 1 ? 'Your other device was' : `Your ${msg.signedOut} other devices were`} signed out: sign in there with the new password.`
        : 'Password saved. Sign in anywhere with your name and password.');
    case 'devices':
      state.devices = msg.devices;
      return renderDevices();
    case 'signin.alert':
      state.signinAlert = msg.device;
      sounds.ping?.();
      return renderAccountBars();
    case 'pin.reset': {
      const name = state.users.get(msg.uid)?.name;
      return confirmSheet({
        title: 'Temporary password',
        text: name
          ? `${name}’s temporary password: ${msg.pin}. Send it to ${name}; they’ll choose their own after signing in.`
          : `Temporary password: ${msg.pin}. Send it to them; they’ll choose their own after signing in.`,
        ok: 'Copy',
        cancel: 'Done',
      }).then((copy) => copy && navigator.clipboard.writeText(msg.pin).then(() => toast('Copied.'), () => copyByHand('Temporary password', msg.pin)));
    }
    case 'session.join':
      if (msg.session.sid === state.staleSid) return; // our own old session (see onReady)
      state.sessions.set(msg.session.sid, msg.session);
      renderChannels();
      return renderMembers();
    case 'session.leave':
      if (msg.sid === state.staleSid) state.staleSid = null;
      // Someone whose call we're keeping across their reconnect stays (as a ghost) until they're back or it times out.
      if (!state.ghosts.has(msg.sid)) {
        state.sessions.delete(msg.sid);
        voice.drop(msg.sid);
      }
      boardCursors?.remove(msg.sid);
      spaceWorld?.remove(msg.sid);
      watchParty.onAudience();
      renderLinks();
      renderChannels();
      return renderMembers();
    case 'channel': {
      state.channels.set(msg.channel.id, msg.channel);
      if (msg.channel.id === state.voiceWanted) {
        voice.setChannelBitrate(msg.channel.bitrate ?? null);
        callSettingsChanged(); // (the call report notes the channel's quality)
        renderVoicePanel();
      }
      const wanted = state.awaitingChannel;
      if (wanted && wanted.kind === msg.channel.kind && msg.channel.kind === 'text') {
        state.awaitingChannel = null;
        openChannel(msg.channel.id);
      } else if (wanted && msg.channel.kind === 'voice') {
        state.awaitingChannel = null;
      }
      return renderChannels();
    }
    case 'channel.archived':
      state.channels.delete(msg.channelId);
      if (settings.channel === msg.channelId) openChannel(firstTextChannel()?.id); // (what you'd typed there comes along)
      state.drafts.delete(msg.channelId);
      return renderChannels();
    case 'voice': return onVoice(msg);
    case 'emoji':
      state.emojis.set(msg.emoji.id, msg.emoji);
      return emojiPicker?.render();
    case 'emoji.deleted':
      state.emojis.delete(msg.id);
      return emojiPicker?.render();
    case 'signal': {
      const from = state.sessions.get(msg.from);
      if (state.voiceChannel && from?.voice?.channelId === state.voiceChannel) voice.handleSignal(msg.from, msg.data);
      return;
    }
    case 'error':
      if (msg.code === 'banned' || msg.code === 'denied') {
        conn.stop(); // don't knock on the door forever
        return showGate(msg.code === 'banned' ? "You can't join this server" : 'Not let in', msg.message);
      }
      if (msg.code === 'pending') return;
      if (msg.re === 'hello') return onHelloError(msg);
      if (msg.re === 'send') return onSendError(msg);
      if (msg.re === 'invite.resetMain') state.resettingMain = false;
      if (msg.re === 'account.pin') {
        $('pin-error').textContent = msg.message;
        $('pin-error').hidden = false;
        return;
      }
      if (msg.code !== 'no_peer') toast(msg.message);
      return;
  }
}

function onReady(msg) {
  const admitted = state.status === 'waiting'; // let in just now
  hideGate();
  document.documentElement.classList.remove('signed-out');
  if (msg.token) save({ token: msg.token });
  state.helloPayload = null;
  state.hasPin = !msg.needsPin;
  state.secretKind = msg.secretKind ?? 'pin';
  state.weakPin = Boolean(msg.weakPin);
  state.weakReason = msg.weakReason ?? null; // pin | temporary | guessable: the prompt's words
  state.notify = msg.notify ?? { mode: 'off', topic: null, quiet: null };
  state.sounds = new Map((msg.sounds ?? []).map((s) => [s.id, s]));
  state.hangs = new Map((msg.hangs ?? []).map((h) => [h.id, h]));
  soundboard?.render();
  state.waiting = new Map((msg.pending ?? []).map((u) => [u.id, u]));
  // Members get the main key, so a personal invite being revoked later can't lock this device
  // out, and a device that connected with an old one (the main link was reset) gets the new one.
  useKey(msg.key);
  if (msg.links) state.links = msg.links;
  state.uploadToken = msg.uploadToken ?? null;
  syncVault(msg.vault ?? null);
  if (msg.storageWarning) setTimeout(() => toast('Heads up: Yap\'s storage is over 80% full. Settings › Owner & mods shows how much.'), 1500);
  // Signed in by a link in a plain tab, as an account this browser wasn't on: is it yours?
  if (state.linkSignIn) {
    if (msg.uid !== state.linkSignIn.was) confirmLinkSignIn(msg.users.find((u) => u.id === msg.uid)?.name ?? 'someone');
    state.linkSignIn = null;
  }
  // Merged accounts: the server answers with the canonical id; keep it.
  if (msg.uid && msg.uid !== settings.uid) save({ uid: msg.uid });
  const me = msg.users.find((u) => u.id === settings.uid);
  if (me && (me.name !== settings.name || me.color !== settings.color)) save({ name: me.name, color: me.color });
  if (state.status === 'offline') callNote('Connected to Yap again');
  state.sid = msg.sid;
  voice.mySid = msg.sid;
  state.resumeKey = msg.resumeKey ?? null;
  state.status = 'online';
  state.failures = 0;
  state.channels = new Map(msg.channels.map((c) => [c.id, c]));
  state.users = new Map(msg.users.map((u) => [u.id, u]));
  // A device that has never read anything starts caught up, not with every
  // channel unread since the beginning (empty channels get 1: anything new counts).
  if (!Object.keys(settings.lastRead).length) {
    save({ lastRead: Object.fromEntries([...state.channels.values()].filter((c) => c.kind !== 'voice').map((c) => [c.id, c.lastMessageAt || 1])) });
  }
  const before = state.sessions;
  // A half-dead old socket of ours can still be listed (the server hasn't noticed it's gone):
  // that's us, not someone to call. Its later leave is ignored too.
  state.staleSid = state.lastSid && state.lastSid !== msg.sid ? state.lastSid : null;
  state.lastSid = null;
  const sessions = msg.sessions.filter((s) => s.sid !== state.staleSid);
  state.sessions = new Map(sessions.map((s) => [s.sid, s]));
  if (state.resume) {
    // Our call survived the reconnect. Whoever's connection we kept but isn't
    // in this list yet may be reconnecting too (a deploy drops everyone):
    // keep them as ghosts. Anyone who's already back under a new sid: move over.
    for (const [sid, peer] of voice.peers) {
      if (state.sessions.has(sid)) continue;
      state.sessions.set(sid, before.get(sid) ?? { sid, uid: peer.uid, client: '', voice: { channelId: state.resume.channelId, muted: false, deafened: false }, view: null });
      holdGhost(sid);
    }
    for (const s of sessions) adoptResumed(s);
    // Don't wait for apps that never resume (they hung up when their own socket
    // dropped), nor for anyone already back with a plain join.
    for (const sid of [...state.ghosts.keys()]) {
      const ghost = state.sessions.get(sid);
      const back = [...state.sessions.values()].some((s) => s.uid === ghost?.uid && s.client !== 'obs' && !state.ghosts.has(s.sid) && s.voice?.channelId === ghost?.voice?.channelId);
      if (!ghost?.voice?.resume || back) {
        dropGhost(sid, { quiet: true });
        if (!back && ghost?.client !== 'obs') deferLeaveChime(ghost?.uid); // lost, not left: no chimes if they're back within 5 s (none for an OBS link)
      }
    }
  }
  state.emojis = new Map((msg.emojis ?? []).map((e) => [e.id, e]));
  emojiPicker?.render();
  // Standard emoji list (~45 KB) so :shortcodes: convert on send; not urgent.
  setTimeout(() => loadStandardEmojis().catch(() => {}), 1500);
  // Fresh history after every (re)connect, so nothing missed while away. Whatever
  // was waiting for a page of the old one (a jump to a message) goes on with the new one.
  const waiters = [...state.history.values()].flatMap((h) => h.waiters?.splice(0) ?? []);
  state.history.clear();
  const current = state.channels.get(settings.channel);
  if (!current || current.kind === 'voice') save({ channel: firstTextChannel()?.id ?? 'general' });
  rememberSince(settings.channel);
  loadLatest(settings.channel);
  for (const resolve of waiters) resolve();
  sendOutbox(); // what you wrote while offline (or that the old connection never got an answer for)
  if (state.view !== 'chat') {
    conn.send({ t: 'view', view: state.view });
    // The view's own state may have been lost with the old socket.
    if (state.view === 'space') conn.send({ t: 'space.state' });
    if (state.view === 'draw') drawBoard?.open();
    if (state.view === 'when') whenPlanner?.open();
  }
  watchParty.onReady(); // what's on + back into the audience (the video itself never stopped)
  if (state.voiceWanted) {
    const wanted = state.voiceWanted;
    state.voiceWanted = null;
    if (state.channels.has(wanted)) joinVoice(wanted);
    else {
      endResume();
      voice.stop();
    }
  }
  if (state.restoreView) {
    const view = state.restoreView;
    state.restoreView = null;
    setView(view);
  }
  // A link straight to a place (the desktop/iPhone apps open /#place=when).
  if (state.openPlace && VIEWS[state.openPlace]) {
    setView(state.openPlace);
    state.openPlace = null;
  }
  if (state.openMessage) {
    const { channelId, id } = state.openMessage;
    state.openMessage = null;
    openFromPush(channelId, id);
  }
  renderAccountBars();
  onBuild(msg.build);
  ensureConfig();
  syncBrowserPush();
  renderHomeBar();
  renderAll();
  requestUnreadCounts();
  if (admitted) welcomeIn();
}

/** A tapped push (sw.js): that message, or its channel; once we're connected if we aren't yet. */
function openFromPush(channelId, id) {
  if (state.status !== 'online' || !state.channels.size) return (state.openMessage = { channelId, id });
  if (!state.channels.has(channelId)) return;
  if (id) openAndJump(channelId, id);
  else openChannel(channelId);
}

/** Give up on keeping the call across a reconnect after this long (others keep ours for GHOST_MS). */
const RESUME_MS = 20_000;

let resumeTimer = null;

/**
 * After a sign-in link in a plain tab (readInviteFromUrl) put this browser on
 * an account: is it yours? Anyone can post a link with their own pass, and
 * what you then write would go to their account. "Not me" signs this device
 * out again.
 */
async function confirmLinkSignIn(name) {
  const yes = await confirmSheet({
    title: `Signed in as ${name}`,
    text: `That link signed this browser in as ${name}. Is that you? If someone sent you their own link, choose “Not me” and you’re signed out again.`,
    ok: 'That’s me',
    cancel: 'Not me',
  });
  if (yes) return;
  conn?.send({ t: 'account.signout' });
  save({ token: '', name: '', uid: '' });
  setTimeout(() => location.reload(), 300);
}

function onDisconnect({ detail } = {}) {
  // A wrong key with a token the server doesn't know (signed out meanwhile): nothing gets in
  // until there's a working link. (A ban says so first and stops the connection itself.)
  if (detail?.code === 4003 && detail.reason === 'wrong invite key' && !conn.stopped) {
    conn.stop();
    save({ token: '' });
    showWelcome({ needKey: true, mode: 'login', error: 'This invite link stopped working. Paste the new one, then sign in.' });
  }
  // In a call? Keep it: audio is peer to peer and doesn't need the socket.
  // (Servers that predate this send no resumeKey: hang up as before.)
  if (state.voiceChannel && state.resumeKey && !state.resume) {
    state.resume = { sid: state.sid, key: state.resumeKey, at: Date.now(), channelId: state.voiceChannel };
    callNote('Your connection to Yap dropped; keeping the call (audio goes straight between you)');
    voice.suspend();
    clearTimeout(resumeTimer);
    resumeTimer = setTimeout(() => {
      callNote(`Gave up keeping the call: no connection to Yap for ${RESUME_MS / 1000} s`);
      endResume();
      voice.hangupAll();
      renderAll();
    }, RESUME_MS);
  } else if (!state.resume) {
    if (state.voiceChannel) callNote("Your connection to Yap dropped (this server can't keep calls across that): hanging up until it's back");
    voice.hangupAll();
    state.peerStates.clear();
  }
  state.rejoining = Boolean(state.voiceWanted);
  state.status = 'offline';
  onSendsLost();
  state.lastSid = state.sid ?? state.lastSid; // (the server may still list it for a while: see onReady)
  state.sid = null;
  state.voiceChannel = null;
  state.resumeKey = null;
  state.speaking.clear();
  renderAll();
  // Repeated failures without ever getting in: maybe the key was changed. (Signed in here:
  // the server still lets this device in with its token and sends the new link.)
  if (++state.failures === 3 && !settings.token) {
    checkKey(settings.key).then((check) => {
      if (check && !check.ok) showWelcome({ needKey: true, error: 'This invite link stopped working. Paste the new one.' });
    });
  }
}

export function endResume() {
  clearTimeout(resumeTimer);
  resumeTimer = null;
  state.resume = null;
}

// ── Rendering ─────────────────────────────────────────────────────────────

export function renderAll() {
  renderConn();
  renderRequests();
  renderChannels();
  renderHeader();
  renderMessages();
  renderTyping();
  renderMembers();
  renderVoicePanel();
  renderSharePanel();
  renderUserPanel();
  updateTitle();
}

// ── UI wiring ─────────────────────────────────────────────────────────────

function bindUi() {
  document.addEventListener('click', onClick);
  // A browser push tapped while this tab was open (sw.js): go to its message.
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.addEventListener('message', (e) => {
      if (e.data?.t === 'yap.open' && typeof e.data.channelId === 'string') openFromPush(e.data.channelId, typeof e.data.messageId === 'string' ? e.data.messageId : null);
    });
  }
  // Per-person volume (People sheet and the voice popover; Ctrl/⌘ + scroll over their row in a voice channel).
  document.addEventListener('input', (e) => {
    const uid = e.target.dataset?.volumeUid;
    if (!uid) return;
    e.target.closest('.field')?.querySelector('output')?.replaceChildren(`${e.target.value}%`);
    setVolumeFor(uid, Number(e.target.value) / 100);
  });
  $('voice-channels').addEventListener('wheel', onVoiceRowWheel, { passive: false });
  document.addEventListener('change', (e) => {
    if (!e.target.matches('.vp-quality')) return;
    const value = e.target.value;
    const warn = e.target.closest('#popover')?.querySelector('.vp-quality-warn');
    if (warn) warn.hidden = Number(value) < HIGH_BITRATE; // (256 and 510 kbps: no dropout protection, several times the upload)
    conn?.send({ t: 'channel.bitrate', channelId: e.target.dataset.channel, bitrate: value ? Number(value) : null });
  });

  bindComposer();

  // Bars that come and go above the chat (a join request, an update) change its
  // height, and pictures and GIFs grow as they load: if you were reading the
  // newest message, keep it in view.
  const scroller = $('messages');
  let atEnd = true;
  scroller.addEventListener('scroll', () => {
    atEnd = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    if (scroller.scrollTop < 300) loadOlder(settings.channel);
    renderJumpPill();
    renderNewPill();
  }, { passive: true });
  const stayAtEnd = new ResizeObserver(() => {
    if (atEnd && !scroller.hidden) scroller.scrollTop = scroller.scrollHeight;
  });
  stayAtEnd.observe(scroller);
  stayAtEnd.observe($('message-list'));
  // Things that act like buttons but aren't <button>s (spoilers, someone in a
  // voice channel, a row in People) work from the keyboard too.
  document.addEventListener('keydown', (e) => {
    if ((e.key === 'Enter' || e.key === ' ') && !e.repeat && e.target.matches?.('[role="button"][data-action]:not(button)')) {
      e.preventDefault();
      e.target.click();
    }
  });
  $('chime-file').addEventListener('change', async (e) => {
    const file = e.target.files[0];
    e.target.value = '';
    if (!file) return;
    try {
      const clip = await makeClip(file, 2000);
      await postSound(clip.blob, { kind: 'chime' });
      toast(clip.cut ? 'Your chime is set (trimmed to 2 seconds).' : 'Your chime is set.');
    } catch (err) {
      toast(`Couldn't use that clip: ${err.message}`);
    }
  });

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') markRead(settings.channel);
    updateTitle();
  });

  // Push to talk (while Yap is focused; native clients do it globally).
  const typingTarget = (e) => /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName) && e.key.length === 1;
  addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      // Floating panels close (the doodle pad and soundboard too), and the keyboard goes back
      // to what opened them (the message box for the GIF and emoji pickers), not to the page.
      const panel = document.activeElement?.closest?.('#gif-picker, #emoji-picker, #react-picker, #doodle-pad, #soundboard');
      closePopover();
      gifPicker?.close();
      emojiPicker?.close();
      reactPicker?.close();
      doodlePad?.close();
      soundboard?.close();
      if (panel) {
        const back = openedBy.get(panel.id);
        (back?.isConnected ? back : $('input')).focus();
      }
    }
    if (settings.mode === 'ptt' && e.code === settings.pttKey && !e.repeat && !typingTarget(e) && !bindingKey) voice.setPtt(true);
    // Push-to-mute ("cough button"): muted while held, whatever the mode.
    if (settings.pushToMuteKey && e.code === settings.pushToMuteKey && !e.repeat && !typingTarget(e) && !bindingKey) voice.setPushToMute(true);
  });
  addEventListener('keyup', (e) => {
    if (e.code === settings.pttKey) voice.setPtt(false);
    if (e.code === settings.pushToMuteKey) voice.setPushToMute(false);
  });
  addEventListener('blur', () => {
    voice.setPtt(false);
    voice.setPushToMute(false);
  });
  // Mute and deafen from the keyboard, anywhere in Yap (typing too): Ctrl/⌘+Shift+M and Ctrl/⌘+Shift+D.
  // (By the letter on the key; by its place where the layout has no Latin letters.)
  document.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey || e.metaKey) || !e.shiftKey || e.altKey || e.repeat || bindingKey) return;
    const key = /^[a-z]$/i.test(e.key) ? e.key.toLowerCase() : { KeyM: 'm', KeyD: 'd' }[e.code];
    if (key === 'm') {
      e.preventDefault();
      toggleMute();
    } else if (key === 'd') {
      e.preventDefault();
      toggleDeafen();
    }
  });

  bindConfirmSheet();
  bindWelcome();
  bindPinDialog();
  bindChannelDialog();
  bindLinkDialog();
  bindVaultDialog();
  bindSearch();
  bindShareDialog();
  bindSettings();
  bindCallReport();
}

function onClick(e) {
  const el = e.target.closest('[data-action]');
  if (noHover.matches) tapMessage(e, el);
  if (!e.target.closest('#popover') && !el?.matches('[data-action="member-menu"], [data-action="voice-more"]')) closePopover();
  if (gifPicker?.isOpen && !e.target.closest('#gif-picker') && !el?.matches('[data-action="toggle-gifs"]')) gifPicker.close();
  if (emojiPicker?.isOpen && !e.target.closest('#emoji-picker') && !el?.matches('[data-action="toggle-emojis"]')) emojiPicker.close();
  if (reactPicker?.isOpen && !e.target.closest('#react-picker') && !el?.matches('[data-action="react-open"]')) reactPicker.close();
  if (doodlePad?.isOpen && !e.target.closest('#doodle-pad') && !el?.matches('[data-action="doodle-open"]')) doodlePad.close();
  if (soundboard?.isOpen && !e.target.closest('#soundboard') && !el?.matches('[data-action="soundboard"]')) soundboard.close();
  if (!el) return;
  const id = el.dataset.id;
  switch (el.dataset.action) {
    case 'reveal-spoiler': return el.classList.toggle('revealed');
    case 'watch-together':
      // Plays right away if nothing's on; otherwise it joins the queue.
      conn?.send({ t: 'watch.queue', add: el.dataset.video });
      return setView('watch');
    case 'save-personal': return saveToPersonal(id);
    case 'doodle-open': return openDoodle(el);
    case 'doodle-view': return viewDoodle(el);
    case 'doodle-remove':
      closePopover();
      return conn?.send({ t: 'react', id, emoji: el.dataset.emoji, on: false });
    case 'poll-vote': {
      const m = findMessage(id);
      const option = Number(el.dataset.option);
      const mine = m?.poll?.votes.findIndex((v) => v.includes(settings.uid));
      return conn?.send({ t: 'poll.vote', id, option: mine === option ? null : option });
    }
    case 'soundboard': return openSoundboard(el);
    case 'shelf': return openShelf();
    case 'shelf-more': return conn?.send({ t: 'shelf', channelId: state.shelf.channelId, before: state.shelf.items.at(-1)?.ts });
    case 'close-shelf': return $('shelf-dialog').close();
    case 'picture':
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || !el.dataset.id) return; // a new tab, as asked
      e.preventDefault();
      return openPicture(el);
    case 'close-picture': return $('picture-dialog').close();
    case 'picture-shelf':
      $('picture-dialog').close();
      return openShelf();
    case 'catchup': return openCatchup();
    case 'close-catchup': return $('catchup-dialog').close();
    case 'catchup-read-all': return markAllRead();
    case 'jump-unread':
      $('new-divider')?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
      return;
    case 'new-below': {
      state.newBelow = false;
      renderNewPill();
      const scroller = $('messages');
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
      if (!coarsePointer) $('input').focus({ preventScroll: true });
      return;
    }
    case 'switcher': return openSwitcher();
    case 'hang-join': {
      const hang = state.hangs.get(id);
      state.hangNow = null;
      renderHangs();
      return hang?.channelId && joinVoice(hang.channelId);
    }
    case 'hang-dismiss':
      state.hangNow = null;
      return renderHangs();
    case 'hang-cancel': {
      const hang = state.hangs.get(id);
      if (!hang) return;
      return confirmSheet({ title: `Cancel “${hang.title}”?`, text: 'Everyone sees it’s off.', ok: 'Cancel it', cancel: 'Keep it', danger: true })
        .then((yes) => yes && conn?.send({ t: 'hang.cancel', id }));
    }
    case 'away-set': return setAway();
    case 'away-clear': return conn?.send({ t: 'profile', away: null });
    case 'chime-preview': {
      const chime = $('settings-form').chime.value;
      return playChime(chime, chime.startsWith('s:') ? state.sounds.get(chime.slice(2)) : null, { preview: true });
    }
    case 'chime-upload': return $('chime-file').click();
    case 'quiet-snooze': {
      const until = new Date();
      until.setDate(until.getDate() + 1);
      until.setHours(9, 0, 0, 0);
      return sendQuiet(until.getTime());
    }
    case 'quiet-unsnooze': return sendQuiet(null);
    case 'open-channel': return openChannel(id);
    case 'join-voice':
      $('app').classList.remove('sidebar-open');
      return joinVoice(id);
    case 'leave-voice': return leaveVoice();
    case 'rejoin-voice': return rejoinVoice();
    case 'rejoin-dismiss': return dismissRejoin();
    case 'voice-more': return openVoiceMore(el);
    case 'copy-call-report':
      closePopover();
      return copyCallReport();
    case 'toggle-mute': return toggleMute();
    case 'toggle-deafen': return toggleDeafen();
    case 'open-settings': return openSettings(el.dataset.tab);
    case 'start-over': return startOver();
    case 'update-now': return reloadForUpdate();
    case 'new-channel': return openChannelDialog(el.dataset.kind);
    case 'archive-channel': {
      const channel = state.channels.get(id);
      if (!channel) return;
      return confirmSheet({ title: `Archive ${channel.name}?`, text: 'It disappears for everyone. Its messages are kept, and the owner can bring it back.', ok: 'Archive', danger: true })
        .then((yes) => yes && conn?.send({ t: 'channel.archive', channelId: id }));
    }
    case 'edit-message': return startEdit(id);
    case 'add-link': return openLinkDialog();
    case 'attach-image': return $('image-file').click();
    case 'unstage': return clearStaged();
    case 'add-to-home': return addToHomeScreen();
    case 'vault-setup': return showVaultDialog('setup');
    case 'vault-unlock': return askUnlock();
    case 'vault-change': return showVaultDialog('change');
    case 'vault-forgot': return showVaultDialog('reset');
    case 'vault-lock':
      state.vault.key = null;
      state.vault.keyCt = null;
      state.notes.clear();
      forgetSealedImages();
      forgetKey(settings.uid);
      toast('Locked on this device. Your passcode opens it again.');
      return renderAll();
    case 'paste-signin':
      return navigator.clipboard.readText().then(
        (text) => applySignInLink(text) || toast('That isn’t a Yap sign-in link. In Safari, tap “Show me” again.'),
        () => {
          $('welcome-key-field').hidden = false;
          $('welcome-form').key.focus();
          toast('Paste the link into the box.');
        },
      );
    case 'dismiss-home':
      try { localStorage.setItem('yap.homeDismissed', String(Date.now())); } catch {}
      return ($('home-bar').hidden = true);
    case 'remove-link': {
      const link = (state.links ?? []).find((l) => l.id === id);
      if (!link) return;
      return confirmSheet({ title: `Remove ${link.name}?`, text: 'It goes from Places for everyone.', ok: 'Remove', danger: true })
        .then((yes) => yes && conn?.send({ t: 'link.remove', id }));
    }
    case 'create-obs':
      if (!$('obs-channel').value) return toast('Make a voice channel first.');
      return conn?.send({ t: 'obs.create' });
    case 'copy-obs':
      return navigator.clipboard.writeText($('obs-link').value).then(() => toast('OBS link copied. In OBS: Sources › + › Browser › paste it as the URL.'), () => $('obs-link').select());
    case 'show-all':
      showingAll.add(el.dataset.list);
      return el.dataset.list === 'modlog' ? renderModLog() : renderInvites();
    case 'revoke-obs':
      return confirmSheet({ title: 'Turn off your OBS links?', text: 'Every OBS link you made stops working, and anything using them stops playing.', ok: 'Turn off', danger: true })
        .then((yes) => yes && conn?.send({ t: 'obs.revoke' }));
    case 'reset-main':
      return confirmSheet({
        title: 'Reset main link?',
        text: "The old link stops working for anyone who hasn't joined yet. Everyone already signed in keeps working, and their apps get the new link by themselves. Personal invite links keep working.",
        ok: 'Reset link',
        danger: true,
      }).then((yes) => {
        if (!yes) return;
        state.resettingMain = true; // show the new link when it comes back (`key`)
        conn?.send({ t: 'invite.resetMain' });
      });
    case 'copy-main-link': return copyMainLink();
    case 'create-invite':
      return conn?.send({ t: 'invite.create', label: $('invite-label').value.trim() });
    case 'copy-made-invite':
      return navigator.clipboard.writeText($('invite-link').value).then(() => toast('Invite link copied.'), () => $('invite-link').select());
    case 'revoke-invite': {
      const invite = (state.invites ?? []).find((i) => i.id === id);
      if (!invite) return;
      return confirmSheet({ title: `Turn off “${invite.label}”?`, text: 'Nobody new can join with this link. People already in stay in.', ok: 'Turn off', danger: true })
        .then((yes) => yes && conn?.send({ t: 'invite.revoke', id }));
    }
    case 'cancel-edit': return cancelEdit();
    case 'delete-message':
      if (e.shiftKey) return deleteMessage(id);
      return askDelete(id);
    case 'member-menu': return openMemberMenu(el);
    case 'toggle-sidebar': return $('app').classList.toggle('sidebar-open');
    case 'edit-rows': {
      // Touch screens: a row's archive / remove button only shows while editing (one mis-tap away otherwise).
      const editing = $('sidebar').classList.toggle('editing');
      for (const b of document.querySelectorAll('[data-action="edit-rows"]')) {
        b.textContent = editing ? 'Done' : 'Edit';
        b.setAttribute('aria-pressed', String(editing));
      }
      return;
    }
    case 'reveal': return toggleReveal(el);
    case 'devices':
      $('devices-list').innerHTML = '<li class="muted">Loading…</li>';
      if (!$('devices-dialog').open) $('devices-dialog').showModal();
      return conn?.send({ t: 'account.devices' });
    case 'close-devices': return $('devices-dialog').close();
    case 'revoke-device': return conn?.send({ t: 'account.revoke', id: el.dataset.id });
    case 'signout-others':
      return confirmSheet({ title: 'Sign out everywhere else?', text: 'Every other device (and your OBS links) is signed out. You stay signed in here.', ok: 'Sign out', danger: true }).then((yes) => {
        if (!yes) return;
        state.signinAlert = null;
        renderAccountBars();
        if (!$('devices-dialog').open) $('devices-dialog').showModal();
        toast('Signed out everywhere else. If it wasn’t you, change your password now.');
        conn?.send({ t: 'account.signoutOthers' });
      });
    case 'dismiss-signin':
      state.signinAlert = null;
      return renderAccountBars();
    case 'password-later': return passwordLater();
    case 'people':
    case 'show-members':
      return openPeople();
    case 'close-people': return $('people-dialog').close();
    case 'person':
      state.personOpen = state.personOpen === el.dataset.uid ? null : el.dataset.uid;
      return renderMembers();
    case 'reply-message': return startReply(id);
    case 'toggle-full-res':
      if (state.staged?.kind !== 'image') return;
      state.staged.full = el.checked;
      shrinkStaged(state.staged);
      return renderStaged();
    case 'share-screen': return toggleScreenShare();
    case 'share-change': return changeScreenShare();
    case 'share-stop': return voice.stopScreen();
    case 'watch-screen':
      closePopover(); // (from someone's popover under the voice channel)
      return watchScreen(el.dataset.sid);
    case 'stop-watching':
      closePopover();
      return stopWatchingScreen();
    case 'pin-message': {
      const m = findMessage(id) ?? state.pinnedLoose.get(id);
      if (m) conn?.send({ t: 'pin', id, on: !m.pinnedAt });
      return;
    }
    case 'search': return openSearch();
    case 'close-search': return $('search-dialog').close();
    case 'search-everywhere':
      $('search-form').scope.value = 'all';
      return runSearch();
    case 'pins':
      state.pinsChannel = settings.channel;
      $('pins-title').textContent = `Pinned in ${state.channels.get(settings.channel)?.name ?? 'this channel'}`;
      $('pins-list').innerHTML = '<li class="muted">Loading…</li>';
      if (!$('pins-dialog').open) $('pins-dialog').showModal();
      return conn?.send({ t: 'pins', channelId: settings.channel });
    case 'close-pins': return $('pins-dialog').close();
    case 'open-result':
      for (const d of ['search-dialog', 'pins-dialog', 'shelf-dialog', 'catchup-dialog']) $(d).close();
      return openAndJump(el.dataset.channel, el.dataset.id);
    case 'cancel-reply': return cancelReply();
    case 'jump-to': return jumpTo(el.dataset.id);
    case 'toggle-long':
      if (state.expanded.has(id)) state.expanded.delete(id);
      else state.expanded.add(id);
      return renderMessages();
    case 'close-drawers': return $('app').classList.remove('sidebar-open');
    case 'copy-invite': return copyInvite();
    case 'change-pin': return showPinDialog();
    case 'react-open': return openReactPicker(el);
    case 'react-toggle': {
      const h = state.history.get(settings.channel);
      const r = h?.messages.find((x) => x.id === id)?.reactions?.find((x) => x.emoji === el.dataset.emoji);
      return conn?.send({ t: 'react', id, emoji: el.dataset.emoji, on: !r?.uids.includes(settings.uid) });
    }
    case 'ping':
      return confirmSheet({
        title: 'Ping everyone?',
        text: 'Everyone with Yap open hears a chime; people with pushes on get a notification.', // (the iPhone/Mac app's words)
        input: { label: 'Add a note (optional)', placeholder: 'hop on!', maxlength: 140 },
        ok: 'Ping',
      }).then((text) => text !== null && conn?.send({ t: 'ping', text: text.trim() }));
    case 'upload-avatar': return $('avatar-file').click();
    case 'remove-avatar': return conn?.send({ t: 'profile', avatar: null });
    case 'copy-topic':
      navigator.clipboard.writeText(state.notify?.topic ?? '').then(() => toast('Topic copied.'), () => {});
      return;
    case 'sign-out':
      return confirmSheet({
        title: 'Sign out on this device?',
        text: 'You can sign back in with your name and password.',
        ok: 'Sign out',
        danger: true,
      }).then((yes) => {
        if (!yes) return;
        conn?.send({ t: 'account.signout' });
        save({ token: '', name: '', uid: '' });
        setTimeout(() => location.reload(), 300);
      });
    case 'reset-pin': {
      const user = state.users.get(el.dataset.uid);
      closePopover();
      if (!user) return;
      return confirmSheet({ title: `Reset ${user.name}’s password?`, text: `${user.name} gets a temporary password to sign in with, and their current one stops working.`, ok: 'Reset password', danger: true })
        .then((yes) => yes && conn?.send({ t: 'member.resetPin', uid: user.id }));
    }
    case 'toggle-gifs': return toggleGifs();
    case 'open-view': return setView(el.dataset.view);
    case 'approve': return conn?.send({ t: 'member.approve', uid: el.dataset.uid });
    case 'deny': return conn?.send({ t: 'member.deny', uid: el.dataset.uid });
    case 'ban': {
      const user = state.users.get(el.dataset.uid);
      closePopover();
      if (!user) return;
      return confirmSheet({ title: `Ban ${user.name}?`, text: 'They’re signed out and can’t come back. Their messages stay (mods can delete them).', ok: 'Ban', danger: true })
        .then((yes) => yes && conn?.send({ t: 'member.ban', uid: user.id }));
    }
    case 'set-role':
      conn?.send({ t: 'member.role', uid: el.dataset.uid, role: el.dataset.role });
      return closePopover();
    case 'claim-owner': {
      const key = $('owner-key').value.trim();
      if (key) conn?.send({ t: 'owner.claim', key });
      $('owner-key').value = '';
      return;
    }
    case 'copy-mc':
      navigator.clipboard.writeText(el.dataset.address).then(() => toast(`Copied ${el.dataset.address}`), () => copyByHand('Minecraft address', el.dataset.address));
      return;
    case 'toggle-emojis': return toggleEmojis();
    case 'mic-test': return startMicTest();
    case 'hear-yourself': return hearYourself();
    case 'bind-ptt': return bindKey('pttKey');
    case 'bind-ptm': return bindKey('pushToMuteKey');
    case 'clear-ptm': return clearPushToMute();
  }
}

bindUpdates();
boot();
