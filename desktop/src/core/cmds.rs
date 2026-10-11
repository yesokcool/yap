//! What the UI asks for: every command, by part of the app, and the chat
//! operations they share.

use super::*;

/// Write `bytes` to Downloads (else home, else temp) as `<stem>-<date-time>.<ext>`;
/// the file's name, or why it couldn't.
fn save_to_downloads(stem: &str, ext: &str, bytes: &[u8]) -> Result<String, String> {
    let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir);
    let name = format!("{stem}-{}.{ext}", chrono::Local::now().format("%Y-%m-%d-%H-%M-%S"));
    std::fs::write(dir.join(&name), bytes).map_err(|e| format!("couldn't write {name}: {e}"))?;
    Ok(name)
}

/// The part of the core a command is for.
enum Part {
    Chat,
    Views,
    Audio,
    Account,
    Extras,
    Updates,
    Vault,
    Screen,
}

/// Which handler a command goes to (exhaustive: a new command has to be
/// given one).
fn cmd_group(cmd: &Cmd) -> Part {
    match cmd {
        Cmd::OpenChannel(..)
        | Cmd::LoadOlder(..)
        | Cmd::Send { .. }
        | Cmd::SendGif { .. }
        | Cmd::Delete(..)
        | Cmd::Edit { .. }
        | Cmd::SendPicture { .. }
        | Cmd::SendStagedPicture { .. }
        | Cmd::PictureUploaded { .. }
        | Cmd::Search { .. }
        | Cmd::Pin { .. }
        | Cmd::Pins(..)
        | Cmd::JumpTo { .. }
        | Cmd::Typing
        | Cmd::CreateChannel { .. }
        | Cmd::ArchiveChannel(..)
        | Cmd::DeleteEmoji(..)
        | Cmd::UploadEmoji { .. }
        | Cmd::React { .. }
        | Cmd::Ping(..)
        | Cmd::SaveToPersonal { .. }
        | Cmd::SavePicture { .. } => Part::Chat,
        Cmd::SetView(..)
        | Cmd::DrawSeg { .. }
        | Cmd::DrawStroke(..)
        | Cmd::DrawUndo
        | Cmd::DrawClear
        | Cmd::DrawWipe(..)
        | Cmd::SaveDrawing { .. }
        | Cmd::SpaceMove { .. }
        | Cmd::SpaceGame { .. }
        | Cmd::SpaceEmote(..)
        | Cmd::SetLook(..)
        | Cmd::Proximity(..)
        | Cmd::OpenWeb { .. } => Part::Views,
        Cmd::SetChannelBitrate { .. }
        | Cmd::JoinVoice(..)
        | Cmd::LeaveVoice
        | Cmd::ToggleMute
        | Cmd::ToggleDeafen
        | Cmd::SetVolume { .. }
        | Cmd::AudioSettings { .. }
        | Cmd::CheckRelay
        | Cmd::MicTest(..)
        | Cmd::HearYourself(..)
        | Cmd::HearDone
        | Cmd::AudioRestart { .. }
        | Cmd::PlaySound(..)
        | Cmd::DeleteSound(..)
        | Cmd::PreviewSound(..)
        | Cmd::UploadSound { .. }
        | Cmd::SetChime(..)
        | Cmd::StopAudioIfIdle => Part::Audio,
        Cmd::CreateInvite(..)
        | Cmd::ListInvites
        | Cmd::RevokeInvite(..)
        | Cmd::AddLink { .. }
        | Cmd::RemoveLink(..)
        | Cmd::ModLog
        | Cmd::Profile { .. }
        | Cmd::Reconnect
        | Cmd::SignIn(..)
        | Cmd::SetPassword { .. }
        | Cmd::SignOut
        | Cmd::ResetPassword { .. }
        | Cmd::Devices
        | Cmd::RevokeDevice(..)
        | Cmd::SignOutOthers
        | Cmd::Ban { .. }
        | Cmd::SetRole { .. }
        | Cmd::Approve { .. }
        | Cmd::Wake
        | Cmd::DropSocket
        | Cmd::Quit => Part::Account,
        Cmd::SendPoll { .. }
        | Cmd::PollVote { .. }
        | Cmd::Shelf { .. }
        | Cmd::UnreadCounts
        | Cmd::MarkAllRead
        | Cmd::SetQuiet(..)
        | Cmd::SetHang { .. }
        | Cmd::CancelHang(..)
        | Cmd::SetAway(..)
        | Cmd::Doodle { .. } => Part::Extras,
        Cmd::CheckUpdate { .. }
        | Cmd::UpdateChecked { .. }
        | Cmd::InstallUpdate
        | Cmd::UpdateInstalled(..)
        | Cmd::ServerConfig(..)
        | Cmd::Minecraft(..) => Part::Updates,
        Cmd::Vault(..)
        | Cmd::SealBatches { .. }
        | Cmd::VaultDone(..) => Part::Vault,
        Cmd::ShareScreen { .. }
        | Cmd::StopSharing
        | Cmd::ShareQuality(..)
        | Cmd::ShareSound(..)
        | Cmd::ShareSource(..)
        | Cmd::WatchScreen(..)
        | Cmd::StopWatching
        | Cmd::ScreenVolume { .. } => Part::Screen,
    }
}

impl Core {
    pub(super) async fn on_cmd(&mut self, cmd: Cmd) {
        match cmd_group(&cmd) {
            Part::Chat => self.on_chat_cmd(cmd).await,
            Part::Views => self.on_views_cmd(cmd).await,
            Part::Audio => self.on_audio_cmd(cmd).await,
            Part::Account => self.on_account_cmd(cmd).await,
            Part::Extras => self.on_extras_cmd(cmd).await,
            Part::Updates => self.on_updates_cmd(cmd).await,
            Part::Vault => self.on_vault_cmd(cmd).await,
            Part::Screen => self.on_screen_cmd(cmd).await,
        }
    }

    /// Screen sharing: ours, and watching someone's.
    async fn on_screen_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::ShareScreen { source, quality, sound } => self.share_start(source, quality, sound),
            Cmd::StopSharing => self.share_stop(),
            Cmd::ShareQuality(q) => self.share_quality(q),
            Cmd::ShareSound(on) => self.share_sound(on),
            Cmd::ShareSource(source) => self.share_source(source),
            Cmd::WatchScreen(sid) => self.watch_screen(sid).await,
            Cmd::StopWatching => self.stop_watching(),
            Cmd::ScreenVolume { uid } => self.screen_volume(&uid),
            _ => {}
        }
    }

    /// Channels, messages, pictures, pins, search, reactions, emojis, Save to Personal.
    pub(super) async fn on_chat_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::OpenChannel(id) => self.open_channel(id),
            Cmd::LoadOlder(id) => self.load_older(&id),
            Cmd::Send { body, reply_to } => {
                let body = body.trim_end().to_string();
                if body.trim().is_empty() {
                    return;
                }
                let channel = self.shared.model.lock().config.channel.clone();
                match self.seal_for(&channel) {
                    Err(()) => return self.locked_toast(),
                    Ok(Some(key)) => {
                        let sealed = crate::vault::encrypt_note(&key, &crate::vault::Note { text: body.clone(), ..Default::default() });
                        self.post(&channel, sealed, None, None, None, Some(body));
                    }
                    Ok(None) => self.post(&channel, body.clone(), None, None, reply_to, Some(body)),
                }
                self.last_typing_sent = None;
            }
            Cmd::SendGif { gif, caption, reply_to } => {
                let channel = self.shared.model.lock().config.channel.clone();
                match self.seal_for(&channel) {
                    Err(()) => self.locked_toast(),
                    // Encrypted: the GIF rides inside the note.
                    Ok(Some(key)) => {
                        let sealed = crate::vault::encrypt_note(&key, &crate::vault::Note { text: caption.clone(), gif: Some(gif), image: None });
                        self.post(&channel, sealed, None, None, None, Some(caption));
                    }
                    Ok(None) => self.post(&channel, caption.clone(), Some(gif), None, reply_to, Some(caption)),
                }
            }
            Cmd::Delete(id) => {
                // An encrypted note's picture is erased along with it.
                let blobs: Vec<String> = {
                    let m = self.shared.model.lock();
                    let body = m.history.values().flat_map(|h| h.messages.iter()).find(|x| x.id == id).map(|x| x.body.clone()).unwrap_or_default();
                    match (crate::vault::is_note(&body), m.vault_key) {
                        (true, Some(key)) => crate::vault::decrypt_note(&key, &body).ok().and_then(|n| n.image).map(|i| vec![i.id]).unwrap_or_default(),
                        _ => vec![],
                    }
                };
                log::info!("deleting {id} ({} encrypted picture(s) with it)", blobs.len());
                if blobs.is_empty() {
                    self.send(out::delete(&id));
                } else {
                    self.send(out::delete_with_blobs(&id, &blobs));
                }
            }
            Cmd::Edit { id, body } => {
                let body = body.trim_end().to_string();
                let sealed = {
                    // Show it right away; the server's `edited` confirms.
                    let mut m = self.shared.model.lock();
                    let key = m.vault_key;
                    let ch = m.config.channel.clone();
                    let Some(msg) = m.history.get_mut(&ch).and_then(|h| h.messages.iter_mut().find(|x| x.id == id)) else { return };
                    let new_body = if crate::vault::is_note(&msg.body) {
                        // Encrypted note: re-seal the new text with whatever GIF/picture it had.
                        let Some(key) = key else {
                            m.toast("Unlock your Personal space to edit that.");
                            drop(m);
                            return self.repaint();
                        };
                        let Ok(mut note) = crate::vault::decrypt_note(&key, &msg.body) else { return };
                        if note.text == body {
                            return;
                        }
                        note.text = body;
                        crate::vault::encrypt_note(&key, &note)
                    } else {
                        if msg.body == body {
                            return;
                        }
                        body
                    };
                    msg.body = new_body.clone();
                    msg.edited_at = Some(now_ms());
                    new_body
                };
                self.send(out::edit(&id, &sealed));
                self.repaint();
            }
            Cmd::SendPicture { bytes, caption } => {
                let slot: crate::picture::PrepSlot = std::sync::Arc::new(parking_lot::Mutex::new(Some(crate::picture::prepare(&bytes, false))));
                return Box::pin(self.on_cmd(Cmd::SendStagedPicture { slot, caption, reply_to: None })).await;
            }
            Cmd::SendStagedPicture { slot, caption, reply_to } => {
                let (server, key, sid, ut, channel) = {
                    let m = self.shared.model.lock();
                    (m.config.server.clone(), m.config.key.clone(), m.sid.clone(), m.upload_token.clone().unwrap_or_default(), m.config.channel.clone())
                };
                let Some(sid) = sid else {
                    self.shared.model.lock().toast("Not connected yet.");
                    return self.repaint();
                };
                let seal = match self.seal_for(&channel) {
                    Err(()) => return self.locked_toast(),
                    Ok(k) => k,
                };
                {
                    let mut m = self.shared.model.lock();
                    m.sending_picture = true;
                    m.toast("Sending picture…");
                }
                self.repaint();
                let tx = self.cmd_tx.clone();
                let shared = self.shared.clone();
                tokio::spawn(async move {
                    // Usually it's long done (it shrank while you typed).
                    let mut waited = 0;
                    let pic = loop {
                        if let Some(r) = slot.lock().clone() {
                            break r;
                        }
                        waited += 1;
                        if waited > 1200 {
                            break Err("took too long to shrink".into());
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    };
                    let r = match pic {
                        Ok(pic) => tokio::task::spawn_blocking(move || -> Result<(String, Option<ImageRef>), String> {
                            let (w, h) = (pic.width as f32, pic.height as f32);
                            match seal {
                                // Encrypted: the picture is sealed before upload; the note says where it is.
                                Some(k) => {
                                    let sealed = crate::vault::encrypt_blob(&k, &pic.bytes);
                                    let id = crate::http::upload_blob(&server, &key, &sid, &ut, sealed)?;
                                    let note = crate::vault::Note { text: caption, gif: None, image: Some(crate::vault::NoteImage { id, width: w, height: h, mime: pic.mime.to_string() }) };
                                    Ok((crate::vault::encrypt_note(&k, &note), None))
                                }
                                None => {
                                    let id = crate::http::upload_image(&server, &key, &sid, &ut, pic.bytes, pic.mime)?;
                                    Ok((caption, Some(ImageRef { id, width: w, height: h })))
                                }
                            }
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string())),
                        Err(e) => Err(e),
                    };
                    shared.model.lock().sending_picture = false;
                    match r {
                        Ok((body, image)) => {
                            let _ = tx.send(Cmd::PictureUploaded { channel, body, image, reply_to });
                        }
                        Err(e) => {
                            shared.model.lock().toast(format!("Couldn't send that picture: {e}"));
                            (shared.repaint)();
                        }
                    }
                });
            }
            Cmd::PictureUploaded { channel, body, image, reply_to } => {
                self.shared.model.lock().toasts.retain(|(t, _)| t != "Sending picture…");
                // (An encrypted picture's caption is inside its note: nothing to give back.)
                let caption = (!crate::vault::is_note(&body)).then(|| body.clone());
                self.post(&channel, body, None, image, reply_to, caption);
            }
            Cmd::Search { q, channel } => self.search(q, channel),
            Cmd::Pin { id, on } => {
                self.send(out::pin(&id, on));
            }
            Cmd::Pins(channel) => {
                self.send(out::pins(&channel));
            }
            Cmd::JumpTo { channel, id } => self.jump_to(channel, id),
            Cmd::Typing => {
                if self.last_typing_sent.is_none_or(|t| t.elapsed() > TYPING_SEND_EVERY) {
                    self.last_typing_sent = Some(Instant::now());
                    let (ch, personal) = {
                        let m = self.shared.model.lock();
                        (m.config.channel.clone(), m.channel(&m.config.channel).is_some_and(|c| c.is_personal()))
                    };
                    // Nobody else is in your Personal space: no typing there.
                    if !personal {
                        self.send(out::typing(&ch));
                    }
                }
            }
            Cmd::CreateChannel { name, kind } => {
                self.shared.model.lock().awaiting_channel = Some(kind.clone());
                self.send(out::channel_create(&name, &kind));
            }
            Cmd::ArchiveChannel(id) => {
                self.send(out::channel_archive(&id));
            }
            Cmd::DeleteEmoji(id) => {
                self.send(out::emoji_delete(&id));
            }
            Cmd::UploadEmoji { name, bytes } => {
                let (server, key, sid, ut) = {
                    let m = self.shared.model.lock();
                    (m.config.server.clone(), m.config.key.clone(), m.sid.clone(), m.upload_token.clone().unwrap_or_default())
                };
                let Some(sid) = sid else {
                    self.shared.model.lock().toast("Not connected yet.");
                    return self.repaint();
                };
                let shared = self.shared.clone();
                tokio::spawn(async move {
                    let r = tokio::task::spawn_blocking(move || crate::http::upload_emoji(&server, &key, &sid, &ut, &name, bytes)).await;
                    let text = match r {
                        Ok(Ok(e)) => format!(":{}: added.", e.name),
                        Ok(Err(e)) => e,
                        Err(e) => e.to_string(),
                    };
                    shared.model.lock().toast(text);
                    (shared.repaint)();
                });
            }
            Cmd::React { id, emoji, on } => {
                self.send(out::react(&id, &emoji, on));
            }
            Cmd::Ping(text) => {
                self.send(out::ping(text.trim()));
            }
            Cmd::SaveToPersonal { id, comment } => {
                let (personal, note) = {
                    let mut m = self.shared.model.lock();
                    let personal = m.channels.iter().find(|c| c.is_personal()).map(|c| c.id.clone());
                    let first = m.vault.is_some() && !m.config.saved_note_seen;
                    if first {
                        m.config.saved_note_seen = true;
                        m.config_dirty = true;
                    }
                    (personal, first)
                };
                let Some(personal) = personal else { return };
                let comment = comment.trim().to_string();
                let body = match self.seal_for(&personal) {
                    Err(()) => return self.locked_toast(),
                    Ok(Some(key)) if !comment.is_empty() => crate::vault::encrypt_note(&key, &crate::vault::Note { text: comment, ..Default::default() }),
                    _ => comment,
                };
                let nonce = crate::config::random_hex(16);
                // (No grey copy: a bookmark that's refused just isn't saved; the error says so.)
                self.send_message(nonce.clone(), out::send(&personal, &body, &nonce, None, Some(&id)), None, false);
                self.shared.model.lock().toast(if note {
                    "Saved to your Personal space. (It's encrypted, but Yap's server can still see which message you saved.)"
                } else {
                    "Saved to your Personal space."
                });
                self.repaint();
            }
            Cmd::SavePicture { url, key } => {
                let shared = self.shared.clone();
                tokio::task::spawn_blocking(move || {
                    let saved = (|| -> Result<String, String> {
                        let (bytes, _) = crate::http::get_bytes(&url)?;
                        let bytes = match key {
                            Some(k) => crate::vault::decrypt_blob(&k, &bytes)?,
                            None => bytes,
                        };
                        let ext = image::guess_format(&bytes).ok().and_then(|f| f.extensions_str().first().copied()).unwrap_or("bin");
                        save_to_downloads("yap-picture", ext, &bytes)
                    })();
                    shared.model.lock().toast(match saved {
                        Ok(name) => format!("Saved {name} to Downloads."),
                        Err(e) => format!("Couldn't save it: {e}"),
                    });
                    (shared.repaint)();
                });
            }
            _ => {}
        }
    }

    /// Shared views: Draw, Space, and the web-only places.
    pub(super) async fn on_views_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetView(v) => self.set_view(v),
            Cmd::DrawSeg { sk, c, w, e, p, l } => {
                self.send(out::draw_seg(&sk, &c, w, e, &p, l));
            }
            Cmd::DrawStroke(mut stroke) => {
                {
                    let mut m = self.shared.model.lock();
                    stroke.uid = m.config.uid.clone();
                    m.draw.mine.push(stroke.id.clone());
                    m.draw.strokes.push(stroke.clone());
                    m.draw.version += 1;
                }
                self.send(out::draw_stroke(&stroke));
                self.repaint();
            }
            Cmd::DrawUndo => {
                let last = {
                    let mut m = self.shared.model.lock();
                    let last = m.draw.mine.pop();
                    if let Some(id) = &last {
                        m.draw.strokes.retain(|s| &s.id != id);
                        m.draw.version += 1;
                    }
                    last
                };
                if let Some(id) = last {
                    self.send(out::draw_undo(&id));
                }
                self.repaint();
            }
            Cmd::DrawClear => {
                self.send(out::draw_clear());
            }
            Cmd::DrawWipe(l) => {
                // Everyone on the board hears `draw.wipe`, us included; that's when it goes.
                self.send(out::draw_wipe(l));
            }
            Cmd::SaveDrawing { strokes, gif, wiggle, download, share, then } => {
                let (server, key, sid, ut, channel) = {
                    let m = self.shared.model.lock();
                    let ch = m.channel(&m.config.channel).filter(|c| c.is_text()).map(|c| c.id.clone()).or_else(|| m.first_text_channel().map(|c| c.id.clone()));
                    (m.config.server.clone(), m.config.key.clone(), m.sid.clone().unwrap_or_default(), m.upload_token.clone().unwrap_or_default(), ch)
                };
                let tx = self.cmd_tx.clone();
                let shared = self.shared.clone();
                tokio::spawn(async move {
                    let r = tokio::task::spawn_blocking(move || -> Result<(crate::protocol::SavedDrawing, Option<String>), String> {
                        let (bytes, mime, ext) = if gif { (crate::drawing::gif(&strokes, wiggle)?, "image/gif", "gif") } else { (crate::drawing::png(&strokes)?, "image/png", "png") };
                        let mut saved = None;
                        if download {
                            saved = Some(save_to_downloads("yap-drawing", ext, &bytes)?);
                        }
                        Ok((crate::http::upload_drawing(&server, &key, &sid, &ut, bytes, mime)?, saved))
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    match r {
                        Ok((drawing, _)) if share => match channel {
                            Some(ch) => {
                                let image = ImageRef { id: drawing.media.clone(), width: crate::drawing::PIX_W as f32, height: crate::drawing::PIX_H as f32 };
                                let _ = tx.send(Cmd::PictureUploaded { channel: ch.clone(), body: String::new(), image: Some(image), reply_to: None });
                                let name = shared.model.lock().channel(&ch).map(|c| c.name.clone()).unwrap_or_default();
                                shared.model.lock().toast(format!("Shared in {name}."));
                            }
                            None => shared.model.lock().toast("No text channel to share it in."),
                        },
                        Ok((_, Some(name))) => shared.model.lock().toast(format!("Saved {name} to Downloads and the gallery.")),
                        Ok(_) => {
                            if let Some(after) = then {
                                let _ = tx.send(match after {
                                    AfterSave::Clear => Cmd::DrawClear,
                                    AfterSave::Wipe(l) => Cmd::DrawWipe(l),
                                });
                            }
                            shared.model.lock().toast("Saved to the gallery.");
                        }
                        Err(e) => match then {
                            Some(AfterSave::Clear) => shared.model.lock().toast(format!("Couldn't keep the page in the gallery, so it wasn't cleared: {e}")),
                            Some(AfterSave::Wipe(l)) => shared.model.lock().toast(format!("Couldn't keep the page in the gallery, so layer {} wasn't cleared: {e}", l as usize + 1)),
                            None => shared.model.lock().toast(format!("Couldn't save the drawing: {e}")),
                        },
                    }
                    (shared.repaint)();
                });
            }
            Cmd::SpaceMove { x, y, d } => {
                self.send(out::space_move(x, y, d));
            }
            Cmd::SpaceGame { g, d } => {
                self.send(out::space_game(&g, &d));
            }
            Cmd::SpaceEmote(e) => {
                let sid = self.shared.model.lock().sid.clone().unwrap_or_default();
                self.shared.model.lock().space.emotes.insert(sid, (e.clone(), Instant::now()));
                self.send(out::space_emote(&e));
                self.repaint();
            }
            Cmd::SetLook(look) => {
                self.send(out::profile_look(look.as_deref()));
            }
            Cmd::Proximity(list) => {
                let m = &mut *self.shared.model.lock();
                for (sid, f) in list {
                    if (f - 1.0).abs() < 0.001 {
                        m.proximity.remove(&sid);
                    } else {
                        m.proximity.insert(sid.clone(), f);
                    }
                    self.shared.audio.set_volume(&sid, m.session_volume(&sid));
                }
            }
            Cmd::OpenWeb { place, video } => {
                if let Some(v) = &video {
                    self.send(out::watch_queue(v));
                }
                if self.send(out::device_handoff()) {
                    self.handoff = Some((place, Instant::now()));
                } else {
                    let url = self.web_url(place.as_deref(), None);
                    self.shared.model.lock().open_url = Some(url);
                    self.repaint();
                }
            }
            _ => {}
        }
    }

    /// Voice and sound: joining, mute/deafen, audio settings, the mic test, the soundboard, chimes.
    pub(super) async fn on_audio_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetChannelBitrate { channel_id, bitrate } => {
                self.send(out::channel_bitrate(&channel_id, bitrate));
            }
            Cmd::JoinVoice(id) => self.join_voice(id).await,
            Cmd::LeaveVoice => {
                // Leaving voice stops sharing (and watching): the video rides on the call.
                self.share_stop();
                self.stop_watching();
                self.leave_voice();
            }
            Cmd::ToggleMute => {
                let (muted, sounds) = {
                    let mut m = self.shared.model.lock();
                    if m.config.deafened {
                        m.config.deafened = false;
                        m.config.muted = false;
                    } else {
                        m.config.muted = !m.config.muted;
                    }
                    m.config_dirty = true;
                    (m.config.muted, m.config.sounds)
                };
                self.call_note(if muted { "You muted" } else { "You unmuted" });
                self.shared.apply_gate();
                self.send_voice_state();
                if sounds {
                    self.shared.audio.mixer.play_notes(&[if muted { 392.0 } else { 587.33 }], 0.075, 0.08, 0.05, true);
                }
                self.repaint();
            }
            Cmd::ToggleDeafen => {
                let (deaf, sounds) = {
                    let mut m = self.shared.model.lock();
                    m.config.deafened = !m.config.deafened;
                    m.config_dirty = true;
                    (m.config.deafened, m.config.sounds)
                };
                self.call_note(if deaf { "You deafened" } else { "You undeafened" });
                self.shared.apply_gate();
                self.send_voice_state();
                if sounds {
                    self.shared.audio.mixer.play_notes(&[if deaf { 392.0 } else { 587.33 }], 0.075, 0.08, 0.05, true);
                }
                self.repaint();
            }
            Cmd::SetVolume { uid } => {
                let m = self.shared.model.lock();
                for s in m.sessions.values().filter(|s| s.uid == uid) {
                    self.shared.audio.set_volume(&s.sid, m.session_volume(&s.sid));
                }
            }
            Cmd::AudioSettings { restart_input, restart_output, renegotiate } => {
                self.shared.apply_gate();
                let bitrate = self.shared.model.lock().voice_bitrate();
                self.voice.settings.bitrate.store(bitrate, Ordering::Relaxed);
                if restart_input && self.shared.audio.input_running() {
                    self.shared.audio.stop_input();
                    self.prepare_audio().await;
                    self.send_voice_state();
                }
                if restart_output && self.shared.audio.output_running() {
                    self.shared.audio.stop_output();
                    self.prepare_audio().await;
                }
                if renegotiate {
                    self.voice.renegotiate_all();
                }
                self.shared.model.lock().config_dirty = true;
                self.repaint();
            }
            Cmd::CheckRelay => {
                let (server, key) = {
                    let m = self.shared.model.lock();
                    (m.config.server.clone(), m.config.key.clone())
                };
                let shared = self.shared.clone();
                tokio::task::spawn_blocking(move || {
                    let relay = crate::http::has_relay(&crate::http::ice(&server, &key));
                    shared.model.lock().relay = Some(relay);
                    (shared.repaint)();
                });
            }
            Cmd::MicTest(on) => {
                self.shared.model.lock().mic_test = on;
                if on {
                    if !self.shared.audio.input_running() {
                        self.prepare_input_only().await;
                    }
                } else if self.shared.model.lock().voice_wanted.is_none() {
                    self.shared.audio.stop();
                }
                self.shared.apply_gate();
                self.repaint();
            }
            Cmd::HearYourself(save) => self.hear_yourself(save).await,
            Cmd::HearDone => {
                let mut m = self.shared.model.lock();
                if m.hear == HearState::Playing {
                    m.hear = HearState::Done;
                }
                drop(m);
                self.emit(CoreEvent::Extra("HEAR", "done".into()));
                self.repaint();
            }
            Cmd::AudioRestart { input, output } => self.restart_audio(input, output).await,
            Cmd::PlaySound(id) => {
                self.send(out::sound_play(&id));
            }
            Cmd::DeleteSound(id) => {
                self.send(out::sound_delete(&id));
            }
            Cmd::PreviewSound(what) => {
                if self.shared.audio_override.lock().is_some() {
                    return;
                }
                let (vol, device) = {
                    let m = self.shared.model.lock();
                    (m.config.sound_volume.clamp(0.0, 1.0), m.config.output_device.clone())
                };
                let audio = self.shared.audio.clone();
                match what {
                    Preview::Chime(id) => self.play_builtin_chime(&id, &device),
                    Preview::Sound(id) => {
                        if let Some(sound) = self.shared.model.lock().sound(&id).cloned() {
                            let gain = sound.gain.min(1.0) * vol;
                            self.with_pcm(&sound, move |pcm| audio.play_clip(pcm, gain, crate::audio::clips::Bus::Board, &device));
                        }
                    }
                    Preview::Wav(bytes) => {
                        if let Ok(w) = crate::audio::wav::decode_mono(&bytes) {
                            // Not measured yet: the same target the server will use.
                            let (peak, rms) = crate::audio::clips::measure(&w.samples);
                            let gain = (0.25 / rms.max(1e-6)).min(0.71 / peak.max(1e-6)).min(1.0) * vol;
                            let pcm = Arc::new(crate::audio::wav::resample_mono(&w.samples, w.rate, crate::audio::RATE));
                            audio.play_clip(pcm, gain, crate::audio::clips::Bus::Board, &device);
                        }
                    }
                }
            }
            Cmd::UploadSound { kind, name, emoji, wav } => {
                let (server, key, sid, ut) = {
                    let m = self.shared.model.lock();
                    (m.config.server.clone(), m.config.key.clone(), m.sid.clone().unwrap_or_default(), m.upload_token.clone().unwrap_or_default())
                };
                let shared = self.shared.clone();
                let tx = self.observer.clone();
                tokio::task::spawn_blocking(move || {
                    let r = crate::http::upload_sound(&server, &key, &sid, &ut, &kind, &name, emoji.as_deref(), wav);
                    let text = match &r {
                        Ok(s) if s.kind == "chime" => "Your join chime is set.".to_string(),
                        Ok(s) => format!("Added {} to the soundboard.", s.name),
                        Err(e) => format!("Couldn't add that sound: {e}"),
                    };
                    if let Some(o) = tx {
                        let _ = o.send(CoreEvent::Extra("SOUND", match &r {
                            Ok(s) => format!("uploaded {} {} gain {:.2}", s.id, s.kind, s.gain),
                            Err(e) => format!("upload failed: {e}"),
                        }));
                    }
                    shared.model.lock().toast(text);
                    (shared.repaint)();
                });
            }
            Cmd::SetChime(chime) => {
                self.send(out::profile_chime(chime.as_deref()));
            }
            Cmd::StopAudioIfIdle => {
                let idle = {
                    let m = self.shared.model.lock();
                    m.voice_wanted.is_none() && !m.mic_test
                };
                if idle {
                    self.shared.audio.stop();
                    self.repaint();
                }
            }
            _ => {}
        }
    }

    /// The account and the server: sign-in, passwords, devices, invites, places links, moderation, reconnecting.
    pub(super) async fn on_account_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::CreateInvite(label) => {
                self.shared.model.lock().new_invite = None;
                self.send(out::invite_create(label.trim()));
            }
            Cmd::ListInvites => {
                self.send(out::invite_list());
            }
            Cmd::RevokeInvite(id) => {
                self.send(out::invite_revoke(&id));
                self.send(out::invite_list());
            }
            Cmd::AddLink { name, url } => {
                self.send(out::link_add(name.trim(), url.trim()));
            }
            Cmd::RemoveLink(id) => {
                self.send(out::link_remove(&id));
            }
            Cmd::ModLog => {
                self.send(out::modlog());
            }
            Cmd::Profile { name, color } => {
                let (n, c) = {
                    let mut m = self.shared.model.lock();
                    m.config_dirty = true;
                    (m.config.name.clone(), m.config.color.clone())
                };
                self.send(out::profile(name.then_some(n.as_str()), color.then_some(c.as_str())));
            }
            Cmd::Reconnect => {
                self.say_goodbye_in_voice().await;
                let token = {
                    let m = self.shared.model.lock();
                    crate::config::Tokens::load(&self.shared.config_path).servers.get(&m.config.server).cloned()
                };
                {
                    let mut m = self.shared.model.lock();
                    m.token = token;
                    m.channels.clear();
                    m.sessions.clear();
                    m.users.clear();
                    m.history.clear();
                    m.pending.clear();
                    m.composer_restore.clear();
                    m.failures = 0;
                    m.voice_wanted = None;
                    m.voice_channel = None;
                }
                self.outbox.clear();
                self.voice.hangup_all();
                self.shared.audio.stop();
                self.shared.save_config();
                self.connect();
            }
            Cmd::SignIn(auth) => {
                self.say_goodbye_in_voice().await;
                {
                    let mut m = self.shared.model.lock();
                    match &auth {
                        Auth::Signup { name, color, .. } => {
                            m.config.name = name.clone();
                            m.config.color = color.clone();
                        }
                        Auth::Login { name, .. } => m.config.name = name.clone(),
                    }
                    m.auth = Some(auth);
                    m.token = None;
                    m.failures = 0;
                    m.config_dirty = true;
                }
                self.voice.hangup_all();
                self.shared.audio.stop();
                self.connect();
            }
            Cmd::SetPassword { password, current } => {
                self.send(out::account_pin(&password, current.as_deref()));
            }
            Cmd::SignOut => {
                if self.shared.model.lock().voice_wanted.is_some() {
                    self.leave_voice();
                }
                self.send(out::account_signout());
                tokio::time::sleep(Duration::from_millis(150)).await;
                self.shared.store_token(None);
                {
                    let mut m = self.shared.model.lock();
                    m.auth = None;
                    m.channels.clear();
                    m.sessions.clear();
                    m.history.clear();
                    m.pending.clear();
                    m.composer_restore.clear();
                }
                self.outbox.clear();
                self.connect();
            }
            Cmd::ResetPassword { uid } => {
                self.send(out::member_reset_pin(&uid));
            }
            Cmd::Devices => {
                self.send(out::account_devices());
            }
            Cmd::RevokeDevice(id) => {
                self.send(out::account_revoke(&id));
            }
            Cmd::SignOutOthers => {
                self.shared.model.lock().signin_alert = None;
                self.send(out::account_signout_others());
                self.shared.model.lock().toast("Signed out everywhere else. If it wasn't you, change your password too.");
                self.repaint();
            }
            Cmd::Ban { uid } => {
                self.send(out::member_ban(&uid));
            }
            Cmd::SetRole { uid, role } => {
                self.send(out::member_role(&uid, &role));
            }
            Cmd::Approve { uid, approved } => {
                self.send(out::member_approve(&uid, approved));
                self.shared.model.lock().join_requests.retain(|u| u.id != uid);
                self.repaint();
            }
            Cmd::DropSocket => {
                if let Some(n) = &self.net {
                    n.drop_now.notify_one();
                }
            }
            Cmd::Wake => {
                if !self.online
                    && let Some(n) = &self.net
                {
                    n.reconnect_now.notify_one();
                }
            }
            Cmd::Quit => {}
            _ => {}
        }
    }

    /// The v0.36 extras: polls, the shelf, catch up, quiet hours, hangs, away messages, doodles.
    pub(super) async fn on_extras_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SendPoll { question, options } => {
                let channel = self.shared.model.lock().config.channel.clone();
                let nonce = crate::config::random_hex(16);
                {
                    let mut m = self.shared.model.lock();
                    let msg = Message {
                        id: nonce.clone(),
                        channel_id: channel.clone(),
                        uid: m.config.uid.clone(),
                        body: question.clone(),
                        ts: now_ms(),
                        gif: None,
                        image: None,
                        reactions: vec![],
                        edited_at: None,
                        reply: None,
                        pinned_at: None,
                        poll: Some(crate::protocol::Poll { options: options.clone(), votes: vec![vec![]; options.len()] }),
                        roll: None,
                        reply_to: None,
                        pending: true,
                    };
                    m.pending.push((nonce.clone(), msg));
                    m.scroll = Some(ScrollHint::Bottom);
                }
                let typed = format!("/poll {question} | {}", options.join(" | "));
                let restore = super::outbox::Restore { channel: channel.clone(), text: typed };
                self.send_message(nonce.clone(), out::send_poll(&channel, &question, &nonce, &options), Some(restore), true);
                self.repaint();
            }
            Cmd::PollVote { id, option } => {
                // Show it right away; the server's `poll` confirms.
                {
                    let mut m = self.shared.model.lock();
                    let me = m.config.uid.clone();
                    for h in m.history.values_mut() {
                        if let Some(p) = h.messages.iter_mut().find(|x| x.id == id).and_then(|x| x.poll.as_mut()) {
                            for (i, v) in p.votes.iter_mut().enumerate() {
                                v.retain(|u| *u != me);
                                if Some(i) == option {
                                    v.push(me.clone());
                                }
                            }
                        }
                    }
                }
                self.send(out::poll_vote(&id, option));
                self.repaint();
            }
            Cmd::Shelf { channel, more } => {
                let before = {
                    let mut m = self.shared.model.lock();
                    let st = m.shelf.entry(channel.clone()).or_default();
                    if st.loading || (more && !st.has_more) {
                        return;
                    }
                    if !more {
                        st.items.clear();
                    }
                    st.loading = true;
                    if more { st.items.last().map(|x| x.ts) } else { None }
                };
                self.send(out::shelf(&channel, before));
                self.repaint();
            }
            Cmd::MarkAllRead => {
                self.shared.model.lock().mark_all_read();
                self.emit(CoreEvent::Extra("UNREAD", "all read".into()));
                self.repaint();
            }
            Cmd::UnreadCounts => {
                let since = self.shared.model.lock().unread_since();
                if since.is_empty() {
                    self.shared.model.lock().unread_counts = Some(HashMap::new());
                } else {
                    self.send(out::unread_counts(&since));
                }
            }
            Cmd::SetQuiet(q) => {
                self.shared.model.lock().quiet = q.clone();
                self.send(out::notify_quiet(q.as_ref()));
                self.repaint();
            }
            Cmd::SetHang { at, title, channel } => {
                self.send(out::hang_set(at, title.trim(), channel.as_deref()));
            }
            Cmd::CancelHang(id) => {
                self.send(out::hang_cancel(&id));
            }
            Cmd::SetAway(away) => {
                self.send(out::profile_away(away.as_ref().map(|(t, u)| (t.as_str(), *u))));
            }
            Cmd::Doodle { message, bytes } => {
                let (server, key, sid, ut) = {
                    let m = self.shared.model.lock();
                    (m.config.server.clone(), m.config.key.clone(), m.sid.clone().unwrap_or_default(), m.upload_token.clone().unwrap_or_default())
                };
                let shared = self.shared.clone();
                let tx = self.cmd_tx.clone();
                tokio::task::spawn_blocking(move || match crate::http::upload_doodle(&server, &key, &sid, &ut, bytes) {
                    Ok(id) => {
                        let _ = tx.send(Cmd::React { id: message, emoji: format!("<d:{id}>"), on: true });
                    }
                    Err(e) => {
                        shared.model.lock().toast(format!("Couldn't send the doodle: {e}"));
                        (shared.repaint)();
                    }
                });
            }
            _ => {}
        }
    }

    /// Updates, the server's config and its Minecraft status.
    pub(super) async fn on_updates_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::CheckUpdate { manual } => self.check_update(manual),
            Cmd::UpdateChecked { result, manual } => {
                let mut m = self.shared.model.lock();
                // Don't clobber a download/install in progress.
                if matches!(m.update, UpdateState::Downloading { .. } | UpdateState::Installed(_)) {
                    return;
                }
                m.update = match result {
                    Ok(Some(a)) => {
                        log::info!("update {} available", a.version);
                        UpdateState::Available(a)
                    }
                    Ok(None) => if manual { UpdateState::UpToDate } else { UpdateState::Idle },
                    Err(e) => if manual { UpdateState::Failed(e) } else { UpdateState::Idle },
                };
                drop(m);
                self.repaint();
            }
            Cmd::InstallUpdate => {
                let avail = match &self.shared.model.lock().update {
                    UpdateState::Available(a) => a.clone(),
                    _ => return,
                };
                self.shared.model.lock().update = UpdateState::Downloading { version: avail.version.clone(), done: 0, total: avail.file.size };
                let shared = self.shared.clone();
                let tx = self.cmd_tx.clone();
                tokio::spawn(async move {
                    let version = avail.version.clone();
                    let s2 = shared.clone();
                    let r = tokio::task::spawn_blocking(move || {
                        crate::update::install(&avail, |done, total| {
                            s2.model.lock().update = UpdateState::Downloading { version: avail.version.clone(), done, total };
                            (s2.repaint)();
                        })
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r);
                    let _ = tx.send(Cmd::UpdateInstalled(r.map(|_| version)));
                });
                self.repaint();
            }
            Cmd::UpdateInstalled(r) => {
                self.shared.model.lock().update = match r {
                    Ok(v) => UpdateState::Installed(v),
                    Err(e) => UpdateState::Failed(e),
                };
                self.repaint();
            }
            Cmd::ServerConfig(c) => {
                self.shared.model.lock().server_config = Some(c);
                self.last_mc_poll = None;
                self.repaint();
            }
            Cmd::Minecraft(st) => {
                self.shared.model.lock().minecraft = Some(st);
                self.repaint();
            }
            _ => {}
        }
    }

    /// `/api/config` (once per connection) in the background.
    /// Post a message: optimistic copy now, the server's echo replaces it.
    /// `typed`: what goes back in the message box if the server refuses it.
    pub(super) fn post(&mut self, channel: &str, body: String, gif: Option<crate::protocol::Gif>, image: Option<ImageRef>, reply_to: Option<String>, typed: Option<String>) {
        let nonce = crate::config::random_hex(16);
        {
            let mut m = self.shared.model.lock();
            let reply = reply_to.as_ref().and_then(|id| {
                let orig = m.history.get(channel)?.messages.iter().find(|x| &x.id == id)?;
                Some(crate::protocol::ReplyRef { id: id.clone(), uid: orig.uid.clone(), channel_id: Some(channel.to_string()), body: orig.body.chars().take(200).collect(), gif: orig.gif.is_some(), image: orig.image.is_some(), deleted: false })
            });
            let msg = Message {
                id: nonce.clone(),
                channel_id: channel.to_string(),
                uid: m.config.uid.clone(),
                body: body.clone(),
                ts: now_ms(),
                gif: gif.clone(),
                image: image.clone(),
                reactions: vec![],
                edited_at: None,
                reply,
                pinned_at: None,
                poll: None,
                roll: None,
                reply_to: reply_to.clone(),
                pending: true,
            };
            m.pending.push((nonce.clone(), msg));
            m.scroll = Some(ScrollHint::Bottom);
        }
        let frame = match &image {
            Some(img) => out::send_image(channel, &body, &nonce, img, reply_to.as_deref()),
            None => out::send(channel, &body, &nonce, gif.as_ref(), reply_to.as_deref()),
        };
        let restore = typed.map(|text| super::outbox::Restore { channel: channel.to_string(), text });
        self.send_message(nonce, frame, restore, true);
        self.repaint();
    }

    pub(super) fn set_view(&mut self, view: Option<View>) {
        let prev = {
            let mut m = self.shared.model.lock();
            let prev = m.view;
            m.view = view;
            prev
        };
        if prev == view {
            return;
        }
        if prev == Some(View::Space) {
            // Everyone back to normal volume; my spot is kept for next time.
            let sids: Vec<String> = {
                let mut m = self.shared.model.lock();
                m.space = SpaceModel::default();
                m.proximity.drain().map(|(k, _)| k).collect()
            };
            let m = self.shared.model.lock();
            for sid in sids {
                self.shared.audio.set_volume(&sid, m.session_volume(&sid));
            }
        }
        if !self.online {
            return self.repaint();
        }
        self.send(out::view(view.map(View::name)));
        match view {
            Some(View::Draw) => {
                self.send(out::draw_state());
            }
            Some(View::Space) => {
                self.shared.model.lock().space = SpaceModel::default();
                self.send(out::space_state());
            }
            None => {}
        }
        self.emit(CoreEvent::Space(format!("view {}", view.map(View::name).unwrap_or("chat"))));
        self.repaint();
    }

    /// `<server>/#key=…&place=…&handoff=…&name=…` (the web app, signed in).
    pub(super) fn web_url(&self, place: Option<&str>, token: Option<&str>) -> String {
        use crate::config::percent_encode as enc;
        let m = self.shared.model.lock();
        let mut url = crate::config::invite_link(&m.config);
        let mut sep = if url.contains('#') { "&" } else { "/#" };
        let mut add = |k: &str, v: &str| {
            url.push_str(&format!("{sep}{k}={}", enc(v)));
            sep = "&";
        };
        if let Some(p) = place {
            add("place", p);
        }
        if let Some(t) = token {
            add("handoff", t);
            add("name", &m.config.name);
        }
        url
    }

    pub(super) fn load_latest(&mut self, channel_id: &str) {
        if !self.online {
            return;
        }
        self.shared
            .model
            .lock()
            .history
            .insert(channel_id.to_string(), History { messages: vec![], has_more: true, loading: true, older: false });
        self.send(out::history(channel_id, None, 50));
    }

    pub(super) fn load_older(&mut self, channel_id: &str) {
        if !self.online {
            return;
        }
        let before = {
            let mut m = self.shared.model.lock();
            let Some(h) = m.history.get_mut(channel_id) else { return };
            if h.loading || !h.has_more || h.messages.is_empty() {
                return;
            }
            h.loading = true;
            h.older = true;
            let oldest = &h.messages[0];
            (oldest.ts, oldest.id.clone())
        };
        self.send(out::history(channel_id, Some((before.0, &before.1)), 50));
    }

    pub(super) fn open_channel(&mut self, id: String) {
        if self.shared.model.lock().view.is_some() {
            self.set_view(None);
        }
        let need_pins = !self.shared.model.lock().pins.contains_key(&id);
        if need_pins && self.online {
            self.send(out::pins(&id));
        }
        let need_load = {
            let mut m = self.shared.model.lock();
            // "New since …": remember what you'd read here, for the divider (this channel only).
            let unread = m.channel(&id).is_some_and(|c| m.is_unread(c) || c.id == m.config.channel && c.last_message_at > m.config.last_read.get(&c.id).copied());
            let mark = m.config.last_read.get(&id).copied().unwrap_or(0);
            m.read_marks.retain(|k, _| *k == id);
            let fresh_mark = unread && mark > 0 && !m.read_marks.contains_key(&id);
            if fresh_mark {
                m.read_marks.insert(id.clone(), mark);
            }
            m.config.channel = id.clone();
            m.config_dirty = true;
            m.scroll = Some(ScrollHint::Bottom);
            m.mark_read(&id, None);
            (!m.history.contains_key(&id), fresh_mark.then_some(mark))
        };
        let (need_load, mark) = need_load;
        // Where the first unread message is (it may be further back than one page).
        if let Some(mark) = mark.filter(|_| self.online) {
            self.send(out::unread_counts(&HashMap::from([(id.clone(), mark)])));
        }
        if need_load {
            self.load_latest(&id);
        }
        self.repaint();
    }
}
