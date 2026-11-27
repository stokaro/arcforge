# Development

## Toolchain and prerequisites

Use the pinned **Rust 1.100.1** toolchain from `rust-toolchain.toml`. Commit `Cargo.lock` and use locked dependencies for repeatable local and CI builds. Install the native linker, SDK, and desktop development libraries required by the target platform; these are separate from the Rust toolchain.

Linux desktop builds need the window-system development dependencies used by egui/eframe. The CLI and core can be developed without launching a desktop session. CI defines the packages installed for its tested Linux environment; keep that list synchronized with changes to the UI backend.

## Shared build interface

Run commands from the repository root:

```sh
cargo xtask check
cargo xtask layers
cargo xtask notices
cargo xtask skills
cargo xtask build
cargo xtask test
cargo xtask build --release
cargo xtask package --release
```

`check` is the standard local gate: formatting, scoped skills, dependency boundaries, license notices, Clippy, tests, and builds. `layers`, `notices`, and `skills` run their respective checks alone. `skills-sync` regenerates Claude entry points from canonical repository skills; see [skill maintenance](agent-skills.md). `build`, `test`, and `package` use the shared Rust orchestration rather than requiring a particular shell. The supported interface is `cargo xtask {check,build,test,package,layers,notices,skills,skills-sync} [--target TARGET] [--release]`; target/profile options affect Cargo build/test steps, not source-only checks. Packaging always builds the release profile; supplying `--release` to `package` is optional. The target and profile can be supplied explicitly:

```sh
cargo xtask build --target x86_64-unknown-linux-gnu --release
cargo xtask test --target x86_64-unknown-linux-gnu --release
cargo xtask package --target x86_64-unknown-linux-gnu --release
```

A target triple alone does not install a linker, SDK, emulator, or runner. Use a compatible native environment for executing tests and packaging OS-specific applications. Cross-compilation and native smoke testing are separate results.

For focused iteration, run the affected Rust package directly with Cargo's `--locked` option. Use the shared checks before opening a PR so dependency-boundary and formatting regressions are caught as well.

## Run the scaffold

```sh
cargo run --locked -p arcforge
cargo run --locked -p arcforge-cli -- --help
cargo run --locked -p arcforge-cli -- commands --json
cargo run --locked -p arcforge-cli -- doctor --json
```

The desktop opens a bootstrap workspace. The CLI currently implements `version`, `commands`, and `doctor`. `doctor` reports compiled capabilities; it does not probe the machine, and a successful exit means the report was produced, not that archive operations are available. Planned archive commands return an explicit not-implemented error without opening or changing input files.

## Primary target matrix

| OS | x86-64 | ARM64 |
| --- | --- | --- |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |
| Linux | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |

The workflow matrix expresses the intended build/test coverage. Treat a target as verified only after its job has run successfully. A Linux local build does not validate macOS application startup or Windows integration.

## Development packages

The bootstrap packages **unsigned portable development bundles** containing the desktop application and CLI, with applicable license notices:

- macOS: a ZIP containing `ArcForge.app` and the headless CLI.
- Windows: a ZIP containing the executables.
- Linux: a `.tar.gz` containing the executables.

Each bundle includes the repository documentation and applicable source-reuse notices, with a SHA-256 sidecar for the artifact. These bundles are for testing the scaffold. DMG, MSI/NSIS, AppImage, DEB/RPM, signing/notarization, automatic updates, and production installation/upgrade tests remain work in the epic. Development packaging must not require signing credentials.

The desktop enables the default fonts in egui/eframe 0.37. The bundle includes the [dependency terms](../licenses/dependencies.md), [original source notices](../licenses/dependency-source-notices.md), and [font licenses/metadata](../licenses/fonts/README.md), alongside the direct-source reuse notices. `cargo xtask notices` checks their hashes against Cargo.lock and the workspace configuration; `check` and `package` run that gate too.

After dependencies or features change, follow [license bundle regeneration](../licenses/README.md): install the pinned cargo-about 0.9.3 maintainer tool, populate the locked Cargo cache, run `python3 scripts/update_dependency_licenses.py`, review the result, and run `cargo xtask notices`. Normal builds use the committed snapshot and do not require Python or cargo-about.

## Verification

Tests should use synthetic archives and isolated temporary directories as archive behavior is implemented. Test the same shared operation through core and CLI contracts, and add UI/native integration tests for behavior that the core cannot prove. Include independent archive-reader/writer interoperability tests when a provider is introduced.

For visual work, record the fixture, window size, scale, theme, OS, and reference version. Compare repeatable states rather than unrelated screenshots. Report the commands and platforms actually verified in the PR, along with any pending checks.
