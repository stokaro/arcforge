# Working on ArcForge

ArcForge provides ZIP/ZIP64 browsing, small-entry previews, and bounded extraction. Classic ZIP supports staged deletion with Undo, Redo and Save As to a new file. See the [ZIP contracts](docs/zip-browser.md) and [implementation epic](https://github.com/stokaro/arcforge/issues/1).

## Boundaries

- Keep models, capabilities, commands, and validation in `crates/core`, independent of UI/windowing libraries. Keep presentation in `crates/ui-egui`; it calls shared operations.
- Keep `apps/arcforge` and `apps/arcforge-cli` thin. The CLI must work without a display server or GUI dependencies.
- Keep the macOS AppKit shell in `crates/platform-macos`, independent of the archive core and egui. The desktop entry point translates its typed commands and capability state. Confine audited Objective-C interoperation to that crate's private macOS module; do not weaken the workspace-wide unsafe-code prohibition.
- Keep portable build orchestration in `xtask`. Register workspace crates/dependencies in its boundary checker; correct an accidental dependency direction instead of disabling the check.
- Keep capabilities truthful: planned, unsupported, and implemented operations are distinct. Preserve the source archive on failed/cancelled updates. Keep blocking I/O and archive processing off the UI thread.
- Use the pinned toolchain and `Cargo.lock`. Document native prerequisites. Record source adaptations with pinned paths, revisions, modifications, and full license notices in the relevant reuse inventory linked from `THIRD_PARTY_NOTICES.md`.
- Require **BetterZip 6 look and feel**, with **macOS as the primary visual target**: a familiar user should recognize the original application's appearance and interaction on first use. Use native platform file/destination panels and actual macOS toolbar/window behavior. Use OpenZip only to resolve details that BetterZip evidence leaves unclear; retain attribution and full notices for any MIT-covered source/asset adaptation. The [reference inventory](docs/reference/betterzip-6.md) records evidence, decisions, and remaining differences.

## Load only the relevant skills

Read the matching canonical `SKILL.md` when its task arises. Do not preload the entire catalog or restart completed workflow stages for a new message.

| Task | Skill |
| --- | --- |
| Diagnose a concrete failure or regression | [arcforge-debug](.agents/skills/arcforge-debug/SKILL.md) |
| Diagnose GitHub Actions checks or a failed native CI leg | [arcforge-ci](.agents/skills/arcforge-ci/SKILL.md) |
| Implement or review Rust ownership, APIs, errors, or performance | [arcforge-rust](.agents/skills/arcforge-rust/SKILL.md) |
| Design, implement, or review interface appearance, interaction, or UX code | [arcforge-design-ux](.agents/skills/arcforge-design-ux/SKILL.md) |
| Change observable behavior or a preservation guarantee | [arcforge-tdd](.agents/skills/arcforge-tdd/SKILL.md) |
| Implement/review archive providers, extraction, mutation, helpers, or placement | [arcforge-archive-safety](.agents/skills/arcforge-archive-safety/SKILL.md) |
| Assess a coherent code change or requested review | [arcforge-review](.agents/skills/arcforge-review/SKILL.md) |
| Verify implementation claims or prepare a code PR | [arcforge-verify](.agents/skills/arcforge-verify/SKILL.md) |

Several skills may apply at different stages of one task. Prose, formatting, and other changes without executable behavior need appropriate content checks, not a test-first ceremony. Maintain valid prior decisions and user authorization; skills do not introduce extra approval stages or grant permission for unrelated actions.

## Update the affected contract

| Change | Contract to inspect and update when behavior changes |
| --- | --- |
| Document state, background jobs, caches, or provider settings | [Snapshot and generation rules](docs/architecture.md#document-generations-and-background-results) |
| Archive offsets, sizes, streaming, or allocation | [Large-file boundaries](docs/architecture.md#large-files-and-checked-offsets) |
| Target coverage, bundle layout, or release evidence | [Release manifest and verification](docs/release.md) |
| Agent workflow or borrowed source | [Skill maintenance](docs/agent-skills.md) and the relevant source reuse inventory |

Keep requirements proportional to the change. A contract for a future provider does not imply the provider is implemented. When filtering diagnostic output, preserve the raw log and the original command's exit status; a successful filter does not make a failed command pass.

## Verification and pull requests

Use targeted tests during iteration. Add meaningful regression coverage for changed archive behavior, command contracts, and data-preservation guarantees with isolated temporary fixtures. Before a code PR, run `cargo xtask check` for the final relevant code state. For documentation/skill-text changes, validate content, links, `cargo xtask skills`, and `cargo xtask notices` when license inputs change. See [development commands](docs/development.md).

Record the verified snapshot, exact commands, results, and platform. Reuse evidence while its relevant inputs remain unchanged; rerun affected checks after a change invalidates it. Report failed/skipped gates honestly. Compilation and startup flags do not prove native desktop behavior.

For visible interface changes, commit actual application screenshots under `screenshots/<topic>/`, following the [screenshot evidence conventions](screenshots/README.md). Cover macOS, Windows and Linux; record a concrete failed/blocked capture as unverified, not as a pass. Include reproducible scenes, fixture, source snapshot, and OS/window/scale/theme details. Distinguish client-area frames from captures including native chrome. Compare typography, toolbar, proportions, rows and inspector against BetterZip 6 separately; three panes and a passing build do not establish visual parity.

For a substantial code change, request one independent review when available; otherwise identify the self-review. Follow up on concrete findings rather than repeating broad reviews indefinitely. PRs explain the problem, resulting behavior, verification, and limitations; include a compact Mermaid diagram when a flow changes. Update epic checkboxes only for completed work.

Edit canonical skills under `.agents/skills`, then run `cargo xtask skills-sync` to update Claude entry points. [Skill maintenance and provenance](docs/agent-skills.md) records the scoped adaptations and reference licenses.
