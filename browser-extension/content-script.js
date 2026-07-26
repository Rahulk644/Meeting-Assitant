(function () {
  "use strict";

  function platformAndActive() {
    const host = location.hostname.toLowerCase();
    const path = location.pathname;
    if (host === "meet.google.com") {
      return { platform: "meet", active: /^\/[a-z]{3}-[a-z]{4}-[a-z]{3}(?:\/|$)/i.test(path) };
    }
    if (host.endsWith(".zoom.us")) {
      return { platform: "zoom", active: /^\/(?:wc|j)\/[^/]+/i.test(path) };
    }
    if (host === "teams.microsoft.com") {
      return {
        platform: "teams",
        active: /\/(?:l\/meetup-join|v2\/|_#\/modern-calling)\b/i.test(path + location.hash),
      };
    }
    return null;
  }

  let last = "";
  function publish() {
    const state = platformAndActive();
    if (!state) return;
    const message = {
      type: "page-observation",
      platform: state.platform,
      active: state.active,
      title: String(document.title || "").slice(0, 160),
    };
    const fingerprint = JSON.stringify(message);
    if (fingerprint === last) return;
    last = fingerprint;
    chrome.runtime.sendMessage(message).catch(function () {});
  }

  publish();
  new MutationObserver(publish).observe(document.documentElement, {
    subtree: true,
    childList: true,
  });
  addEventListener("hashchange", publish);
  addEventListener("popstate", publish);
  setInterval(publish, 5000);
})();
