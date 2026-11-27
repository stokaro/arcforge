---
name: arcforge-verify
description: "Verify ArcForge implementation changes before reporting them fixed or preparing a code PR. Match claims to command results, source snapshots, and tested platforms; use valid evidence instead of rerunning unchanged checks for each new message."
---

# Verify claims against evidence

Read `AGENTS.md` and `docs/development.md`. Identify what changed and what each proposed claim requires: a successful build, a behavioral regression, a package inspection, or native desktop interaction. A passing command establishes only what that command exercises.

## Workflow

1. Record the commit and relevant working-tree changes, lockfile/toolchain state, OS/architecture, and selected build target. Inspect delegated changes and their actual evidence before treating a report as verified.
2. During iteration, run targeted checks that address the remaining risk. Use an affected package, named regression, or specific package smoke test. For documentation-only changes, check the changed content, links, and applicable metadata rather than claiming code validation was necessary.
3. Before a code PR, run `cargo xtask check` for the final relevant code state. Read its complete result and exit status. Record failed checks, skipped work, or missing prerequisites explicitly; do not translate an interrupted or blocked run into a pass.
4. Run additional platform or packaging checks only when the change or an existing gate requires them. Native CI on one target does not establish the other five targets. GUI `--version` tests process startup without proving window rendering, accessibility, or drag-and-drop.
5. Match the final statement to verified behavior. In the current bootstrap, successful `doctor` output reports capabilities; it does not imply archive readiness. No archive provider is implemented.

Keep an evidence record containing the verified snapshot, exact commands, exit/results, platform, and material limitations. Reuse a completed check when its relevant inputs and environment remain unchanged. Rerun affected checks after changes invalidate that evidence, and satisfy the full gate for the final code state; do not rerun a full suite merely because another message arrived.

Report verified outcomes and outstanding failures separately. Continue already authorized work without adding approval stages. Do not claim publication, installation, or CI success unless the corresponding operation was observed.

[Source and adaptation notes](../../../docs/agent-skills.md)
