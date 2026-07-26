const test = require("node:test");
const assert = require("node:assert/strict");
const protocol = require("../../browser-extension/protocol.js");

test("browser bridge emits only the bounded observation contract", () => {
  const value = protocol.observation({
    sequence: 7,
    observedAt: 123456,
    platform: "meet",
    tabId: 42,
    title: "  Customer\u0000   review  ",
    active: true,
  });
  assert.equal(protocol.isMessage(value), true);
  assert.deepEqual(value.observation, {
    identity: "meet|chrome-tab-42",
    app: "meet",
    title: "Customer review",
    bundle: "com.google.Chrome",
    family: "browser",
  });
  assert.equal("url" in value.observation, false);
  assert.equal("transcript" in value.observation, false);
});

test("browser bridge fails closed on unsupported sources and malformed messages", () => {
  assert.throws(
    () =>
      protocol.observation({
        sequence: 1,
        observedAt: 123,
        platform: "youtube",
        tabId: 8,
        title: "Video",
        active: true,
      }),
    /unsupported platform/,
  );
  assert.equal(
    protocol.isMessage({
      schemaVersion: 1,
      type: "meeting-observation",
      sequence: 1,
      observedAt: 123,
      observation: { identity: "../../socket", app: "meet" },
    }),
    false,
  );
});
