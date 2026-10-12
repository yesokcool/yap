/*
 * Lower shared media while friends talk (PROTOCOL.md › Shared media, for
 * yourself; Settings › Voice, per device, off by default): while anyone else
 * in your call is speaking, Watch together and screen-share sound ease down
 * to 50 % of your volume over ~150 ms, and back over ~600 ms once nobody has
 * spoken for 1 s. It's a factor on top of the volume you saved, which it
 * never changes. Plain logic (test/unit.test.mjs); js/app/voice-ui.js runs it.
 */

export const DUCK = { level: 0.5, downMs: 150, upMs: 600, quietMs: 1000 };

export class Ducker {
  /** × your volume right now (1 = as you set it). */
  factor = 1;
  #lastTalk = -Infinity;
  #at = null;

  /** Move on to `now` (ms): `talking` = someone else in the call is speaking; `on` = the setting (and in a call). */
  step(now, talking, on) {
    const dt = this.#at == null ? 0 : Math.max(0, now - this.#at);
    this.#at = now;
    if (on && talking) this.#lastTalk = now;
    const target = on && now - this.#lastTalk < DUCK.quietMs ? DUCK.level : 1;
    const span = 1 - DUCK.level;
    if (this.factor > target) this.factor = Math.max(target, this.factor - (span * dt) / DUCK.downMs);
    else if (this.factor < target) this.factor = Math.min(target, this.factor + (span * dt) / DUCK.upMs);
    return this.factor;
  }
}
