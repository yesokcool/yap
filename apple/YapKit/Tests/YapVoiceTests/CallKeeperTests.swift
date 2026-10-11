import Foundation
import Testing
import YapCore
@testable import YapVoice

/// The mesh, as far as the keeper can tell: who we have a connection with.
@MainActor
final class FakeMesh: CallMesh {
    struct Conn: Equatable {
        var uid: String
        var state: VoiceEngine.PeerState = .connected
        var held = false
        /// Bumped by restart(): a brand-new connection.
        var generation = 0
    }

    var online = true
    var conns: [String: Conn] = [:]
    var log: [String] = []
    var peerSids: [String] { conns.keys.sorted() }
    /// What the engine reports back (VoiceController wires it to the keeper).
    var onPeerState: ((String, VoiceEngine.PeerState?) -> Void)?
    var uidOf: (String) -> String = { _ in "?" }

    func peerUid(_ sid: String) -> String? { conns[sid]?.uid }
    func connectionState(of sid: String) -> VoiceEngine.PeerState? { conns[sid]?.state }
    func drop(_ sid: String) {
        guard conns.removeValue(forKey: sid) != nil else { return }
        log.append("drop \(sid)")
        onPeerState?(sid, nil)
    }
    func hangUpAll() {
        log.append("hangUpAll")
        for sid in peerSids { drop(sid) }
        online = true
    }
    func enter(_ sids: [String]) {
        guard !sids.isEmpty else { return }
        log.append("enter \(sids.sorted().joined(separator: ","))")
        for sid in sids where conns[sid] == nil { conns[sid] = Conn(uid: uidOf(sid), state: .new) }
    }
    @discardableResult func rekey(from old: String, to new: String) -> Bool {
        guard let conn = conns[old], old != new else { return false }
        if conns[new] != nil {
            drop(old)
            return false
        }
        log.append("rekey \(old)→\(new)")
        conns.removeValue(forKey: old)
        conns[new] = Conn(uid: conn.uid, state: conn.state, held: false, generation: conn.generation)
        onPeerState?(old, nil)
        return true
    }
    func resync(_ sid: String) { log.append("resync \(sid)") }
    func restart(_ sid: String) {
        guard conns[sid] != nil else { return }
        log.append("restart \(sid)")
        conns[sid]!.generation += 1
        conns[sid]!.state = .new
    }
    func setHeld(_ sid: String, _ held: Bool) { conns[sid]?.held = held }

    func connect(_ sid: String, uid: String) { conns[sid] = Conn(uid: uid) }
    func setState(_ sid: String, _ state: VoiceEngine.PeerState) {
        conns[sid]?.state = state
        onPeerState?(sid, state)
    }
}

@MainActor
final class FakeDirectory: CallDirectory {
    var sessions: [String: Session] = [:]
    var held: Set<String> = []
    func holdSession(_ session: Session) {
        sessions[session.sid] = session
        held.insert(session.sid)
    }
    func releaseSession(_ sid: String) {
        held.remove(sid)
        sessions.removeValue(forKey: sid)
    }

    func add(_ sid: String, uid: String, in channel: String? = "lounge", resume: Bool = true, prev: String? = nil) {
        sessions[sid] = Session(sid: sid, uid: uid, client: "web", voice: channel.map { VoiceState(channelId: $0, muted: false, deafened: false, resume: resume, prev: prev) })
    }
}

/// A hand-cranked clock.
@MainActor
final class ManualClock: CallClock {
    final class Timer: CallTimer {
        let at: TimeInterval
        let body: @MainActor () -> Void
        var cancelled = false
        init(at: TimeInterval, body: @escaping @MainActor () -> Void) {
            self.at = at
            self.body = body
        }
        func cancel() { cancelled = true }
    }

    var now: TimeInterval = 0
    var timers: [Timer] = []

    func after(_ seconds: TimeInterval, _ body: @escaping @MainActor () -> Void) -> any CallTimer {
        let timer = Timer(at: now + seconds, body: body)
        timers.append(timer)
        return timer
    }

    func advance(_ seconds: TimeInterval) {
        let end = now + seconds
        while let next = timers.filter({ !$0.cancelled && $0.at <= end }).min(by: { $0.at < $1.at }) {
            now = next.at
            next.cancelled = true
            next.body()
        }
        now = end
        timers.removeAll { $0.cancelled }
    }
}

/// PROTOCOL.md › Voice › Keeping calls across reconnects: our socket
/// dropping, theirs dropping, moving connections to new sids, the timeouts
/// and the chimes.
@MainActor
@Suite("Keeping calls across reconnects")
final class CallKeeperTests {
    let mesh = FakeMesh()
    let directory = FakeDirectory()
    let clock = ManualClock()
    let keeper: CallKeeper
    var chimes: [VoiceEvent] = []
    var mine: String? = "lounge"

    init() {
        keeper = CallKeeper(mesh: mesh, directory: directory, clock: clock)
        keeper.onChime = { [unowned self] in self.chimes.append($0) }
        keeper.channelId = { [unowned self] in self.mine }
        mesh.onPeerState = { [unowned self] sid, state in self.keeper.peerStateChanged(sid, state) }
        mesh.uidOf = { [unowned self] sid in self.directory.sessions[sid]?.uid ?? "?" }
        // Me (s-me, uid me) in Lounge with Ada (s-ada) and Ben (s-ben).
        directory.add("s-me", uid: "me")
        directory.add("s-ada", uid: "ada")
        directory.add("s-ben", uid: "ben")
        mesh.connect("s-ada", uid: "ada")
        mesh.connect("s-ben", uid: "ben")
    }

    /// The server's `voice` event, applied to the directory like YapClient does.
    func voice(_ sid: String, channel: String?, lost: Bool = false, prev: String? = nil, resume: Bool = true) {
        let before = directory.sessions[sid]?.voice
        directory.sessions[sid]?.voice = channel.map { VoiceState(channelId: $0, muted: false, deafened: false, resume: resume, prev: prev) }
        keeper.voiceChanged(VoiceChange(sid: sid, channelId: channel, before: before, lost: lost, prev: prev), mine: mine)
    }

    /// A new session appears (`session.join`) and joins voice.
    func newSession(_ sid: String, uid: String, prev: String? = nil, resume: Bool = true) {
        directory.sessions[sid] = Session(sid: sid, uid: uid, client: "web")
        voice(sid, channel: "lounge", prev: prev, resume: resume)
    }

    func sessionLeave(_ sid: String) {
        if !directory.held.contains(sid) { directory.sessions.removeValue(forKey: sid) }
        keeper.sessionLeft(sid)
    }

    /// Our socket drops, comes back as `s-me2`, the server takes our resume.
    func dropAndResume(readySessions: [Session]) {
        keeper.socketDropped(mySid: "s-me", key: "k-me", channelId: "lounge")
        mine = nil
        directory.sessions = Dictionary(uniqueKeysWithValues: readySessions.map { ($0.sid, $0) })
        directory.held = []
        keeper.ready()
    }

    // MARK: Our socket

    @Test func ourDropKeepsEveryConnectionAndQueuesSignals() {
        keeper.socketDropped(mySid: "s-me", key: "k-me", channelId: "lounge")
        #expect(!mesh.online, "signals wait")
        #expect(mesh.peerSids == ["s-ada", "s-ben"], "nobody hung up on")
        #expect(!mesh.log.contains("hangUpAll"))
        #expect(keeper.joinProof(for: "lounge")?.prev == "s-me")
        #expect(keeper.joinProof(for: "lounge")?.proof == "k-me")
        #expect(keeper.joinProof(for: "other") == nil, "only the call we kept")
        #expect(chimes.isEmpty)
    }

    @Test func handoverKeepsResumableConnectionsAndStartsOverWithOlderApps() {
        directory.sessions["s-ben"]?.voice?.resume = false // Ben's app predates resuming
        // Both are back (Ada as s-ada2 continuing s-ada; Ben hung up and rejoined as s-ben2).
        let ada2 = Session(sid: "s-ada2", uid: "ada", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true, prev: "s-ada"))
        let ben2 = Session(sid: "s-ben2", uid: "ben", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false))
        dropAndResume(readySessions: [ada2, ben2])
        #expect(mesh.log.contains("rekey s-ada→s-ada2"), "Ada's connection moved to her new session")
        #expect(mesh.peerSids.contains("s-ada2"))
        #expect(!mesh.peerSids.contains("s-ben"), "Ben's old connection is dead weight: he's back with a plain join")
        #expect(keeper.ghosts.isEmpty)

        directory.add("s-me2", uid: "me")
        let handedOver = keeper.joined(prev: "s-me", others: ["s-ada2", "s-ben2"])
        mine = "lounge"
        #expect(handedOver, "no join sound")
        #expect(mesh.online)
        #expect(mesh.log.contains("resync s-ada2"), "queued signals go out to her current sid")
        #expect(mesh.log.contains("enter s-ben2"), "an older app gets a newcomer's offer")
        #expect(keeper.resume == nil)
        #expect(chimes.isEmpty, "a reconnect makes no sound")
    }

    @Test func ourOldSessionStillListedIsNotSomeoneToCall() {
        // The server hadn't noticed our old socket die: ready still lists it.
        let oldMe = Session(sid: "s-me", uid: "me", client: "ios", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))
        let ada = Session(sid: "s-ada", uid: "ada", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))
        let ben = Session(sid: "s-ben", uid: "ben", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))
        dropAndResume(readySessions: [oldMe, ada, ben])
        #expect(directory.sessions["s-me"] == nil, "that's us")
        keeper.joined(prev: "s-me", others: ["s-me", "s-ada", "s-ben"])
        #expect(!mesh.peerSids.contains("s-me"))
        #expect(!mesh.log.contains { $0.hasPrefix("enter") }, "everyone's connection was kept")
        #expect(mesh.log.filter { $0.hasPrefix("resync") }.count == 2)
    }

    @Test func peopleNotBackYetAreKeptWhileTheyReconnectToo() {
        // A deploy: everyone drops. We're back first; Ada and Ben aren't listed yet.
        dropAndResume(readySessions: [])
        #expect(Set(keeper.ghosts.keys) == ["s-ada", "s-ben"], "kept as reconnecting…")
        #expect(directory.held == ["s-ada", "s-ben"], "still listed")
        #expect(mesh.conns["s-ada"]?.held == true, "their signals wait too")
        keeper.joined(prev: "s-me", others: [])
        mine = "lounge"
        // Ada comes back continuing her call.
        newSession("s-ada2", uid: "ada", prev: "s-ada")
        #expect(mesh.log.contains("rekey s-ada→s-ada2"))
        #expect(keeper.ghosts["s-ada"] == nil)
        #expect(directory.sessions["s-ada"] == nil, "listed once, under her new session")
        #expect(chimes.isEmpty)
    }

    @Test func notBackIn20sGivesUp() {
        var gaveUp = false
        keeper.onGaveUp = { gaveUp = true }
        keeper.socketDropped(mySid: "s-me", key: "k-me", channelId: "lounge")
        clock.advance(19.9)
        #expect(keeper.resume != nil && mesh.peerSids.count == 2)
        clock.advance(0.2)
        #expect(gaveUp)
        #expect(keeper.resume == nil)
        #expect(mesh.peerSids.isEmpty, "every connection closed: we join as a newcomer next")
        #expect(mesh.online)
        #expect(keeper.joinProof(for: "lounge") == nil)
    }

    @Test func aJoinWithoutPrevStartsOver() {
        dropAndResume(readySessions: [])
        // The server didn't take our resume (or it's a different server now).
        directory.add("s-ada9", uid: "ada")
        let handedOver = keeper.joined(prev: nil, others: ["s-ada9"])
        #expect(!handedOver)
        #expect(mesh.log.contains("hangUpAll"))
        #expect(mesh.peerSids == ["s-ada9"], "offered to everyone as a newcomer")
        #expect(keeper.ghosts.isEmpty && directory.held.isEmpty)
    }

    @Test func aKeptConnectionNotUp4sAfterTheHandoverIsRebuilt() {
        let ada = Session(sid: "s-ada", uid: "ada", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))
        let ben = Session(sid: "s-ben", uid: "ben", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))
        dropAndResume(readySessions: [ada, ben])
        mesh.conns["s-ben"]?.state = .disconnected
        keeper.joined(prev: "s-me", others: ["s-ada", "s-ben"])
        clock.advance(3.9)
        #expect(!mesh.log.contains("restart s-ben"))
        clock.advance(0.2)
        #expect(mesh.log.contains("restart s-ben"), "fresh start: the far side sees a new fingerprint and follows")
        #expect(!mesh.log.contains("restart s-ada"), "a connected one is left alone")
    }

    @Test func noSocketNoKeyOrNoCallHangsUpLikeBefore() {
        // A server that predates resume keys.
        keeper.socketDropped(mySid: "s-me", key: nil, channelId: "lounge")
        #expect(mesh.log.contains("hangUpAll") && mesh.peerSids.isEmpty)
        #expect(keeper.resume == nil)
    }

    // MARK: Their socket

    @Test func theirLostConnectionIsKeptQuietlyAndMovedWhenTheyResume() {
        voice("s-ada", channel: nil, lost: true)
        #expect(keeper.isGhost("s-ada"))
        #expect(directory.sessions["s-ada"]?.voice?.channelId == "lounge", "still listed in Lounge (reconnecting…)")
        sessionLeave("s-ada")
        #expect(mesh.peerSids.contains("s-ada"), "session.leave doesn't close it either")
        #expect(directory.sessions["s-ada"] != nil)
        newSession("s-ada2", uid: "ada", prev: "s-ada")
        #expect(mesh.log.contains("rekey s-ada→s-ada2"))
        #expect(!keeper.isGhost("s-ada") && !keeper.isGhost("s-ada2"))
        #expect(directory.sessions["s-ada"] == nil)
        clock.advance(60)
        #expect(chimes.isEmpty, "no leave, no join")
    }

    @Test func aResumeByAnotherAccountIsNotMatched() {
        voice("s-ada", channel: nil, lost: true)
        newSession("s-eve", uid: "eve", prev: "s-ada")
        #expect(!mesh.log.contains { $0.hasPrefix("rekey") })
        #expect(keeper.isGhost("s-ada"))
        #expect(chimes == [.peerJoined(uid: "eve")])
    }

    @Test func aGhostIsLetGoAfter30s() {
        voice("s-ada", channel: nil, lost: true)
        clock.advance(29.9)
        #expect(keeper.isGhost("s-ada"))
        clock.advance(0.2)
        #expect(!keeper.isGhost("s-ada"))
        #expect(!mesh.peerSids.contains("s-ada"))
        #expect(directory.sessions["s-ada"] == nil)
        #expect(chimes == [.peerLeft(uid: "ada")], "now they've left")
    }

    @Test func aGhostWhoseConnectionFailsOrStaysDown5sIsLetGo() {
        voice("s-ada", channel: nil, lost: true)
        voice("s-ben", channel: nil, lost: true)
        mesh.setState("s-ada", .failed)
        #expect(!keeper.isGhost("s-ada"), "failed: at once")
        mesh.setState("s-ben", .disconnected)
        clock.advance(4)
        mesh.setState("s-ben", .connected)
        clock.advance(2)
        #expect(keeper.isGhost("s-ben"), "back up within 5 s")
        mesh.setState("s-ben", .disconnected)
        clock.advance(5.1)
        #expect(!keeper.isGhost("s-ben"))
        #expect(chimes == [.peerLeft(uid: "ada"), .peerLeft(uid: "ben")])
    }

    @Test func aPlainRejoinBySomeoneWeWereKeepingReplacesThemQuietly() {
        voice("s-ada", channel: nil, lost: true)
        newSession("s-ada2", uid: "ada") // her app couldn't resume: a plain join, she offers
        #expect(!keeper.isGhost("s-ada"))
        #expect(!mesh.peerSids.contains("s-ada"))
        #expect(chimes.isEmpty, "back within moments: neither chime")
    }

    @Test func anOlderAppsLostConnectionChimesOnlyAfter5s() {
        directory.sessions["s-ben"]?.voice?.resume = false
        voice("s-ben", channel: nil, lost: true)
        #expect(!mesh.peerSids.contains("s-ben"), "nothing to keep: it hung up on us")
        clock.advance(4.9)
        #expect(chimes.isEmpty)
        clock.advance(0.2)
        #expect(chimes == [.peerLeft(uid: "ben")])
    }

    @Test func backWithin5sMeansNeitherChime() {
        directory.sessions["s-ben"]?.voice?.resume = false
        voice("s-ben", channel: nil, lost: true)
        clock.advance(2)
        newSession("s-ben2", uid: "ben", resume: false)
        clock.advance(10)
        #expect(chimes.isEmpty)
    }

    @Test func aDeliberateLeaveChimesAtOnce() {
        voice("s-ada", channel: nil)
        #expect(!mesh.peerSids.contains("s-ada"))
        #expect(!keeper.isGhost("s-ada"))
        #expect(chimes == [.peerLeft(uid: "ada")])
        newSession("s-cleo", uid: "cleo")
        #expect(chimes.last == .peerJoined(uid: "cleo"))
    }

    @Test func leavingDropsGhostsAndHeldChimesQuietly() {
        voice("s-ada", channel: nil, lost: true)
        directory.sessions["s-ben"]?.voice?.resume = false
        voice("s-ben", channel: nil, lost: true)
        keeper.leave()
        mine = nil
        clock.advance(60)
        #expect(keeper.ghosts.isEmpty && directory.held.isEmpty)
        #expect(chimes.isEmpty)
    }

    // MARK: Who's polite

    @Test func politeIsRecomputedFromTheCurrentSids() {
        #expect(Peer.isPolite(mine: "b", theirs: "a"))
        #expect(!Peer.isPolite(mine: "a", theirs: "b"))
        // After a resume either sid may have changed: the same pair can swap roles.
        #expect(!Peer.isPolite(mine: "a2", theirs: "b"))
        #expect(Peer.isPolite(mine: "c", theirs: "b"))
        #expect(!Peer.isPolite(mine: nil, theirs: "a"))
    }

    @Test func aNewFingerprintIsAFreshStart() {
        let sdp = { (print: String) in "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\na=group:BUNDLE 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=fingerprint:sha-256 \(print)\r\na=setup:actpass\r\n" }
        #expect(SDPTuner.fingerprint(in: sdp("AB:CD:EF")) == "AB:CD:EF")
        #expect(SDPTuner.fingerprint(in: sdp("AB:CD:EF")) != SDPTuner.fingerprint(in: sdp("12:34:56")))
        #expect(SDPTuner.fingerprint(in: "v=0\r\nm=audio 9 RTP/AVP 0\r\n") == nil)
    }
}
