# Third-party notices

ArcForge's original source is licensed under the MIT license in [LICENSE](LICENSE). Third-party source and dependencies retain their own licenses.

## PhotoCraft source adaptations

ArcForge adapts Cargo process-environment handling and dependency-boundary checks from:

- Repository: <https://github.com/storytold/photocraft>
- Commit: `71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af`
- Upstream paths: `xtask/src/cargo_env.rs`, `xtask/src/layers.rs`
- Copyright: **Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors**
- Selected license: **MIT**, from the upstream MIT OR Apache-2.0 choice.
- Full notice: [licenses/artcraft-photocraft-MIT.txt](licenses/artcraft-photocraft-MIT.txt).

## Craft Anything references

Workspace organization, shared command registries, generated operation catalogs, and build-tool structure were studied in:

- Repository: <https://github.com/storytold/craft-anything>
- Commit: `e7c2c36c62f798175d7b990e5c9d1580398dff82`
- Copyright: **Copyright (c) 2026 ArtCraft Team and the Craft Anything contributors**
- Available code license: **MIT OR Apache-2.0**.
- Retained MIT notice: [licenses/artcraft-craft-anything-MIT.txt](licenses/artcraft-craft-anything-MIT.txt).

No Craft Anything application implementation is copied in this bootstrap. The exact distinction between copied/adapted source and architectural inspiration is recorded in [docs/artcraft-reuse.md](docs/artcraft-reuse.md).

## Assets and dependencies

ArtCraft marks, wordmarks, and the shared raven app icon are excluded from the upstream code licenses and are not included in ArcForge. The source reuse above does not import upstream fonts, icon packs, image assets, or their unrelated third-party dependencies.

Rust dependencies remain subject to their respective package licenses. `Cargo.lock` records the selected dependency graph; this file is an inventory of direct source reuse, not a claim that every transitive dependency is MIT. Binary distributions must retain the notices required by the dependencies and assets they actually contain.

The desktop currently enables `default_fonts` in **egui/eframe 0.37**. The font assets supplied by `epaint_default_fonts` retain their upstream font licenses. Their complete notices and original font copyright/license metadata are retained in [licenses/fonts/](licenses/fonts/README.md).

The full selected dependency terms and crate mappings are in [licenses/dependencies.md](licenses/dependencies.md). Original packaged and pinned upstream notices are preserved in [licenses/dependency-source-notices.md](licenses/dependency-source-notices.md), including copyright information absent from generic SPDX templates. These reports cover the active locked graph across all targets and activated workspace features, conservatively including build/development dependencies.

The complete `licenses/` directory accompanies binary bundles. [licenses/dependencies.lock.json](licenses/dependencies.lock.json) records the input/output hashes checked by `cargo xtask notices`. See [maintenance instructions](licenses/README.md) for the pinned generator and regeneration procedure.
