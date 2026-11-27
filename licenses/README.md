# License bundle maintenance

This directory is shipped with the portable development bundles. It includes:

- `artcraft-photocraft-MIT.txt` and `artcraft-craft-anything-MIT.txt`: source reuse/reference notices described in `THIRD_PARTY_NOTICES.md`.
- `dependencies.md`: full selected SPDX terms and crate/version mappings generated from cargo-about's dependency graph.
- `dependency-source-notices.md`: original packaged notices, pinned upstream files omitted from published crates, and source copyright headers.
- `fonts/`: complete font licenses and original copyright/license metadata read from `epaint_default_fonts` font files.
- `upstream/` and `upstream-sources.json`: reviewed source notices with exact repository paths, revisions, and package mappings.
- `dependencies.lock.json`: SHA-256 snapshot verified by `cargo xtask notices`, `check`, and `package`.

The dependency graph includes all targets and all activated workspace features, including build and development dependencies. This is a conservative superset of a single platform's binary. Inactive optional packages present only in Cargo.lock are not included. The snapshot hashes Cargo.lock and all workspace manifests so dependency or feature changes require regeneration.

## Regenerate

Use **cargo-about 0.9.3** and **Python 3.10 or later**. These are maintainer tools; ordinary builds use the committed notices and do not require Python or cargo-about.

```sh
cargo install cargo-about --version 0.9.3 --locked --features cli
cargo fetch --locked
python3 scripts/update_dependency_licenses.py
cargo xtask notices
```

Run from the repository root. On Windows, `py -3 scripts/update_dependency_licenses.py` is equivalent. To use an already installed binary at a different location, pass `--cargo-about /path/to/cargo-about`.

The generator invokes `cargo-about generate --frozen --workspace --all-features --fail --format json`. It neither compiles the application nor downloads source or modifies the Cargo cache. `about.toml` selects the preferred alternative in SPDX OR expressions while retaining every required AND term. The script uses `scripts/dependency-licenses.md.in` to render the final notice.

Review generated differences. New dependency versions with missing notices require a reviewed entry in `upstream-sources.json`, with full upstream files pinned to a source revision, before the snapshot is refreshed. If upstream provides only SPDX metadata and no separate notice, record that fact explicitly; do not invent copyright holders. Cargo-about's generic placeholder copyright line is removed only from synthesized standard text, never from an original upstream notice.

The source report deduplicates complete texts by SHA-256 and retains a package/source-to-notice mapping. Generated documents normalize CRLF to LF for stable cross-platform checkout hashes; original packaged-file hashes remain recorded in the report. License wording and original copyright statements are retained, including any placeholders that already occur in an upstream file itself. Protocol XML/IDL copyright blocks are included alongside wrapper-crate licenses.

Font notices are copied in full. Font SFNT name records preserve author copyright and license text that is not necessarily present in the standalone font license. Source hashes and any CRLF normalization are recorded in `fonts/provenance.json`; font binaries themselves are supplied through the locked Rust dependency, not copied here.

The snapshot includes tool configuration, the generator/template, Cargo.lock, workspace manifests, generated output, original notices, and font notices. Keep all of these changes in the same PR. The snapshot is an integrity/freshness check, not a replacement for reviewing a dependency's licensing information.

Tool references: [cargo-about configuration](https://github.com/EmbarkStudios/cargo-about/blob/main/docs/src/cli/generate/config.md) and [output schema](https://github.com/EmbarkStudios/cargo-about/blob/main/docs/src/cli/generate/output.md).
