---
name: arcforge-review
description: "Review a coherent ArcForge code change or PR for correctness, regression risks, dependency boundaries, and agreement with its requirements. Use when a change is ready for independent review or review is requested; do not create a review cycle for every message or minor iteration."
---

# Review a coherent ArcForge change

Read `AGENTS.md` and the affected requirements. Establish the review boundary with base/head commits or a working-tree diff. Include untracked files when they belong to the proposed change. Evaluate claims against current read-only ZIP/ZIP64 metadata, bounded previews, shared CLI, and desktop worker; extraction, editing, and full reference fidelity remain planned.

## Independent pass

If coordinating implementation, request one independent review for the coherent change when a reviewer is available. If assigned as that reviewer, perform the pass directly; do not delegate another review of the same change. Supply the requirements, relevant raw diff/files, source snapshot, exact verification commands/results, and tested platform. Give enough context to assess behavior without supplying the author's desired conclusion. If independent review is unavailable, perform and identify a self-review; do not manufacture a new permission gate.

Ask the reviewer to prioritize concrete correctness, data loss, command contracts, portability, and dependency-direction problems. Keep optional style preferences separate. Require each finding to identify the affected path, triggering conditions, evidence, consequence, and severity justified by demonstrated impact. Label unresolved suspicions as hypotheses rather than confirmed defects.

## Resolve findings

Check each finding against the source and, where practical, a focused reproduction. Accept valid findings, explain counterevidence for incorrect ones, and preserve relevant regression coverage. Review a follow-up only for the addressed finding or newly changed behavior. Request another broad pass only when a material change introduces a new risk; do not keep spawning reviewers until they agree.

During fixes, run targeted tests. Before a code PR, run the full `cargo xtask check` for its final relevant code state. Reuse matching verification evidence while relevant code, dependencies, toolchain, and environment remain unchanged; a new message alone does not invalidate it.

Summarize the reviewed snapshot, confirmed findings and disposition, commands/results/platform, and remaining blockers. Do not claim tested behavior from source inspection or treat a review as authority to merge or publish. Continue within existing user authorization; reviewer preferences do not create extra approval requirements.

[Source and adaptation notes](../../../docs/agent-skills.md)
