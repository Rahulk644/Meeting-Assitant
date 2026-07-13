"use strict";
// Recording gate — composes auto-detection with the user's manual override into one
// effective "are we recording?" state. Pure: no I/O, no timers, no globals, so the
// server can unit-test every transition (see test/gate.test.js).
//
// Two inputs compose:
//   detected — the capturer's self-reported "in a call".
//   manual   — the UI override: null = follow auto-detect, 'on' | 'paused' | 'off'.
// A call-relative override expires when the call state changes, so the NEXT call still
// auto-records. A "hard" manual recording (started with no call under it) is exempt —
// it persists until the user stops it, so a stray detection can't hijack it.

class Gate {
  constructor(autoDetect = true) {
    this.detected = false;
    this.manual = null;
    this.autoDetect = autoDetect;
    this._manualDetected = false; // call state when `manual` was set (for expiry)
    this._manualHard = false;     // manual recording with no call to fall back to
  }

  get recording() {
    if (this.manual === "on") return true;
    if (this.manual === "paused" || this.manual === "off") return false;
    return this.autoDetect && this.detected;
  }

  get paused() {
    return this.manual === "paused";
  }

  setAutoDetect(on) {
    this.autoDetect = !!on;
  }

  /** Apply a raw detection. Returns true if `detected` actually changed. */
  setDetected(d) {
    d = !!d;
    if (d === this.detected) return false;
    this.detected = d;
    if (this.manual !== null && !this._manualHard && this.detected !== this._manualDetected) {
      this.manual = null; // the call this override was scoped to has ended → hand back to auto
    }
    return true;
  }

  /** Apply a UI override. Returns false for an unknown action (leaves state untouched). */
  setManual(action) {
    switch (action) {
      case "start":  this.manual = "on";     this._manualHard = !(this.autoDetect && this.detected); break;
      case "resume": this.manual = "on";     this._manualHard = false; break;
      case "pause":  this.manual = "paused"; this._manualHard = false; break;
      case "stop":   this.manual = "off";    this._manualHard = false; break;
      case "auto":   this.manual = null;     this._manualHard = false; break;
      default: return false;
    }
    this._manualDetected = this.detected;
    return true;
  }
}

module.exports = { Gate };
