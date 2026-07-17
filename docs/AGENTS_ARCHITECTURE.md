# Agents: deferred product boundary

Agents is not part of the initial My Assistant product. This short document exists only to prevent dormant
research code or historical commits from being mistaken for a current product capability.

## Production contract

- The sidebar renders one greyed-out, disabled **Agents · Coming soon** label.
- The label is not clickable and has no route.
- Production exposes no Agent explanation, request field, settings, shortcut, permission request, compute
  option, account flow, or action.
- Agent native helpers are not packaged.
- Agent initialization does not run at application boot.
- Saved state and direct route attempts resolve to a shipped surface.
- The release artifact must remain valid when all dormant Agent resources are absent.

These statements are enforced by `app/release.js`, `app/test/release.test.js`,
`desktop/src-tauri/tauri.conf.json`, and `script/verify_app_bundle.sh`.

## Dormant source

The repository history and internal preview source may contain experiments created before the launch
boundary was narrowed. They are research only. They must not be described as shipped, used as release
evidence, packaged into the DMG, enabled by production settings, or allowed to expand the initial app's
permissions or network behavior.

Removing dormant source is a separate refactor because it must preserve Dictation and Meetings behavior and
their test coverage. Until that decision is made, production gating and package exclusion are mandatory.

## Reopening condition

Agents can be reconsidered only through a new product decision that defines, before implementation is
exposed:

1. the user problem and intentionally narrow first scope;
2. the privacy and data-retention model;
3. the permission and network boundary;
4. confirmation, reversibility, and failure behavior;
5. model/runtime packaging and commercial constraints;
6. synthetic, adversarial, and physical acceptance gates; and
7. a separate release plan and documentation review.

Until all seven exist, **Coming soon** is the complete and only product claim.
