---
name: arcforge-tdd
description: "Develop or repair ArcForge behavior through focused regression tests when changing commands, data-preservation guarantees, or other observable contracts. Use for substantive features, fixes, and behavior-sensitive refactoring; do not require test-first ceremony for prose, formatting, or changes with no executable behavior."
---

# Develop behavior with focused tests

Follow `AGENTS.md`. Select an observable contract and describe the production defect that would make its test fail. Keep core tests independent of UI; exercise real CLI processes for exit codes, stdout/stderr, and JSON contracts. The initial bootstrap has no archive providers: test truthful rejection of planned operations instead of inventing successful extraction.

1. **Specify one behavior.** Use a descriptive test name and minimal input. Prefer real code and isolated temporary fixtures. Mock only an unavoidable boundary, with its side effects understood. Avoid assertions that merely repeat implementation details.
2. **Establish the regression.** Run the focused test against the faulty or missing behavior and inspect the failure. A compiler error, missing tool, or broken fixture does not demonstrate the product defect. If implementation already exists, preserve it; compare against a known baseline in an isolated worktree when useful. Never delete working code to satisfy test order. State when a failing baseline was not observed.
3. **Implement the necessary change.** Make the smallest coherent correction, retaining existing data-preservation and dependency boundaries. Run the focused test again and inspect its actual result. Add related edge cases when the contract requires them.
4. **Refactor with coverage.** Improve structure only as needed for the change, keeping the same observable contract. Use deterministic conditions instead of arbitrary sleeps. Do not introduce production abstractions solely to make a test convenient.
5. **Complete verification.** Iterate with affected tests, then run the full `cargo xtask check` before a code PR. Record every observed failure and platform limitation rather than treating a focused pass as a green workspace.

Record the source snapshot, red/green observations, exact commands, results, and platform. Reuse valid verification until relevant inputs change; a new message does not require restarting the cycle. Existing authorization remains sufficient for routine implementation and testing.

[Source and adaptation notes](../../../docs/agent-skills.md)
