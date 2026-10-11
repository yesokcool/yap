import Foundation
import Observation
import YapCore

/// What the keeper does to the mesh (`VoiceEngine`; a fake in tests).
@MainActor
protocol CallMesh: AnyObject {
    var online: Bool { get set }
    var peerSids: [String] { get }
    func peerUid(_ sid: String) -> String?
    func connectionState(of sid: String) -> VoiceEngine.PeerState?
    func drop(_ sid: String)
    func hangUpAll()
    func enter(_ sids: [String])
    @discardableResult func rekey(from old: String, to new: String) -> Bool
    func resync(_ sid: String)
    func restart(_ sid: String)
    func setHeld(_ sid: String, _ held: Bool)
}

extension VoiceEngine: CallMesh {}

/// Who's around (`YapClient`; a fake in tests).
@MainActor
protocol CallDirectory: AnyObject {
    var sessions: [String: Session] { get }
    func holdSession(_ session: Session)
    func releaseSession(_ sid: String)
}

extension YapClient: CallDirectory {}

/// A timer the keeper can cancel.
@MainActor
protocol CallTimer: AnyObject {
    func cancel()
}

/// Real time in the app, a hand-cranked clock in tests.
@MainActor
protocol CallClock {
    func after(_ seconds: TimeInterval, _ body: @escaping @MainActor () -> Void) -> any CallTimer
}

@MainActor
struct TaskClock: CallClock {
    final class Timer: CallTimer {
        var task: Task<Void, Never>?
        func cancel() { task?.cancel() }
    }

    func after(_ seconds: TimeInterval, _ body: @escaping @MainActor () -> Void) -> any CallTimer {
        let timer = Timer()
        timer.task = Task { @MainActor in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            body()
        }
        return timer
    }
}

/// PROTOCOL.md › Voice › Keeping calls across reconnects, the bookkeeping
/// half: our own socket dropping (keep every connection, queue signals,
/// rejoin with `prev` + `proof`, give up after 20 s), someone else's
/// dropping (keep them up to 30 s as "reconnecting…"), moving connections
/// to new session ids, and holding back chimes so a quick reconnect makes no
/// sound. `VoiceController` feeds it the socket's events.
@MainActor
@Observable
final class CallKeeper {
    struct Resume: Equatable {
        /// Our session before the socket dropped, and its resume key.
        var sid: String
        var key: String
        var channelId: String
    }

    struct Ghost {
        var timer: any CallTimer
        /// Their connection is down too: gone if it stays down 5 s.
        var down: (any CallTimer)?
    }

    static let giveUpAfter: TimeInterval = 20
    static let ghostFor: TimeInterval = 30
    static let ghostDownFor: TimeInterval = 5
    static let handoverCheck: TimeInterval = 4
    static let chimeHold: TimeInterval = 5

    /// Our call while our socket is down.
    private(set) var resume: Resume?
    /// People whose socket dropped mid-call: we keep their connection a while.
    private(set) var ghosts: [String: Ghost] = [:]

    /// Chimes and blips (`.peerJoined` / `.peerLeft`).
    @ObservationIgnored var onChime: ((VoiceEvent) -> Void)?
    /// The channel the server says we're in (nil while the socket is down).
    @ObservationIgnored var channelId: () -> String? = { nil }
    /// Gave up keeping our call (20 s without getting back).
    @ObservationIgnored var onGaveUp: (() -> Void)?

    @ObservationIgnored private let mesh: any CallMesh
    @ObservationIgnored private unowned let directory: any CallDirectory
    @ObservationIgnored private let clock: any CallClock
    @ObservationIgnored private var resumeTimer: (any CallTimer)?
    @ObservationIgnored private var handoverTimer: (any CallTimer)?
    /// uid → a leave chime held back 5 s.
    @ObservationIgnored private var leaveChimes: [String: any CallTimer] = [:]
    /// The far sides' sessions when our socket dropped (to keep listing them).
    @ObservationIgnored private var snapshot: [String: Session] = [:]

    init(mesh: any CallMesh, directory: any CallDirectory, clock: any CallClock = TaskClock()) {
        self.mesh = mesh
        self.directory = directory
        self.clock = clock
    }

    func isGhost(_ sid: String) -> Bool { ghosts[sid] != nil }

    private func resumable(_ sid: String) -> Bool { directory.sessions[sid]?.voice?.resume == true }

    // MARK: Our socket

    /// Our socket dropped. In a call (and the server gave us a resume key):
    /// keep it — connections stay up, their signals wait. Otherwise hang up.
    func socketDropped(mySid: String?, key: String?, channelId: String?) {
        if resume != nil { return } // already keeping it: the first drop's 20 s still count
        if let mySid, let key, let channelId {
            resume = Resume(sid: mySid, key: key, channelId: channelId)
            let kept = Set(mesh.peerSids)
            snapshot = directory.sessions.filter { kept.contains($0.key) }
            mesh.online = false
            resumeTimer = clock.after(Self.giveUpAfter) { [weak self] in self?.giveUp() }
        } else {
            for sid in Array(ghosts.keys) { dropGhost(sid, quiet: true) }
            mesh.hangUpAll()
        }
    }

    /// Not back in time: close everything; the next join is a newcomer's.
    private func giveUp() {
        guard resume != nil else { return }
        endResume()
        for sid in Array(ghosts.keys) { dropGhost(sid, quiet: true) }
        mesh.hangUpAll()
        onGaveUp?()
    }

    /// A `ready` (new session) while keeping our call: match who's back,
    /// keep waiting for who isn't (after a deploy everyone reconnects).
    func ready() {
        guard let resume else { return }
        // Our own old session may still be listed (the server hadn't noticed): it's us.
        directory.releaseSession(resume.sid)
        for sid in mesh.peerSids where directory.sessions[sid] == nil {
            let session = snapshot[sid] ?? Session(sid: sid, uid: mesh.peerUid(sid) ?? "",
                                                   voice: VoiceState(channelId: resume.channelId, muted: false, deafened: false))
            directory.holdSession(session)
            holdGhost(sid)
        }
        for session in Array(directory.sessions.values) where session.voice != nil {
            adopt(sid: session.sid, uid: session.uid, prev: session.voice?.prev)
        }
        // Don't wait for apps that never resume (they hung up when their own
        // socket dropped), nor for anyone already back with a plain join.
        for sid in Array(ghosts.keys) {
            let ghost = directory.sessions[sid]
            let back = directory.sessions.values.contains {
                $0.sid != sid && $0.uid == ghost?.uid && ghosts[$0.sid] == nil && $0.voice?.channelId == ghost?.voice?.channelId
            }
            if ghost?.voice?.resume != true || back {
                dropGhost(sid, quiet: true)
                if !back, let uid = ghost?.uid { deferLeaveChime(uid) }
            }
        }
    }

    /// `voice.join` extras for `channelId`: continue our call if we're keeping it there.
    func joinProof(for channelId: String) -> (prev: String, proof: String)? {
        guard let resume, resume.channelId == channelId else { return nil }
        return (resume.sid, resume.key)
    }

    /// Our own echo landed in a channel. `prev` = the server took our resume.
    /// `others`: everyone else in the channel. Returns whether the call was
    /// handed over (then there's no join sound).
    @discardableResult
    func joined(prev: String?, others: [String]) -> Bool {
        defer { endResume() }
        let here = others.filter { ghosts[$0] == nil && $0 != resume?.sid }
        if let resume, prev == resume.sid {
            mesh.online = true
            for sid in mesh.peerSids where here.contains(sid) {
                // Older clients hung up on us when our socket dropped: start over with them.
                if resumable(sid) { mesh.resync(sid) } else { mesh.drop(sid) }
            }
            mesh.enter(here.filter { !mesh.peerSids.contains($0) })
            // A kept connection that hasn't come back in 4 s is rebuilt (the far side may have given up on it).
            handoverTimer?.cancel()
            handoverTimer = clock.after(Self.handoverCheck) { [weak self] in
                guard let self else { return }
                for sid in here where self.mesh.peerSids.contains(sid) && self.mesh.connectionState(of: sid) != .connected && self.resumable(sid) {
                    self.mesh.restart(sid)
                }
            }
            return true
        }
        // A fresh join (or the server didn't take our resume): start over.
        for sid in Array(ghosts.keys) { dropGhost(sid, quiet: true) }
        mesh.hangUpAll()
        mesh.enter(here)
        return false
    }

    /// We're out of voice for good (left, signed out, channel gone).
    func leave() {
        endResume()
        handoverTimer?.cancel()
        handoverTimer = nil
        for sid in Array(ghosts.keys) { dropGhost(sid, quiet: true) }
        for timer in leaveChimes.values { timer.cancel() }
        leaveChimes = [:]
    }

    private func endResume() {
        resumeTimer?.cancel()
        resumeTimer = nil
        resume = nil
        snapshot = [:]
        mesh.online = true
    }

    // MARK: Everyone else

    /// Someone else's `voice` event (`mine`: the channel we're in).
    func voiceChanged(_ change: VoiceChange, mine: String?) {
        let session = directory.sessions[change.sid]
        let uid = session?.uid
        // Someone continues an earlier session's call (maybe one we're keeping): move it over.
        let resumed = change.channelId != nil && adopt(sid: change.sid, uid: uid, prev: change.prev)
        guard let mine else { return }
        if change.channelId == mine, change.previous != mine {
            guard let uid, session?.isListener != true else { return }
            var quiet = resumed
            if !resumed {
                // A plain join by someone whose old connection we were keeping:
                // they couldn't resume it (or their app can't), so it's dead weight.
                for sid in Array(ghosts.keys) where directory.sessions[sid]?.uid == uid {
                    dropGhost(sid, quiet: true)
                    quiet = true
                }
            }
            if let held = leaveChimes.removeValue(forKey: uid) {
                // Back within 5 s of losing their connection: neither chime.
                held.cancel()
                quiet = true
            }
            if !quiet { onChime?(.peerJoined(uid: uid)) }
        } else if change.previous == mine, change.channelId != mine {
            if change.lost, change.before?.resume == true, mesh.peerSids.contains(change.sid), var kept = session {
                // Their socket dropped, but the call may well go on: keep it, quietly.
                kept.voice = change.before
                directory.holdSession(kept)
                holdGhost(change.sid)
            } else {
                mesh.drop(change.sid)
                guard let uid, session?.isListener != true else { return }
                if change.lost { deferLeaveChime(uid) } else { onChime?(.peerLeft(uid: uid)) }
            }
        }
    }

    /// `session.leave`: close the connection, unless we're keeping it.
    func sessionLeft(_ sid: String) {
        if ghosts[sid] == nil { mesh.drop(sid) }
    }

    /// A connection's state changed (nil = closed): a kept one that fails, or
    /// stays down 5 s, is let go.
    func peerStateChanged(_ sid: String, _ state: VoiceEngine.PeerState?) {
        guard var ghost = ghosts[sid] else { return }
        switch state {
        case nil, .failed?, .closed?:
            dropGhost(sid)
        case .connected?:
            ghost.down?.cancel()
            ghost.down = nil
            ghosts[sid] = ghost
        default:
            guard ghost.down == nil else { return }
            ghost.down = clock.after(Self.ghostDownFor) { [weak self] in self?.dropGhost(sid) }
            ghosts[sid] = ghost
        }
    }

    /// A session that continues `prev`'s call: move our connection (and the
    /// listing) over. Returns whether there was something to move.
    @discardableResult
    private func adopt(sid: String, uid: String?, prev: String?) -> Bool {
        guard let prev, prev != sid, prev != resume?.sid else { return false }
        // The server checked it's the same account; check again, cheaply.
        if let old = directory.sessions[prev], old.uid != uid { return false }
        if let peerUid = mesh.peerUid(prev), peerUid != uid { return false }
        let had = ghosts[prev] != nil || mesh.peerSids.contains(prev)
        guard had || directory.sessions[prev] != nil else { return false }
        clearGhost(prev)
        directory.releaseSession(prev)
        mesh.rekey(from: prev, to: sid)
        return had
    }

    private func holdGhost(_ sid: String) {
        clearGhost(sid)
        ghosts[sid] = Ghost(timer: clock.after(Self.ghostFor) { [weak self] in self?.dropGhost(sid) })
        mesh.setHeld(sid, true)
        peerStateChanged(sid, mesh.connectionState(of: sid))
    }

    private func clearGhost(_ sid: String) {
        guard let ghost = ghosts.removeValue(forKey: sid) else { return }
        ghost.timer.cancel()
        ghost.down?.cancel()
        mesh.setHeld(sid, false)
    }

    /// They didn't come back: hang up on the kept connection (and chime, unless `quiet`).
    private func dropGhost(_ sid: String, quiet: Bool = false) {
        guard ghosts[sid] != nil else { return }
        let uid = directory.sessions[sid]?.uid
        clearGhost(sid)
        directory.releaseSession(sid)
        mesh.drop(sid)
        guard !quiet, let uid, let mine = channelId() else { return }
        let stillHere = directory.sessions.values.contains { $0.uid == uid && $0.voice?.channelId == mine }
        if !stillHere { onChime?(.peerLeft(uid: uid)) }
    }

    /// A connection lost without a goodbye: chime in 5 s, unless they're back by then.
    private func deferLeaveChime(_ uid: String) {
        leaveChimes[uid]?.cancel()
        leaveChimes[uid] = clock.after(Self.chimeHold) { [weak self] in
            guard let self else { return }
            self.leaveChimes.removeValue(forKey: uid)
            if self.channelId() != nil { self.onChime?(.peerLeft(uid: uid)) }
        }
    }
}
