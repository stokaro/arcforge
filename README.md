# ArcForge

A cross-platform archive-manager project written in Rust, targeting the archive-editing workflows of [BetterZip](https://macitbetter.com/) and the desktop interface of [OpenZip](https://github.com/burakgon/openzip-mac).

**Status: working Rust bootstrap; archive operations are not implemented yet.** The workspace contains a desktop shell, a headless CLI, a shared command catalog, architecture checks, and development packaging. The complete product is tracked in the [implementation epic](https://github.com/stokaro/arcforge/issues/1).

## Run and build

Install Rust and the native linker/SDK for your platform. The repository pins Rust **1.100.1**; rustup selects it automatically. Linux desktop builds also need the development libraries listed in [CI](.github/workflows/ci.yml).

```sh
cargo run --locked -p arcforge
cargo run --locked -p arcforge-cli -- commands --json
cargo run --locked -p arcforge-cli -- doctor --json
cargo xtask check
cargo xtask package
```

`cargo xtask check` runs formatting, dependency-boundary checks, Clippy, tests, and builds. Packaging creates an unsigned release-profile development bundle and its SHA-256 checksum in `dist/`. See [development.md](docs/development.md) for target selection, prerequisites, and verification limits.

## What works now

- A three-pane desktop shell: folder sidebar, file-table area, and information inspector, plus theme controls and command discovery.
- One command catalog shared by GUI and CLI, with explicit implemented/planned states.
- CLI help, version, command discovery, and JSON capability diagnostics.
- Rust build orchestration, enforced dependency boundaries, portable packaging, and a six-target native CI matrix.

The startup window is empty and archive actions are disabled. `doctor` reports compiled capabilities, including zero archive providers. A planned archive command returns an explicit error and does not open or modify files.

## Workspace

| Path | Responsibility |
| --- | --- |
| `crates/core` | Portable models, capabilities, command catalog, and shared behavior |
| `crates/ui-egui` | egui presentation over the shared core |
| `apps/arcforge` | Thin eframe desktop entry point |
| `apps/arcforge-cli` | Standalone CLI without GUI dependencies |
| `xtask` | Checks, builds, tests, and development packaging |

The core and CLI cannot depend on UI/windowing libraries. `cargo xtask layers` checks the resolved dependency graph, including transitive, development, build, and target-specific dependencies. New workspace crates must be registered in that checker. `cargo xtask notices` checks that dependency and font notices match their recorded inputs; the same gate runs before packaging. See [architecture.md](docs/architecture.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

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

## Platforms and distribution

| Platform | Architectures | Bootstrap bundle | Planned production distribution |
| --- | --- | --- | --- |
| macOS | Apple Silicon and Intel | ZIP with `.app` and CLI | App bundle and DMG |
| Windows | x86_64 and ARM64 | ZIP with executables | Installer and portable archive |
| Linux | x86_64 and ARM64 | Tarball with executables | AppImage, distribution packages, and tarball |

The [CI workflow](.github/workflows/ci.yml) builds, tests, and packages each target on a native runner. This matrix describes configured coverage; successful jobs are the evidence for a specific commit. A target triple alone does not provide cross-compilation SDKs or linkers. FreeBSD core/CLI support is secondary; browser/WASM support is a later portability option.

Development bundles are unsigned and are not production installers. A version tag runs the same checks and prepares a **draft** GitHub release; it does not automatically publish a release. Signing, notarization, installer integration, and upgrade tests remain product work.

## Implementation approach

Follow the engine-first approach demonstrated by [ArtCraft / PhotoCraft](https://github.com/storytold/photocraft) and the study/build/compare workflow described by [Craft Anything](https://github.com/storytold/craft-anything): document reference behavior, implement a UI-independent Rust command engine, build a thin Rust desktop UI, and verify both behavior and appearance against repeatable scenarios.

The bootstrap reuses PhotoCraft's Cargo-environment helper and adapts its dependency-layer checker under the MIT option of its dual license. Workspace organization, shared commands, and packaging conventions also draw on the ArtCraft ecosystem. [The reuse inventory](docs/artcraft-reuse.md) records exact upstream commits, files, modifications, and license notices. ArtCraft branding and application assets are not used.

egui/eframe is the initial GUI toolkit. Its native integration and the future archive providers must pass the capability checks in the epic; choosing the scaffold toolkit does not complete that evaluation. RAR creation is planned through an optional external provider.

## License

ArcForge's original source is licensed under the [MIT License](LICENSE). Adapted ArtCraft source retains its MIT notices. Rust dependencies and embedded assets retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/](licenses/). No OpenZip application source is included in the current bootstrap.
