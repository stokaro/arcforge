# Contributing

The [implementation epic](https://github.com/stokaro/arcforge/issues/1) defines ArcForge's planned archive-editor behavior. The repository currently supplies the project structure, shared command/capability scaffolding, desktop shell, CLI, and build tooling. It does not yet provide production archive operations.

## Start here

1. Read [AGENTS.md](AGENTS.md), [architecture.md](docs/architecture.md), and [development.md](docs/development.md).
2. Choose a focused part of the epic and describe its observable result in an issue or PR.
3. Implement shared behavior in the core before exposing it through the GUI or CLI.
4. Add automated tests for the changed contract, including failure cases relevant to archive contents or source preservation.
5. Run `cargo xtask check` and the affected tests. For build or packaging changes, exercise the appropriate native platform and record the artifact produced.

## Reviewable changes

Keep PRs focused. Explain why the change is needed, the before/after behavior, and how it was verified. Add a small Mermaid diagram when a processing or user flow changes. Include screenshots for visible UI changes using repeatable fixtures and window sizes. Distinguish a tested result from an intended capability.

Builds on Windows, macOS, and Linux are part of the design. Avoid introducing a dependency on a particular shell, a globally installed archive utility, or the GUI into the CLI. New native dependencies need explicit target and packaging coverage.

## Source reuse and licensing

Original contributions use the repository's [MIT license](LICENSE). Existing adapted source retains its upstream notices. Record new source reuse in [docs/artcraft-reuse.md](docs/artcraft-reuse.md) or a corresponding inventory, with a pinned source URL and the applicable license text in `licenses/`.

The ArtCraft code references include files offered under MIT or Apache-2.0; ArcForge selects MIT for its recorded adaptations. ArtCraft branding and the shared raven icon are not part of that permission and are not used here. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
