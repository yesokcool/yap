import CryptoKit
import Foundation

/// The encrypted Personal space: sealing and opening notes, the key, setting up / unlocking / changing the passcode, sealing earlier notes.
extension YapClient {
    /// My Personal space with encryption on (new notes are sealed).
    public func isSealed(_ channelId: String) -> Bool {
        vaultWrap != nil && channelsById[channelId]?.isPersonal == true
    }

    /// The space is encrypted but this device doesn't hold the key yet.
    public var vaultLocked: Bool { vaultWrap != nil && !vaultUnlocked }

    public enum NoteView: Equatable, Sendable {
        /// Not an encrypted note: show the message as it is.
        case plain
        /// Encrypted, and this device is locked.
        case locked
        case note(Vault.Note)
        /// Sealed with a key this device doesn't have (an old passcode).
        case unreadable
    }

    /// What a message body says once opened. Cached; cheap to call from views.
    public func openNote(_ body: String) -> NoteView {
        guard Vault.isEncryptedNote(body) else { return .plain }
        // Reading the observed flag makes views redraw when the space unlocks or locks.
        guard vaultUnlocked, let key = vaultKey else { return .locked }
        if let hit = noteCache[body] { return hit }
        let view: NoteView = (try? Vault.open(note: body, with: key)).map(NoteView.note) ?? .unreadable
        noteCache[body] = view
        return view
    }

    /// An encrypted picture's bytes (from `/media/<id>`), opened.
    public func openBlob(_ data: Data) throws -> Data {
        guard let key = vaultKey else { throw Vault.Failure.cantOpen }
        return try Vault.open(blob: data, with: key)
    }

    func seal(_ note: Vault.Note) -> String? {
        guard let key = vaultKey, let body = try? Vault.seal(note, with: key) else {
            notice = Notice(text: "Unlock your Personal space to write.")
            return nil
        }
        guard body.utf16.count <= MessageLength.maxSealedChars else {
            notice = Notice(text: "That note is too long to encrypt.")
            return nil
        }
        noteCache[body] = .note(note)
        return body
    }

    func syncVault(_ wrap: VaultWrap?) {
        if wrap == vaultWrap { vaultBefore = nil }
        vaultWrap = wrap
        noteCache = [:]
        guard let wrap else {
            setVaultKey(nil, tag: nil)
            return
        }
        if vaultKey != nil {
            if vaultKeyTag == wrap.ct { return }
            // Changed or reset on another device: we can't tell which without
            // the passcode, so ask for it again rather than seal with a stale key.
            setVaultKey(nil, tag: nil)
            return
        }
        guard !uid.isEmpty, let saved = credentials.vaultKey(for: uid) else { return }
        if saved.ct == wrap.ct, saved.key.count == 32 {
            vaultKey = SymmetricKey(data: saved.key)
            vaultKeyTag = saved.ct
            vaultUnlocked = true
        } else {
            credentials.setVaultKey(nil, for: uid)
        }
    }

    func setVaultKey(_ key: SymmetricKey?, tag: String?) {
        vaultKey = key
        vaultKeyTag = key == nil ? nil : tag
        vaultUnlocked = key != nil
        noteCache = [:]
        guard !uid.isEmpty else { return }
        if let key, let tag {
            credentials.setVaultKey(StoredVaultKey(key: key.withUnsafeBytes { Data($0) }, ct: tag), for: uid)
        } else {
            credentials.setVaultKey(nil, for: uid)
        }
    }

    /// Turn encryption on (or start over: a new key; old notes stay locked).
    /// PBKDF2 runs off the main actor.
    public func setUpVault(passcode: String, iterations: Int = Vault.iterations) async throws {
        guard isConnected else { throw ServerAPI.UploadError.notConnected }
        let started = session
        let (wrap, key) = try await Task.detached(priority: .userInitiated) { try Vault.create(passcode: passcode, iterations: iterations) }.value
        try stillSignedIn(started)
        vaultBefore = (vaultWrap, vaultKey, vaultKeyTag)
        send(.vaultSet(wrap))
        vaultWrap = wrap
        setVaultKey(key, tag: wrap.ct)
        await sealOldNotes()
    }

    /// Notes written before encryption was on get sealed too: ask for the
    /// plain ones, encrypt each (re-uploading its picture sealed), and hand
    /// the `e1:` bodies back; the server swaps them in place.
    /// Returns how many couldn't be sealed.
    @discardableResult
    public func sealOldNotes() async -> Int {
        guard let key = vaultKey, let endpoint, let sid = mySid, isConnected else { return 0 }
        let started = session
        let notes: [PlainNote] = await withCheckedContinuation { continuation in
            var done = false
            plainWaiter = { notes in
                guard !done else { return }
                done = true
                continuation.resume(returning: notes)
            }
            send(.personalPlain)
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(10))
                guard !done else { return }
                done = true
                self?.plainWaiter = nil
                continuation.resume(returning: [])
            }
        }
        guard !notes.isEmpty, started == session else { return 0 }
        notice = Notice(text: "Encrypting your \(notes.count) earlier \(notes.count == 1 ? "note" : "notes")…")
        let api = ServerAPI(endpoint: endpoint, session: urlSession)
        var batch: [SealedNote] = []
        var batchBytes = 0
        var failed = 0
        // A note only counts once the server says so (`edited {sealed: true}`):
        // a frame it refuses (`bad_frame`) seals nothing.
        let waiting = SealCount()
        sealWaiter = { id in waiting.confirm(id) }
        defer { if started == session { sealWaiter = nil } }
        /// Hands a batch back; a frame that can't go out counts its notes as not sealed.
        func flush() {
            guard !batch.isEmpty else { return }
            if send(.personalSeal(batch)) { waiting.expect(batch.map(\.id)) } else { failed += batch.count }
            batch = []
            batchBytes = 0
        }
        for note in notes {
            guard started == session else { return failed }
            do {
                var payload = Vault.Note(text: note.body, gif: note.gif)
                if let image = note.image {
                    let (bytes, response) = try await urlSession.data(from: endpoint.mediaURL(image.id))
                    let http = response as? HTTPURLResponse
                    guard (http?.statusCode ?? 200) < 400 else { throw Vault.Failure.cantOpen }
                    let id = try await api.uploadBlob(try Vault.seal(blob: bytes, with: key), sid: sid, uploadToken: uploadToken ?? "")
                    payload.image = Vault.SealedImage(id: id, width: image.width, height: image.height,
                                                      mime: http?.value(forHTTPHeaderField: "Content-Type") ?? "image/webp")
                }
                let body = try Vault.seal(payload, with: key)
                guard started == session else { return failed }
                noteCache[body] = .note(payload)
                let size = SealedNote.frameBytes(id: note.id, body: body)
                // Too big for any frame (a 40,000-character note with a long picture id can't be): not sealed.
                guard size <= SealedNote.maxFrameBytes else {
                    failed += 1
                    continue
                }
                // By size as well as count: the server refuses frames over 384 KB.
                if !batch.isEmpty, batch.count >= SealedNote.maxPerFrame || batchBytes + size > SealedNote.maxFrameBytes { flush() }
                batch.append(SealedNote(id: note.id, body: body))
                batchBytes += size
            } catch {
                failed += 1
            }
        }
        flush()
        // Up to 20 s for the server to confirm them all.
        let deadline = ContinuousClock.now + .seconds(20)
        while !waiting.isDone, ContinuousClock.now < deadline, started == session {
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard started == session else { return failed }
        failed += waiting.unconfirmed
        notice = Notice(text: failed > 0
                        ? "Encrypted your earlier notes, but \(failed) couldn’t be (try turning it on again later)."
                        : "Your earlier notes are encrypted too.")
        return failed
    }

    /// Open the space on this device; `Vault.Failure.wrongPasscode` if it doesn't fit.
    public func unlockVault(passcode: String) async throws {
        guard let wrap = vaultWrap else { return }
        let started = session
        let key = try await Task.detached(priority: .userInitiated) { try Vault.open(wrap, passcode: passcode) }.value
        guard started == session, vaultWrap == wrap else { return }
        setVaultKey(key, tag: wrap.ct)
    }

    /// New passcode, same key: notes stay as they are.
    public func changeVaultPasscode(old: String, new: String) async throws {
        guard let wrap = vaultWrap else { return }
        guard isConnected else { throw ServerAPI.UploadError.notConnected }
        let started = session
        let (next, key) = try await Task.detached(priority: .userInitiated) {
            (try Vault.rewrap(wrap, old: old, new: new), try Vault.open(wrap, passcode: old))
        }.value
        try stillSignedIn(started)
        vaultBefore = (vaultWrap, vaultKey, vaultKeyTag)
        send(.vaultSet(next))
        vaultWrap = next
        setVaultKey(key, tag: next.ct)
    }

    /// Forget the key on this device (the passcode opens it again).
    public func lockVault() {
        setVaultKey(nil, tag: nil)
    }
}

/// Sealing earlier notes: which ones went out and which the server confirmed.
@MainActor
final class SealCount {
    private var sent: Set<String> = []
    private var confirmed: Set<String> = []

    func expect(_ ids: [String]) { sent.formUnion(ids) }
    func confirm(_ id: String) { confirmed.insert(id) }
    var unconfirmed: Int { sent.subtracting(confirmed).count }
    var isDone: Bool { unconfirmed == 0 }
}
