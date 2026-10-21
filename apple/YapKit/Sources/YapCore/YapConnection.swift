import Foundation
import Network
import os

/// The one WebSocket to the server, with heartbeat and automatic reconnect
/// (PROTOCOL.md › WebSocket framing).
///
/// Everything runs on the main actor: the receive loop awaits the socket off
/// the main thread and resumes here, so callbacks need no hopping. Each
/// `connect()` bumps `generation`; work from an older socket is dropped.
@MainActor
public final class YapConnection {
    public enum CloseReason: Sendable, Equatable {
        /// Network dropped, server restarted, heartbeat timed out: we retry.
        case lost
        /// The invite key is wrong: the upgrade was refused with 401 (servers
        /// before the main link could be reset), or the socket was closed
        /// with 4003 (a wrong key may only say `hello { token }`, and that
        /// token didn't work). No retry.
        case badKey
        /// `stop()` was called. No retry.
        case stopped
    }

    public var onOpen: (() -> Void)?
    public var onMessage: ((ServerMessage) -> Void)?
    public var onClose: ((CloseReason) -> Void)?

    public private(set) var isOpen = false
    public private(set) var isStopped = true

    /// Where the next `connect()` goes, and with which key (`retarget`).
    private(set) var url: URL
    private(set) var protocols: [String]
    private let session: URLSession
    private var task: URLSessionWebSocketTask?
    private var generation = 0
    private var retry = 0
    private var retryTask: Task<Void, Never>?
    private var runTask: Task<Void, Never>?
    private var heartbeatTask: Task<Void, Never>?
    private var openWatch: Task<Void, Never>?
    private var pongAt: ContinuousClock.Instant?
    private var pathMonitor: NWPathMonitor?
    private var lastPath: NetworkPathSignature?
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "socket")

    /// Ping every 15 s; no pong within 10 s means the socket is dead.
    var pingInterval: Duration = .seconds(15)
    var pongTimeout: Duration = .seconds(10)
    /// After a network change (Wi-Fi ↔ cellular, with the screen off too):
    /// a socket that doesn't answer this fast is on the old network.
    var probeTimeout: Duration = .seconds(4)
    /// An upgrade that hasn't finished by then is stuck (a network that
    /// died under it, a captive portal): try again. URLSession would wait
    /// 60 s, and nothing (coming back on screen, a new network) hurried it.
    var openTimeout: Duration = .seconds(12)

    public init(url: URL, protocols: [String] = [], session: URLSession = .shared) {
        self.url = url
        self.protocols = protocols
        self.session = session
    }

    public func connect() {
        retryTask?.cancel()
        retryTask = nil
        isStopped = false
        watchNetwork()
        teardown()
        generation += 1
        let gen = generation
        let task = protocols.isEmpty ? session.webSocketTask(with: url) : session.webSocketTask(with: url, protocols: protocols)
        task.maximumMessageSize = 16 << 20
        self.task = task
        task.resume()
        runTask = Task { [weak self] in await self?.run(task, gen: gen) }
        openWatch = Task { [weak self] in
            guard let timeout = self?.openTimeout else { return }
            try? await Task.sleep(for: timeout)
            guard let self, !Task.isCancelled, gen == self.generation, !self.isOpen else { return }
            self.log.info("socket didn't open in time")
            self.lost(badKey: false)
        }
    }

    /// The main invite key changed (`key`, `ready.key`): reconnects use the
    /// new one. The socket that's open stays as it is.
    public func retarget(url: URL, protocols: [String]) {
        self.url = url
        self.protocols = protocols
    }

    /// The close code the server uses for a wrong invite key (PROTOCOL.md ›
    /// Resetting the main link).
    static let wrongKeyCloseCode = 4003

    /// A socket that ended this way had a wrong invite key: the upgrade's 401
    /// (older servers), or 4003 from servers that let a wrong key through
    /// for `hello { token }` only.
    static func endedForWrongKey(status: Int?, closeCode: Int) -> Bool {
        status == 401 || closeCode == wrongKeyCloseCode
    }

    /// Give up for good (signed out, banned, turned away): no more reconnects.
    public func stop() {
        isStopped = true
        retryTask?.cancel()
        retryTask = nil
        pathMonitor?.cancel()
        pathMonitor = nil
        lastPath = nil
        let wasOpen = isOpen || task != nil
        teardown()
        generation += 1
        if wasOpen { onClose?(.stopped) }
    }

    /// Skip the backoff wait if we're disconnected (app came back, network returned).
    public func reconnectNow() {
        guard !isStopped else { return }
        if retryTask != nil {
            retry = 0
            connect()
        } else if isOpen {
            sendPing(timeout: pongTimeout)
        }
    }

    @discardableResult
    public func send(_ message: ClientMessage) -> Bool {
        guard isOpen, let task else { return false }
        do {
            let text = try message.frame()
            task.send(.string(text)) { @Sendable error in
                if let error { Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "socket").debug("send failed: \(error.localizedDescription)") }
            }
            return true
        } catch {
            log.error("encode failed: \(error.localizedDescription)")
            return false
        }
    }

    /// Tests: lose the socket as if the network had (it reconnects by itself).
    func dropForTesting() {
        guard isOpen || task != nil else { return }
        lost(badKey: false)
    }

    // MARK: - Internals

    private func run(_ task: URLSessionWebSocketTask, gen: Int) async {
        do {
            // The first frame goes out once the upgrade succeeds; until then
            // `send` waits. A refused upgrade (401) throws here.
            let sentAt = ContinuousClock.now
            try await task.send(.string("ping"))
            guard gen == generation else { return }
            isOpen = true
            retry = 0
            openWatch?.cancel()
            openWatch = nil
            expectPong(after: sentAt, within: pongTimeout, gen: gen)
            startHeartbeat(gen: gen)
            onOpen?()
            while gen == generation {
                let frame = try await task.receive()
                guard gen == generation else { return }
                switch frame {
                case .string(let text): handle(text)
                case .data(let data): handle(String(decoding: data, as: UTF8.self))
                @unknown default: break
                }
            }
        } catch {
            guard gen == generation else { return }
            let status = (task.response as? HTTPURLResponse)?.statusCode
            lost(badKey: Self.endedForWrongKey(status: status, closeCode: task.closeCode.rawValue))
        }
    }

    private func handle(_ text: String) {
        if text == "pong" {
            pongAt = .now
            return
        }
        do {
            onMessage?(try ServerMessage.decode(text))
        } catch {
            log.debug("undecodable frame: \(error.localizedDescription)")
        }
    }

    /// A ping every `pingInterval`; each one has its own deadline (so an
    /// extra ping — the app coming back, a network change — is never judged
    /// by the regular one's clock).
    private func startHeartbeat(gen: Int) {
        heartbeatTask?.cancel()
        heartbeatTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let interval = self?.pingInterval else { return }
                try? await Task.sleep(for: interval)
                guard let self, gen == self.generation, !Task.isCancelled else { return }
                self.sendPing(timeout: self.pongTimeout)
            }
        }
    }

    private func sendPing(timeout: Duration) {
        guard isOpen, let task else { return }
        let sentAt = ContinuousClock.now
        task.send(.string("ping")) { @Sendable _ in }
        expectPong(after: sentAt, within: timeout, gen: generation)
    }

    /// No pong since `sentAt` by the deadline: the socket is dead.
    private func expectPong(after sentAt: ContinuousClock.Instant, within timeout: Duration, gen: Int) {
        Task { [weak self] in
            try? await Task.sleep(for: timeout)
            guard let self, gen == self.generation, self.isOpen else { return }
            if !Heartbeat.answered(sentAt: sentAt, pongAt: self.pongAt) {
                self.log.info("heartbeat timed out")
                self.lost(badKey: false)
            }
        }
    }

    // MARK: Network changes

    /// Wi-Fi ↔ cellular (or any new route): ask the socket at once instead
    /// of waiting up to 25 s for the heartbeat; if it's down, skip the backoff.
    private func watchNetwork() {
        guard pathMonitor == nil else { return }
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            let signature = NetworkPathSignature(path)
            Task { @MainActor [weak self] in self?.networkChanged(signature) }
        }
        monitor.start(queue: DispatchQueue(label: "yap.socket.path"))
        pathMonitor = monitor
    }

    private func networkChanged(_ path: NetworkPathSignature) {
        defer { lastPath = path }
        guard let lastPath, lastPath != path, !isStopped, path.satisfied else { return }
        log.info("network changed: \(path.interfaces, privacy: .public)")
        if isOpen {
            sendPing(timeout: probeTimeout)
        } else if retryTask != nil || task != nil {
            // Waiting to retry, or an upgrade under way on the old route: go now.
            retry = 0
            connect()
        }
    }

    private func lost(badKey: Bool) {
        teardown()
        generation += 1
        onClose?(badKey ? .badKey : .lost)
        guard !isStopped, !badKey else {
            isStopped = true
            return
        }
        let base = min(10.0, 0.5 * pow(2.0, Double(retry)))
        retry += 1
        let delay = base * Double.random(in: 0.75...1.25)
        retryTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard let self, !Task.isCancelled, !self.isStopped else { return }
            self.retryTask = nil
            self.connect()
        }
    }

    private func teardown() {
        isOpen = false
        heartbeatTask?.cancel()
        heartbeatTask = nil
        openWatch?.cancel()
        openWatch = nil
        runTask?.cancel()
        runTask = nil
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        pongAt = nil
    }
}

/// The heartbeat's rule (PROTOCOL.md › WebSocket framing): a ping is
/// answered by any pong that came after it.
enum Heartbeat {
    static func answered(sentAt: ContinuousClock.Instant, pongAt: ContinuousClock.Instant?) -> Bool {
        guard let pongAt else { return false }
        return pongAt >= sentAt
    }
}

/// What about a network path matters to an open socket: whether it's up
/// and which interfaces it goes through (a new one = a new route).
struct NetworkPathSignature: Equatable, Sendable {
    var satisfied: Bool
    var interfaces: String

    init(satisfied: Bool, interfaces: String) {
        self.satisfied = satisfied
        self.interfaces = interfaces
    }

    init(_ path: NWPath) {
        satisfied = path.status == .satisfied
        interfaces = path.availableInterfaces.map(\.name).joined(separator: ",")
    }
}
