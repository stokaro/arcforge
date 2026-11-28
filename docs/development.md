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
cargo xtask ci-matrix
cargo xtask verify-package --target x86_64-unknown-linux-gnu
```

`check` is the standard local gate: formatting, scoped skills, dependency boundaries, license notices, Clippy, tests, and builds. `layers`, `notices`, and `skills` run their respective checks alone. `skills-sync` regenerates Claude entry points from canonical repository skills; see [skill maintenance](agent-skills.md). `build`, `test`, and `package` use the shared Rust orchestration rather than requiring a particular shell. Target/profile options affect Cargo build/test steps, not source-only checks. Packaging always builds the release profile; supplying `--release` to `package` is optional. `ci-matrix`, `verify-package`, and `verify-release` share the [target manifest and release contract](release.md); see `cargo xtask --help` for their options. The target and profile can be supplied explicitly:

```sh
cargo xtask build --target x86_64-unknown-linux-gnu --release
cargo xtask test --target x86_64-unknown-linux-gnu --release
cargo xtask package --target x86_64-unknown-linux-gnu --release
```

A target triple alone does not install a linker, SDK, emulator, or runner. Use a compatible native environment for executing tests and packaging OS-specific applications. Cross-compilation and native smoke testing are separate results.

For focused iteration, run the affected Rust package directly with Cargo's `--locked` option. Use the shared checks before opening a PR so dependency-boundary and formatting regressions are caught as well.

## Run the application

```sh
cargo run --locked -p arcforge
cargo run --locked -p arcforge -- example.zip
cargo run --locked -p arcforge-cli -- --help
cargo run --locked -p arcforge-cli -- commands --json
cargo run --locked -p arcforge-cli -- doctor --json
cargo run --locked -p arcforge-cli -- list example.zip --json
cargo run --locked -p arcforge-cli -- info example.zip --json
cargo run --locked -p arcforge-cli -- preview example.zip --entry 0 --json
```

The desktop opens a read-only ZIP/ZIP64 browser. It accepts one native path at startup, a path in the Open window, or a file drop; use `-- PATH` for a startup path beginning with a dash. Search, filename interpretation, archive reads, and preview preparation run on a bounded worker outside the UI thread. The CLI implements `version`, `commands`, `doctor`, `list`, `info`, and bounded `preview` through the shared core. `doctor` reports compiled capabilities without probing the machine or opening an archive. Planned commands still return explicit not-implemented errors. See [the ZIP contract](zip-browser.md) for limits, filename options, metadata revisions, and unsupported cases.

## Primary target matrix

| OS | x86-64 | ARM64 |
| --- | --- | --- |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |
| Linux | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |

[`release/targets.json`](../release/targets.json) defines the native CI matrix and package layouts. Treat a target as verified only after its job has run successfully. A Linux local build does not validate macOS application startup or Windows integration.

## Development packages

The packager creates **unsigned portable development bundles** containing the desktop application and CLI, with applicable license notices:

- macOS: a ZIP containing `ArcForge.app` and the headless CLI.
- Windows: a ZIP containing the executables.
- Linux: a `.tar.gz` containing the executables.

Each bundle includes the repository documentation and applicable source-reuse notices, with a SHA-256 sidecar for the artifact. These bundles are for development testing. DMG, MSI/NSIS, AppImage, DEB/RPM, signing/notarization, automatic updates, and production installation/upgrade tests remain work in the epic. Development packaging must not require signing credentials.

CI runs commands from the unpacked bundle and writes native evidence for its archive hash and source revision. A final job requires the complete six-target set of archives, checksums, and evidence. Tag builds verify that set before creating a draft release and compare the downloaded uploaded assets with the prepared files. See [release verification](release.md) for the commands, exact asset inventory, and limits of the evidence.

The desktop enables the default fonts in egui/eframe 0.37. The bundle includes the [dependency terms](../licenses/dependencies.md), [original source notices](../licenses/dependency-source-notices.md), and [font licenses/metadata](../licenses/fonts/README.md), alongside the direct-source reuse notices. `cargo xtask notices` checks their hashes against Cargo.lock and the workspace configuration; `check` and `package` run that gate too.

After dependencies or features change, follow [license bundle regeneration](../licenses/README.md): install the pinned cargo-about 0.9.3 maintainer tool, populate the locked Cargo cache, run `python3 scripts/update_dependency_licenses.py`, review the result, and run `cargo xtask notices`. Normal builds use the committed snapshot and do not require Python or cargo-about.

## Verification

Tests use synthetic archives and isolated temporary directories for archive behavior. Test the same shared operation through core and CLI contracts, and add UI/native integration tests for behavior that the core cannot prove. The committed [Python zipfile fixture](../tests/fixtures/python-zipfile.hex) supplies independent Stored/Deflate ZIP input to tests and package smoke verification; the optional [generator](../scripts/generate_zip_fixture.py) regenerates it. Normal tests, builds, and package verification consume the committed bytes and do not require Python. Add interoperability fixtures as provider coverage grows.

For visual work, record the fixture, window size, scale, theme, OS, and reference version. Compare repeatable states rather than unrelated screenshots. Report the commands and platforms actually verified in the PR, along with any pending checks.
