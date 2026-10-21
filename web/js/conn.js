/** How long a connection attempt may take, and how long a ping may go unanswered, before we try again. */
const CONNECT_MS = 15_000;
const PONG_MS = 10_000;

/**
 * The one WebSocket to the server, with heartbeat and automatic reconnect.
 * Events: 'open', 'message' (detail = parsed frame), 'close' (detail =
 * { code, reason } when the server closed it). `url` and `protocols` can
 * change between connections (a new invite key).
 */
export class Connection extends EventTarget {
  #ws = null;
  #retry = 0;
  #retryTimer = null;
  #connectTimer = null;
  #pingTimer = null;
  #probeTimer = null;
  #pingSentAt = 0;
  #pongAt = 0;
  #stopped = false;

  constructor(url, protocols) {
    super();
    this.url = url;
    this.protocols = protocols;
    // Come back fast when the network or the tab does: retry now if we're
    // waiting to, and if the socket looks open, check it still answers (a phone
    // that slept, or a laptop that changed networks, can hold a dead one).
    const wake = () => {
      this.reconnectNow();
      this.#probe();
    };
    addEventListener('online', wake);
    document.addEventListener('visibilitychange', () => document.visibilityState === 'visible' && wake());
  }

  get open() {
    return this.#ws?.readyState === WebSocket.OPEN;
  }

  connect() {
    clearTimeout(this.#retryTimer);
    this.#retryTimer = null;
    const ws = new WebSocket(this.url, this.protocols);
    this.#ws = ws;
    // A connection attempt can hang for a minute or more (a network that
    // swallows it): give up on it and try again, like any other drop.
    clearTimeout(this.#connectTimer);
    this.#connectTimer = setTimeout(() => ws.readyState === WebSocket.CONNECTING && this.#lost(ws), CONNECT_MS);
    ws.onopen = () => {
      clearTimeout(this.#connectTimer);
      this.#retry = 0;
      this.#startHeartbeat();
      this.#emit('open');
    };
    ws.onmessage = (e) => {
      if (e.data === 'pong') {
        this.#pongAt = Date.now();
        return;
      }
      let msg;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      this.#emit('message', msg);
    };
    ws.onclose = (e) => this.#lost(ws, e);
    ws.onerror = () => {}; // a close event always follows
  }

  /** Give up for good (banned / turned away): no more reconnects. */
  stop() {
    this.#stopped = true;
    clearTimeout(this.#retryTimer);
    this.#retryTimer = null;
    try {
      this.#ws?.close();
    } catch {}
  }

  get stopped() {
    return this.#stopped;
  }

  /** Start again after stop() (a new invite link was pasted). */
  restart() {
    this.#stopped = false;
    if (!this.#ws) {
      this.#retry = 0;
      this.connect();
    }
  }

  /** Tests: lose the connection as if the network dropped it (then reconnect as usual). */
  drop() {
    if (this.#ws) this.#lost(this.#ws);
  }

  /** Skip the backoff wait if we're currently disconnected. */
  reconnectNow() {
    if (this.#retryTimer && !this.#stopped) {
      this.#retry = 0;
      this.connect();
    }
  }

  send(msg) {
    if (!this.open) return false;
    this.#ws.send(JSON.stringify(msg));
    return true;
  }

  #lost(ws, event) {
    if (this.#ws !== ws) return;
    ws.onopen = ws.onmessage = ws.onclose = null;
    try {
      ws.close();
    } catch {}
    this.#ws = null;
    clearTimeout(this.#connectTimer);
    clearTimeout(this.#probeTimer);
    this.#probeTimer = null;
    clearInterval(this.#pingTimer);
    this.#emit('close', { code: event?.code ?? 0, reason: event?.reason ?? '' });
    if (this.#stopped) return;
    const delay = Math.min(10_000, 500 * 2 ** this.#retry++) * (0.75 + Math.random() * 0.5);
    this.#retryTimer = setTimeout(() => this.connect(), delay);
  }

  #startHeartbeat() {
    clearInterval(this.#pingTimer);
    this.#pingSentAt = 0;
    this.#pingTimer = setInterval(() => {
      // A ping that's gone 10 s without a pong means the socket is dead even
      // if the browser hasn't noticed. (Timers throttle in background tabs,
      // so compare against when the ping actually went out.)
      if (this.#pingSentAt && this.#pongAt < this.#pingSentAt && Date.now() - this.#pingSentAt > PONG_MS) {
        return this.#lost(this.#ws);
      }
      if (this.open) {
        this.#ws.send('ping');
        this.#pingSentAt = Date.now();
      }
    }, 15_000);
  }

  /** One ping now; no pong within PONG_MS means the socket is dead (the heartbeat would only notice up to 25 s later). */
  #probe() {
    if (!this.open || this.#probeTimer) return;
    const ws = this.#ws;
    const sent = Date.now();
    ws.send('ping');
    this.#pingSentAt = sent;
    this.#probeTimer = setTimeout(() => {
      this.#probeTimer = null;
      if (this.#ws === ws && this.#pongAt < sent) this.#lost(ws);
    }, PONG_MS);
  }

  #emit(type, detail) {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}
