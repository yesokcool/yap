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
  #pingTimer = null;
  #pingSentAt = 0;
  #pongAt = 0;
  #stopped = false;

  constructor(url, protocols) {
    super();
    this.url = url;
    this.protocols = protocols;
    // Come back fast when the network or the tab does.
    addEventListener('online', () => this.reconnectNow());
    document.addEventListener('visibilitychange', () => document.visibilityState === 'visible' && this.reconnectNow());
  }

  get open() {
    return this.#ws?.readyState === WebSocket.OPEN;
  }

  connect() {
    clearTimeout(this.#retryTimer);
    this.#retryTimer = null;
    const ws = new WebSocket(this.url, this.protocols);
    this.#ws = ws;
    ws.onopen = () => {
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
      if (this.#pingSentAt && this.#pongAt < this.#pingSentAt && Date.now() - this.#pingSentAt > 10_000) {
        return this.#lost(this.#ws);
      }
      if (this.open) {
        this.#ws.send('ping');
        this.#pingSentAt = Date.now();
      }
    }, 15_000);
  }

  #emit(type, detail) {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}
