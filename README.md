# ArcForge

A cross-platform archive-manager project written in Rust, with **BetterZip 6 look and feel as a mandatory target**. [OpenZip](https://github.com/burakgon/openzip-mac) supplies fallback UX/implementation references where BetterZip evidence is unclear; MIT-covered reuse retains attribution.

**Status: ZIP/ZIP64 browsing, bounded previews, extraction, and staged deletion for classic ZIP.** The desktop supports Undo, Redo and Save As. The desktop and headless CLI share filename interpretation, streaming extraction, and the verified ZIP copy writer. See the [ZIP contracts](docs/zip-browser.md) and [implementation epic](https://github.com/stokaro/arcforge/issues/1).

## Run and build

Install Rust and the native linker/SDK for your platform. The repository pins Rust **1.100.1**; rustup selects it automatically. Linux desktop builds also need the development libraries listed in [CI](.github/workflows/ci.yml).

```sh
cargo run --locked -p arcforge
cargo run --locked -p arcforge -- example.zip
cargo run --locked -p arcforge-cli -- commands --json
cargo run --locked -p arcforge-cli -- doctor --json
cargo run --locked -p arcforge-cli -- list example.zip --json
cargo run --locked -p arcforge-cli -- preview example.zip --entry 0 --json
cargo run --locked -p arcforge-cli -- extract example.zip --output new-output --json
cargo run --locked -p arcforge-cli -- delete example.zip --entry 0 --output edited.zip --json
cargo xtask check
cargo xtask package
```

`cargo xtask check` runs formatting, dependency-boundary checks, Clippy, tests, and builds. Packaging creates an unsigned release-profile development bundle and its SHA-256 checksum in `dist/`. See [development.md](docs/development.md) for target selection, prerequisites, and verification limits.

## What works now

- ZIP/ZIP64 metadata, stable entry IDs within a snapshot, and original filename/extra-field/comment bytes.
- A BetterZip 6-oriented workspace with filesystem Favorites, a virtualized Outline/Flat contents table, selection, search, and Info/Preview. Open one archive through a native picker, startup argument, or file drop. [Reference decisions and remaining gaps](docs/reference/betterzip-6.md) and [actual screenshots](screenshots/README.md) record the current fidelity.
- Bounded text/hex previews of complete Stored/Deflate entries up to 256 KiB, with CRC32 verification. Archive reads, filename reinterpretation, search preparation, and previews run on one cancellable worker outside the UI thread.
- Selected/all Stored/Deflate extraction into a new directory, with portable name/collision checks, bounded streaming, CRC verification, cancellation, and an explicit report of completed and partial output. Existing destinations are never merged or overwritten.
- Delete selected files or folders in classic ZIP, undo or redo staged changes, and use Save As to create a verified ZIP at a new filename.
- Filename evidence and manual archive/per-entry interpretation controls, including explicit recovery. Reinterpretation preserves source bytes and entry identity.
- One command catalog shared by GUI and CLI, with explicit implemented/planned states.
- CLI help, version, command discovery, capability diagnostics, `list`, `info`, `preview`, `extract` and `delete`, including JSON output.
- Rust build orchestration, enforced dependency boundaries, portable packaging, and a six-target native CI matrix.

`doctor` reports compiled capabilities. Creating new archives, adding, renaming and replacing entries, and saving in place remain in the M1 roadmap. ZIP64 remains available for browsing and supported extraction. Further filename-encoding work is tracked in [issue #7](https://github.com/stokaro/arcforge/issues/7).

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

Browse an archive like a folder, preview its contents, extract selected items, edit files in another application, and save changes back into the archive. The design must follow BetterZip 6: filesystem Favorites, a central archive outline, detailed file table, preview and information inspector, compact toolbar, and familiar desktop workflows. The [design/UX skill](.agents/skills/arcforge-design-ux/SKILL.md) requires source-backed decisions and native screenshot evidence. Full visual and interaction parity remains work in progress.

## Planned capabilities

- Additional view modes, expanded previews, and multiple windows.
- Add, rename, move and replace entries, and save changes in place.
- External-editor round trips and explicit handling of nested archive changes.
- Existing-directory extraction and conflict handling, ZIP/ZIP64 writing; 7z, TAR and compressed streams, plus verified providers for RAR and additional formats.
- Encryption, split-volume handling, presets, filters, conflict resolution, and independent batch jobs.
- Further desktop and CLI operations through the shared Rust core.
- Platform integrations and automated build, package, and release workflows.

The epic defines the format-by-format capability matrix, implementation phases, and acceptance criteria. Planned capabilities are not claims of existing implementation.

## Platforms and distribution

| Platform | Architectures | Development bundle | Planned production distribution |
| --- | --- | --- | --- |
| macOS | Apple Silicon and Intel | ZIP with `.app` and CLI | App bundle and DMG |
| Windows | x86_64 and ARM64 | ZIP with executables | Installer and portable archive |
| Linux | x86_64 and ARM64 | Tarball with executables | AppImage, distribution packages, and tarball |

The [CI workflow](.github/workflows/ci.yml) builds, tests, and packages each target on a native runner. This matrix describes configured coverage; successful jobs are the evidence for a specific commit. A target triple alone does not provide cross-compilation SDKs or linkers. FreeBSD core/CLI support is secondary; browser/WASM support is a later portability option.

Development bundles are unsigned and are not production installers. A version tag runs the same checks and prepares a **draft** GitHub release; it does not automatically publish a release. Signing, notarization, installer integration, and upgrade tests remain product work.

## Implementation approach

Follow the engine-first approach demonstrated by [ArtCraft / PhotoCraft](https://github.com/storytold/photocraft) and the study/build/compare workflow described by [Craft Anything](https://github.com/storytold/craft-anything): document reference behavior, implement a UI-independent Rust command engine, build a thin Rust desktop UI, and verify both behavior and appearance against repeatable scenarios.

The workspace reuses PhotoCraft's Cargo-environment helper and adapts its dependency-layer checker under the MIT option of its dual license. Workspace organization, shared commands, and packaging conventions also draw on the ArtCraft ecosystem. [The reuse inventory](docs/artcraft-reuse.md) records exact upstream commits, files, modifications, and license notices. ArtCraft branding and application assets are not used.

egui/eframe is the initial GUI toolkit. Native integration, broader format support, and reference fidelity still require the capability checks in the epic. Headless rendering tests do not establish native menus, accessibility, file associations, or complete drag-and-drop behavior. RAR creation is planned through an optional external provider.

## License

ArcForge's original source is licensed under the [MIT License](LICENSE). Adapted ArtCraft source retains its MIT notices. Rust dependencies and embedded assets retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/](licenses/). No OpenZip application source is included.
