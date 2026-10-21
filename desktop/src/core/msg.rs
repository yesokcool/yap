//! What the server says: every message, by part of the app.

use super::*;

/// The part of the core a server message is for.
enum Part {
    Account,
    Chat,
    People,
    Draw,
    Space,
    Extras,
}

/// Which handler a server message goes to (exhaustive: a new message type
/// has to be given one).
fn msg_group(msg: &ServerMsg) -> Part {
    match msg {
        ServerMsg::Ready { .. }
        | ServerMsg::Pending { .. }
        | ServerMsg::Account { .. }
        | ServerMsg::Devices { .. }
        | ServerMsg::SigninAlert { .. }
        | ServerMsg::PinReset { .. }
        | ServerMsg::JoinRequest { .. }
        | ServerMsg::JoinResolved { .. }
        | ServerMsg::Error { .. }
        | ServerMsg::InviteCreated { .. }
        | ServerMsg::Invites { .. }
        | ServerMsg::Key { .. }
        | ServerMsg::Links { .. }
        | ServerMsg::Vault { .. }
        | ServerMsg::PersonalPlain { .. }
        | ServerMsg::ModLog { .. }
        | ServerMsg::Notify { .. }
        | ServerMsg::DeviceHandoff { .. }
        | ServerMsg::Unknown => Part::Account,
        ServerMsg::History { .. }
        | ServerMsg::Message { .. }
        | ServerMsg::Reaction { .. }
        | ServerMsg::Ping { .. }
        | ServerMsg::Deleted { .. }
        | ServerMsg::Typing { .. }
        | ServerMsg::Channel { .. }
        | ServerMsg::ChannelArchived { .. }
        | ServerMsg::Emoji { .. }
        | ServerMsg::EmojiDeleted { .. }
        | ServerMsg::Edited { .. }
        | ServerMsg::SearchResults { .. }
        | ServerMsg::Pinned { .. }
        | ServerMsg::Pins { .. } => Part::Chat,
        ServerMsg::User { .. }
        | ServerMsg::SessionJoin { .. }
        | ServerMsg::SessionLeave { .. }
        | ServerMsg::Voice { .. }
        | ServerMsg::WatchState { .. }
        | ServerMsg::Watching { .. }
        | ServerMsg::Signal { .. }
        | ServerMsg::View { .. } => Part::People,
        ServerMsg::DrawState { .. }
        | ServerMsg::DrawSeg { .. }
        | ServerMsg::DrawStroke { .. }
        | ServerMsg::DrawUndo { .. }
        | ServerMsg::DrawClear { .. }
        | ServerMsg::DrawWipe { .. }
        | ServerMsg::Drawing { .. }
        | ServerMsg::DrawGallery { .. } => Part::Draw,
        ServerMsg::SpaceState { .. }
        | ServerMsg::SpacePos(..)
        | ServerMsg::SpaceGame { .. }
        | ServerMsg::SpaceEmote { .. } => Part::Space,
        ServerMsg::Poll { .. }
        | ServerMsg::Sound { .. }
        | ServerMsg::SoundAdded { .. }
        | ServerMsg::SoundDeleted { .. }
        | ServerMsg::Away { .. }
        | ServerMsg::Shelf { .. }
        | ServerMsg::UnreadCounts { .. }
        | ServerMsg::Hang { .. }
        | ServerMsg::HangCancelled { .. }
        | ServerMsg::HangNow { .. } => Part::Extras,
    }
}

impl Core {
    pub(super) async fn on_msg(&mut self, msg: ServerMsg) {
        match msg_group(&msg) {
            Part::Account => self.on_account_msg(msg).await,
            Part::Chat => self.on_chat_msg(msg).await,
            Part::People => self.on_people_msg(msg).await,
            Part::Draw => self.on_draw_msg(msg).await,
            Part::Space => self.on_space_msg(msg).await,
            Part::Extras => self.on_extras_msg(msg).await,
        }
    }

    /// Sign-in, the account, devices, the waiting room, invites, places links, errors.
    pub(super) async fn on_account_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Ready { sid, uid, channels, users, sessions, emojis, pending, token, needs_pin, key, links, upload_token, vault, weak_pin, weak_reason, secret_kind, notify, sounds, hangs, resume_key } => {
                self.got_token(token);
                self.resume_key = resume_key.filter(|k| !k.is_empty());
                self.sync_vault(vault);
                {
                    let mut m = self.shared.model.lock();
                    m.upload_token = upload_token;
                    m.sounds = sounds;
                    m.hangs = hangs;
                    m.hangs.sort_by_key(|h| h.at);
                    m.quiet = notify.and_then(|n| n.quiet);
                    m.unread_counts = None;
                    m.shelf.clear();
                    m.weak_pin = weak_pin;
                    // Why (the prompt's words); an older server doesn't say.
                    m.weak_reason = weak_reason.as_deref().and_then(crate::core::WeakReason::from_wire).filter(|_| weak_pin);
                    m.secret_kind = secret_kind.unwrap_or_else(|| "pin".into());
                    m.join_requests = pending.clone();
                    m.needs_pin = needs_pin;
                    if links.is_some() {
                        m.links = links;
                    }
                }
                // Keep the main key: a personal invite we came in with may be revoked
                // later, and the owner may have reset the main link while we were away
                // (our device token got us in with the old one).
                if let Some(k) = key {
                    self.save_main_key(k);
                }
                for u in &pending {
                    self.emit(CoreEvent::JoinRequest { uid: u.id.clone(), name: u.name.clone(), invited_by: u.invited_by.clone(), used_by: u.invite.as_ref().map(|i| i.used_by.clone()).unwrap_or_default() });
                }
                self.on_ready(sid, uid, channels, users, sessions, emojis).await
            }
            ServerMsg::Pending { message, token, uid } => {
                self.got_token(token);
                if let Some(uid) = uid.filter(|u| !u.is_empty()) {
                    let mut m = self.shared.model.lock();
                    if m.config.uid != uid {
                        m.config.uid = uid;
                        m.config_dirty = true;
                    }
                }
                let msg = if message.is_empty() { "Waiting for the owner or a mod to let you in.".to_string() } else { message };
                self.set_status(Status::Pending, &msg);
            }
            ServerMsg::Account { has_pin, token, kind } => {
                self.got_token(token);
                {
                    let mut m = self.shared.model.lock();
                    m.needs_pin = !has_pin;
                    if has_pin {
                        m.weak_pin = false;
                        m.weak_reason = None;
                        // Always "password" from a server that has passwords only.
                        m.secret_kind = kind.unwrap_or_else(|| "password".into());
                        m.toast("Password saved. Sign in anywhere with your name and password.");
                    }
                    m.password_error = None;
                    m.password_seq += 1;
                }
                self.emit(CoreEvent::Extra("PASSWORD", "saved".into()));
                self.repaint();
            }
            ServerMsg::Devices { devices } => {
                self.shared.model.lock().devices = Some(devices);
                self.repaint();
            }
            ServerMsg::SigninAlert { device } => {
                let (notify, label) = {
                    let mut m = self.shared.model.lock();
                    let label = device.label.clone();
                    m.signin_alert = Some(device);
                    (m.config.notifications, label)
                };
                if notify {
                    crate::notify::show("New sign-in to your Yap account", &format!("{label}, just now. Not you? Sign out everywhere else, then change your password."));
                }
                self.emit(CoreEvent::SigninAlert(label));
                self.repaint();
            }
            ServerMsg::PinReset { uid, pin } => {
                let mut m = self.shared.model.lock();
                let name = m.user_name(&uid);
                m.pin_reset = Some((name, pin));
                drop(m);
                self.repaint();
            }
            ServerMsg::JoinRequest { user } => {
                let (new, focused) = {
                    let mut m = self.shared.model.lock();
                    let new = !m.join_requests.iter().any(|u| u.id == user.id);
                    m.join_requests.retain(|u| u.id != user.id);
                    m.join_requests.push(user.clone());
                    (new, m.window_focused)
                };
                // Only the owner and mods let people in (an older server told everyone).
                let (quiet, moderator) = {
                    let m = self.shared.model.lock();
                    (m.quiet(false), m.am_moderator())
                };
                if new && !quiet && moderator {
                    self.chime(Chime::Mention);
                    if !focused && self.notifications_on() {
                        crate::notify::show(&format!("🚪 {} wants to join", user.name), "Open Yap to let them in or turn them away.");
                    }
                }
                self.emit(CoreEvent::JoinRequest { uid: user.id.clone(), name: user.name.clone(), invited_by: user.invited_by.clone(), used_by: user.invite.as_ref().map(|i| i.used_by.clone()).unwrap_or_default() });
                self.repaint();
            }
            ServerMsg::JoinResolved { uid, approved, by, user } => {
                let joined = {
                    let mut m = self.shared.model.lock();
                    let waiting = m.join_requests.iter().find(|u| u.id == uid).cloned();
                    m.join_requests.retain(|u| u.id != uid);
                    let known = m.users.get(&uid).cloned();
                    let about_me = uid == m.config.uid;
                    who_joined(waiting, approved, user, known, about_me).and_then(|who| {
                        let by_name = if by == m.config.uid {
                            "you".to_string()
                        } else if let Some(u) = m.users.get(&by) {
                            u.name.clone()
                        } else if by == "admin" {
                            "the owner".into()
                        } else {
                            "someone".into()
                        };
                        if !approved {
                            // Turned away: just a word (not to whoever did it).
                            if by != m.config.uid {
                                let mut by = by_name;
                                if let Some(c) = by.get(..1) {
                                    by = c.to_uppercase() + &by[1..];
                                }
                                m.toast(format!("{by} turned away {}.", who.name));
                            }
                            return None;
                        }
                        // Let in: a calm line in the first channel for this session, never a popup.
                        let text = format!(" joined — {}", join_line_by(&who).map(|b| format!("invited by {b}")).unwrap_or_else(|| format!("let in by {by_name}")));
                        let first = m.first_text_channel().map(|c| c.id.clone())?;
                        m.add_local_note(LocalNote { key: format!("join:{}", who.id), channel_id: first, ts: now_ms(), icon: "user-plus", bold: who.name.clone(), color: Some(who.color.clone()), text: text.clone() });
                        Some(format!("{}{text}", who.name))
                    })
                };
                if let Some(line) = joined {
                    self.emit(CoreEvent::Extra("JOINED", line));
                }
                self.repaint();
            }
            ServerMsg::Error { code, message, re } => {
                if code == "banned" || code == "denied" {
                    // The server closes the socket; don't keep knocking.
                    if let Some(n) = self.net.take() {
                        n.stop.notify_waiters();
                    }
                    self.net_rx = None;
                    self.online = false;
                    self.on_disconnect();
                    self.set_status(Status::Removed, &message);
                    self.emit(CoreEvent::Error { code, message });
                    return;
                }
                if re.as_deref() == Some("hello") {
                    // Sign-in failed (bad_token, bad_login, locked, no_pin, pin_required,
                    // name_taken, bad_pin…): stop knocking, show the sign-in screen.
                    if let Some(n) = self.net.take() {
                        n.stop.notify_waiters();
                    }
                    self.net_rx = None;
                    self.online = false;
                    if code == "bad_token" {
                        self.shared.store_token(None);
                    }
                    self.shared.model.lock().auth = None;
                    let text = sign_in_error(&code, &message);
                    self.set_status(Status::SignIn, &text);
                    self.emit(CoreEvent::Error { code, message });
                    return;
                }
                // A password change turned away (wrong current one, too short, too
                // easy to guess): the form that sent it shows the server's words.
                if re.as_deref() == Some("account.pin") {
                    {
                        let mut m = self.shared.model.lock();
                        m.password_error = Some(message.clone());
                        m.password_seq += 1;
                    }
                    self.emit(CoreEvent::Error { code, message });
                    self.repaint();
                    return;
                }
                // Resetting the main link was turned away (not the owner, too often…):
                // the dialog that asked shows the server's words.
                if re.as_deref() == Some("invite.resetMain") {
                    {
                        let mut m = self.shared.model.lock();
                        m.main_reset_error = Some(message.clone());
                        m.main_key_seq += 1;
                    }
                    self.emit(CoreEvent::Error { code, message });
                    self.repaint();
                    return;
                }
                if re.as_deref() == Some("device.handoff") {
                    // No pass right now (too many at once): open the web anyway; it asks to sign in.
                    if let Some((place, _)) = self.handoff.take() {
                        let url = self.web_url(place.as_deref(), None);
                        self.shared.model.lock().open_url = Some(url);
                        self.repaint();
                    }
                    return;
                }
                // Our oldest unanswered send was turned away: `slow_down` goes again
                // quietly; anything else drops its grey copy (and says why, below).
                if re.as_deref() == Some("send") && self.send_refused(&code) {
                    return;
                }
                // (`watch.get` to a server from before Watch together: nothing to say.)
                if code != "no_peer" && !re.as_deref().is_some_and(|r| r.starts_with("watch.")) {
                    self.shared.model.lock().toast(message.clone());
                    self.repaint();
                }
                self.emit(CoreEvent::Error { code, message });
            }
            ServerMsg::InviteCreated { invite, key } => {
                let link = {
                    let mut m = self.shared.model.lock();
                    let link = format!("{}/#key={key}", m.config.server);
                    m.new_invite = Some((invite.label.clone(), link.clone()));
                    m.invites.retain(|i| i.id != invite.id);
                    m.invites.insert(0, invite.clone());
                    link
                };
                self.emit(CoreEvent::Invite { label: invite.label, link });
                self.repaint();
            }
            ServerMsg::Invites { invites } => {
                self.shared.model.lock().invites = invites;
                self.repaint();
            }
            ServerMsg::Key { key } => {
                // The owner reset the main link (maybe us): everyone connected keeps working, with this as their link.
                self.save_main_key(key);
            }
            ServerMsg::Links { links } => {
                let names = links.iter().map(|l| l.name.clone()).collect();
                self.shared.model.lock().links = Some(links);
                self.emit(CoreEvent::Links(names));
                self.repaint();
            }
            ServerMsg::Vault { wrap } => {
                self.sync_vault(wrap);
                self.repaint();
            }
            ServerMsg::PersonalPlain { notes } => self.seal_old_notes(notes),
            ServerMsg::ModLog { entries } => {
                self.shared.model.lock().modlog = entries;
                self.repaint();
            }
            ServerMsg::Notify { quiet } => {
                self.shared.model.lock().quiet = quiet;
                self.emit(CoreEvent::Extra("QUIET", "updated".into()));
                self.repaint();
            }
            ServerMsg::DeviceHandoff { token } => {
                if let Some((place, _)) = self.handoff.take() {
                    let url = self.web_url(place.as_deref(), Some(&token));
                    self.shared.model.lock().open_url = Some(url);
                    self.repaint();
                }
            }
            ServerMsg::Unknown => {}
            _ => {}
        }
    }

    /// Channels, messages, reactions, pings, typing, emojis, edits, search, pins.
    pub(super) async fn on_chat_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::History { channel_id, messages, has_more } => {
                let mut m = self.shared.model.lock();
                let current = m.config.channel.clone();
                let Some(h) = m.history.get_mut(&channel_id) else { return };
                if h.older {
                    let mut merged = messages;
                    merged.append(&mut h.messages);
                    h.messages = merged;
                } else {
                    // Keep anything that arrived live while the page was loading.
                    let ids: HashSet<&str> = messages.iter().map(|x| x.id.as_str()).collect();
                    let newest = messages.last().map(|x| x.ts).unwrap_or(0);
                    let live: Vec<Message> = h.messages.drain(..).filter(|x| !ids.contains(x.id.as_str()) && x.ts > newest).collect();
                    h.messages = messages;
                    h.messages.extend(live);
                }
                h.has_more = has_more;
                h.loading = false;
                let older = h.older;
                if channel_id == current {
                    m.scroll = Some(if older { ScrollHint::Anchor } else { ScrollHint::Bottom });
                    if m.window_focused {
                        m.mark_read(&channel_id, None);
                    }
                }
                drop(m);
                self.continue_seek(&channel_id);
                self.continue_personal_search(&channel_id);
                self.repaint();
            }
            ServerMsg::Message { message, nonce } => {
                if let Some(n) = &nonce {
                    self.outbox.answered(n);
                }
                let mut m = self.shared.model.lock();
                if let Some(n) = &nonce {
                    m.pending.retain(|(k, _)| k != n);
                }
                if let Some(c) = m.channels.iter_mut().find(|c| c.id == message.channel_id) {
                    c.last_message_at = Some(c.last_message_at.unwrap_or(0).max(message.ts));
                }
                if let Some(h) = m.history.get_mut(&message.channel_id)
                    && !h.messages.iter().any(|x| x.id == message.id)
                {
                    h.messages.push(message.clone());
                }
                if let Some(t) = m.typing.get_mut(&message.channel_id) {
                    t.remove(&message.uid);
                }
                // The media shelf, if it's open for this channel, gets new pictures and GIFs.
                if (message.image.is_some() || message.gif.is_some())
                    && let Some(st) = m.shelf.get_mut(&message.channel_id)
                    && !st.items.iter().any(|x| x.id == message.id)
                {
                    st.items.insert(0, message.clone());
                }
                let mine = message.uid == m.config.uid;
                // Sent from this device (our nonce) = read. From another device
                // (e.g. a note to self in the Personal space) it stays unread.
                let personal = m.channel(&message.channel_id).is_some_and(|c| c.is_personal());
                if mine && (nonce.is_some() || !personal) {
                    m.mark_read(&message.channel_id, Some(message.ts));
                }
                if message.channel_id == m.config.channel {
                    if nonce.is_some() {
                        m.scroll = Some(ScrollHint::Bottom);
                    }
                    if m.window_focused {
                        m.mark_read(&message.channel_id, Some(message.ts));
                    }
                }
                let from = m.user_name(&message.uid);
                let mention = m.mentions_me(&message);
                let focused = m.window_focused;
                let elsewhere = message.channel_id != m.config.channel;
                if mention && (elsewhere || !focused) {
                    *m.mentions.entry(message.channel_id.clone()).or_insert(0) += 1;
                }
                let notify = m.config.notifications;
                let notify_all = notify && m.config.notify_all && !mine && nonce.is_none();
                let where_ = m.channel(&message.channel_id).map(|c| c.name.clone()).unwrap_or_default();
                drop(m);
                // Quiet hours hold back this app's own notifications (and their sounds) too.
                let quiet = self.shared.model.lock().quiet(mention);
                if mention {
                    if (elsewhere || !focused) && !quiet {
                        self.chime(Chime::Mention);
                    }
                    if !focused && !quiet {
                        self.shared.model.lock().attention += 1;
                    }
                    if !focused && notify && !quiet {
                        let replied = message.reply.as_ref().is_some_and(|r| r.uid == self.shared.model.lock().config.uid);
                        let title = if replied { format!("{from} replied") } else { format!("{from} mentioned you") };
                        crate::notify::show(&title, &plain_text(&message.body, true));
                    }
                    self.emit(CoreEvent::Mention { from: from.clone(), body: message.body.clone() });
                } else if notify_all && !focused && !quiet {
                    // "Every message" notifications.
                    let text = if let Some(p) = &message.poll {
                        format!("📊 {} ({} options)", hide_spoilers(&message.body), p.options.len())
                    } else if !message.body.is_empty() {
                        plain_text(&message.body, true)
                    } else if message.image.is_some() {
                        "📷 Picture".to_string()
                    } else {
                        "GIF".to_string()
                    };
                    crate::notify::show(&format!("{from} in {where_}"), &text);
                }
                if let Some(r) = &message.roll {
                    self.emit(CoreEvent::Extra("ROLL", format!("{} {} {} {:?} = {}", message.id, r.kind, r.spec, r.rolls, r.total)));
                }
                if let Some(p) = &message.poll {
                    self.emit(CoreEvent::Extra("POLL", format!("new {} {:?} {:?}", message.id, message.body, p.options)));
                }
                self.emit(CoreEvent::Message {
                    id: message.id.clone(),
                    channel: message.channel_id.clone(),
                    from,
                    body: message.body.clone(),
                    gif: message.gif.is_some(),
                    image: message.image.is_some(),
                    reply: message.reply.as_ref().map(|r| r.id.clone()),
                });
                self.repaint();
            }
            ServerMsg::Reaction { message_id, channel_id, emoji, uid, on } => {
                let from = {
                    let mut m = self.shared.model.lock();
                    if let Some(msg) = m.history.get_mut(&channel_id).and_then(|h| h.messages.iter_mut().find(|x| x.id == message_id)) {
                        let idx = match msg.reactions.iter().position(|r| r.emoji == emoji) {
                            Some(i) => i,
                            None => {
                                msg.reactions.push(crate::protocol::Reaction { emoji: emoji.clone(), uids: vec![] });
                                msg.reactions.len() - 1
                            }
                        };
                        let r = &mut msg.reactions[idx];
                        r.uids.retain(|u| *u != uid);
                        if on {
                            r.uids.push(uid.clone());
                        }
                        msg.reactions.retain(|r| !r.uids.is_empty());
                    }
                    m.user_name(&uid)
                };
                self.emit(CoreEvent::Reaction { message_id, emoji, from, on });
                self.repaint();
            }
            ServerMsg::Ping { from, text } => {
                let (who, mine, focused, notify) = {
                    let mut m = self.shared.model.lock();
                    let who = m.user_name(&from);
                    let note = if text.is_empty() { "!".to_string() } else { format!(": {text}") };
                    m.toast(format!("📣 {who} pinged everyone{note}"));
                    (who, from == m.config.uid, m.window_focused, m.config.notifications)
                };
                let quiet = self.shared.model.lock().quiet(true);
                if !quiet {
                    self.chime(Chime::Ping);
                }
                if !mine && !focused && notify && !quiet {
                    crate::notify::show(&format!("📣 {who} pinged everyone"), if text.is_empty() { "Hop on Yap!" } else { &text });
                }
                self.emit(CoreEvent::Ping { from: who, text });
                self.repaint();
            }
            ServerMsg::Deleted { id, channel_id } => {
                {
                    let mut m = self.shared.model.lock();
                    if let Some(list) = m.pins.get_mut(&channel_id) {
                        list.retain(|x| x.id != id);
                    }
                    if let Some(st) = m.search.as_mut() {
                        st.server.retain(|x| x.id != id);
                        st.local.retain(|x| x.id != id);
                    }
                    if let Some(st) = m.shelf.get_mut(&channel_id) {
                        st.items.retain(|x| x.id != id);
                    }
                    m.away_lines.remove(&id);
                }
                if let Some(h) = self.shared.model.lock().history.get_mut(&channel_id) {
                    h.messages.retain(|x| x.id != id);
                    // Replies to it now quote "Original message was deleted".
                    for x in h.messages.iter_mut() {
                        if let Some(r) = x.reply.as_mut().filter(|r| r.id == id) {
                            r.body.clear();
                            r.gif = false;
                            r.image = false;
                            r.deleted = true;
                        }
                    }
                }
                self.emit(CoreEvent::Deleted { id });
                self.repaint();
            }
            ServerMsg::Typing { channel_id, uid } => {
                self.shared
                    .model
                    .lock()
                    .typing
                    .entry(channel_id)
                    .or_default()
                    .insert(uid, Instant::now() + Duration::from_millis(TYPING_MS));
                self.repaint();
            }
            ServerMsg::Channel { channel } => {
                let before = self.shared.model.lock().voice_bitrate();
                let open = {
                    let mut m = self.shared.model.lock();
                    let kind = channel.kind.clone();
                    let id = channel.id.clone();
                    match m.channels.iter_mut().find(|c| c.id == channel.id) {
                        Some(c) => *c = channel,
                        None => m.channels.push(channel),
                    }
                    m.channels.sort_by_key(|c| c.position);
                    let wanted = m.awaiting_channel.clone();
                    if wanted.as_deref() == Some(kind.as_str()) {
                        m.awaiting_channel = None;
                        (kind == "text").then_some(id)
                    } else {
                        None
                    }
                };
                if let Some(id) = open {
                    self.open_channel(id);
                }
                // The channel's quality changed while we're in it: renegotiate.
                let (after, in_call) = {
                    let m = self.shared.model.lock();
                    (m.voice_bitrate(), m.voice_channel.is_some())
                };
                if after != before {
                    self.voice.settings.bitrate.store(after, Ordering::Relaxed);
                    if in_call {
                        self.voice.renegotiate_all();
                    }
                }
                self.repaint();
            }
            ServerMsg::ChannelArchived { channel_id } => {
                let reopen = {
                    let mut m = self.shared.model.lock();
                    m.channels.retain(|c| c.id != channel_id);
                    (m.config.channel == channel_id).then(|| m.first_text_channel().map(|c| c.id.clone())).flatten()
                };
                if let Some(id) = reopen {
                    self.open_channel(id);
                }
                self.repaint();
            }
            ServerMsg::Emoji { emoji } => {
                self.shared.model.lock().emojis.insert(emoji.id.clone(), emoji);
                self.repaint();
            }
            ServerMsg::EmojiDeleted { id } => {
                self.shared.model.lock().emojis.remove(&id);
                self.repaint();
            }
            ServerMsg::Edited { id, channel_id, body, edited_at, sealed } => {
                if sealed {
                    self.sealed(&id);
                }
                {
                    let mut m = self.shared.model.lock();
                    let pins = m.pins.get_mut(&channel_id).map(|l| l.iter_mut().filter(|x| x.id == id).collect::<Vec<_>>()).unwrap_or_default();
                    for x in pins {
                        x.body = body.clone();
                    }
                    if let Some(st) = m.search.as_mut() {
                        for x in st.server.iter_mut().filter(|x| x.id == id) {
                            x.body = body.clone();
                        }
                    }
                }
                if let Some(msg) = self.shared.model.lock().history.get_mut(&channel_id).and_then(|h| h.messages.iter_mut().find(|x| x.id == id)) {
                    msg.body = body.clone();
                    msg.edited_at = (edited_at > 0).then_some(edited_at);
                    if sealed {
                        // Encrypted in place: the picture/GIF are inside the note now.
                        msg.gif = None;
                        msg.image = None;
                    }
                }
                self.emit(CoreEvent::Edited { id, body });
                self.repaint();
            }
            ServerMsg::SearchResults { q, channel_id, messages } => {
                {
                    let mut m = self.shared.model.lock();
                    if let Some(st) = m.search.as_mut().filter(|st| st.q == q && st.channel == channel_id) {
                        st.server = messages;
                        st.loading = false;
                    }
                }
                self.emit(CoreEvent::SearchResults(self.shared.model.lock().search.as_ref().map(|s| s.results().iter().map(|x| x.id.clone()).collect()).unwrap_or_default()));
                self.repaint();
            }
            ServerMsg::Pinned { id, channel_id, pinned_at, by } => {
                let refresh = {
                    let mut m = self.shared.model.lock();
                    let mut found: Option<Message> = None;
                    if let Some(x) = m.history.get_mut(&channel_id).and_then(|h| h.messages.iter_mut().find(|x| x.id == id)) {
                        x.pinned_at = pinned_at;
                        found = Some(x.clone());
                    }
                    if let Some(st) = m.search.as_mut() {
                        for x in st.server.iter_mut().chain(st.local.iter_mut()).filter(|x| x.id == id) {
                            x.pinned_at = pinned_at;
                        }
                    }
                    let mut refresh = false;
                    if let Some(list) = m.pins.get_mut(&channel_id) {
                        list.retain(|x| x.id != id);
                        if pinned_at.is_some() {
                            match found {
                                Some(msg) => list.insert(0, msg),
                                None => refresh = true,
                            }
                        }
                    }
                    if pinned_at.is_some() && by != m.config.uid && channel_id == m.config.channel {
                        let who = m.user_name(&by);
                        m.toast(format!("{who} pinned a message."));
                    }
                    refresh
                };
                if refresh {
                    self.send(out::pins(&channel_id));
                }
                self.emit(CoreEvent::Pinned { id, on: pinned_at.is_some() });
                self.repaint();
            }
            ServerMsg::Pins { channel_id, messages } => {
                self.emit(CoreEvent::Pins { channel: channel_id.clone(), count: messages.len() });
                self.shared.model.lock().pins.insert(channel_id, messages);
                self.repaint();
            }
            _ => {}
        }
    }

    /// People and sessions: who joined, voice, signaling, Watch together, shared views.
    pub(super) async fn on_people_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::User { user } => {
                {
                    let mut m = self.shared.model.lock();
                    // Kept for names on old messages; `is_member()` hides
                    // pending/banned users from the member list.
                    m.users.insert(user.id.clone(), user);
                }
                self.repaint();
            }
            ServerMsg::SessionJoin { session } => {
                // Our own old session (see `old_sids`) isn't someone joining.
                if !self.old_sids.contains(&session.sid) {
                    self.shared.model.lock().sessions.insert(session.sid.clone(), session);
                    self.repaint();
                }
            }
            ServerMsg::SessionLeave { sid } => {
                if self.old_sids.contains(&sid) {
                    // Our own old session going at last: nobody left.
                    self.shared.model.lock().sessions.remove(&sid);
                    return self.repaint();
                }
                {
                    let mut m = self.shared.model.lock();
                    // Someone whose call we're keeping across their reconnect stays
                    // (as a ghost) until they're back or it times out.
                    if m.ghosts.contains_key(&sid) {
                        if let Some(s) = m.sessions.get_mut(&sid) {
                            s.view = None;
                            s.watching = false;
                        }
                        drop(m);
                        return self.repaint();
                    }
                    // Not in voice or on a shared view any more (there's no presence).
                    // Our own session stays: it's how we know who we are.
                    if m.sid.as_deref() == Some(sid.as_str()) {
                        if let Some(s) = m.sessions.get_mut(&sid) {
                            s.voice = None;
                        }
                        drop(m);
                        return self.repaint();
                    }
                    m.sessions.remove(&sid);
                    m.peer_states.remove(&sid);
                    m.speaking.remove(&sid);
                }
                self.voice.drop_peer(&sid);
                self.repaint();
            }
            ServerMsg::Voice { sid, channel_id, muted, deafened, screen, resume, prev, lost } => {
                self.on_voice(VoiceMsg { sid, channel_id, muted, deafened, screen, resume, prev, lost });
            }
            ServerMsg::WatchState { state } => {
                let what = match (&state.video_id, &state.title) {
                    (Some(_), Some(t)) => format!("state {t}"),
                    (Some(id), None) => format!("state {id}"),
                    _ => "state none".into(),
                };
                self.shared.model.lock().watch = Some(state);
                self.emit(CoreEvent::Watch(what));
                self.repaint();
            }
            ServerMsg::Watching { sid, on } => {
                if let Some(s) = self.shared.model.lock().sessions.get_mut(&sid) {
                    s.watching = on;
                }
                self.emit(CoreEvent::Watch(format!("watching {sid} {}", if on { "on" } else { "off" })));
                self.repaint();
            }
            // Someone asking for (or done with) our screen.
            ServerMsg::Signal { from, data } if data.get("screen").is_some() => {
                let want = data.get("screen").and_then(|v| v.as_str()) == Some("want");
                self.on_screen_signal(&from, want);
            }
            ServerMsg::Signal { from, data } => {
                let (ok, volume) = {
                    let m = self.shared.model.lock();
                    let ok = m.voice_channel.is_some()
                        && m.sessions.get(&from).and_then(|s| s.voice.as_ref()).map(|v| Some(&v.channel_id) == m.voice_channel.as_ref()).unwrap_or(false);
                    let volume = m.session_volume(&from);
                    (ok, volume)
                };
                if ok {
                    match serde_json::from_value::<SignalData>(data) {
                        Ok(d) => self.voice.handle_signal(&from, d, volume),
                        Err(e) => log::warn!("bad signal from {from}: {e}"),
                    }
                    *self.shared.voice_peers.lock() = self.voice.peers();
                }
            }
            ServerMsg::View { sid, view } => {
                {
                    let mut m = self.shared.model.lock();
                    if let Some(s) = m.sessions.get_mut(&sid) {
                        s.view = view.clone();
                    }
                    if view.as_deref() != Some("space") {
                        m.space.players.remove(&sid);
                        m.space.emotes.remove(&sid);
                        if m.proximity.remove(&sid).is_some() {
                            let v = m.session_volume(&sid);
                            self.shared.audio.set_volume(&sid, v);
                        }
                        if m.pan.remove(&sid).is_some() {
                            self.shared.audio.set_pan(&sid, 0.0);
                        }
                    }
                }
                self.repaint();
            }
            _ => {}
        }
    }

    /// The shared drawing board.
    pub(super) async fn on_draw_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::DrawState { strokes, gallery } => {
                {
                    let mut m = self.shared.model.lock();
                    let me = m.config.uid.clone();
                    let d = &mut m.draw;
                    d.mine = strokes.iter().filter(|s| s.uid == me).map(|s| s.id.clone()).collect();
                    d.strokes = strokes;
                    d.gallery = gallery;
                    d.live.clear();
                    d.version += 1;
                    d.loaded = true;
                }
                self.emit(CoreEvent::Draw("state".into()));
                self.repaint();
            }
            ServerMsg::DrawSeg { sid, sk, c, w, e, p, l } => {
                {
                    let mut m = self.shared.model.lock();
                    let entry = m.draw.live.entry(format!("{sid}:{sk}")).or_insert_with(|| LiveStroke { sid: sid.clone(), c, w, e, l, p: vec![], at: Instant::now() });
                    entry.p.extend(p);
                    entry.at = Instant::now();
                }
                self.repaint();
            }
            ServerMsg::DrawStroke { stroke } => {
                {
                    let mut m = self.shared.model.lock();
                    let d = &mut m.draw;
                    let suffix = format!(":{}", stroke.id);
                    d.live.retain(|k, _| !k.ends_with(&suffix));
                    if !d.strokes.iter().any(|s| s.id == stroke.id) {
                        d.strokes.push(stroke.clone());
                        d.version += 1;
                    }
                }
                self.emit(CoreEvent::Draw(format!("stroke {} by {}", stroke.id, stroke.uid)));
                self.repaint();
            }
            ServerMsg::DrawUndo { id } => {
                {
                    let mut m = self.shared.model.lock();
                    m.draw.strokes.retain(|s| s.id != id);
                    m.draw.mine.retain(|s| *s != id);
                    m.draw.version += 1;
                }
                self.emit(CoreEvent::Draw(format!("undo {id}")));
                self.repaint();
            }
            ServerMsg::DrawClear { by } => {
                {
                    let mut m = self.shared.model.lock();
                    m.draw.strokes.clear();
                    m.draw.mine.clear();
                    m.draw.live.clear();
                    m.draw.version += 1;
                    if by != m.config.uid {
                        let who = m.user_name(&by);
                        m.toast(format!("{who} started a fresh page (the old one is in the gallery)."));
                    }
                }
                self.emit(CoreEvent::Draw("clear".into()));
                self.repaint();
            }
            ServerMsg::DrawWipe { l, by } => {
                let gone = {
                    let mut m = self.shared.model.lock();
                    let gone = m.draw.wipe(l);
                    if by != m.config.uid {
                        let who = m.user_name(&by);
                        m.toast(format!("{who} cleared layer {} (the page is in the gallery).", l as usize + 1));
                    }
                    gone
                };
                self.emit(CoreEvent::Draw(format!("wipe {l} by {by} ({gone} gone)")));
                self.repaint();
            }
            ServerMsg::Drawing { drawing } => {
                {
                    let mut m = self.shared.model.lock();
                    if !m.draw.gallery.iter().any(|d| d.id == drawing.id) {
                        m.draw.gallery.insert(0, drawing.clone());
                    }
                    // Space's Gallery hangs it too (newest first).
                    if m.space.gallery_loaded && !m.space.gallery.iter().any(|d| d.id == drawing.id) {
                        m.space.gallery.insert(0, drawing.clone());
                        m.space.gallery.truncate(60);
                    }
                }
                self.emit(CoreEvent::Draw(format!("saved {}", drawing.media)));
                self.repaint();
            }
            ServerMsg::DrawGallery { gallery } => {
                let n = gallery.len();
                {
                    let mut m = self.shared.model.lock();
                    m.space.gallery = gallery.into_iter().take(60).collect();
                    m.space.gallery_loaded = true;
                }
                self.emit(CoreEvent::Space(format!("gallery {n}")));
                self.repaint();
            }
            _ => {}
        }
    }

    /// Space: positions, mini-games, emotes.
    pub(super) async fn on_space_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::SpaceState { players, me } => {
                {
                    let mut m = self.shared.model.lock();
                    let mine = m.sid.clone().unwrap_or_default();
                    // The full list: anyone not on it left while we weren't listening.
                    m.space.players = players.into_iter().filter(|p| p.sid != mine && !self.old_sids.contains(&p.sid)).map(|p| (p.sid.clone(), p)).collect();
                    m.space.state_seq += 1;
                    if !m.space.got_state {
                        m.space.me = me;
                        m.space.got_state = true;
                    }
                }
                self.emit(CoreEvent::Space(format!("state, {} others", self.shared.model.lock().space.players.len())));
                // The Gallery's drawings (new ones arrive as `drawing`).
                self.send(out::draw_gallery());
                self.repaint();
            }
            ServerMsg::SpacePos(p) => {
                let mine = self.shared.model.lock().sid.clone().unwrap_or_default();
                if p.sid != mine {
                    self.emit(CoreEvent::Space(format!("pos {} {:.0},{:.0}", p.sid, p.x, p.y)));
                    self.shared.model.lock().space.players.insert(p.sid.clone(), p);
                    self.repaint();
                }
            }
            ServerMsg::SpaceGame { sid, g, d } => {
                self.emit(CoreEvent::Space(format!("game {g} from {sid}: {d}")));
                let mut m = self.shared.model.lock();
                if m.view == Some(View::Space) && m.space.games.len() < 200 {
                    m.space.games.push((sid, g, d));
                }
                drop(m);
                self.repaint();
            }
            ServerMsg::SpaceEmote { sid, e } => {
                self.emit(CoreEvent::Space(format!("emote {sid} {e}")));
                self.shared.model.lock().space.emotes.insert(sid, (e, Instant::now()));
                self.repaint();
            }
            _ => {}
        }
    }

    /// The v0.36 extras: polls, the soundboard, away messages, the shelf, catch up, hangs.
    pub(super) async fn on_extras_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Poll { id, channel_id, votes } => {
                {
                    let mut m = self.shared.model.lock();
                    let m = &mut *m;
                    let lists = m.history.get_mut(&channel_id).map(|h| &mut h.messages).into_iter().chain(m.pins.get_mut(&channel_id)).chain(m.search.as_mut().map(|s| &mut s.server));
                    for list in lists {
                        for x in list.iter_mut().filter(|x| x.id == id) {
                            if let Some(p) = x.poll.as_mut() {
                                p.votes = votes.clone();
                            }
                        }
                    }
                }
                let n: Vec<usize> = votes.iter().map(Vec::len).collect();
                self.emit(CoreEvent::Extra("POLL", format!("{id} votes {n:?}")));
                self.repaint();
            }
            ServerMsg::Sound { id, uid } => {
                let (who, what) = {
                    let m = self.shared.model.lock();
                    (m.user_name(&uid), m.sound(&id).map(|s| format!("{}{}", s.emoji.as_deref().map(|e| format!("{e} ")).unwrap_or_default(), s.name)))
                };
                let what = what.unwrap_or_else(|| "a sound".into());
                {
                    let mut m = self.shared.model.lock();
                    m.last_sound = Some((format!("{who} played {what}"), Instant::now()));
                }
                let played = self.play_board_sound(&id, &uid);
                self.emit(CoreEvent::Extra("SOUND", format!("{id} by {who} ({what}) {}", if played { "played" } else { "silent" })));
                self.repaint();
            }
            ServerMsg::SoundAdded { sound } => {
                self.emit(CoreEvent::Extra("SOUND", format!("added {} {} {} gain {:.2} {} ms", sound.id, sound.kind, sound.name, sound.gain, sound.ms)));
                let mut m = self.shared.model.lock();
                m.sounds.retain(|x| x.id != sound.id);
                m.sounds.push(sound);
                drop(m);
                self.repaint();
            }
            ServerMsg::SoundDeleted { id } => {
                self.shared.model.lock().sounds.retain(|x| x.id != id || x.kind == "chime");
                self.emit(CoreEvent::Extra("SOUND", format!("deleted {id}")));
                self.repaint();
            }
            ServerMsg::Away { uid, text, until } => {
                {
                    let mut m = self.shared.model.lock();
                    // Under our newest message here (the one that mentioned or replied to them).
                    let ch = m.config.channel.clone();
                    let me = m.config.uid.clone();
                    let target = m.history.get(&ch).and_then(|h| h.messages.iter().rev().find(|x| x.uid == me).map(|x| x.id.clone()));
                    if let Some(id) = target {
                        let lines = m.away_lines.entry(id).or_default();
                        lines.retain(|(u, _, _)| *u != uid);
                        lines.push((uid.clone(), text.clone(), until));
                    }
                }
                self.emit(CoreEvent::Extra("AWAY", format!("{uid}: {text}")));
                self.repaint();
            }
            ServerMsg::Shelf { channel_id, items, has_more } => {
                {
                    let mut m = self.shared.model.lock();
                    let st = m.shelf.entry(channel_id.clone()).or_default();
                    for x in items {
                        if !st.items.iter().any(|y| y.id == x.id) {
                            st.items.push(x);
                        }
                    }
                    st.items.sort_by_key(|x| std::cmp::Reverse(x.ts));
                    st.has_more = has_more;
                    st.loading = false;
                }
                let n = self.shared.model.lock().shelf.get(&channel_id).map(|s| s.items.len()).unwrap_or(0);
                self.emit(CoreEvent::Extra("SHELF", format!("#{channel_id} {n}")));
                self.repaint();
            }
            ServerMsg::UnreadCounts { counts } => {
                let mut line: Vec<String> = counts.iter().map(|(c, u)| format!("{c}:{}{}", u.n, u.mentions.map(|n| format!(" ({n} for you)")).unwrap_or_default())).collect();
                line.sort();
                // Mentions that came while we were away get their badge.
                let seeded = {
                    let mut m = self.shared.model.lock();
                    let seeded = m.seed_mentions(&counts);
                    m.unread_counts.get_or_insert_with(HashMap::new).extend(counts);
                    seeded
                };
                self.emit(CoreEvent::Extra("UNREAD", line.join(" ")));
                for (c, n) in seeded {
                    self.emit(CoreEvent::Extra("MENTIONS", format!("{c} {n} while away")));
                }
                self.repaint();
            }
            ServerMsg::Hang { hang } => {
                self.emit(CoreEvent::Extra("HANG", format!("{} {} at {}", hang.id, hang.title, hang.at)));
                let mut m = self.shared.model.lock();
                m.hangs.retain(|h| h.id != hang.id);
                m.hangs.push(hang);
                m.hangs.sort_by_key(|h| h.at);
                drop(m);
                self.repaint();
            }
            ServerMsg::HangCancelled { id } => {
                {
                    let mut m = self.shared.model.lock();
                    m.hangs.retain(|h| h.id != id);
                    if m.hang_now.as_deref() == Some(id.as_str()) {
                        m.hang_now = None;
                    }
                }
                self.emit(CoreEvent::Extra("HANG", format!("cancelled {id}")));
                self.repaint();
            }
            ServerMsg::HangNow { id } => {
                let (title, where_, notify) = {
                    let mut m = self.shared.model.lock();
                    m.hang_now = Some(id.clone());
                    let h = m.hangs.iter().find(|h| h.id == id).cloned();
                    let where_ = h.as_ref().and_then(|h| h.channel_id.as_deref()).and_then(|c| m.channel(c)).map(|c| format!(" · {}", c.name)).unwrap_or_default();
                    let notify = m.config.notifications && !m.window_focused && !m.quiet(false);
                    (h.map(|h| h.title).unwrap_or_else(|| "A hang".into()), where_, notify)
                };
                if notify {
                    crate::notify::show(&format!("🗓 {title} is starting{where_}"), "Join in on Yap.");
                }
                self.emit(CoreEvent::Extra("HANG", format!("now {id} {title}")));
                self.repaint();
            }
            _ => {}
        }
    }

    pub(super) async fn on_ready(&mut self, sid: String, uid: Option<String>, channels: Vec<Channel>, users: Vec<User>, sessions: Vec<Session>, emojis: Vec<Emoji>) {
        self.voice.set_my_sid(sid.clone());
        let before;
        let admitted;
        let (channel, wanted) = {
            let mut m = self.shared.model.lock();
            // Merged accounts: the server answers with the canonical id; keep it.
            if let Some(uid) = uid.filter(|u| !u.is_empty() && *u != m.config.uid) {
                log::info!("server says our uid is {uid}");
                m.config.uid = uid;
                m.config_dirty = true;
            }
            // A known user whose local name is taken keeps their server name.
            if let Some(me) = users.iter().find(|u| u.id == m.config.uid)
                && (me.name != m.config.name || me.color != m.config.color)
            {
                m.config.name = me.name.clone();
                m.config.color = me.color.clone();
                m.config_dirty = true;
            }
            m.sid = Some(sid.clone());
            // Let in just now from the waiting room.
            admitted = m.status == Status::Pending;
            m.status = Status::Online;
            m.status_detail.clear();
            m.failures = 0;
            m.channels = channels;
            m.channels.sort_by_key(|c| c.position);
            // A device that has never read anything starts caught up.
            if m.start_caught_up() {
                log::info!("a new device: every channel starts read");
            }
            m.users = users.into_iter().map(|u| (u.id.clone(), u)).collect();
            before = std::mem::replace(&mut m.sessions, sessions.into_iter().map(|s| (s.sid.clone(), s)).collect());
            // Our own earlier sessions (a kept call) aren't anyone else.
            for old in &self.old_sids {
                m.sessions.remove(old);
            }
            m.emojis = emojis.into_iter().map(|e| (e.id.clone(), e)).collect();
            // Fresh history after every (re)connect, so nothing is missed while away.
            m.history.clear();
            let current_ok = m.channel(&m.config.channel).is_some_and(|c| c.is_chat());
            if !current_ok {
                let first = m.first_text_channel().map(|c| c.id.clone()).unwrap_or_else(|| "general".into());
                m.config.channel = first;
                m.config_dirty = true;
            }
            // Back after a while: a "New since …" divider in the channel you land in.
            let ch = m.config.channel.clone();
            let mark = m.config.last_read.get(&ch).copied().unwrap_or(0);
            if mark > 0 && m.channel(&ch).and_then(|c| c.last_message_at).is_some_and(|t| t > mark) && !m.read_marks.contains_key(&ch) {
                m.read_marks.insert(ch, mark);
            }
            (m.config.channel.clone(), m.voice_wanted.take())
        };
        self.emit(CoreEvent::Ready { sid });
        // Our call survived the reconnect: match the sessions to the connections we kept.
        self.resume_on_ready(&before);
        // What's new where (PROTOCOL.md › Since you left): every unread channel,
        // with the mentions that came while we were away, and where this
        // channel's first unread message is.
        let since = {
            let m = self.shared.model.lock();
            let mut since = m.unread_since();
            if let Some(t) = m.read_marks.get(&m.config.channel) {
                since.insert(m.config.channel.clone(), *t);
            }
            since
        };
        if !since.is_empty() {
            self.send(out::unread_counts(&since));
        }
        if admitted {
            // A chime and a quiet "say hi" in the first channel.
            self.chime(Chime::Mention);
            let mut m = self.shared.model.lock();
            if let Some(first) = m.first_text_channel().map(|c| c.id.clone()) {
                m.add_local_note(LocalNote { key: "welcome".into(), channel_id: first, ts: now_ms(), icon: "sparkles", bold: "You're in.".into(), color: None, text: " Say hi 👋".into() });
            }
            drop(m);
            self.emit(CoreEvent::Extra("JOINED", "we're in".into()));
        }
        self.fetch_server_config();
        self.emit(CoreEvent::Status(Status::Online, String::new()));
        self.load_latest(&channel);
        // Pinned messages (fresh each connection) and the shared view we were on.
        self.shared.model.lock().pins.clear();
        self.send(out::pins(&channel));
        // Watch together: what's on (shown under its entry in Places).
        self.send(out::watch_get());
        let view = self.shared.model.lock().view;
        if let Some(v) = view {
            {
                let mut m = self.shared.model.lock();
                m.space = SpaceModel::default();
                m.draw.live.clear();
            }
            self.send(out::view(Some(v.name())));
            let _ = match v {
                View::Draw => self.send(out::draw_state()),
                View::Space => self.send(out::space_state()),
            };
        }
        // Messages still grey go again, as they were sent (same nonce), in the order written.
        for (nonce, frame) in self.outbox.reconnect() {
            if !self.send(frame) {
                self.outbox.unsent(&nonce);
            }
        }
        if let Some(w) = wanted {
            let exists = self.shared.model.lock().channel(&w).is_some();
            if exists {
                self.join_voice(w).await;
            } else {
                self.end_resume();
                self.voice.hangup_all();
                *self.shared.voice_peers.lock() = Vec::new();
                self.shared.audio.stop();
            }
        }
        self.repaint();
    }
}

/// What the sign-in screen says after a refused sign-in: the server's words,
/// plus who can help after a wrong password (the web's words: someone still on
/// an old PIN only sees a Password field, so say the PIN works there too).
fn sign_in_error(code: &str, message: &str) -> String {
    match code {
        // (The server's own words may already say so.)
        "name_taken" if !message.contains("I have an account") => format!("{message} If it's you, choose “I have an account”."),
        "bad_login" => format!("{message} An old PIN works here too. Forgot it? The owner or a mod can reset it."),
        "locked" => format!("{message} Forgot your password? The owner or a mod can reset it."),
        _ => message.to_string(),
    }
}

/// Whom a `join.resolved` is about, for its line: the waiting-room entry if
/// we saw the request (the owner and mods), else, when they were let in, the
/// server's `join.resolved.user` (members never get `join.request`), else
/// whoever the member list says they are. Someone turned away whom we never
/// saw waiting: nothing to say. About us (just let in): nothing either, we
/// get "You're in. Say hi 👋" instead.
fn who_joined(waiting: Option<User>, approved: bool, resolved: Option<User>, known: Option<User>, about_me: bool) -> Option<User> {
    if about_me {
        return None;
    }
    waiting.or_else(|| approved.then(|| resolved.or(known)).flatten())
}

/// Who invited someone, for the "joined" line: the link's maker (`invite.by`),
/// else `invitedBy` without its "(for …)".
fn join_line_by(who: &User) -> Option<String> {
    who.invite.as_ref().map(|i| i.by.clone()).filter(|b| !b.is_empty()).or_else(|| {
        let by = who.invited_by.as_deref()?;
        let plain = match by.rfind(" (") {
            Some(i) if by.ends_with(')') => &by[..i],
            _ => by,
        };
        (!plain.is_empty()).then(|| plain.to_string())
    })
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn the_joined_line_names_whose_link() {
        let user = |j: serde_json::Value| -> User { serde_json::from_value(j).unwrap() };
        let base = serde_json::json!({ "id": "t", "name": "Theo", "color": "#fff", "role": "pending" });
        assert_eq!(join_line_by(&user(base.clone())), None);
        let mut with = base.clone();
        with["invitedBy"] = "Priya (for Theo)".into();
        assert_eq!(join_line_by(&user(with.clone())).as_deref(), Some("Priya"));
        with["invite"] = serde_json::json!({ "by": "Priya Ann", "label": "Theo", "usedBy": [] });
        assert_eq!(join_line_by(&user(with)).as_deref(), Some("Priya Ann"));
    }

    #[test]
    fn a_wrong_password_says_who_can_help() {
        assert_eq!(sign_in_error("bad_login", "Wrong name or password."), "Wrong name or password. An old PIN works here too. Forgot it? The owner or a mod can reset it.");
        assert_eq!(sign_in_error("locked", "Too many wrong tries. Try again in 2 min."), "Too many wrong tries. Try again in 2 min. Forgot your password? The owner or a mod can reset it.");
        assert_eq!(sign_in_error("no_pin", "x"), "x");
    }

    /// Members never see the request: the line comes from `join.resolved.user`
    /// ("Cleo joined — invited by Ada"), else the member list ("let in by …").
    #[test]
    fn who_joined_without_the_request() {
        let user = |j: serde_json::Value| -> User { serde_json::from_value(j).unwrap() };
        let waiting = user(serde_json::json!({ "id": "c", "name": "Cleo", "color": "#fff", "role": "pending", "invite": { "by": "Ada", "label": "Cleo", "usedBy": [] } }));
        let resolved = user(serde_json::json!({ "id": "c", "name": "Cleo", "color": "#30a46c", "invitedBy": "Ada (for Cleo)" }));
        let known = user(serde_json::json!({ "id": "c", "name": "Cleo", "color": "#30a46c", "role": "member" }));
        // The owner and mods: the waiting-room entry, with its link.
        let who = who_joined(Some(waiting.clone()), true, Some(resolved.clone()), Some(known.clone()), false).unwrap();
        assert_eq!(who.invite.map(|i| i.by).as_deref(), Some("Ada"));
        // A member: join.resolved.user, so it still says who invited them.
        let who = who_joined(None, true, Some(resolved.clone()), Some(known.clone()), false).unwrap();
        assert_eq!((who.name.as_str(), join_line_by(&who).as_deref()), ("Cleo", Some("Ada")));
        // An older server without it: the member list (then "let in by …").
        let who = who_joined(None, true, None, Some(known.clone()), false).unwrap();
        assert_eq!(join_line_by(&who), None);
        assert_eq!(who_joined(None, true, None, None, false), None);
        // Cleo herself, just let in: "You're in" instead of a line about herself.
        assert_eq!(who_joined(None, true, Some(resolved.clone()), Some(known.clone()), true), None);
        // Turned away: only whoever saw them waiting hears of it.
        assert!(who_joined(Some(waiting), false, None, None, false).is_some());
        assert_eq!(who_joined(None, false, Some(resolved), Some(known), false), None);
    }
}
