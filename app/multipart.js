"use strict";
// Append extra text fields to an existing multipart/form-data body without re-parsing the
// file part. Dictation POSTs only the WAV; we splice in language / translate / prompt so the
// STT engine transcribes in the right script. Pure and binary-safe — see test/multipart.test.js.

/**
 * @param {Buffer} buf         the raw multipart body (e.g. from curl -F file=@…)
 * @param {string} contentType the request Content-Type (carries boundary=…)
 * @param {Object} fields      name → value; null/"" values are skipped
 * @returns {Buffer} the body with `fields` appended before the closing boundary
 *                   (returns `buf` unchanged if no multipart boundary is present)
 */
function injectMultipartFields(buf, contentType, fields) {
  const bm = /boundary=(?:"([^"]+)"|([^";,\s]+))/i.exec(contentType || "");
  if (!bm) return buf;
  const boundary = bm[1] || bm[2];
  const parts = Object.entries(fields)
    .filter(([, v]) => v != null && v !== "")
    .map(([name, v]) => `--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${v}\r\n`)
    .join("");
  if (!parts) return buf;
  const closing = Buffer.from(`--${boundary}--`);
  const cut = buf.lastIndexOf(closing);
  const head = cut >= 0 ? buf.subarray(0, cut) : buf; // drop the old closing boundary, re-add it after our fields
  return Buffer.concat([head, Buffer.from(parts + `--${boundary}--\r\n`)]);
}

module.exports = { injectMultipartFields };
