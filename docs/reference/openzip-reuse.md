# OpenZip fallback and reuse inventory

[OpenZip](https://github.com/burakgon/openzip-mac) is the approved secondary design and implementation reference when available BetterZip 6 evidence leaves a specific detail unclear. BetterZip 6 remains the primary look-and-feel target.

Inspect a fixed revision and the specific file's origin before reuse. Record the upstream path, local destination, revision, source license/copyright, and modifications. Preserve the complete license notice for copied or substantially adapted MIT source. Check any separate third-party terms for bundled helpers, dependencies, fonts, symbols, and images; the top-level MIT notice does not override them.

The source-inspection inventory and any actual code/asset adaptations are recorded below. Conceptual references do not imply that Swift/AppKit code or bundled binaries are present in ArcForge.

## Reviewed snapshot

Reviewed on **2026-11-28** at **`83f27d3fd2d753779797b60311c2986f8f034e04`**, commit subject *OpenZip 1.1.0*. The inspected remote default branch and local source matched that revision. Its original code is MIT, **Copyright (c) 2026 Burak Goncu**. The complete unchanged notice is retained in [licenses/openzip-MIT.txt](../../licenses/openzip-MIT.txt).

| Unresolved detail / reference | Pinned source | Application to ArcForge |
| --- | --- | --- |
| Empty startup, inspected before the owner supplied the missing BetterZip scene | [ArchiveWindow.swift, NoArchiveView](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/OpenZip/Browser/ArchiveWindow.swift) | OpenZip replaces the browser with a compact Open/drop surface. **Superseded as an ArcForge layout decision** by the owner's later BetterZip empty-document capture: retain places, central drop target and inspector. No Swift code or assets were copied. Normal Open still uses a real native file picker. |
| Extraction form contents when BetterZip's exact panel geometry is unavailable | [SessionDialogs.swift, ExtractSheet](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/OpenZip/Dialogs/SessionDialogs.swift) | Selection-aware title, explicit destination and Cancel/Extract actions. The current portable path form requires a new directory. Native picker, recent destinations, flattening and overwrite choices are not implied. |
| Compact progress and persistent failures | [ArchiveWindow.swift, operation status views](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/OpenZip/Browser/ArchiveWindow.swift) | Real progress, a stop/cancel action and a readable terminal result. Show actual bytes/items; do not invent ETA or pause support. |
| Avoid per-file UI update backlog | [AppModel.swift, ProgressSink](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/OpenZip/App/AppModel.swift) | Source reference for coalesced latest progress. ArcForge uses its existing bounded Rust worker and ticket-bound latest-state slot rather than porting Swift dispatch code. |
| Selection with selected parents/children | [ArchiveSession.swift](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/OpenZip/Browser/ArchiveSession.swift) | Avoid duplicate extraction when selecting both a folder and descendants. ArcForge retains stable central-directory IDs and archive-relative paths; OpenZip's basename rebasing is not imported. |
| Failure/cancel placement semantics | [ArchiveEngine+Extract.swift](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/Packages/OpenZipKit/Sources/OpenZipKit/Engine/ArchiveEngine+Extract.swift), [Placement.swift](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/Packages/OpenZipKit/Sources/OpenZipKit/Mac/Placement.swift) | Inspect actual sequential placement and partial-result behavior. ArcForge explicitly retains and reports partial output; it does not claim all-or-nothing extraction or hide cancellation. |

These are source-informed UX and architecture references. **No OpenZip application code, system symbol artwork, icon assets, bundled executables, or Swift dependencies are copied in this slice.** ArcForge's skill, egui UI, controller, and extraction implementation are original Rust/project work. Retaining the notice also makes the approved upstream and terms visible for later substantive adaptations.

## Separate terms

The [upstream README](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/README.md) and [7zz build script](https://github.com/burakgon/openzip-mac/blob/83f27d3fd2d753779797b60311c2986f8f034e04/scripts/build-7zz.sh) identify separate 7-Zip/LGPL/unRAR terms. The separately obtained RARLAB helper and Sparkle also have their own terms. They are not included by this UI reference work. Apple's `systemImage`/system-symbol names in Swift source are API references, not MIT-licensed image files; ArcForge draws its own line icons.
