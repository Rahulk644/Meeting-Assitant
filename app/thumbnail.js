"use strict";

const PLATFORMS = Object.freeze({
  zoom: ["Zoom", "Z", "#2D8CFF", "#EAF3FF"],
  teams: ["Microsoft Teams", "T", "#5B5FC7", "#EEEEFF"],
  meet: ["Google Meet", "M", "#0F9D58", "#E7F7EE"],
  webex: ["Webex", "W", "#087E8B", "#E5F6F7"],
  facetime: ["FaceTime", "F", "#34C759", "#E8F8EC"],
  slack: ["Slack Huddle", "S", "#611F69", "#F5EAF6"],
  discord: ["Discord", "D", "#5865F2", "#ECEEFF"],
  jitsi: ["Jitsi Meet", "J", "#205081", "#E8F0F7"],
  whereby: ["Whereby", "W", "#FF5A5F", "#FFF0F0"],
  chime: ["Amazon Chime", "C", "#1F73B7", "#E9F3FA"],
  ringcentral: ["RingCentral", "R", "#066FAC", "#E8F4FA"],
  goto: ["GoTo Meeting", "G", "#6D57FF", "#F0EDFF"],
  riverside: ["Riverside", "R", "#7C3AED", "#F1EAFF"],
  around: ["Around", "A", "#FF6B35", "#FFF0EA"],
  gather: ["Gather", "G", "#6B4EFF", "#F0EDFF"],
  butter: ["Butter", "B", "#FFB000", "#FFF7DF"],
  skype: ["Skype", "S", "#00AFF0", "#E5F7FD"],
  whatsapp: ["WhatsApp", "W", "#25D366", "#E8F9EE"],
  telegram: ["Telegram", "T", "#229ED9", "#E8F6FC"],
  signal: ["Signal", "S", "#3A76F0", "#EAF0FE"],
});

function platformIdentity(app) {
  return PLATFORMS[String(app || "").trim().toLowerCase()] || ["Meeting", "M", "#4F64C8", "#EEF0FA"];
}

function appThumbnailSvg(app) {
  const [label, mark, accent, background] = platformIdentity(app);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240" viewBox="0 0 240 240" role="img" aria-label="${label}"><rect width="240" height="240" rx="42" fill="${background}"/><rect x="58" y="58" width="124" height="124" rx="36" fill="${accent}"/><text x="120" y="142" text-anchor="middle" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="66" font-weight="700" fill="white">${mark}</text></svg>`;
}

module.exports = { appThumbnailSvg, platformIdentity };
