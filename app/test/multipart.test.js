"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { injectMultipartFields } = require("../multipart.js");

const B = "X-BOUNDARY-123";
const CT = `multipart/form-data; boundary=${B}`;

/** Build a minimal one-file multipart body, as curl -F would. */
function fileBody(bytes = Buffer.from([0, 1, 2, 3])) {
  const head = Buffer.from(
    `--${B}\r\nContent-Disposition: form-data; name="file"; filename="a.wav"\r\n` +
      `Content-Type: audio/wav\r\n\r\n`
  );
  const tail = Buffer.from(`\r\n--${B}--\r\n`);
  return Buffer.concat([head, bytes, tail]);
}

test("injects fields before the closing boundary and preserves the file bytes", () => {
  const audio = Buffer.from([9, 8, 7, 6, 5]);
  const out = injectMultipartFields(fileBody(audio), CT, { language: "en", translate: "false" });
  const s = out.toString("latin1");
  assert.ok(s.includes('name="language"\r\n\r\nen\r\n'));
  assert.ok(s.includes('name="translate"\r\n\r\nfalse\r\n'));
  assert.ok(out.includes(audio), "original audio bytes survive intact");
  assert.ok(s.endsWith(`--${B}--\r\n`), "ends with exactly one closing boundary");
  assert.equal((s.match(new RegExp(`--${B}--`, "g")) || []).length, 1, "no duplicate closing boundary");
});

test("skips null / empty field values", () => {
  const out = injectMultipartFields(fileBody(), CT, { language: "en", prompt: "", vocab: null });
  const s = out.toString("latin1");
  assert.ok(s.includes('name="language"'));
  assert.ok(!s.includes('name="prompt"'));
  assert.ok(!s.includes('name="vocab"'));
});

test("no boundary in Content-Type → body returned unchanged", () => {
  const body = fileBody();
  assert.equal(injectMultipartFields(body, "application/octet-stream", { language: "en" }), body);
});

test("quoted boundary is parsed", () => {
  const out = injectMultipartFields(fileBody(), `multipart/form-data; boundary="${B}"`, { language: "hi" });
  assert.ok(out.toString("latin1").includes('name="language"\r\n\r\nhi\r\n'));
});

test("all-empty fields → body returned unchanged", () => {
  const body = fileBody();
  assert.equal(injectMultipartFields(body, CT, { prompt: "", vocab: null }), body);
});
