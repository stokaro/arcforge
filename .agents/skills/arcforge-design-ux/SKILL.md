---
name: arcforge-design-ux
description: "Design, implement, or review ArcForge interface and UX code: panes, toolbars, tables, selection, navigation, dialogs, themes, shortcuts, and native screenshots. Require BetterZip 6 look and feel with macOS as the primary visual target; use attributed MIT OpenZip only for unresolved details. Apply to visual-only changes and cross-platform capture/review."
---

# Implement the reference interface

Read [AGENTS.md](../../../AGENTS.md), the affected contracts, and the [reference evidence and parity inventory](../../../docs/reference/betterzip-6.md). Keep **BetterZip 6 look and feel mandatory, with macOS the primary visual target**. Preserve Windows/Linux support. Use OpenZip for unresolved details, not as an alternative theme.

## Resolve the design from evidence

1. Identify the action and states being changed: entry conditions, selection, focus, loading, success, failure, cancellation, and return path. Include empty and narrow-window states where affected.
2. Consult the latest user-supplied BetterZip 6 capture, then versioned official images/documentation or an available native reference. Do not let an older beta image override a clearer current capture. Distinguish observed pixels, documented behavior, source-derived behavior, and ArcForge adaptations. Images alone do not establish hover, shortcut, timing, or error behavior.
3. For a concrete unresolved detail, inspect the relevant OpenZip source at a fixed commit. Record the question, source path/revision, chosen behavior, and why it fills that gap. Preserve BetterZip's surrounding layout and command model. If both references are inconclusive, implement a small platform-appropriate decision, label it an adaptation, and retain the gap for later comparison.
4. Reuse MIT-covered OpenZip implementation or assets when useful. Check the specific file's origin and any separate terms; retain its copyright and complete license, and record destination, revision, and modifications in the reuse inventory. A repository MIT license does not relicense bundled helpers, fonts, system symbol artwork, or third-party files. Use BetterZip as a visual/behavior reference with original ArcForge code/assets. Consult [reuse rules](../../../docs/reference/openzip-reuse.md).

## Preserve the visual and interaction structure

Keep a compact desktop workspace: integrated filename/toolbar, rounded groups of line-icon actions, archive search at the upper right, filesystem Favorites at the left, central archive Outline, and right Info/Preview. Put navigation and breadcrumbs above the contents. Use actual native window controls; integrate the macOS titlebar without painting imitation traffic lights.

Compare proportions, type weights, spacing and states individually. The supplied wide-window scene has approximately **18% Favorites / 49% contents / 33% inspector**; use these as initial proportions, preserving resizing and smaller windows. Treat pixel measurements as belonging to their source image, not as known logical points when display scale is unknown. Use a genuine regular font and a genuine semibold face for headings, consistent readable icons, continuing full-height row stripes, restrained separators and full-row selection. Center the selected basename in the inspector, separate its path, align quiet labels against values, and give previews useful space. A thin fallback font, text-button toolbar or narrow debug-style inspector is an open fidelity defect.

Keep long names, Unicode, unknown metadata, scrolling and narrow layouts usable. Preserve identities and location across search/view changes; map Command to Ctrl where appropriate. Keep archive and Favorites filtering separate. Retain raw metadata under deliberate disclosure rather than dominating ordinary file inspection.

Build functional controls. Bind menus, toolbar, shortcuts, selection, and dialogs to the same command/state model. Keep blocking work off the UI thread and bound progress delivery. Preserve native paths separately from display strings. Prevent stale job results from replacing current state. Show actual counts and capabilities; do not fill the screen with invented folders, metadata, progress, or available actions.

For destructive or partial operations, explain destination, scope, overwrite policy, progress, cancellation and retained output. A completed job stays completed after a late cancel. Keep unsupported operations explicit. Recording an adaptation does not waive the look-and-feel requirement; leave unresolved differences open.

## Verify and deliver

Run relevant controller/core regressions for changed behavior; do not create implementation-mirroring tests for cosmetic changes. Launch the real native build and exercise the changed path with a redistributable fixture. Inspect focus, selection, text clipping, pane resizing, scrolling, disabled actions, and applicable failure/cancel states. A version flag or build pass is not a desktop check.

Follow [screenshot conventions](../../../screenshots/README.md): retain real, unaltered native captures for **macOS, Windows and Linux**, with source snapshot, fixture, reproducible scenes, OS/architecture, actual window/pixel dimensions, scale/theme and limits. Readiness must follow completed loading/selection/layout, not a guessed delay. Distinguish native framebuffer/client-area captures from OS window captures including chrome; neither a Linux image with a Mac frame nor a headless reconstruction proves macOS. If a runner cannot render/capture, retain the failure and mark that platform unverified.

Inspect every image against the primary reference. Report the actual result for toolbar/titlebar, typography, pane proportions, outline rows, inspector and preview separately. Treat successful build/startup checks, three-pane structure and functional extraction as insufficient visual acceptance. Do not declare macOS fidelity complete without an actual macOS image and remaining differences reviewed. Keep proprietary images external unless redistribution is specifically permitted.

Update the parity inventory and PR with the behavior changed, reference used, visible evidence, tests, and remaining gaps. Apply [verification](../arcforge-verify/SKILL.md) and one coherent independent review when required. Continue authorized work without adding approval stages.
