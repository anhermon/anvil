---
name: Platform Change
about: OS-specific fix, execution lock/lifecycle change, adapter compat patch, or control-plane stale-state fix
title: '[PLATFORM] '
labels: ['platform']
assignees: ''
---

## Objective
<!-- One sentence: what this fix achieves and why it matters. -->

## Scope

**Touch:** <!-- Files, systems, or areas to modify -->
**Do not touch:** <!-- Explicit exclusions to prevent scope creep -->

## Verification
- [ ] <!-- Concrete, machine-checkable acceptance criterion -->
- [ ] <!-- Another criterion if needed -->

## Fork Platform Flag

<!-- REQUIRED for all platform changes. See doc/F-SERIES-CONVENTION.md for the full guide. -->

| Field              | Value                                                        |
|--------------------|--------------------------------------------------------------|
| Flag ID            | FP<!-- next available number from the registry --> |
| Scope              | <!-- all / Windows / Linux / macOS -->                       |
| Status             | <!-- applied / partial / rolled-back -->                     |
| Verification       | <!-- How to confirm the fix is in effect -->                 |
| Known Gaps         | <!-- Remaining work not covered — write "none" if complete -->|
| Upstream Alignment | <!-- Upstream FN flag this relates to, or "none" -->         |
| Related Issues     | <!-- Links to other FP-series issues in this fix chain -->   |

## Upstream Conflict Check

*Required for all platform changes — check upstream before implementing.*

- [ ] Searched upstream tracker (paperclipai/paperclip) — no parallel implementation found
- [ ] Prior Art Link: <!-- link if upstream work exists, or "none" -->
- [ ] Reconciliation Scope: <!-- supersede / align with upstream / deliberate fork divergence -->

## Prior Art Search

- Projects searched: <!-- e.g., paperclipai/paperclip, anhermon/anvil -->
- Keywords used: <!-- e.g., "stale lock", "process_lost", "Windows manifest" -->
- Related issues found: <!-- Links or "none" -->
- Why this issue is new vs duplicate/superseded: <!-- brief rationale -->

## Additional Context
<!-- Logs, error messages, or any other information helpful for diagnosis. -->
