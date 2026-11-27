# hiewbie references and source reuse

Reference snapshot: [stokaro/hiewbie at `058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f`](https://github.com/stokaro/hiewbie/tree/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f), reviewed on 2026-11-28.

At this revision, hiewbie is a specification and repository bootstrap. Its architecture and build documents explicitly describe planned work; it does not contain a Rust application, Cargo workspace, or implemented packaging system. ArcForge uses those contracts as design input and implements its own release tooling. It does not claim to import a working hiewbie build system.

## Retained skill material

Canonical ArcForge skill paths below are relative to `.agents/skills/`. The generated `.claude/skills/` files are entry points to the canonical instructions. [agent-skills.md](agent-skills.md) describes their scope, discovery, and maintenance.

| ArcForge destination | Pinned hiewbie source path | Changes |
| --- | --- | --- |
| `arcforge-rust/SKILL.md` | [`.agents/skills/rust-best-practices/SKILL.md`](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/.agents/skills/rust-best-practices/SKILL.md) | Renamed and rewritten for workspace boundaries, document generations, archive byte ranges, the pinned toolchain, and ArcForge's check gate; contextual treatment of upstream heuristics retained |
| `arcforge-rust/references/chapter_01.md` through `chapter_09.md` | [`.agents/skills/rust-best-practices/references/`](https://github.com/stokaro/hiewbie/tree/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/.agents/skills/rust-best-practices/references) | Copied unchanged; loaded selectively by design question |
| `arcforge-ci/SKILL.md` | [`.agents/skills/gh-fix-ci/SKILL.md`](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/.agents/skills/gh-fix-ci/SKILL.md) | Renamed and rewritten for repository/SHA/run/attempt/job provenance, merge-ref source snapshots, API coverage limits, delivered-package evidence, and preservation of required platforms; helper invocation removed |

The Rust guidance originates in [apollographql/skills](https://github.com/apollographql/skills/tree/8d0129fdc96a1b80d999a6682e48fc11e137dc09/skills/rust-best-practices) at commit `8d0129fdc96a1b80d999a6682e48fc11e137dc09`, under **MIT**, copyright **2024 Apollo Graph, Inc.** The nine chapters in hiewbie are unchanged from that revision. The full unchanged upstream root license is retained in [licenses/apollo-MIT.txt](../licenses/apollo-MIT.txt); its Git blob is `a27c8c69a6e0e14352ef9d9beb78bf6a844d1a00`.

The CI guidance originates in [openai/skills](https://github.com/openai/skills/tree/49f948faa9258a0c61caceaf225e179651397431/skills/.curated/gh-fix-ci) at commit `49f948faa9258a0c61caceaf225e179651397431`, path `skills/.curated/gh-fix-ci/`, under **Apache-2.0**. The full unchanged skill license is retained in [licenses/openai-skills-Apache-2.0.txt](../licenses/openai-skills-Apache-2.0.txt); its Git blob is `13e25df86ce06eb6488e6a6bc5c5847f5dedc352`. The adapted instruction identifies its modification and keeps these terms. ArcForge does not copy the upstream/hiewbie Python helper, its tests, agent metadata, or plugin configuration, and does not require Python or `gh` for application builds.

hiewbie's original repository/specification material is **MIT**, copyright **2026 Stokaro**. Its complete license is byte-identical to ArcForge's retained root [LICENSE](../LICENSE), Git blob `66b510ab4c9381953954657ce7660c31d3268f10`. The inherited skill components retain their upstream terms. The repository's MIT license does not replace Apollo's copyright or the Apache-2.0 terms of the adapted CI instruction. [NOTICE](../NOTICE) and [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) include these sources; development bundles retain the `licenses/` directory.

## Architectural and release inspiration

These requirements are adapted to an archive manager. Their application contracts are documented in [architecture.md](architecture.md), the build commands in [development.md](development.md), and the delivered-package/release checks in [release.md](release.md).

| Pinned hiewbie reference | ArcForge adaptation |
| --- | --- |
| [`docs/architecture.md`, document/storage and backend contracts](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/docs/architecture.md) | Document generations and source identity bind background results and caches; checked 64-bit offsets and bounded buffers distinguish archive positions from decompressed-entry positions |
| [`docs/build-and-release.md`, orchestration and release gates](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/docs/build-and-release.md) | A shared target/artifact manifest, validation of unpacked delivered bundles, exact artifact completeness, source/version/hash evidence, and comparison of published assets |
| [`AGENTS.md`](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/AGENTS.md) and [development workflow](https://github.com/stokaro/hiewbie/blob/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f/docs/development-workflow.md) | Scope changes to affected contracts, retain valid evidence, preserve raw logs and exit status, and load only relevant instructions |

The generation and large-file rules are implementation contracts, not a claim that archive providers, asynchronous indexing, or transactional editing already exist. A global document generation is a suitable starting point; narrower invalidation requires a demonstrated need. Compressed and solid archives do not inherit a hex editor's arbitrary random-access or piece-table assumptions.

ArcForge retains its six primary OS/CPU targets: macOS, Windows, and GNU/Linux on x86-64 and ARM64. hiewbie's additional RISC-V, FreeBSD, browser, and universal-CLI artifact requirements are not imported. Unsigned development bundles remain distinct from signed installers, notarization, and verified GUI interaction. No hiewbie application code or build script is copied.

The already adapted Superpowers debugging workflow is not copied again. ArcForge also does not import hiewbie's HIEW-specific behavior, terminal controls, executable-analysis backends, or optional output-filter tooling.
