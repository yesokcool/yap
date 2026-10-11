import Foundation

/// Who this device is: sign up / sign in / sign out, the connection and its hello, sign-in errors, devices, the web sign-in pass.
extension YapClient {
    public var savedName: String? { credentials.name }
    public var savedColor: String? { credentials.color }
    public var hasToken: Bool { credentials.token != nil }

    /// Still signing in with an old 6-digit PIN (Settings › Account says
    /// "Set a password", and the current one may be the PIN).
    public var onPin: Bool { secretKind == "pin" }

    /// Why to ask for a password now (`ready.weakPin`), nil when there's
    /// nothing to ask. The app shows it once per device (PROTOCOL.md › Identity).
    /// The server says why (`ready.weakReason`); an older one doesn't: then
    /// an old PIN if I signed in with one, else a guessable password.
    public var passwordNudge: PasswordNudge? {
        guard weakPin, mySid != nil else { return nil }
        if let reason = weakReason.flatMap(PasswordNudge.init(rawValue:)) { return reason }
        return onPin ? .pin : .guessable
    }

    /// Connect with the stored device token (normal launch).
    public func start() {
        guard endpoint != nil, credentials.token != nil else { return }
        if case .blocked = phase { return }
        pendingHello = nil
        phase = .connecting
        openConnection()
    }

    /// After `/api/check` said the key is good. The same server with a new
    /// key (the old one was rotated) keeps this device signed in; another
    /// server starts over.
    public func useInvite(_ endpoint: ServerEndpoint) {
        let sameServer = self.endpoint?.baseURL == endpoint.baseURL
        if !sameServer {
            connection?.stop()
            connection = nil
            credentials.token = nil
            resetServerState()
        }
        credentials.endpoint = endpoint
        self.endpoint = endpoint
        authError = nil
        if sameServer, credentials.token != nil {
            phase = .connecting
            start()
        } else {
            phase = .signedOut
        }
    }

    public func signUp(name: String, pin: String, color: String) {
        credentials.name = name
        credentials.color = color
        beginAuth(.signup(name: name, pin: pin, color: color))
    }

    public func logIn(name: String, pin: String) {
        credentials.name = name
        beginAuth(.login(name: name, pin: pin))
    }

    func beginAuth(_ hello: HelloAuth) {
        guard endpoint != nil else { return }
        authError = nil
        credentials.token = nil
        pendingHello = hello
        phase = .connecting
        if let connection, connection.isOpen, mySid == nil {
            sendHello()
        } else {
            openConnection()
        }
    }

    /// Revoke this device's token and go back to the sign-in screen.
    public func signOut() {
        if let connection, connection.isOpen {
            // A goodbye first, so nobody keeps our call waiting for us to come back.
            if let mySid, sessions[mySid]?.voice != nil { connection.send(.voiceLeave) }
            connection.send(.accountSignout)
            // Let the frame go out before closing our end.
            Task { [connection] in
                try? await Task.sleep(for: .milliseconds(400))
                connection.stop()
            }
        } else {
            connection?.stop()
        }
        connection = nil
        endSession()
        authError = nil
        phase = endpoint == nil ? .needsInvite : .signedOut
    }

    /// Everything signing out forgets on this device, whether I signed out
    /// here or the server no longer knows this device (signed out from
    /// another one): leave voice for good, forget the Personal space key
    /// (someone else may use this device next) and everything the server
    /// sent. The caller closes the connection.
    func endSession() {
        voice?.clientDidSignOut()
        if !uid.isEmpty { credentials.setVaultKey(nil, for: uid) }
        credentials.token = nil
        pendingHello = nil
        resetServerState()
        uid = ""
    }

    /// Sign out and forget the server too (paste a new invite).
    public func forgetServer() {
        signOut()
        credentials.endpoint = nil
        endpoint = nil
        phase = .needsInvite
    }

    /// Set or change my password (`account.pin`; `current` = my password,
    /// or my old PIN while I'm on one).
    public func changePassword(to pin: String, current: String?) {
        pinChange = .saving
        if !send(.accountPin(pin: pin, current: current?.isEmpty == true ? nil : current)) {
            pinChange = .failed("Not connected right now. Try again in a moment.")
        }
    }

    func openConnection() {
        guard let endpoint else { return }
        connection?.stop()
        let connection = YapConnection(url: endpoint.webSocketURL, protocols: endpoint.webSocketProtocols, session: urlSession)
        connection.onOpen = { [weak self] in self?.sendHello() }
        connection.onMessage = { [weak self] in self?.handle($0) }
        connection.onClose = { [weak self] in self?.connectionClosed($0) }
        self.connection = connection
        connection.connect()
    }

    func sendHello() {
        if let pendingHello {
            send(.hello(pendingHello, client: clientLabel))
        } else if let token = credentials.token {
            send(.hello(.token(token), client: clientLabel))
        } else {
            connection?.stop()
            phase = .signedOut
        }
    }

    func connectionClosed(_ reason: YapConnection.CloseReason) {
        switch reason {
        case .stopped:
            return
        case .badKey:
            mySid = nil
            resumeKey = nil // no coming back to this call
            voice?.clientConnectionDropped()
            authError = AuthError(code: "bad_key", message: "That invite link doesn't work anymore. Paste the new one.", mode: .login)
            phase = .needsInvite
        case .lost:
            let wasOnline = mySid != nil
            mySid = nil
            voice?.clientConnectionDropped()
            resumeKey = nil
            switch phase {
            case .online, .connecting: phase = wasOnline || hasToken ? .reconnecting : phase
            default: break
            }
        }
    }

    /// App came to the foreground / network returned: skip the backoff.
    public func reconnectNow() {
        connection?.reconnectNow()
    }

    @discardableResult
    public func send(_ message: ClientMessage) -> Bool {
        connection?.send(message) ?? false
    }

    public var isConnected: Bool { connection?.isOpen == true && mySid != nil }

    func onError(code: String, message: String, re: String?) {
        switch code {
        case "banned", "denied":
            connection?.stop()
            mySid = nil
            voice?.clientDidSignOut()
            phase = .blocked(title: code == "banned" ? "You can't join this server" : "Not let in", message: message)
        case "pending":
            return
        default:
            if re == "hello" {
                helloFailed(code: code, message: message)
            } else if re == "account.pin" {
                pinChange = .failed(message)
            } else if re == "vault.set", let before = vaultBefore {
                // Refused: put back the space as it was.
                vaultBefore = nil
                vaultWrap = before.wrap
                setVaultKey(before.key, tag: before.tag)
                notice = Notice(text: message)
            } else if code == "no_peer" {
                return
            } else if re == "send" {
                sendRefused(code: code, message: message)
            } else if re == "shelf" {
                // Stop the spinner; say why.
                for key in shelves.keys where shelves[key]?.loading == true { shelves[key]?.loading = false }
                notice = Notice(text: message)
            } else if re == "device.handoff" {
                // No pass (slowed down): the web app just asks for name + password.
                handoffWaiter?(nil)
                handoffWaiter = nil
            } else {
                notice = Notice(text: message)
            }
        }
    }

    func helloFailed(code: String, message: String) {
        let attempted = pendingHello
        pendingHello = nil
        connection?.stop()
        connection = nil
        // This device was signed out elsewhere (revoked, or "Sign out
        // everywhere else"): the same clean slate as signing out here.
        if code == "bad_token" { endSession() }
        var mode: AuthMode = .login
        if case .signup = attempted { mode = .signup }
        if ["bad_token", "bad_login", "locked", "no_pin"].contains(code) { mode = .login }
        authError = AuthError(code: code, message: Self.friendly(code: code, message: message, attempted: attempted), mode: mode)
        phase = .signedOut
    }

    /// Friendly copy for sign-in errors; the server's own message otherwise.
    nonisolated static func friendly(code: String, message: String, attempted: HelloAuth?) -> String {
        switch code {
        case "bad_token": return "This device was signed out. Sign in again with your name and password."
        // The server's words, as the web shows them ("Wrong name or password.");
        // the sign-on screen adds who can help (`AuthText.help`).
        case "bad_login": return message.isEmpty ? "Wrong name or password." : message
        case "name_taken":
            if case let .signup(name, _, _) = attempted {
                return "Someone here is already called \(name). If that's you, choose “I have an account” and sign in with your password."
            }
            return message
        // The server's own words ("Use a password of at least 8 characters.",
        // "That password is too easy to guess.", "Too many wrong tries…").
        case "bad_pin": return message.isEmpty ? "Use a password of at least 8 characters." : message
        case "weak_pin": return message.isEmpty ? "That password is too easy to guess." : message
        case "bad_name": return "Pick a name friends will recognize."
        default: return message
        }
    }

    func resetServerState() {
        // Anything still in flight (an upload, a passcode being stretched)
        // belongs to the session that just ended.
        session += 1
        mySid = nil
        uploadToken = nil
        resumeKey = nil
        channelsById = [:]
        users = [:]
        sessions = [:]
        heldSessions = []
        emojis = [:]
        waitingRoom = [:]
        histories = [:]
        outbox = [:]
        sendLine = SendLine()
        pacer?.cancel()
        pacer = nil
        returnedDraft = nil
        typing = [:]
        activeChannelId = nil
        links = []
        invites = []
        createdInvite = nil
        modLog = nil
        vaultWrap = nil
        vaultKey = nil
        vaultUnlocked = false
        noteCache = [:]
        weakPin = false
        weakReason = nil
        devices = nil
        signinAlert = nil
        search = nil
        pins = [:]
        watch = nil
        watchRequested = false
        sounds = [:]
        hangs = []
        hangStarting = nil
        notifySettings = nil
        unreadCounts = nil
        shelves = [:]
        awayNotes = [:]
        mentionCounts = [:]
        localNotes = []
        pinsWanted = []
        shelfWanted = []
        pendingCreate = nil
        createdChannel = nil
        vaultKeyTag = nil
        vaultBefore = nil
        secretKind = nil
        pinChange = .idle
        pinReset = nil
        lastTypingSent = [:]
        plainWaiter?([])
        plainWaiter = nil
        sealWaiter = nil
        handoffWaiter?(nil)
        handoffWaiter = nil
    }

    /// Throws when the session that started some async work has ended since.
    func stillSignedIn(_ started: Int) throws {
        if started != session { throw CancellationError() }
    }

    /// Where an upload goes: this server, this connection's sid and upload
    /// token. Throws "Not connected right now." when there's no connection.
    func uploadTarget() throws -> (api: ServerAPI, sid: String, token: String) {
        guard let endpoint, let sid = mySid else { throw ServerAPI.UploadError.notConnected }
        return (ServerAPI(endpoint: endpoint, session: urlSession), sid, uploadToken ?? "")
    }

    public func refreshDevices() { send(.accountDevices) }
    public func revokeDevice(_ id: String) { send(.accountRevoke(id: id)) }
    /// Signs out every other device (OBS links included).
    public func signOutOthers() {
        signinAlert = nil
        send(.accountSignoutOthers)
    }

    /// A one-time sign-in pass for the web app (PROTOCOL.md › `device.handoff`),
    /// so Draw / Space / Watch open signed in instead of asking for name +
    /// password. Waits a little for the connection (Draw opened right at launch);
    /// nil if still offline or the server didn't answer in a few seconds.
    public func handoffPass() async -> String? {
        for _ in 0..<40 where !isConnected {
            try? await Task.sleep(for: .milliseconds(200))
        }
        guard isConnected else { return nil }
        handoffWaiter?(nil)
        return await withCheckedContinuation { continuation in
            var done = false
            handoffWaiter = { token in
                guard !done else { return }
                done = true
                continuation.resume(returning: token)
            }
            send(.deviceHandoff)
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(4))
                guard !done else { return }
                done = true
                self?.handoffWaiter = nil
                continuation.resume(returning: nil)
            }
        }
    }
}
