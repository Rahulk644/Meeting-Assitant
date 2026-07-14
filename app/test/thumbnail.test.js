"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { appThumbnailSvg, platformIdentity } = require("../thumbnail.js");

test("meeting thumbnails have a stable platform identity without video", () => {
  assert.deepEqual(platformIdentity("zoom").slice(0, 2), ["Zoom", "Z"]);
  assert.deepEqual(platformIdentity("unknown-client").slice(0, 2), ["Meeting", "M"]);
  assert.deepEqual(platformIdentity("whatsapp").slice(0, 2), ["WhatsApp", "W"]);
  assert.match(appThumbnailSvg("discord"), /aria-label="Discord"/);
  assert.match(appThumbnailSvg("discord"), />D<\/text>/);
});
