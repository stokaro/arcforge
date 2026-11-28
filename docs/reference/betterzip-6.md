# BetterZip 6 design and interaction baseline

## Requirement and source priority

The project owner's requirement, clarified on **2026-11-28**, is that ArcForge's desktop **look and feel must follow BetterZip 6**. If its behavior or appearance is unclear from available BetterZip evidence, use [OpenZip](https://github.com/burakgon/openzip-mac) to resolve that specific detail. Suitable MIT-covered OpenZip code and assets may be reused with full attribution. This supersedes the original OpenZip-first layout description in issue #1.

Use the [arcforge-design-ux skill](../../.agents/skills/arcforge-design-ux/SKILL.md) for design, UX code, visual changes, and review. Keep the interface in Rust and retain the shared-core and portable-build contracts.

Evidence has four distinct meanings:

| Label | What it establishes |
| --- | --- |
| Observed image | Visible layout and state in that image, at its displayed version and scale |
| Official documentation | Described behavior; not a measured dialog layout or live execution |
| OpenZip source | Behavior implemented by the pinned source, including its platform assumptions |
| ArcForge adaptation | An explicit choice for current capability or platform constraints; an outstanding difference where appropriate |

The native BetterZip and OpenZip applications have not been run for this baseline. The official image was visually inspected. ArcForge captures are separate evidence from an actual ArcForge build.

## Primary evidence

Reviewed 2026-11-28:

| Source | Observations and limits |
| --- | --- |
| [Official main-window image](https://macitbetter.com/img/library/BetterZip-Main-Window@2x.png) | 2540 × 1644 source image; title shows `BetterZip-6.0b3.zip`. Useful version-6 visual evidence, explicitly a beta-labelled scene rather than a capture of the final application. |
| [Main-window guide](https://macitbetter.com/library/betterzip/docs/main-window/) | Favorites, archive contents, selected-file info/preview, toolbar menus, separate archive/Favorites filtering, navigation bar, hashes. |
| [View modes](https://macitbetter.com/library/betterzip/docs/view-modes/) | Outline is the default; Flat, Columns, and Icons are distinct modes. Folders expand in place in Outline. |
| [Extraction](https://macitbetter.com/library/betterzip/docs/extract/) | Whole-archive and selected-item extraction workflows. |
| [Extract options](https://macitbetter.com/library/betterzip/docs/extract-options/) | Destination is chosen through a standard macOS panel with extraction options. Presets, filters, flattening, and post-processing are separate capabilities. |
| [Favorites](https://macitbetter.com/library/betterzip/docs/favorites/) | Favorites represent filesystem locations and their archives, rather than the current archive's internal directory tree. |
| [Keyboard shortcuts](https://macitbetter.com/library/betterzip/docs/keyboard-shortcuts/) | Primary actions, pane toggles, search, and view shortcuts. Use the corresponding Ctrl conventions on Windows/Linux. |
| [What's new in BetterZip 6](https://macitbetter.com/library/betterzip/articles/whats-new-in-betterzip-6/) | Version context and changes; do not silently infer that an older screen documents every version-6 state. |

These are references for independently implemented appearance and behavior. BetterZip images, logos, fonts, or application code are not incorporated into ArcForge.

## Workspace contract

Preserve the arrangement observed in the main-window image:

- Use a quiet, compact title/toolbar area with the archive filename, groups of line-icon actions, and archive search at the upper right. Keep native OS window controls rather than drawing macOS controls on every platform.
- Put filesystem Favorites/archives in the left pane. Display the archive's expandable hierarchy in the central contents view. A flat listing and a left folder tree do not together satisfy Outline.
- Place back/forward and the current location above the contents table. Keep Info and selected-content Preview in the right pane.
- Use readable, closely spaced rows, restrained alternating backgrounds and separators, folder/file indicators, disclosure triangles, and blue full-row selection. Support modifier selection, range selection, and Select All.
- The observed visible table columns are **Name, Modified, Size, Kind, Ratio**. Other available metadata belongs in configurable columns or the inspector. Missing data is unknown; it must not become a fabricated date or zero.
- Keep toolbar placement and density comparable at an equivalent logical viewport. Exact icon-to-action mappings or dimensions that cannot be established from the image remain unresolved, rather than being asserted as measured facts.

Use shared styling tokens and native scale factors. Evaluate real text, long paths, non-ASCII names, smaller windows, and light/dark appearances. Keep archive search distinct from Favorites filtering. Treat fake folder counts, disabled controls that look enabled, and progress without a running job as fidelity defects.

## State and parity inventory

This table records intended behavior and the boundaries of the current extraction slice. A target is not a claim that its acceptance criteria are complete.

| State/action | Reference | ArcForge target / acceptance evidence | Remaining scope |
| --- | --- | --- | --- |
| Main archive workspace | Observed image + main-window guide | Filename/toolbar, Favorites, central contents, right Info/Preview; native screenshots | Exact geometry and all toolbar secondary actions require further comparison |
| Outline / Flat | View-mode documentation | Real expandable hierarchy and full-list mode; preserve stable entry identities and selection | Columns and Icons, persistent preferences |
| Favorites | Main-window/Favorites documentation | Only real opened archive paths in the initial session list | Filesystem folder browsing, persistent favorites, badges and dedicated Favorites search |
| Navigation and selection | Image; documented view/shortcut model | Breadcrumbs above table, modifier/range selection, Select All; exercise with nested fixture | Full native drag/drop and platform keyboard coverage |
| Info / Preview | Main-window documentation | Real selected-entry metadata, bounded text/hex preview, visible unknown/unsupported states | Image/PDF previews, external editors, further hashes |
| Extract All / Selected | Extraction documentation | Explicit scope and new destination; shared core/CLI behavior and real completion/error scenes | Existing-directory conflict policies, presets, filters, metadata restoration |
| Destination dialog | Standard macOS panel described by BetterZip; OpenZip source fallback | Current new-directory form is an explicit portable adaptation | Native platform picker/accessory integration remains a fidelity gap |
| Running / cancelled / failed extraction | Documented operations; shared-core safety contract | Actual byte/entry progress, cancellable worker, preserved partial-output report | ETA, independent queued jobs and longer-job cancellation confirmation |
| Empty / loading / filtered / error | Reference evidence per state, OpenZip where unclear | Clear status, stable selection and focus, honest action availability | Expanded visual inventory across all OS families |
| Save / dirty / conflicts | BetterZip staged-document model | Remain visibly unavailable until implemented | Editing, undo, Save/Save As and external-editor round trips |

Native ArcForge evidence is indexed in [screenshots](../../screenshots/README.md). Build/CLI smoke results cover their stated targets only; they cannot establish native desktop parity on other operating systems.

The [ZIP extraction scenes](../../screenshots/zip-extraction/README.md) record five actual Linux captures: workspace, selected destination, completion, existing-destination rejection, and cancellation with retained output. Native interaction and an independent file-content check accompany the images. The captures also expose current adaptations, including text toolbar menus, the additional status footer and the portable destination form; they are not a full-parity sign-off.

## Filling a reference gap

Record the unresolved question before selecting a fallback. Inspect the relevant file in the pinned OpenZip tree; record the actual behavior and the piece being adapted. Prefer the narrowest reusable solution that preserves BetterZip's surrounding layout and staged editing model. In particular, do not import immediate destructive editing merely because the fallback application uses it.

Maintain exact source paths, revision, modifications and required notices in [OpenZip reuse](openzip-reuse.md). When neither reference answers a question, label the choice as an ArcForge adaptation and keep it in the parity backlog. No additional approval stage is required for ordinary implementation choices already authorized by the project owner.
