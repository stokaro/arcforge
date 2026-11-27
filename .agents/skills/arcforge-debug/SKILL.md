---
name: arcforge-debug
description: "Investigate ArcForge test failures, crashes, incorrect command behavior, performance regressions, or build and packaging failures. Apply within this repository when a concrete failure needs diagnosis and a verified fix; do not activate for unrelated projects or ordinary discussion."
---

# Diagnose an ArcForge failure

Read the repository's `AGENTS.md` and relevant implementation before changing behavior. Continue within existing user authorization; routine investigation and reversible fixes need no additional design approval.

1. **Capture the failure.** Record expected and observed behavior, exact reproduction command or UI steps, inputs, OS/architecture, toolchain, and the commit plus relevant uncommitted changes. Read the complete diagnostic and inspect recent changes. Distinguish a product failure from unavailable prerequisites.
2. **Locate the failing boundary.** Trace data backward through shared core commands, CLI formatting, UI presentation, or `xtask` and the native toolchain. Compare a working path. Add narrowly scoped diagnostics when needed; never dump credentials or unrelated environment variables.
3. **Test one hypothesis.** State the proposed cause and evidence that would distinguish it from alternatives. Use the smallest reversible experiment. If experiments fail, reassess assumptions and boundaries instead of stacking speculative fixes. Ask for missing information only when it prevents further useful work.
4. **Fix and protect the behavior.** Add a focused regression where feasible, confirm that its failure represents the bug, then fix the originating cause. Preserve unrelated work and avoid opportunistic refactoring. The current bootstrap has no archive providers: an explicitly unsupported operation is not a broken extractor.
5. **Verify the changed path.** During iteration, run the affected package/test, such as `cargo test --locked -p arcforge-cli --test cli`. Before a code PR, run the full `cargo xtask check` gate for the final relevant source state and disclose any failure or environmental blocker.

Report the cause, change, reproduction before/after, exact commands, results, and tested platform. Associate evidence with the verified snapshot. Reuse it while relevant code, dependencies, toolchain, and environment remain unchanged; a new message alone does not require another full run. Distinguish a source-based diagnosis from a reproduced runtime fix.

[Source and adaptation notes](../../../docs/agent-skills.md)
