# Dictation target helper

`dictation-target` is the short-lived native boundary between speech recognition and a focused text field.
It classifies the focused application and Accessibility role as natural language, code/terminal, secure, or
unknown. It may inspect at most a small prefix immediately before the cursor to determine whether sentence
capitalization is safe.

For a compatible live-dictation field, the helper retains a content-free target identity, reacquires that
same control after a WebView or Electron re-render, and revises only the exact provisional span owned by the
current session. Updates use normal Unicode input events so controlled editors receive real typing events.
Every mutation is verified before the next one. Target, cursor, selection, or text drift stops mutation and
lets the parent preserve the authoritative final result on the clipboard.

The helper does not walk an Accessibility tree, capture screenshots, persist content, use the network, or
emit field text. Secure, code, terminal, selected-text, and unsupported targets never receive live updates.

Build and validate it through the production-native entry point:

```sh
./script/build_native_resources.sh
```

The complete safety contract is in [`../SECURITY.md`](../SECURITY.md#dictation-target-boundary).
