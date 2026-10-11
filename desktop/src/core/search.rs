//! Search and jumping to a message (loading older pages until it's there),
//! encrypted notes included.

use super::*;

impl Core {
    /// Keep looking for a jump target after a page arrived.
    pub(super) fn continue_seek(&mut self, channel_id: &str) {
        let Some((ch, id, pages)) = self.seek.clone() else { return };
        if ch != channel_id {
            return;
        }
        let (found, has_more) = {
            let m = self.shared.model.lock();
            let h = m.history.get(&ch);
            (h.is_some_and(|h| h.messages.iter().any(|x| x.id == id)), h.is_some_and(|h| h.has_more))
        };
        if found {
            self.seek = None;
            self.shared.model.lock().jump_to = Some((ch, id.clone()));
            self.emit(CoreEvent::Jumped { id, pages });
        } else if has_more && pages < 20 {
            self.seek = Some((ch.clone(), id, pages + 1));
            self.load_older(&ch);
        } else {
            self.seek = None;
            let text = if has_more { "That one is further back — scroll up to find it." } else { "Couldn't find that message (it may have been deleted)." };
            self.shared.model.lock().toast(text);
            self.emit(CoreEvent::Jumped { id, pages: u8::MAX });
        }
    }

    pub(super) fn jump_to(&mut self, channel: String, id: String) {
        let current = self.shared.model.lock().config.channel.clone();
        if channel != current || self.shared.model.lock().view.is_some() {
            self.open_channel(channel.clone());
        }
        let (found, ready) = {
            let m = self.shared.model.lock();
            let h = m.history.get(&channel);
            (h.is_some_and(|h| h.messages.iter().any(|x| x.id == id)), h.is_some_and(|h| !h.loading))
        };
        if found {
            self.shared.model.lock().jump_to = Some((channel, id.clone()));
            self.emit(CoreEvent::Jumped { id, pages: 0 });
            return self.repaint();
        }
        self.seek = Some((channel.clone(), id, 0));
        if ready {
            self.continue_seek(&channel);
        }
        self.repaint();
    }

    /// Encrypted notes never match on the server: search them here once the
    /// Personal space's history is loaded (up to 20 older pages).
    pub(super) fn continue_personal_search(&mut self, channel_id: &str) {
        let more = {
            let mut m = self.shared.model.lock();
            let personal = m.channel(channel_id).is_some_and(|c| c.is_personal());
            let Some(st) = m.search.as_ref() else { return };
            if !personal || m.vault_key.is_none() || st.channel.as_deref().is_some_and(|c| c != channel_id) {
                return;
            }
            let pages = st.personal_pages;
            let q = st.q.clone();
            let local = local_matches(&m, channel_id, &q);
            let has_more = m.history.get(channel_id).is_some_and(|h| h.has_more && !h.loading);
            if let Some(st) = m.search.as_mut() {
                st.local = local;
                if has_more && pages < 20 {
                    st.personal_pages += 1;
                }
            }
            has_more && pages < 20
        };
        if more {
            self.load_older(channel_id);
        }
    }

    pub(super) fn search(&mut self, q: String, channel: Option<String>) {
        let q = q.trim().to_string();
        if q.is_empty() {
            self.shared.model.lock().search = None;
            return self.repaint();
        }
        let personal = {
            let mut m = self.shared.model.lock();
            m.search = Some(SearchState { q: q.clone(), channel: channel.clone(), loading: true, ..Default::default() });
            let personal = m.channels.iter().find(|c| c.is_personal()).map(|c| c.id.clone());
            personal.filter(|p| m.vault_key.is_some() && channel.as_ref().is_none_or(|c| c == p))
        };
        self.send(out::search(&q, channel.as_deref()));
        // Encrypted notes: search on the device (loading the Personal space's history first).
        if let Some(p) = personal {
            let loaded = self.shared.model.lock().history.get(&p).map(|h| !h.loading);
            match loaded {
                None => self.load_latest(&p),
                Some(true) => self.continue_personal_search(&p),
                Some(false) => {}
            }
        }
        self.repaint();
    }
}
