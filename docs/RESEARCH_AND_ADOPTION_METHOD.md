# Research and adoption method

This project studies a fast-moving dictation and meeting-assistant ecosystem. This document defines how a
claim becomes engineering evidence and how an external component may enter the repository.

## Evidence levels

Use the lowest honest level that supports a statement:

| Level | Evidence | What it can prove |
|---|---|---|
| E0 — idea | Product copy, roadmap, or unverified report | A question worth investigating; never implementation or support |
| E1 — source/contract | Official documentation, public protocol, or reviewed source at an exact revision | The documented or coded behavior, not runtime quality |
| E2 — automated | Deterministic unit, contract, parser, build, security, or packaging check | The tested invariant on the named environment |
| E3 — installed smoke | The exact installed candidate performs one named local workflow | That workflow on that machine, not a platform matrix |
| E4 — physical matrix | Repeatable human/device/app cases with privacy-safe evidence | The explicitly tested support boundary |
| E5 — release | Signed artifact, clean install, rollback, SBOM/licenses, and all required E4 gates | A distributable release for the declared scope |

Automated input is never reported as a physical microphone, Accessibility, two-person call, display/Space,
device-switch, clean-install, or recovery pass. A screenshot demonstrates one state, not a complete journey.

## External-source classes

### Open-source components

Before code is copied, vendored, linked, or executed in production, record:

1. upstream repository and exact revision/version;
2. license and retained copyright/notice files;
3. the smallest internal adapter or sidecar boundary;
4. local patches and why they exist;
5. security, privacy, resource, quality, and platform gates;
6. update owner and previous known-good rollback revision; and
7. SBOM and consolidated-license impact.

Prefer a maintained library behind a trait, then a pinned sidecar, then a separately deployed service. Do
not fork an entire application to obtain one subsystem.

### Proprietary products

Official documentation and black-box product behavior may define interoperability and acceptance criteria.
Installed-package metadata or compiled packages may be inspected where lawful to understand process,
permission, and protocol boundaries, but the result is research evidence—not a source dependency.

Do not commit or redistribute proprietary source/compiled code, assets, private endpoints, desktop session
tokens, Keychain material, local databases, user data, or copied implementation details. A contributor who
reviewed proprietary internals must implement from a written product contract or independently licensed
source and should document that boundary. When provenance is unclear, stop and reimplement from public
requirements.

## Adoption record

Every proposed component should answer this template in its issue or pull request:

```text
User problem:
Source and exact revision:
License and notices:
Internal boundary:
Data read/written/transmitted:
Idle and active resource cost:
Security and failure behavior:
Qualification evidence:
Unsupported cases:
Rollback:
```

The component table in [`OSS_COMPONENT_STRATEGY.md`](OSS_COMPONENT_STRATEGY.md) records decisions. A roadmap
entry is not adoption, and an upstream claim is not local qualification.

## Reference-app research

Competitive observations belong in [`COMPETITIVE_INTELLIGENCE.md`](COMPETITIVE_INTELLIGENCE.md) and should
identify when and how they were made. Turn observations into implementation-neutral requirements such as:

- recording must begin without waiting for the remote participant to speak;
- ambiguous identity remains `Unknown speaker`;
- losing a capture callback becomes a visible recovering/failed state;
- post-call output preserves source transcript and provenance; and
- a connector write requires explicit scope, destination, idempotency, and a receipt.

Product names and screenshots are never a substitute for a tested contract, and visual identity/assets are
not copied.
