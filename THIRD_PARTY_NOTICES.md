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

## Superpowers workflow adaptations

Four repository agent skills adapt focused workflows from:

- Repository: <https://github.com/obra/superpowers>
- Commit: `22f0b463e7a5e579904dd6211a520321035526f7`
- Upstream paths: `skills/systematic-debugging/SKILL.md`, `skills/verification-before-completion/SKILL.md`, `skills/test-driven-development/SKILL.md`, and `skills/requesting-code-review/SKILL.md`.
- Copyright: **Copyright (c) 2025 Jesse Vincent**.
- License: **MIT**.
- Full unchanged notice: [licenses/superpowers-MIT.txt](licenses/superpowers-MIT.txt).

The exact file mappings, changes, and scope are in [docs/agent-skills.md](docs/agent-skills.md). These adaptations are development instructions; no upstream hooks, installer, or plugin configuration is imported. The project-specific archive-safety skill and the skill validator/generator are original ArcForge work.

## hiewbie, Apollo, and OpenAI adaptations

ArcForge adapts focused Rust and CI instructions through [stokaro/hiewbie](https://github.com/stokaro/hiewbie) at commit `058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f`. hiewbie's original repository/specification material is MIT-licensed, **Copyright (c) 2026 Stokaro**; its full notice is identical to ArcForge's root [LICENSE](LICENSE). The inherited skill components retain their upstream terms. Document-generation, checked-offset, and release-verification ideas are adapted from its specification. No hiewbie application implementation or build script is copied.

The underlying third-party skill sources retain their own terms:

| Material | Original source | Terms and full notice |
| --- | --- | --- |
| `arcforge-rust/SKILL.md` and nine unchanged `references/chapter_*.md` files | [apollographql/skills](https://github.com/apollographql/skills), `8d0129fdc96a1b80d999a6682e48fc11e137dc09`, `skills/rust-best-practices/` | **MIT**, **Copyright (c) 2024 Apollo Graph, Inc.**; [licenses/apollo-MIT.txt](licenses/apollo-MIT.txt) |
| Modified `arcforge-ci/SKILL.md` | [openai/skills](https://github.com/openai/skills), `49f948faa9258a0c61caceaf225e179651397431`, `skills/.curated/gh-fix-ci/SKILL.md` | **Apache-2.0**; [licenses/openai-skills-Apache-2.0.txt](licenses/openai-skills-Apache-2.0.txt) |

Paths above are relative to `.agents/skills/` in ArcForge. The Rust entry is rewritten for ArcForge and the CI entry is modified to use exact revision/run evidence and repository checks. The CI instruction is a development component under Apache-2.0; the project MIT license does not replace those terms. Its generated Claude entry point loads the canonical instruction. The Python helper, upstream agent metadata, hooks, and plugin configuration are not imported. Exact intermediate paths, revisions, changes, and architectural references are in [docs/hiewbie-reuse.md](docs/hiewbie-reuse.md).

## ZIP filename mapping

The CP437 high-byte mapping in `crates/core/src/archive/names.rs` is adapted from [zip-rs/zip2](https://github.com/zip-rs/zip2/blob/37a9a8156893c7bce8ac3bf5c3c7cf264f7242ba/src/cp437.rs), commit `37a9a8156893c7bce8ac3bf5c3c7cf264f7242ba` (published zip 9.8.2). The match table becomes a 128-character lookup table with an ASCII branch. Copyright **(c) 2014 Mathijs van de Nes**; **MIT**. The full unchanged [upstream license](licenses/zip-rs-MIT.txt) is retained. See [provider rationale](docs/zip-browser.md#provider-and-source-provenance) for the adapter boundary.

## OpenZip design reference

OpenZip is the secondary interface/implementation reference for details unresolved by BetterZip 6 evidence. The reviewed source is [burakgon/openzip-mac](https://github.com/burakgon/openzip-mac/tree/83f27d3fd2d753779797b60311c2986f8f034e04), commit `83f27d3fd2d753779797b60311c2986f8f034e04`, MIT, **Copyright (c) 2026 Burak Goncu**. The complete [MIT notice](licenses/openzip-MIT.txt) is retained. [Source paths and adaptation boundaries](docs/reference/openzip-reuse.md) distinguish source-informed UX from copied implementation. No OpenZip application code, icon assets, system symbols, helpers, or Swift dependencies are copied by this slice. BetterZip remains a visual/behavior reference, with original ArcForge code and artwork.

## Assets and dependencies

ArtCraft marks, wordmarks, and the shared raven app icon are excluded from the upstream code licenses and are not included in ArcForge. The source reuse above does not import upstream fonts, icon packs, image assets, or their unrelated third-party dependencies.

Rust dependencies remain subject to their respective package licenses. `Cargo.lock` records the selected dependency graph; this file is an inventory of direct source reuse, not a claim that every transitive dependency is MIT. Binary distributions must retain the notices required by the dependencies and assets they actually contain.

The desktop currently enables `default_fonts` in **egui/eframe 0.37**. The font assets supplied by `epaint_default_fonts` retain their upstream font licenses. Their complete notices and original font copyright/license metadata are retained in [licenses/fonts/](licenses/fonts/README.md).

ArcForge also embeds unmodified **Open Sans Regular and Semibold, internal Version 1.10**, from the fixed Debian `fonts-open-sans 1.11-2` source distribution as readable proportional fallbacks. These specific files are **Apache-2.0**, with original Google Corporation copyright records retained in [OpenSans-NOTICE.txt](licenses/fonts/OpenSans-NOTICE.txt) and the complete original [Apache license](licenses/fonts/OpenSans-Apache-2.0.txt). [Typography provenance](docs/reference/typography.md) records exact source URLs, hashes, actual weights, and the system-font selection boundary. Installed Apple/Microsoft fonts are read only at runtime on the corresponding user's system and are not redistributed in ArcForge.

The full selected dependency terms and crate mappings are in [licenses/dependencies.md](licenses/dependencies.md). Original packaged and pinned upstream notices are preserved in [licenses/dependency-source-notices.md](licenses/dependency-source-notices.md), including copyright information absent from generic SPDX templates. These reports cover the active locked graph across all targets and activated workspace features, conservatively including build/development dependencies.

The complete `licenses/` directory accompanies binary bundles. [licenses/dependencies.lock.json](licenses/dependencies.lock.json) records the input/output hashes checked by `cargo xtask notices`. See [maintenance instructions](licenses/README.md) for the pinned generator and regeneration procedure.
