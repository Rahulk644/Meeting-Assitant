#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import process from "node:process";

const API_ROOT = "https://platform-api.wisprflow.ai/api/v1/dash";
const MAX_AUDIO_BYTES = 25 * 1024 * 1024;
const EMPTY_WAV_BASE64 = "UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAIA+AAACABAAZGF0YQAAAAA=";

function usage() {
  console.log(`Usage:
  node script/probe_wispr_flow_api.mjs --probe
  WISPR_FLOW_API_KEY=fl-... node script/probe_wispr_flow_api.mjs <16-kHz-mono.wav> [language]
  WISPR_FLOW_CLIENT_KEY=... node script/probe_wispr_flow_api.mjs <16-kHz-mono.wav> [language]

--probe verifies the documented service and authorization boundary without credentials or audio.
The script never reads Wispr Flow desktop credentials and never prints the supplied API key.`);
}

async function responseBody(response) {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function probeBoundary() {
  const warmup = await fetch(`${API_ROOT}/warmup_dash`);
  const warmupBody = await responseBody(warmup);

  const transcription = await fetch(`${API_ROOT}/api`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ audio: EMPTY_WAV_BASE64, language: ["en"] }),
  });
  const transcriptionBody = await responseBody(transcription);

  console.log(JSON.stringify({
    warmup: { status: warmup.status, body: warmupBody },
    transcription: { status: transcription.status, body: transcriptionBody },
  }, null, 2));

  if (!warmup.ok || transcription.status !== 401) {
    throw new Error("Wispr Flow's documented public API boundary did not return the expected responses");
  }
}

function bearer(value) {
  const trimmed = value.trim();
  return trimmed.startsWith("Bearer ") ? trimmed : `Bearer ${trimmed}`;
}

function validateWav(audio, path) {
  if (audio.length > MAX_AUDIO_BYTES) {
    throw new Error(`${path} exceeds Wispr Flow's documented 25 MB REST limit`);
  }
  if (audio.length < 44 || audio.toString("ascii", 0, 4) !== "RIFF" || audio.toString("ascii", 8, 12) !== "WAVE") {
    throw new Error(`${path} is not a WAV file`);
  }

  let offset = 12;
  while (offset + 8 <= audio.length) {
    const chunkID = audio.toString("ascii", offset, offset + 4);
    const chunkSize = audio.readUInt32LE(offset + 4);
    const chunkStart = offset + 8;
    if (chunkStart + chunkSize > audio.length) break;
    if (chunkID === "fmt " && chunkSize >= 16) {
      const format = audio.readUInt16LE(chunkStart);
      const channels = audio.readUInt16LE(chunkStart + 2);
      const sampleRate = audio.readUInt32LE(chunkStart + 4);
      const bitsPerSample = audio.readUInt16LE(chunkStart + 14);
      if (format !== 1 || channels !== 1 || sampleRate !== 16_000 || bitsPerSample !== 16) {
        throw new Error(`${path} must be 16-kHz, mono, 16-bit PCM WAV audio`);
      }
      return;
    }
    offset = chunkStart + chunkSize + (chunkSize % 2);
  }

  throw new Error(`${path} has no valid WAV format chunk`);
}

async function transcribe(path, language) {
  const organizationKey = process.env.WISPR_FLOW_API_KEY?.trim();
  const clientKey = process.env.WISPR_FLOW_CLIENT_KEY?.trim();
  if (Boolean(organizationKey) === Boolean(clientKey)) {
    throw new Error("Set exactly one of WISPR_FLOW_API_KEY or WISPR_FLOW_CLIENT_KEY");
  }

  const audio = await readFile(path);
  validateWav(audio, path);

  const endpoint = clientKey ? "client_api" : "api";
  const authorization = bearer(clientKey ?? organizationKey);
  const warmup = await fetch(`${API_ROOT}/warmup_dash`, {
    headers: { authorization },
  });
  if (!warmup.ok) {
    throw new Error(`Wispr Flow warm-up failed with HTTP ${warmup.status}`);
  }

  const response = await fetch(`${API_ROOT}/${endpoint}`, {
    method: "POST",
    headers: {
      authorization,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      audio: audio.toString("base64"),
      language: language ? language.split(",").map((item) => item.trim()).filter(Boolean) : undefined,
      context: {
        app: { name: "My Assistant API probe", type: "other" },
        dictionary_context: [],
      },
    }),
  });
  const body = await responseBody(response);
  if (!response.ok) {
    const detail = typeof body === "object" && body ? body.detail : body;
    throw new Error(`Wispr Flow transcription failed with HTTP ${response.status}${detail ? `: ${detail}` : ""}`);
  }

  console.log(JSON.stringify(body, null, 2));
}

const [first, second] = process.argv.slice(2);

try {
  if (first === "--help" || first === "-h") {
    usage();
  } else if (first === "--probe") {
    await probeBoundary();
  } else if (first) {
    await transcribe(first, second);
  } else {
    usage();
    process.exitCode = 2;
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
