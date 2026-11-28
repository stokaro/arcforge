# BetterZip 6 design and interaction baseline

## Requirement and source priority

The project owner's requirement, clarified on **2026-11-28**, is that ArcForge's desktop **look and feel must follow BetterZip 6**, with **macOS the primary visual target** and actual screenshots required on macOS, Windows and Linux. If its behavior or appearance is unclear from available BetterZip evidence, use [OpenZip](https://github.com/burakgon/openzip-mac) to resolve that specific detail. Suitable MIT-covered OpenZip code and assets may be reused with full attribution. This supersedes the original OpenZip-first layout description in issue #1.

Use the [arcforge-design-ux skill](../../.agents/skills/arcforge-design-ux/SKILL.md) for design, UX code, visual changes, and review. Keep the interface in Rust and retain the shared-core and portable-build contracts.

The owner's acceptance criterion is **faithful recreation**: a person familiar with BetterZip 6 should recognize the original application's appearance and interaction on first use. The ArtCraft reference concerns this fidelity goal as well as repository/build organization. A loosely inspired layout, a developer demonstration, or a list of documented differences does not meet it. Evaluate the ordinary first-use flow: launch, Open, archive navigation, selection, inspector/preview, menus and extraction panels.

Evidence has four distinct meanings:

| Label | What it establishes |
| --- | --- |
| Observed image | Visible layout and state in that image, at its displayed version and scale |
| Official documentation | Described behavior; not a measured dialog layout or live execution |
| OpenZip source | Behavior implemented by the pinned source, including its platform assumptions |
| ArcForge adaptation | An explicit choice for current capability or platform constraints; an outstanding difference where appropriate |

The native BetterZip and OpenZip applications have not been run for this baseline. The official image and the owner-supplied images inventoried below were visually inspected. ArcForge captures are separate evidence from actual ArcForge builds. The owner's later captures take priority where they show a clearer version-6 appearance than the earlier beta image.

## Primary evidence

Reviewed 2026-11-28:

| Source | Observations and limits |
| --- | --- |
| Owner-supplied `image(20261128-090848).png`, received 2026-11-28 | 2048 × 1271 pixels; `Vacation Photos.zip` workspace, expanded folders and a JPEG selected. Strongest supplied visual target: unified titlebar, rounded action groups, regular/semibold type, wide inspector, full-height striping and large image preview. OS version, native logical size and display scale were not supplied. SHA-256 `b139977134f33949a29011b68dbe40343fc486dabe7a03e6a83ed7ebfce83990`. The proprietary reference image is not redistributed in this public repository. |
| [Official main-window image](https://macitbetter.com/img/library/BetterZip-Main-Window@2x.png) | 2540 × 1644 source image; title shows `BetterZip-6.0b3.zip`. Useful version-6 visual evidence, explicitly a beta-labelled scene rather than a capture of the final application. |
| [Main-window guide](https://macitbetter.com/library/betterzip/docs/main-window/) | Favorites, archive contents, selected-file info/preview, toolbar menus, separate archive/Favorites filtering, navigation bar, hashes. |
| [View modes](https://macitbetter.com/library/betterzip/docs/view-modes/) | Outline is the default; Flat, Columns, and Icons are distinct modes. Folders expand in place in Outline. |
| [Extraction](https://macitbetter.com/library/betterzip/docs/extract/) | Whole-archive and selected-item extraction workflows. |
| [Extract options](https://macitbetter.com/library/betterzip/docs/extract-options/) | Destination is chosen through a standard macOS panel with extraction options. Presets, filters, flattening, and post-processing are separate capabilities. |
| [Favorites](https://macitbetter.com/library/betterzip/docs/favorites/) | Favorites represent filesystem locations and their archives, rather than the current archive's internal directory tree. |
| [Keyboard shortcuts](https://macitbetter.com/library/betterzip/docs/keyboard-shortcuts/) | Primary actions, pane toggles, search, and view shortcuts. Use the corresponding Ctrl conventions on Windows/Linux. |
| [What's new in BetterZip 6](https://macitbetter.com/library/betterzip/articles/whats-new-in-betterzip-6/) | Version context and changes; do not silently infer that an older screen documents every version-6 state. |

These are references for independently implemented appearance and behavior. BetterZip images, logos, fonts, or application code are not incorporated into ArcForge.

### Additional owner-supplied scenes

The owner supplied the following further images on 2026-11-28. Their display scale and exact application/OS versions were not supplied. They remain external reference material; no proprietary image or application asset is copied into this repository.

| Image | Pixels | Observed state / implementation consequence | SHA-256 |
| --- | --- | --- | --- |
| `image(20261128-100559).png` | 2048 × 1325 | `BetterZip-6.0b3.zip` workspace, expanded filesystem Favorites, narrow scrollable Info panel and icon preview. Shows sidebar counts and archive children; does not license copying its icon artwork. | `4cbc5b801bda785bd356bd997936cc850e6a44f023c5715d51fed06facadd64a` |
| `image(20261128-100609).png` | 1926 × 1300 | Filtering settings with a modal Edit Filter sheet: All/Any group, field/operator/value rows, add/remove controls, filter name and Cancel/OK. Keep this as the target for the planned filter editor, not as evidence that filtering is implemented. | `74ce7dc3cfc4bda2afbe81b687678697c979b2d54176b074c66b3f1a2d055ec6` |
| `image(20261128-100624).png` | 1024 × 716 | Empty Untitled document: places at left, central dashed drop target, No files selected inspector. The chrome is visibly from an older macOS style; use its observed empty-document structure, while retaining the current v6 shell reference. This supersedes the proposed OpenZip pane-free empty layout. | `0b2bd817d641047ef2b20ebfed9ff2226fe5c5131cdd0fbd8f1d03a62d17d635` |
| `image(20261128-100632).png` | 2047 × 1301 | General Settings with tabs General, Presets, Helpers, Filtering, Archive Types, Files, Passwords, Places, Extensions, Advanced. Startup, Finder-opening, metadata and conflict settings are visible; functionality remains planned where unsupported. | `655d65716c35fcaa2f257bbc42ad84b82201f38e340af4ed1b9dbb2a7f2b8fb5` |
| `image(20261128-100656).png` | 2047 × 1301 | Native Extract menu with Downloads/Documents/Desktop, Other Folder, preset section and Edit Presets; toolbar icon-and-label mode. Resolves action order as Extract, Save, Test, Move, Add, View, Delete, New Folder, Collect, Direct Mode, Share, Search. | `55f48e9485706f93eb7ffbb03aaff2e7dbd3f278995a02f7f1a7b4d4ad551f81` |
| `image(20261128-102424).png` | 2048 × 1309 | A compact New password sheet over a dimmed archive workspace, with masked Password and Verify fields, focus on Verify, help and lock-shaped controls, Cancel and a blue default OK button. This supplies a future password-confirmation layout, not evidence of an implemented ArcForge password/encryption workflow. | `6e2ccd7dfe4d7dde6e54ba605646eac94ee4426dc5618aed39b83d10568d419b` |
| `image(20261128-102430).png` | 2032 × 1170 | A Finder `Docs` window with four PNG files selected and an external Compress with Preset toolbar menu. Named compression presets are followed by Add Items to New Archive in BetterZip and Start collecting items in a new archive. This is Finder integration evidence, not the BetterZip archive-window toolbar or a view mode inside ArcForge. | `f64b40471a23ee273d87250dec41f170e1202b9809646711545818114298ac0a` |
| `image(20261128-102439).png` | 2047 × 1688 | Settings → Presets with the Save segment selected: named presets at left, add/remove/more controls, and an options form at right. Visible fields include default/name, destination, archive format, encryption, compression, split size, filename/comment, filters, scripts, renaming, source handling and completion choices. Save/extract presets and these options remain planned. | `13b96e204bc5cc91d074b40d626ec84aea8ced12be8c50a0018f7a96bbe48647` |

Normal Open belongs in the File menu, its platform shortcut, and the empty-document action; it must not replace the reference's first Extract toolbar action. Keep reference actions in their observed groups/order. Unsupported operations remain disabled with an honest explanation. Native Extract destinations must preserve the currently supported create-new-directory contract; a familiar menu is not permission to introduce silent overwrite or source deletion.

The password scene does not establish the meaning of either lock-shaped control, password storage, a generation/reveal action, encryption algorithms, validation timing or failure behavior. The Presets image does not establish the star/more-control semantics, preset serialization, default resolution, or effects of script/source-handling choices. The Finder image establishes the visible external menu and selected-file context; it does not establish the extension API, launch/queue behavior, or error handling. Resolve those questions from further reference evidence before claiming parity. Passwords, encryption, presets, archive creation and Finder integration are not implemented by the current browsing/extraction slice.

## Correction after the owner's comparison

The initial Linux workspace capture in [ZIP extraction](../../screenshots/zip-extraction/README.md) demonstrates working browsing and extraction, but it does **not** satisfy the required BetterZip 6 appearance. The owner's side-by-side comparison exposed material differences that a broad three-pane review had accepted too readily. Treat the following as concrete acceptance dimensions, not optional polish:

| Dimension | Observation from supplied BetterZip image | Acceptance direction |
| --- | --- | --- |
| Native shell | Window controls and title/action area form one macOS workspace | Use native macOS controls in a unified titlebar; verify the actual Mac build. Keep platform controls on Windows/Linux. |
| Toolbar | Distinct rounded groups of line icons; filename has its own hierarchy | Group actual commands into rounded icon controls with accessible names/tooltips; avoid boxed text menus as the primary visual language. |
| Typography | Regular-weight rows and values, clearly heavier filename/selected basename | Use genuine regular/semibold faces; verify fallback fonts and non-ASCII text. A color change does not create font weight. |
| Pane proportions | Roughly 18% Favorites, 49% contents, 33% inspector within the visible window | Start wide-window defaults near these proportions, keeping pane resizing and smaller-window behavior. Ratios are estimates from raster boundaries, not measured native point values. |
| Outline | Recognizable colored file/folder icons; alternating rows continue below the last file | Preserve disclosure/selection/navigation behavior and continue row striping through the empty table area. |
| Inspector | Centered selected basename; path separately; gray right-aligned labels and readable values | Give ordinary file inspection visual priority. Put ZIP encoding/raw-byte diagnostics under disclosure. |
| Preview | Image uses substantial inspector width and lower-pane space | Render supported real content with bounded background decoding and an aspect-preserving image view. Keep unsupported/oversized content explicit. |
| Evidence | Owner is targeting macOS | Capture the same fixture/state on real macOS, Windows and Linux. Label client-only frames and native-chrome captures precisely; compare each dimension and leave differences open. |

Matching a few panel positions or recording a difference as an adaptation does not close it. Functional tests and successful native bundles are separate evidence from visual acceptance. Native material effects, system menus/pickers and incomplete capabilities must remain explicit gaps until implemented and exercised.

### Rejected native Mac iteration

The owner subsequently rejected ArcForge's empty-window/Open scene in `Screenshot 2026-11-28 at 10.56.59.png` (2048 × 1276 pixels; SHA-256 `7871f826e82e842e3a7780abbea69684225b65e81d2be9971c9428e3f29bc652`). The supplied scene visibly combines real macOS window controls at a different vertical position from the painted toolbar, a handmade `Open ZIP` path-entry window, and largely empty workspace panels. Its exact application commit, OS version and scale were not supplied. This is failed visual acceptance, not evidence that the revised appearance is sufficient.

Normal archive opening must use the native platform file picker. Manual path entry may remain an explicit advanced fallback, not the first user-facing workflow. macOS titlebar controls, toolbar placement/materials and window interaction must be evaluated as one native shell. The renderer choice is an implementation detail and cannot waive that requirement; introduce an appropriate native presentation boundary when necessary while preserving the shared Rust archive core. Include actual empty startup and Open-panel captures alongside loaded text/image scenes. Compare equivalent window sizes and retain constrained-runner scenes as separate narrow-window evidence.

## Workspace contract

Preserve the arrangement observed in the main-window image:

- Use a quiet, compact title/toolbar area with the archive filename, rounded groups of line-icon actions, and archive search at the upper right. Keep native OS window controls rather than drawing macOS controls on every platform.
- Put filesystem Favorites/archives in the left pane. Display the archive's expandable hierarchy in the central contents view. A flat listing and a left folder tree do not together satisfy Outline.
- Place back/forward and the current location above the contents table. Keep Info and selected-content Preview in the right pane.
- Use readable, closely spaced rows, restrained alternating backgrounds and separators, folder/file indicators, disclosure triangles, and blue full-row selection. Support modifier selection, range selection, and Select All.
- The observed visible table columns are **Name, Modified, Size, Kind, Ratio**. Other available metadata belongs in configurable columns or the inspector. Missing data is unknown; it must not become a fabricated date or zero.
- Keep toolbar placement and density comparable at an equivalent logical viewport. Exact icon-to-action mappings or dimensions that cannot be established from the image remain unresolved, rather than being asserted as measured facts.

Use shared styling tokens and native scale factors. Evaluate real text, long paths, non-ASCII names, smaller windows, and light/dark appearances. Keep archive search distinct from Favorites filtering. Treat fake folder counts, disabled controls that look enabled, and progress without a running job as fidelity defects.

## State and parity inventory

This table records intended behavior and the boundaries of the current extraction slice. A target is not a claim that its acceptance criteria are complete. The native shell, pickers, filesystem Favorites and revised empty document described below are wired in the current code; fresh native runtime captures and interaction verification of that revision are still pending. Earlier captures do not verify these replacements.

| State/action | Reference | ArcForge target / acceptance evidence | Remaining scope |
| --- | --- | --- | --- |
| Main archive workspace | Owner's image + main-window guide | macOS now attaches an AppKit `NSToolbar` and `NSMenu`, unified native titlebar, grouped system-symbol actions and native archive search; Windows/Linux retain the portable toolbar. Genuine font weights and approximately 18/49/33 default panes remain the target | Actual native shell/material/spacing comparison is pending; unsupported toolbar actions, customization and further platform integration remain open |
| Open archive | Main-window guide; rejected handmade Open scene | Normal Open uses a parented native file picker, shared by File/Open, the platform shortcut and the empty-document action; manual path entry is an explicit secondary action | Native picker presentation, focus, cancellation and platform behavior still need fresh runtime evidence |
| Outline / Flat | View-mode documentation | Real expandable hierarchy and full-list mode; preserve stable entry identities and selection | Columns and Icons, persistent preferences |
| Favorites | Main-window/Favorites documentation; owner's expanded filesystem scene | Actual existing Desktop/Documents/Downloads locations, with an existing Home fallback; independent bounded worker, immediate folder listings, disclosures, native paths, refresh and separate Favorites filtering. ZIP-candidate counts appear only for complete error-free scans | Fresh native interaction/capture verification, persistent/custom places and filesystem change watching; a ZIP extension is not a validated archive |
| Navigation and selection | Image; documented view/shortcut model | Breadcrumbs above table, modifier/range selection, Select All; exercise with nested fixture | Full native drag/drop and platform keyboard coverage |
| Info / Preview | Owner's image + main-window documentation | Centered basename, separate path and aligned metadata; bounded text/hex and small PNG/JPEG previews; explicit unknown/unsupported states | Images beyond the 256 KiB preview cap, PDF, color management, external editors and further hashes |
| Extract All / Selected | Extraction documentation | Explicit scope and new destination; shared core/CLI behavior and real completion/error scenes | Existing-directory conflict policies, presets, filters, metadata restoration |
| Destination dialog | Standard macOS panel described by BetterZip; OpenZip source fallback | A parented native folder picker chooses the existing parent, followed by a compact scope/new-folder review; extraction still creates a new destination and never overwrites an existing directory | Native runtime verification and a fully native extraction-options accessory remain open; the compact review is an explicit current adaptation |
| Running / cancelled / failed extraction | Documented operations; shared-core safety contract | Actual byte/entry progress, cancellable worker, preserved partial-output report | ETA, independent queued jobs and longer-job cancellation confirmation |
| Empty document | Owner's `image(20261128-100624).png`, with the current v6 shell reference | Three panes remain present: actual filesystem places, a central dashed archive drop target with native Open action, and the No files selected inspector | Fresh empty-window and real Open-panel captures on native platforms; older reference chrome is not the target |
| Loading / filtered / error | Reference evidence per state, OpenZip where unclear | Clear status, stable selection and focus, honest action availability | Expanded visual inventory across all OS families |
| Save / dirty / conflicts | BetterZip staged-document model | Remain visibly unavailable until implemented | Editing, undo, Save/Save As and external-editor round trips |
| Password / encryption | Owner's New password sheet | Reference inventory only; no password or encryption support is advertised | Provider support, password handling and matching native confirmation/error states are planned |
| Save / extract presets | Owner's Settings → Presets and Extract menu | Reference inventory and disabled planned actions only | Preset model, editor, defaults and supported options are planned; visible names/options do not imply implementation |
| Finder compression menu | Owner's Finder `Docs` scene | External integration reference; not part of the archive workspace implementation | Finder extension, selected-file dispatch, compression presets and creation/collection workflows are planned |

Native ArcForge evidence is indexed in [screenshots](../../screenshots/README.md). Build/CLI smoke results cover their stated targets only; they cannot establish native desktop parity on other operating systems.

The [ZIP extraction scenes](../../screenshots/zip-extraction/README.md) record five actual Linux captures: workspace, selected destination, completion, existing-destination rejection, and cancellation with retained output. Native interaction and an independent file-content check accompany the images. They preserve extraction behavior evidence for their recorded source snapshots. Their text toolbar menus, thin typography and narrow inspector are known visual defects identified by the owner; the captures are not a full-parity sign-off or the target appearance for future work.

## Filling a reference gap

Record the unresolved question before selecting a fallback. Inspect the relevant file in the pinned OpenZip tree; record the actual behavior and the piece being adapted. Prefer the narrowest reusable solution that preserves BetterZip's surrounding layout and staged editing model. In particular, do not import immediate destructive editing merely because the fallback application uses it.

Maintain exact source paths, revision, modifications and required notices in [OpenZip reuse](openzip-reuse.md). When neither reference answers a question, label the choice as an ArcForge adaptation and keep it in the parity backlog. No additional approval stage is required for ordinary implementation choices already authorized by the project owner.
