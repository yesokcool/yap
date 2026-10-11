//! The encrypted Personal space: unlocking, sealing old notes, passcode
//! changes.

use super::*;

/// `personal.seal` batches: at most this much JSON and this many notes (the web's numbers).
const SEAL_BATCH_BYTES: usize = 300_000;
const SEAL_BATCH_NOTES: usize = 50;
/// How long the server has to confirm them.
const SEAL_WAIT: Duration = Duration::from_secs(20);

/// Sealed notes handed back, waiting for the server's `edited { sealed: true }`.
pub(super) struct SealWait {
    pub left: HashSet<String>,
    pub done: usize,
    /// Couldn't be sealed (or too big for a frame).
    pub failed: usize,
    pub until: Instant,
}

impl Core {
    /// Ok(Some(key)): an encrypted Personal space, unlocked. Ok(None): post as is. Err: locked.
    pub(super) fn seal_for(&self, channel: &str) -> Result<Option<crate::vault::Key>, ()> {
        let m = self.shared.model.lock();
        let personal = m.channel(channel).is_some_and(|c| c.is_personal());
        if !personal || m.vault.is_none() {
            return Ok(None);
        }
        m.vault_key.map(Some).ok_or(())
    }

    pub(super) fn locked_toast(&mut self) {
        self.shared.model.lock().toast("Unlock your Personal space to write.");
        self.repaint();
    }

    /// `ready.vault` / `vault`: keep the wrap; find this device's key in the keychain.
    pub(super) fn sync_vault(&mut self, wrap: Option<crate::vault::Wrap>) {
        let (server, uid, need_key) = {
            let mut m = self.shared.model.lock();
            m.vault = wrap.clone();
            // Off, or the passcode changed / started over on another device (no telling
            // which without the passcode): drop the key and lock; the passcode opens it again.
            if wrap.as_ref().is_none_or(|w| w.ct != m.vault_key_ct) {
                if m.vault_key.take().is_some() && wrap.is_some() {
                    m.toast("Your Personal space passcode changed on another device. Unlock it again with the new one.");
                }
                m.vault_key_ct.clear();
            }
            (m.config.server.clone(), m.config.uid.clone(), wrap.is_some() && m.vault_key.is_none())
        };
        match wrap {
            None => {
                tokio::task::spawn_blocking(move || crate::vault::forget(&server, &uid));
            }
            Some(w) if need_key => {
                let shared = self.shared.clone();
                tokio::task::spawn_blocking(move || {
                    if let Some(key) = crate::vault::recall(&server, &uid, &w.ct) {
                        let mut m = shared.model.lock();
                        if m.vault.as_ref().is_some_and(|v| v.ct == w.ct) {
                            m.vault_key = Some(key);
                            m.vault_key_ct = w.ct.clone();
                        }
                        drop(m);
                        (shared.repaint)();
                    }
                });
            }
            _ => {}
        }
    }

    /// `personal.plain`: encrypt the notes written before encryption was on
    /// (pictures re-uploaded sealed), then hand them back in batches.
    pub(super) fn seal_old_notes(&mut self, notes: Vec<crate::protocol::PlainNote>) {
        let (key, server, api_key, sid, ut) = {
            let m = self.shared.model.lock();
            (m.vault_key, m.config.server.clone(), m.config.key.clone(), m.sid.clone().unwrap_or_default(), m.upload_token.clone().unwrap_or_default())
        };
        let Some(key) = key.filter(|_| !notes.is_empty()) else { return };
        let n = notes.len();
        self.shared.model.lock().toast(format!("Encrypting your {n} earlier {}…", if n == 1 { "note" } else { "notes" }));
        self.repaint();
        let tx = self.cmd_tx.clone();
        tokio::task::spawn_blocking(move || {
            let mut sealed_notes = Vec::new();
            let mut failed = 0;
            for note in notes {
                let sealed = (|| -> Result<String, String> {
                    let mut payload = crate::vault::Note { text: note.body.clone(), gif: note.gif.clone(), image: None };
                    if let Some(img) = &note.image {
                        let (bytes, mime) = crate::http::get_bytes(&format!("{server}/media/{}", img.id))?;
                        let id = crate::http::upload_blob(&server, &api_key, &sid, &ut, crate::vault::encrypt_blob(&key, &bytes))?;
                        payload.image = Some(crate::vault::NoteImage { id, width: img.width, height: img.height, mime });
                    }
                    Ok(crate::vault::encrypt_note(&key, &payload))
                })();
                match sealed {
                    Ok(body) => sealed_notes.push((note.id, body)),
                    Err(e) => {
                        log::warn!("couldn't seal note {}: {e}", note.id);
                        failed += 1;
                    }
                }
            }
            // Batches by size, not count: one frame is at most 384 KB, and long
            // notes add up (a frame that's too big is refused whole).
            let (batches, too_big) = crate::vault::seal_batches(sealed_notes, SEAL_BATCH_BYTES, SEAL_BATCH_NOTES);
            if !too_big.is_empty() {
                log::warn!("{} sealed notes are too big for one frame", too_big.len());
            }
            let _ = tx.send(Cmd::SealBatches { batches, failed: failed + too_big.len() });
        });
    }

    /// Hand the sealed notes back, and count a note sealed only when the
    /// server says so (`edited { sealed: true }`), within 20 s.
    pub(super) fn send_sealed(&mut self, batches: Vec<Vec<(String, String)>>, failed: usize) {
        let ids: HashSet<String> = batches.iter().flatten().map(|(id, _)| id.clone()).collect();
        self.seal_wait = Some(SealWait { left: ids, done: 0, failed, until: Instant::now() + SEAL_WAIT });
        for batch in &batches {
            self.send(out::personal_seal(batch));
        }
        self.check_sealed();
    }

    /// `edited { sealed: true }` for one of ours.
    pub(super) fn sealed(&mut self, id: &str) {
        if let Some(w) = self.seal_wait.as_mut()
            && w.left.remove(id)
        {
            w.done += 1;
            self.check_sealed();
        }
    }

    /// All confirmed, or out of time: say how it went.
    pub(super) fn check_sealed(&mut self) {
        let Some(w) = self.seal_wait.as_ref() else { return };
        if !w.left.is_empty() && Instant::now() < w.until {
            return;
        }
        let Some(w) = self.seal_wait.take() else { return };
        let failed = w.failed + w.left.len();
        self.shared.model.lock().toast(match failed {
            0 => "Your earlier notes are encrypted too.".to_string(),
            1 => "Encrypted your earlier notes, but 1 couldn't be (try turning it on again later).".to_string(),
            n => format!("Encrypted your earlier notes, but {n} couldn't be (try turning it on again later)."),
        });
        self.emit(CoreEvent::Sealed(w.done));
        self.repaint();
    }

    pub(super) fn vault_op(&mut self, op: VaultOp) {
        let (wrap, server, uid) = {
            let mut m = self.shared.model.lock();
            if matches!(op, VaultOp::Lock) {
                m.vault_key = None;
                m.toast("Locked on this device. Your passcode opens it again.");
                let (s, u) = (m.config.server.clone(), m.config.uid.clone());
                drop(m);
                tokio::task::spawn_blocking(move || crate::vault::forget(&s, &u));
                return self.repaint();
            }
            m.vault_busy = true;
            m.vault_error = None;
            (m.vault.clone(), m.config.server.clone(), m.config.uid.clone())
        };
        self.repaint();
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let r = tokio::task::spawn_blocking(move || -> Result<VaultResult, String> {
                let result = match op {
                    VaultOp::Setup { pass } | VaultOp::Reset { pass } => {
                        let (wrap, key) = crate::vault::create(&pass, crate::vault::ITERATIONS);
                        VaultResult::Set { wrap, key, toast: "Your Personal space is encrypted. New notes are sealed with your passcode.", seal: true }
                    }
                    VaultOp::Unlock { pass } => {
                        let w = wrap.as_ref().ok_or("It isn't encrypted.")?;
                        VaultResult::Unlocked { key: crate::vault::unlock(w, &pass).map_err(|_| "That passcode doesn't open it.".to_string())? }
                    }
                    VaultOp::Change { old, new } => {
                        let w = wrap.as_ref().ok_or("It isn't encrypted.")?;
                        let key = crate::vault::unlock(w, &old).map_err(|_| "That passcode doesn't open it.".to_string())?;
                        VaultResult::Set { wrap: crate::vault::rewrap(w, &old, &new)?, key, toast: "Passcode changed.", seal: false }
                    }
                    VaultOp::Lock => unreachable!(),
                };
                // Remember the key on this device (keychain; if there's none, just this run).
                let (key, ct) = match &result {
                    VaultResult::Set { wrap, key, .. } => (*key, wrap.ct.clone()),
                    VaultResult::Unlocked { key } => (*key, wrap.as_ref().map(|w| w.ct.clone()).unwrap_or_default()),
                };
                if let Err(e) = crate::vault::remember(&server, &uid, &key, &ct) {
                    log::warn!("no keychain here ({e}); the Personal space key stays in memory");
                }
                Ok(result)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = tx.send(Cmd::VaultDone(r));
        });
    }

    /// The encrypted Personal space.
    pub(super) async fn on_vault_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Vault(op) => self.vault_op(op),
            Cmd::SealBatches { batches, failed } => self.send_sealed(batches, failed),
            Cmd::VaultDone(r) => {
                let set = {
                    let mut m = self.shared.model.lock();
                    m.vault_busy = false;
                    m.vault_seq += 1;
                    match r {
                        Ok(VaultResult::Unlocked { key }) => {
                            m.vault_key = Some(key);
                            m.vault_key_ct = m.vault.as_ref().map(|w| w.ct.clone()).unwrap_or_default();
                            None
                        }
                        Ok(VaultResult::Set { wrap, key, toast, seal }) => {
                            m.vault = Some(wrap.clone());
                            m.vault_key = Some(key);
                            m.vault_key_ct = wrap.ct.clone();
                            m.toast(toast);
                            Some((wrap, seal))
                        }
                        Err(e) => {
                            m.vault_error = Some(e);
                            None
                        }
                    }
                };
                let what = {
                    let m = self.shared.model.lock();
                    match (&m.vault_error, &set) {
                        (Some(e), _) => e.clone(),
                        (None, Some(_)) => "set".into(),
                        (None, None) => "unlocked".into(),
                    }
                };
                self.emit(CoreEvent::Vault(what));
                // A search waiting on the key: now the encrypted notes can be searched too.
                let redo = self.shared.model.lock().search.as_ref().map(|st| (st.q.clone(), st.channel.clone()));
                if let Some((q, channel)) = redo.filter(|_| self.shared.model.lock().vault_key.is_some()) {
                    self.search(q, channel);
                }
                if let Some((wrap, seal)) = set {
                    self.send(out::vault_set(Some(&wrap)));
                    if seal {
                        // Seal what was written before (personal.plain → personal.seal).
                        self.send(out::personal_plain());
                    }
                }
                self.repaint();
            }
            _ => {}
        }
    }
}
