# ArcForge

A planned cross-platform archive manager written in Rust, with the archive-editing workflows of [BetterZip](https://macitbetter.com/) and the desktop interface of [OpenZip](https://github.com/burakgon/openzip-mac).

**Status: specification and repository bootstrap.** Application implementation and release builds are tracked in the [implementation epic](https://github.com/stokaro/arcforge/issues/1).

## Product goal

Browse an archive like a folder, preview its contents, extract selected items, edit files in another application, and save changes back into the archive. The design targets close visual and behavioral fidelity to the reference applications: a folder sidebar, detailed file table, preview and information inspector, familiar toolbar actions, and complete desktop keyboard and drag-and-drop workflows.

## Planned capabilities

- Archive browsing, searching, four view modes, previews, and multiple windows.
- Add, remove, rename, move, and replace entries; staged edits, undo/redo, Save and Save As.
- External-editor round trips and explicit handling of nested archive changes.
- ZIP/ZIP64, 7z, TAR and compressed streams, plus verified providers for RAR and additional formats.
- Encryption, split-volume handling, presets, filters, conflict resolution, batch jobs, and progress/cancellation.
- A standalone CLI using the same Rust commands as the desktop application.
- Platform integrations and automated build, package, and release workflows.

The epic defines the format-by-format capability matrix, implementation phases, and acceptance criteria. Planned capabilities are not claims of existing implementation.

## Platform plan

| Platform | Architectures | Planned distribution |
| --- | --- | --- |
| macOS | Apple Silicon and Intel | App bundle and DMG |
| Windows | x86_64 and ARM64 | Installer and portable archive |
| Linux | x86_64 and ARM64 | AppImage, distribution packages, and tarball |

The core must remain portable. FreeBSD is a secondary target; browser/WASM support is a later portability option. A shared build interface and native CI jobs will produce and test each supported release artifact.

## Implementation approach

Follow the engine-first approach demonstrated by [ArtCraft / PhotoCraft](https://github.com/storytold/photocraft) and the study/build/compare workflow described by [Craft Anything](https://github.com/storytold/craft-anything): document reference behavior, implement a UI-independent Rust command engine, build a thin Rust desktop UI, and verify both behavior and appearance against repeatable scenarios.

The GUI toolkit and archive providers must pass the cross-platform capability checks in the epic. Dependencies and optional external archive helpers retain their own licenses. RAR creation is planned through an optional external provider.

## License

ArcForge's original source is licensed under the [MIT License](LICENSE). Any adapted OpenZip source must retain its MIT attribution. Bundled third-party components must include their applicable notices.
