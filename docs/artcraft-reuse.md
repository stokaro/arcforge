# ArtCraft reuse inventory

ArcForge borrows selected development infrastructure and architectural patterns from ArtCraft projects to accelerate its Rust bootstrap. This inventory distinguishes source adaptation from design inspiration. It does not claim ArtCraft affiliation or endorsement.

## Pinned source snapshots

| Repository | Reviewed commit |
| --- | --- |
| [PhotoCraft](https://github.com/storytold/photocraft) | `71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af` |
| [Craft Anything](https://github.com/storytold/craft-anything) | `e7c2c36c62f798175d7b990e5c9d1580398dff82` |

Both snapshots offer their original code under **MIT OR Apache-2.0**. ArcForge selects MIT for the source adaptations recorded below. ArcForge's original code remains covered by the root [LICENSE](../LICENSE).

## Imported and adapted source

| ArcForge file | Pinned upstream file | Reuse and changes |
| --- | --- | --- |
| `xtask/src/cargo_env.rs` | [PhotoCraft cargo_env.rs](https://github.com/storytold/photocraft/blob/71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af/xtask/src/cargo_env.rs) | Source and tests copied with a provenance header and local formatting. Preserves developer/CI toolchain settings while removing Cargo metadata describing the running xtask binary. |
| `xtask/src/layers.rs` | [PhotoCraft layers.rs](https://github.com/storytold/photocraft/blob/71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af/xtask/src/layers.rs) | Smaller adaptation of the metadata/model/checker separation for ArcForge's crate names. Adds transitive UI-dependency protection for the core and CLI, checks normal/build/dev and conditional edges, and removes PhotoCraft's application exemption and image-specific policies. |

The full upstream MIT notice is retained in [licenses/artcraft-photocraft-MIT.txt](../licenses/artcraft-photocraft-MIT.txt). File headers point to the relevant source and notice. Subsequent adaptations must keep this inventory accurate rather than treating the whole upstream repository as imported.

## Architectural inspiration

- **Workspace organization:** `crates/`, `apps/`, and `xtask`, centralized Cargo settings, and thin executable entry points. References: [PhotoCraft Cargo.toml](https://github.com/storytold/photocraft/blob/71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af/Cargo.toml) and [Craft Anything Cargo.toml](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/Cargo.toml).
- **One operation model for interfaces:** stable operation identities and a shared registry behind desktop and CLI behavior. Reference: [Craft Anything registry.rs](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/crates/engine/src/registry.rs). ArcForge does not import Craft Anything's engine, AI tools, plugin lifetime model, or application-specific command set.
- **Executable documentation:** derive command listings from the registry instead of maintaining an unrelated hand-written list. Reference: [Craft Anything tool_list.rs](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/crates/engine/src/tool_list.rs). Do not adopt an arbitrary minimum command-count gate.
- **Build orchestration:** a `cargo xtask` entry point, explicit check/build/test tasks, and CI invoking the same project tools. Reference: [Craft Anything xtask](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/xtask/src/main.rs).
- **Development method:** study reference behavior, implement shared Rust operations, compare repeatable states, and iterate. Reference: [Craft Anything workflow](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/README.md).

The full [Craft Anything MIT notice](../licenses/artcraft-craft-anything-MIT.txt) is retained for provenance. No Craft Anything application implementation is copied in this bootstrap; its registry/catalog and workspace patterns are conceptual references. If a later change adapts its source, add the exact local and upstream files to the source-adaptation table.

## Material not imported

ArtCraft's [brand terms](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/docs/brand/LICENSE-brand.txt) and [app-icon notice](https://github.com/storytold/craft-anything/blob/e7c2c36c62f798175d7b990e5c9d1580398dff82/assets/app-icon/LICENSE.txt) exclude its marks and shared raven icon from the code licenses. ArcForge does not copy them. Fonts, icons, and other third-party assets have separate licenses and are not imported through this bootstrap reuse.

The bootstrap also does not import ArtCraft's imaging pipelines, custom cryptography, framework forks, release-service integration, signing identities, or organization-wide agent governance. Any future use of those components requires a concrete ArcForge need and its own recorded provenance.
