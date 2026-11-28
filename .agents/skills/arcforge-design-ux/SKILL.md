---
name: arcforge-design-ux
description: "Design, implement, or review ArcForge interface and interaction changes: panes, toolbars, tables, selection, navigation, dialogs, jobs, themes, shortcuts, and screenshots. Require BetterZip 6 look and feel; use MIT OpenZip only for details that BetterZip evidence does not resolve, with source attribution. Apply to visual-only changes as well as UX code."
---

# Implement the reference interface

Read [AGENTS.md](../../../AGENTS.md), the affected contracts, and the [reference evidence and parity inventory](../../../docs/reference/betterzip-6.md). Keep the current user requirement authoritative: **BetterZip 6 look and feel is mandatory**. Treat OpenZip as a fallback for unresolved details, not an alternative theme or a competing primary layout.

## Resolve the design from evidence

1. Identify the action and states being changed: entry conditions, selection, focus, loading, success, failure, cancellation, and return path. Include empty and narrow-window states where affected.
2. Consult versioned BetterZip 6 images, official documentation, or an available native reference. Distinguish observed pixels, documented behavior, source-derived behavior, and ArcForge adaptations. An image alone does not establish hover, shortcut, timing, or error behavior; older or beta evidence does not prove final-version details.
3. For a concrete unresolved detail, inspect the relevant OpenZip source at a fixed commit. Record the question, source path/revision, chosen behavior, and why it fills that gap. Preserve BetterZip's surrounding layout and command model. If both references are inconclusive, implement a small platform-appropriate decision, label it an adaptation, and retain the gap for later comparison.
4. Reuse MIT-covered OpenZip implementation or assets when useful. Check the specific file's origin and any separate terms; retain its copyright and complete license, and record destination, revision, and modifications in the reuse inventory. A repository MIT license does not relicense bundled helpers, fonts, system symbol artwork, or third-party files. Use BetterZip as a visual/behavior reference with original ArcForge code/assets. Consult [reuse rules](../../../docs/reference/openzip-reuse.md).

## Preserve the visual and interaction structure

Keep a compact desktop workspace: integrated filename/toolbar, grouped line-icon actions, archive search at the upper right, filesystem Favorites at the left, archive hierarchy in the central outline table, and Info/Preview at the right. Put navigation and breadcrumbs above the contents. Use native window controls on each OS. Do not substitute a left archive tree for Favorites or a flat table for Outline.

Use real disclosure behavior, restrained separators and alternating rows, blue full-row selection, readable metadata columns, and consistent spacing/type/color tokens. Derive dimensions from evidence and scale them for the tested viewport; do not invent exact reference measurements. Keep long names, Unicode, missing metadata, scrolling, and narrow layouts usable. Preserve identity and logical location across search and view changes; map Command shortcuts to Ctrl where appropriate. Separate archive search from Favorites search.

Build functional controls. Bind menus, toolbar, shortcuts, selection, and dialogs to the same command/state model. Keep blocking work off the UI thread and bound progress delivery. Preserve native paths separately from display strings. Prevent stale job results from replacing current state. Show actual counts and capabilities; do not fill the screen with invented folders, metadata, progress, or available actions.

For destructive or partial operations, make the destination, scope, overwrite policy, progress, cancellation state, and retained outputs understandable. A completed job stays completed after a late cancel request. Keep unsupported operations explicit. Track unfinished parity without claiming the slice completes the reference interface.

## Verify and deliver

Run relevant controller/core regressions for changed behavior; do not create implementation-mirroring tests for cosmetic changes. Launch the real native build and exercise the changed path with a redistributable fixture. Inspect focus, selection, text clipping, pane resizing, scrolling, disabled actions, and applicable failure/cancel states. A version flag or build pass is not a desktop check.

Follow [screenshot conventions](../../../screenshots/README.md): commit real, unaltered application captures under a topic directory, with numbered scenes and a README recording source snapshot, fixture, reproduction steps, environment, window/scale/theme, and limitations. Capture readiness from visible state, not a guessed delay. Inspect every image before claiming it as evidence. Keep proprietary reference images as links unless redistribution is specifically permitted.

Update the parity inventory and PR with the behavior changed, reference used, visible evidence, tests, and remaining gaps. Apply [verification](../arcforge-verify/SKILL.md) and one coherent independent review when required. Continue authorized work without adding approval stages.
