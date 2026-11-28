---
name: arcforge-rust
description: "Implement or review nontrivial Rust ownership, APIs, error handling, unsafe boundaries, checked byte ranges, or measured performance changes in ArcForge. Use for Rust design and refactoring; skip prose-only edits, simple renames, and non-Rust tasks."
---

# Apply Rust guidance to the actual boundary

Read `AGENTS.md`, the changed code, its callers, and relevant tests. Follow the pinned toolchain, workspace dependency direction, and existing error/API conventions. State the invariant or observed problem before choosing an abstraction. The current provider exposes ZIP/ZIP64 metadata, bounded previews, and new-directory extraction through the shared core; keep partial-output reports and native paths intact, and do not invent successful editing behavior.

1. Prefer borrowed input when ownership is unnecessary. Justify clones and shared ownership by lifetimes, background snapshots, and API needs. Keep metadata valid for its document generation; do not retain views into replaced or mutable backing storage without an explicit ownership contract.
2. Propagate expected failures with explicit errors and useful context. Validate external counts, offsets, lengths, and arithmetic before allocation or indexing. Keep archive offsets distinct from offsets inside decompressed entries. Convert to `usize` only after bounded size checks. Document and isolate any unsafe preconditions.
3. Keep blocking I/O and archive processing outside the UI thread. Preserve core/CLI independence from UI libraries. Select generics, trait objects, and type states for a demonstrated boundary or invalid state, accounting for code size and build time.
4. Read only the relevant reference section below. Treat Copy-size cutoffs, one assertion per test, blanket `dyn`/`unwrap` restrictions, benchmark thresholds, and lint choices as contextual suggestions. Repository requirements and measured evidence take precedence over these upstream heuristics.
5. Optimize representative release workloads after measuring the relevant latency, allocations, memory use, or throughput. Include cancellation and bounded-work behavior when they affect responsiveness. Do not add a profiling dependency or framework without a concrete need.
6. Add regression coverage for changed observable behavior and meaningful boundary cases. During iteration, target the affected package/test. Respect ArcForge's configured feature matrix; do not change MSRV or enable unrelated features for convenience. Follow the final `cargo xtask check` gate in `AGENTS.md`, retaining valid prior evidence until relevant inputs change.

## Read one relevant section

Search headings first; do not preload the nine chapters. These are pinned third-party references, not separately enforced repository rules.

| Design question | Reference |
| --- | --- |
| Borrowing, cloning, iteration, abstraction | [Idioms](references/chapter_01.md) |
| A Clippy diagnostic or lint configuration | [Clippy](references/chapter_02.md) |
| A measured performance issue | [Performance](references/chapter_03.md) |
| Error boundaries and `Result` types | [Errors](references/chapter_04.md) |
| Behavior, regressions, and snapshots | [Tests](references/chapter_05.md) |
| Generics versus trait objects | [Dispatch](references/chapter_06.md) |
| A concrete invalid-state problem | [Type states](references/chapter_07.md) |
| Public API documentation | [Documentation](references/chapter_08.md) |
| Pointer choice or `Send`/`Sync` boundary | [Pointers](references/chapter_09.md) |

Report exact commands, exit status, source snapshot, and coverage limits. Keep full raw logs available; if filtering output, preserve the original command's status and inspect raw output when necessary. Compilation does not establish runtime behavior.

Adapted from hiewbie and Apollo guidance; the entry is modified for ArcForge and the reference chapters are unchanged. See [source mappings and modifications](../../../docs/hiewbie-reuse.md) and the retained [Apollo MIT notice](../../../licenses/apollo-MIT.txt).
