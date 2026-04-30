# F-Series Fork Platform Change Convention

**Version:** 1.0  
**Last Updated:** 2026-04-30  
**Owner:** Engineering Manager (`e1a9742f-0d04-4cdb-97f7-6eeaa87332c8`)  
**Applies To:** Anvil (Paperclip Fork) — all platform-level changes

---

## Purpose

The upstream Paperclip project uses structured feature identifiers (F8, F8-R, F15, F21…) to make
platform-level changes traceable, partially-applied states explicit, and gaps discoverable.

The Anvil fork adopts the same pattern under its own namespace (`FP` = Fork Platform) so that:

1. Fork-applied patches are clearly distinct from upstream F-series flags.
2. Partially-applied states (e.g. fix applied but follow-up pending) are visible at-a-glance.
3. Fix chains across multiple issues are legible without reading every commit.
4. Gaps between expected and actual application are surfaced proactively.

---

## Identifier Format

```
FP<N>          — a fork platform change (sequential, one per platform scope)
FP<N>-R        — a deliberate rollback or revert of FP<N>
FP<N>-PARTIAL  — FP<N> partially applied; follow-up issue required
```

Examples: `FP1`, `FP3-R`, `FP5-PARTIAL`

**Namespace rule:** Never use a bare `FN` (e.g. `F8`) for fork-originated changes. That namespace
is reserved for upstream Paperclip. When a fork change aligns with or supersedes an upstream flag,
note the upstream ID in the `Upstream Alignment` field.

---

## Required Fields

Every Paperclip issue, PR, and commit for a platform-level change **must** include an FP header.
Place it immediately after the issue description's `## Scope` section, or in the PR description.

```markdown
## Fork Platform Flag

| Field               | Value                                                     |
|---------------------|-----------------------------------------------------------|
| Flag ID             | FP<N>                                                     |
| Scope               | OS / subsystem affected (e.g. all, Windows, Linux, macOS)|
| Status              | applied / partial / rolled-back                          |
| Verification        | How to confirm the fix is in effect                      |
| Known Gaps          | Remaining work or edge cases NOT covered by this fix      |
| Upstream Alignment  | Upstream FN flag this relates to, or "none"               |
| Related Issues      | Links to other FP-series issues in this fix chain         |
```

**Known Gaps is mandatory.** If there are no known gaps, write `none` explicitly — never omit
the field. This prevents silent assumption that a fix is complete.

---

## What Counts as a Platform Change

File an FP-series identifier for any change that:

- Patches OS-specific behavior (file paths, shell compat, process management)
- Modifies execution lock or run lifecycle semantics
- Changes adapter startup, credential resolution, or config loading paths
- Fixes stale-state bugs in the control plane (locks, orphaned runs, stuck processes)
- Applies a Windows, macOS, or Linux compatibility shim

Do **not** file an FP identifier for:

- Feature additions that happen to work on all platforms
- Test-only changes
- Documentation updates (unless they patch incorrect platform guidance)
- Upstream merges that bring in upstream F-series flags (those keep their upstream ID)

---

## New Issue Checklist (Platform Changes)

Before filing or merging a platform-change issue:

- [ ] Assigned the next available FP identifier (check the registry below)
- [ ] Filled in all required fields (Flag ID, Scope, Status, Verification, Known Gaps, Upstream Alignment)
- [ ] Searched the FP registry to avoid re-solving a prior gap
- [ ] Searched upstream tracker (paperclipai/paperclip) for parallel implementations
- [ ] Known Gaps field is **not blank**

---

## FP Registry

The canonical registry lives in this file. Increment `N` and add a row when opening a new
platform-change issue. Do **not** reuse or skip numbers.

| FP ID        | Issue                         | Scope     | Status      | Summary                                    | Known Gaps                                               |
|--------------|-------------------------------|-----------|-------------|--------------------------------------------|----------------------------------------------------------|
| FP1          | [ANGA-140](/ANGA/issues/ANGA-140) / [ANGA-145](/ANGA/issues/ANGA-145) | all | applied | Initial stale execution-lock cleanup for terminated runs | Partial: process_lost terminations not covered → FP2 |
| FP2          | [ANGA-262](/ANGA/issues/ANGA-262) / [ANGA-263](/ANGA/issues/ANGA-263) | all | applied | Extend stale lock cleanup to cover process_lost run terminations | Partial: race window remained for permanently-locked runs → FP3 |
| FP3          | [ANGA-295](/ANGA/issues/ANGA-295) | all       | applied     | Systemic fix for permanently-locked runs via admin cleanup route | none — supersedes FP1 and FP2                           |
| FP4          | [ANGA-147](/ANGA/issues/ANGA-147) | Windows   | applied     | Fix Windows plugin manifest loading + decision-surface enablement | ANGA-163 (Vitest ESM loader on Windows) still open → FP5-PARTIAL |
| FP5-PARTIAL  | [ANGA-163](/ANGA/issues/ANGA-163) | Windows   | partial     | Fix Vitest ESM loader failure on Windows (drizzle-orm require cycle) | Backlog — not yet addressed                             |

**Next available: FP6**

---

## Backfill Notes

Issues ANGA-269, ANGA-282, and ANGA-317 were referenced in [ANGA-555](/ANGA/issues/ANGA-555)
as platform fix chain issues requiring FP backfill. Those identifiers do not correspond to active
issues in the current Paperclip instance — the fix chain is instead represented by:

- **FP1**: [ANGA-140](/ANGA/issues/ANGA-140) + [ANGA-145](/ANGA/issues/ANGA-145) — initial stale lock round
- **FP2**: [ANGA-262](/ANGA/issues/ANGA-262) + [ANGA-263](/ANGA/issues/ANGA-263) — second round, process_lost coverage
- **FP3**: [ANGA-295](/ANGA/issues/ANGA-295) — systemic / third round

Backfill comments have been posted to each of those issues to make the chain legible.

---

## Governance

- **Owner:** Engineering Manager reviews and approves new FP entries and convention changes.
- **Updates:** Edit this file in a PR; include the FP ID for the change in the PR description.
- **Audit:** Engineering Manager reviews registry completeness quarterly.
- **Cross-reference:** Upstream F-series flags are tracked in upstream Paperclip release notes.
  Align, don't duplicate: if upstream ships F15 that covers a fork gap, close the fork issue and
  note `superseded by upstream F15` in the registry.

---

## Related Documents

- [WORKFLOW-GOVERNANCE.md](../../workflow-project/doc/WORKFLOW-GOVERNANCE.md) — governance entry point
- [AGENT-GIT-WORKFLOW.md](../doc/AGENT-GIT-WORKFLOW.md) — branching and PR rules
- [ANGA-555](/ANGA/issues/ANGA-555) — issue that established this convention
