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

// Developer-harness mirror of the shipped Rust MeetingDetector. The production app has one authoritative
// implementation in meeting_detection.rs; this pure mirror keeps ./run.sh useful without putting policy
// back into the macOS capture helper.
class MeetingDetector {
  constructor(startHits = 2, endMisses = 3) {
    this.startHits = Math.max(1, startHits);
    this.endMisses = Math.max(1, endMisses);
    this.active = null;
    this.candidate = null;
    this.candidateHits = 0;
    this.missingHits = 0;
  }

  reset() {
    const transition = this.active ? "ended" : "none";
    this.active = this.candidate = null;
    this.candidateHits = this.missingHits = 0;
    return { active: null, transition };
  }

  update(observed, policy = {}) {
    const enabled = policy.enabled !== false;
    const allowed = observed && enabled
      && (observed.family === "desktop" ? policy.desktop !== false
        : observed.family === "browser" ? policy.browser !== false : false);
    observed = allowed ? observed : null;
    if (!enabled) return this.reset();

    if (this.active) {
      if (observed && observed.identity === this.active.identity) {
        this.active = observed;
        this.candidate = null;
        this.candidateHits = this.missingHits = 0;
        return { active: this.active, transition: "none" };
      }
      if (observed) {
        this.missingHits = 0;
        if (this.candidate && this.candidate.identity === observed.identity) this.candidateHits += 1;
        else this.candidateHits = 1;
        this.candidate = observed;
        if (this.candidateHits >= this.startHits) {
          this.active = this.candidate;
          this.candidate = null;
          this.candidateHits = 0;
          return { active: this.active, transition: "switched" };
        }
        return { active: this.active, transition: "none" };
      }
      this.candidate = null;
      this.candidateHits = 0;
      this.missingHits += 1;
      if (this.missingHits >= this.endMisses) return this.reset();
      return { active: this.active, transition: "none" };
    }

    this.missingHits = 0;
    if (!observed) {
      this.candidate = null;
      this.candidateHits = 0;
      return { active: null, transition: "none" };
    }
    if (this.candidate && this.candidate.identity === observed.identity) this.candidateHits += 1;
    else this.candidateHits = 1;
    this.candidate = observed;
    if (this.candidateHits >= this.startHits) {
      this.active = this.candidate;
      this.candidate = null;
      this.candidateHits = 0;
      return { active: this.active, transition: "started" };
    }
    return { active: null, transition: "none" };
  }
}

module.exports = { Gate, MeetingDetector };
