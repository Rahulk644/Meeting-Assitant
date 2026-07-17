# Smart Polish runtime

The production app packages `llama-server` built from the exact llama.cpp revision pinned in
[`../script/build_polish_runtime.sh`](../script/build_polish_runtime.sh). Model weights are never committed or
bundled. Users explicitly download, verify, select, and remove supported Gemma files from Dictation settings.

The app starts the runtime only on loopback with per-launch authentication and bounded requests. Smart
Polish runs after Dictation stops; its output is treated as untrusted and accepted only when the lossless
validator preserves protected values, token order, and semantic content. Any runtime or validation failure
returns the deterministic cleanup result.

Build the pinned runtime through the canonical production entry point:

```sh
./script/build_native_resources.sh
```

See [`../SECURITY.md`](../SECURITY.md#model-and-native-code-integrity) for the integrity and privacy contract.
