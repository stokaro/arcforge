# Repository agent skills

ArcForge keeps eight focused workflows in the repository. They support implementation and review of the archive manager described in [issue #1](https://github.com/stokaro/arcforge/issues/1). The application provides ZIP/ZIP64 browsing, bounded previews, and extraction into a new directory; archive editing remains planned. The skills guide development and verification rather than implement those operations.

## Discovery and use

The canonical instruction files are `.agents/skills/<name>/SKILL.md`. Codex supports repository skills in this location. Claude Code discovers project skills in `.claude/skills`; ArcForge generates small entry points there which load the same canonical instructions. These are ordinary files, so a checkout does not depend on Windows symlink privileges. Root `CLAUDE.md` imports `AGENTS.md` to keep project rules in one place.

| Skill | Apply when | Result |
| --- | --- | --- |
| `arcforge-debug` | A concrete bug, failed test, crash, performance regression, or build failure needs diagnosis | Reproduction, supported cause, focused fix, and matching evidence |
| `arcforge-tdd` | A feature/fix changes observable behavior or preservation guarantees | Meaningful regression and observed red/green evidence where feasible |
| `arcforge-archive-safety` | Archive providers, extraction, mutation, helpers, or filesystem placement change | Explicit safety invariants and tests for the changed boundary |
| `arcforge-review` | A coherent code change is ready for review, or review is requested | Concrete findings, dispositions, and remaining risks |
| `arcforge-verify` | Implementation claims or a code PR need verification | Commands, source snapshot, results, and platform limits |
| `arcforge-rust` | Nontrivial Rust ownership, API, error, byte-range, unsafe, or performance work needs guidance | A justified design and focused checks, with relevant reference sections only |
| `arcforge-design-ux` | Interface layout, UX code, dialogs, input, visual changes, or screenshot review | BetterZip 6 fidelity, attributed OpenZip fallback, state coverage, and actual native screenshots |
| `arcforge-ci` | A specified PR/commit has failing, missing, or incomplete CI checks | Revision-bound diagnosis, focused correction, and exact remote-check status |

Use normal task requests, or explicitly invoke a project skill: `$arcforge-debug` in Codex, `/arcforge-debug` in Claude Code. An agent without native skill discovery can follow the links in [AGENTS.md](../AGENTS.md). Client settings, versions, and enabled project instructions affect discovery; the build validator does not exercise a live agent client.

Start with `AGENTS.md`; load a skill's body only when its task applies. For example, a failed-save bug needs debugging, preservation tests, and archive safety; verification applies before reporting it fixed. A documentation correction only needs content/link checks and the applicable metadata gate. No startup hook loads the entire catalog, and no global agent configuration is changed.

The Rust skill indexes nine pinned Apollo reference chapters. Search their headings and read the section needed for the current decision; these references are not added to every task's context. Repository contracts and measured evidence take precedence over contextual upstream style heuristics. The CI skill prefers the connected GitHub integration, accepts an authenticated CLI as an alternative, and adds no Python/helper dependency to the build.

## Design and UX workflow

The owner requires BetterZip 6 look and feel, with macOS the primary visual target and OpenZip a fallback for unresolved details. The [design/UX skill](../.agents/skills/arcforge-design-ux/SKILL.md) turns that hierarchy into an implementation and review workflow: identify affected states, inspect the latest supplied evidence, record fallback decisions, implement through shared commands, exercise the native UI, and retain real macOS/Windows/Linux screenshots. It covers visual-only changes as well as controller code. Its acceptance dimensions separately cover toolbar/titlebar, genuine font weights, pane proportions, full-height outline striping, inspector hierarchy and content preview. An adaptation records an unresolved difference; it does not waive fidelity. The [reference inventory](reference/betterzip-6.md), [OpenZip provenance](reference/openzip-reuse.md), and [screenshots](../screenshots/README.md) are supporting project contracts rather than duplicated skill bodies.

The skill was initialized using skill-creator, constrained to the repository's canonical metadata format, and connected through the existing generated Claude entry point. This is a repository skill; it does not change personal/global agent configuration. Its original instructions add no mandatory approval stage. A fresh read-only agent applied the skill to a realistic extraction-progress/failure task: it retained the reference priority, inspected pinned OpenZip details for the documented gap, kept partial-output semantics explicit, and proposed native state captures. That forward test checks instruction use; it is not evidence of application runtime or complete visual parity.

## Verification scope

Run focused tests during implementation, and `cargo xtask check` before a code PR for the final relevant code state. Retain successful evidence while relevant code, dependencies, toolchain, and environment remain unchanged. A new message or PR-description edit alone does not invalidate it. After a code change, rerun affected checks and satisfy the final code gate.

Use independent review once per coherent substantial change when a reviewer is available. Supply raw source/diff, requirements, and actual verification evidence. Resolve concrete findings and review the affected correction; another broad review needs a material new risk. Label self-review when an independent reviewer is unavailable.

The workflows preserve existing user authorization and do not impose mandatory design approvals on routine work. They do not promise reduced token usage or fewer defects; those outcomes would require measurement.

## Maintain the catalog

1. Edit a canonical `SKILL.md`. Keep its trigger specific and its body concise; aim for 250–500 words, with up to 1,000 allowed for domain requirements.
2. Use exactly two frontmatter fields: an unquoted `name` matching the directory and a single-line, JSON-style double-quoted `description` of 1–1,024 characters. This is a deliberately constrained subset of YAML. Keep the complete file within 500 lines.
3. Run `cargo xtask skills-sync`, then review the generated `.claude/skills` differences. Never edit those entry points independently.
4. Run `cargo xtask skills`. It checks the eight known skills, metadata/body budgets, simple inline relative Markdown links, and exact entry-point contents. It rejects symlinked owned paths, unexpected `arcforge-*` catalog entries, and unrecognized wrapper files rather than deleting or overwriting them. Other skill names are outside this checker.
5. Forward-test substantial instruction changes with a fresh agent given a realistic task and raw artifacts. Do not give it the expected diagnosis. Inspect its decisions and output; a metadata pass does not establish instruction quality.
6. When adapting upstream changes, review them at a fixed commit and update the provenance below. Preserve required notices and refresh the license snapshot if its inputs changed.

`cargo xtask skills` is read-only and runs inside `cargo xtask check`. `skills-sync` validates the complete catalog before writing and changes only missing or stale recognized entry points. It is idempotent; it does not install packages, fetch upstream content, register hooks, or modify personal agent settings. Both commands are implemented in Rust using existing workspace dependencies. The generator is a repository maintenance helper, not an archive security boundary or a general YAML/Markdown validator.

Useful targeted checks:

```sh
cargo xtask skills-sync
cargo xtask skills
cargo test --locked -p xtask skills::tests
```

## Superpowers provenance

Four workflows are adapted from [obra/superpowers](https://github.com/obra/superpowers) at commit **`22f0b463e7a5e579904dd6211a520321035526f7`**, reviewed on 2026-11-27. The upstream copyright is **Copyright (c) 2025 Jesse Vincent**. The selected source is MIT-licensed; the complete unchanged notice is retained in [licenses/superpowers-MIT.txt](../licenses/superpowers-MIT.txt).

| Canonical ArcForge file | Pinned upstream source | Main adaptation |
| --- | --- | --- |
| [arcforge-debug](../.agents/skills/arcforge-debug/SKILL.md) | [systematic-debugging/SKILL.md](https://github.com/obra/superpowers/blob/22f0b463e7a5e579904dd6211a520321035526f7/skills/systematic-debugging/SKILL.md) | Focused root-cause diagnosis, repository boundaries, and bounded experiments |
| [arcforge-verify](../.agents/skills/arcforge-verify/SKILL.md) | [verification-before-completion/SKILL.md](https://github.com/obra/superpowers/blob/22f0b463e7a5e579904dd6211a520321035526f7/skills/verification-before-completion/SKILL.md) | Evidence tied to relevant source/environment state; no repeat solely for a new message |
| [arcforge-tdd](../.agents/skills/arcforge-tdd/SKILL.md) | [test-driven-development/SKILL.md](https://github.com/obra/superpowers/blob/22f0b463e7a5e579904dd6211a520321035526f7/skills/test-driven-development/SKILL.md) | Test behavior-sensitive changes, preserve existing code, disclose unobserved failing baselines |
| [arcforge-review](../.agents/skills/arcforge-review/SKILL.md) | [requesting-code-review/SKILL.md](https://github.com/obra/superpowers/blob/22f0b463e7a5e579904dd6211a520321035526f7/skills/requesting-code-review/SKILL.md) | One independent pass per coherent change and focused follow-up on findings |

The bodies have been shortened, rewritten, and scoped to ArcForge. The original universal activation rules, mandatory brainstorming/design approval stages, and instructions to delete code written before a test are not imported. Upstream hooks, installer scripts, agents, templates, and plugin configuration are not vendored. Generated Claude entry points refer to the adapted files and contain no separate upstream workflow implementation.

`arcforge-archive-safety`, `arcforge-design-ux`, the routing, generator, and tests are original ArcForge work under the repository's [MIT license](../LICENSE). Existing ArtCraft code provenance is unchanged; see [artcraft-reuse.md](artcraft-reuse.md).

## hiewbie, Apollo, and OpenAI skill provenance

The Rust and CI entry instructions adapt the corresponding focused workflows from [stokaro/hiewbie](https://github.com/stokaro/hiewbie/tree/058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f), inspected at commit `058d5b4a9775a4ee2a7ec09db9493b9dc8a3650f`. hiewbie's original repository/specification material is MIT-licensed, copyright 2026 Stokaro; its notice is byte-identical to ArcForge's root [LICENSE](../LICENSE). The inherited skill components retain their upstream terms below.

| ArcForge material | Original source and revision | Retained terms | Adaptation |
| --- | --- | --- | --- |
| `arcforge-rust/SKILL.md` and nine `references/chapter_*.md` files | [apollographql/skills](https://github.com/apollographql/skills/tree/8d0129fdc96a1b80d999a6682e48fc11e137dc09/skills/rust-best-practices), `8d0129fdc96a1b80d999a6682e48fc11e137dc09` | [MIT, copyright 2024 Apollo Graph, Inc.](../licenses/apollo-MIT.txt) | Entry rewritten for ArcForge; nine reference chapters copied unchanged through hiewbie |
| `arcforge-ci/SKILL.md` | [openai/skills](https://github.com/openai/skills/tree/49f948faa9258a0c61caceaf225e179651397431/skills/.curated/gh-fix-ci), `49f948faa9258a0c61caceaf225e179651397431` | [Apache-2.0](../licenses/openai-skills-Apache-2.0.txt) | Entry rewritten through hiewbie for exact source/run evidence, connector use, and ArcForge checks; Python helper not imported |

These are local adaptations, not upstream Apollo or OpenAI releases. The CI instruction retains Apache-2.0 rather than being relabeled MIT. Its generated Claude entry point loads that canonical instruction. No upstream plugin, hooks, installer, agent metadata, or CLI helper is imported. The existing Superpowers debugging workflow is retained without a second copy from hiewbie. [hiewbie-reuse.md](hiewbie-reuse.md) records exact intermediate paths, modifications, and the related architectural inspiration.

## Archive safety references

The domain skill specifies ArcForge requirements. Provider conveniences do not establish the full requirements: lexical name checks alone cannot stop filesystem-component replacement during extraction. In particular, the tar crate's documented threat model excludes concurrent destination-tree mutation. Review the actual selected provider/version and platform adapter before claiming containment.

- [Microsoft: file names, paths, and namespaces](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
- [Microsoft: file streams](https://learn.microsoft.com/en-us/windows/win32/fileio/file-streams).
- [Microsoft: reparse points and file operations](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-points-and-file-operations).
- [zip 9.8.2: `ZipFile::enclosed_name`](https://docs.rs/zip/9.8.2/zip/read/struct.ZipFile.html#method.enclosed_name).
- [tar 0.4.46: security model](https://docs.rs/tar/0.4.46/tar/#security).

These are implementation references, not newly selected application dependencies.

## Agent format references

- [Agent Skills specification](https://agentskills.io/specification).
- [Codex: build skills](https://learn.chatgpt.com/docs/build-skills).
- [Claude Code: project skills](https://code.claude.com/docs/en/skills).
- [Claude Code: project instruction imports](https://code.claude.com/docs/en/memory#import-additional-files).
