(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.MeetingBrowserProtocol = api;
})(typeof globalThis === "object" ? globalThis : self, function () {
  "use strict";

  const VERSION = 1;
  const HOST_NAME = "com.myassistant.desktop.browser_bridge";
  const PLATFORMS = new Set(["meet", "zoom", "teams"]);
  const IDENTITY = /^[a-z]+[|]chrome-tab-[1-9][0-9]{0,9}$/;

  function clean(value, cap) {
    return String(value || "")
      .replace(/[\u0000-\u001f\u007f]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, cap);
  }

  function observation({ sequence, observedAt, platform, tabId, title, active }) {
    const normalizedPlatform = clean(platform, 16).toLowerCase();
    const numericTabId = Number(tabId);
    if (!Number.isSafeInteger(sequence) || sequence < 1) throw new Error("invalid sequence");
    if (!Number.isSafeInteger(observedAt) || observedAt < 1) throw new Error("invalid timestamp");
    if (!PLATFORMS.has(normalizedPlatform)) throw new Error("unsupported platform");
    if (!Number.isSafeInteger(numericTabId) || numericTabId < 1) throw new Error("invalid tab");
    if (typeof active !== "boolean") throw new Error("invalid active state");
    const identity = `${normalizedPlatform}|chrome-tab-${numericTabId}`;
    if (!IDENTITY.test(identity)) throw new Error("invalid identity");
    return {
      schemaVersion: VERSION,
      type: "meeting-observation",
      sequence,
      observedAt,
      observation: active
        ? {
            identity,
            app: normalizedPlatform,
            title: clean(title, 160),
            bundle: "com.google.Chrome",
            family: "browser",
          }
        : null,
    };
  }

  function isMessage(value) {
    if (!value || typeof value !== "object" || Array.isArray(value)) return false;
    if (
      value.schemaVersion !== VERSION ||
      value.type !== "meeting-observation" ||
      !Number.isSafeInteger(value.sequence) ||
      value.sequence < 1 ||
      !Number.isSafeInteger(value.observedAt) ||
      value.observedAt < 1
    ) {
      return false;
    }
    if (value.observation === null) return true;
    const item = value.observation;
    return (
      item &&
      typeof item === "object" &&
      !Array.isArray(item) &&
      IDENTITY.test(item.identity || "") &&
      PLATFORMS.has(item.app) &&
      typeof item.title === "string" &&
      item.title.length <= 160 &&
      item.bundle === "com.google.Chrome" &&
      item.family === "browser"
    );
  }

  return Object.freeze({ VERSION, HOST_NAME, observation, isMessage });
});
