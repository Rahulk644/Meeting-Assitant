"use strict";

const API_PROVIDER_BASES = Object.freeze({
  openai: "https://api.openai.com/v1",
  groq: "https://api.groq.com/openai/v1",
  cerebras: "https://api.cerebras.ai/v1",
});

function providerError(message, status = 400) {
  const error = new Error(message);
  error.status = status;
  return error;
}

function aiProviderBase(provider, custom = "") {
  if (provider !== "custom" && !API_PROVIDER_BASES[provider]) throw providerError("Unknown API provider");
  const value = String(provider === "custom" ? custom : API_PROVIDER_BASES[provider]).trim().replace(/\/+$/, "");
  if (!value) throw providerError("Enter the OpenAI-compatible base URL first");
  let parsed;
  try { parsed = new URL(value); } catch { throw providerError("The provider base URL is invalid"); }
  const loopback = parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost";
  if (parsed.username || parsed.password || (parsed.protocol !== "https:" && !(loopback && parsed.protocol === "http:"))) {
    throw providerError("Custom providers must use HTTPS or a loopback address");
  }
  return value;
}

function sanitizeDeveloperAiModel(raw, providerKeys) {
  try {
    const config = JSON.parse(raw);
    if (!config || typeof config !== "object") return raw;
    const provider = String(config.provider || "local");
    if (config.key) providerKeys[provider] = String(config.key);
    // The developer harness has no durable secret store. A marker from a previous server process must not
    // imply that a key still exists; packaged Rust derives the equivalent state from macOS Keychain.
    const keySaved = !!(config.key || providerKeys[provider]);
    return JSON.stringify({ ...config, key: "", keySaved });
  } catch {
    return raw;
  }
}

async function fetchProviderModels(provider, custom, key, fetchImpl = globalThis.fetch) {
  const base = aiProviderBase(provider, custom);
  if (!key) throw providerError("Save the API key for this developer session first", 409);
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30000);
  try {
    const response = await fetchImpl(base + "/models", {
      headers: { Authorization: "Bearer " + key },
      signal: controller.signal,
    });
    const declared = Number(response.headers.get("content-length") || 0);
    if (declared > 1024 * 1024) throw providerError("Provider model catalog was too large", 502);
    const text = await response.text();
    if (Buffer.byteLength(text, "utf8") > 1024 * 1024) throw providerError("Provider model catalog was too large", 502);
    if (!response.ok) throw providerError(`${response.status} ${text.slice(0, 300)}`, 502);
    let value;
    try { value = JSON.parse(text); } catch { throw providerError("Provider returned invalid JSON", 502); }
    return [...new Set((Array.isArray(value.data) ? value.data : [])
      .map(item => item && typeof item.id === "string" ? item.id : "")
      .filter(id => id && id.length <= 160)
      .slice(0, 200))].sort();
  } catch (error) {
    if (error.name === "AbortError") throw providerError("Provider model catalog request timed out", 502);
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

module.exports = { aiProviderBase, fetchProviderModels, providerError, sanitizeDeveloperAiModel };
