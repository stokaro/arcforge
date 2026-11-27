# Working on ArcForge

ArcForge is a Rust archive-manager project. The current repository is a bootstrap, not a working archive editor. [Issue #1](https://github.com/stokaro/arcforge/issues/1) defines the product scope and acceptance criteria.

## Boundaries

- Put archive models, capabilities, commands, and validation in `crates/core`. Keep this crate independent of UI and windowing libraries.
- Put presentation and interaction in `crates/ui-egui`. The UI calls shared core operations; it does not implement a second archive engine.
- Keep `apps/arcforge` and `apps/arcforge-cli` thin. The CLI must remain usable without a display server or GUI dependencies.
- Put build orchestration and architecture checks in `xtask`. Keep build commands portable; avoid shell-specific orchestration.
- Register new workspace crates and dependencies in the boundary checker. Do not disable checks to accommodate an accidental dependency direction.

## Implementation

- Add a regression test for a changed archive behavior, command contract, or data-preservation guarantee. Use temporary directories and synthetic or redistributable fixtures.
- Keep the capability registry truthful. Planned, unsupported, and implemented operations are distinct; return an explicit unsupported result instead of a success-shaped placeholder.
- Keep blocking I/O and archive processing out of UI rendering. Preserve the source archive on failed or cancelled updates.
- Pin dependencies through `Cargo.lock`; use the toolchain in `rust-toolchain.toml`. Document platform requirements when adding native dependencies.
- When adapting external source, record its upstream path, commit, selected license, and modifications in [the reuse inventory](docs/artcraft-reuse.md). Retain the required full license notices.

## Verification and pull requests

Run `cargo xtask check` and the relevant automated tests. Use `cargo xtask build`, `test`, and `package` as documented in [development.md](docs/development.md). Report which platforms actually ran; compilation alone does not verify desktop behavior.

PR descriptions explain the problem, before/after behavior, implementation, and verification. Include a compact Mermaid diagram when the change alters a flow. State remaining limitations directly. Update the relevant documentation and linked epic checkboxes only for work actually completed.
