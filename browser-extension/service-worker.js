importScripts("protocol.js");

const protocol = globalThis.MeetingBrowserProtocol;
let nativePort = null;
let sequence = 0;
let reconnectTimer = null;
const activeTabs = new Map();

function connect() {
  if (nativePort) return nativePort;
  try {
    nativePort = chrome.runtime.connectNative(protocol.HOST_NAME);
    nativePort.onDisconnect.addListener(function () {
      nativePort = null;
      if (!reconnectTimer) {
        reconnectTimer = setTimeout(function () {
          reconnectTimer = null;
          connect();
        }, 5000);
      }
    });
    return nativePort;
  } catch (_) {
    nativePort = null;
    return null;
  }
}

chrome.runtime.onMessage.addListener(function (message, sender) {
  if (
    !message ||
    message.type !== "page-observation" ||
    !sender.tab ||
    !Number.isSafeInteger(sender.tab.id)
  ) {
    return;
  }
  try {
    if (message.active) activeTabs.set(sender.tab.id, message.platform);
    else activeTabs.delete(sender.tab.id);
    const value = protocol.observation({
      sequence: ++sequence,
      observedAt: Date.now(),
      platform: message.platform,
      tabId: sender.tab.id,
      title: message.title,
      active: message.active,
    });
    if (protocol.isMessage(value)) connect()?.postMessage(value);
  } catch (_) {
    // Invalid page input is ignored at the least-privileged boundary.
  }
});

function publishClosedTab(tabId) {
  const platform = activeTabs.get(tabId);
  if (!platform) return;
  activeTabs.delete(tabId);
  try {
    const value = protocol.observation({
      sequence: ++sequence,
      observedAt: Date.now(),
      platform,
      tabId,
      title: "",
      active: false,
    });
    if (protocol.isMessage(value)) connect()?.postMessage(value);
  } catch (_) {}
}

chrome.tabs.onRemoved.addListener(publishClosedTab);
chrome.tabs.onUpdated.addListener(function (tabId, changeInfo) {
  if (changeInfo.status === "loading") publishClosedTab(tabId);
});
