# Native GUI captures

`cargo xtask gui-capture` starts the real ArcForge desktop on its native host,
opens the original UI fixture through the ordinary archive worker, selects two
entries, and records their finished previews. This is maintainer tooling behind
the `native-capture` feature. Normal development release packages do not enable
that feature and do not expose its launch argument.

## Run and inspect

Use a clean Git worktree at a committed revision, a native desktop session, and
the repository's pinned toolchain. The command
also needs Python 3 to generate the fixture; normal builds and package verification
do not acquire that requirement.

```sh
cargo xtask gui-capture --directory target/native-gui/local-review
```

The output directory must be new. `--target` is optional; if supplied it must
exactly match the native Rust host. The helper builds `arcforge` in the debug
profile with `native-capture`, using the same production Glow backend, shared
native window options, typography selection, UI layout and archive worker. It
does not capture the separately packaged release binary.

The default directory is `target/native-gui/<target>`. On Linux, CI starts a real
X11 window in Xvfb and requests Mesa software OpenGL:

```sh
LIBGL_ALWAYS_SOFTWARE=1 ARCFORGE_CAPTURE_DISPLAY='Xvfb X11; Mesa software OpenGL requested' \
  xvfb-run -a -s '-screen 0 1600x1000x24' \
  cargo xtask gui-capture --directory target/native-gui/linux-review
```

This is an actual X11 client rendered by the shipping backend in a virtual display.
It does not establish behavior on a physical monitor, a particular GPU, Wayland,
or a desktop window manager. Windows and macOS CI launch directly in their hosted
native sessions; the runtime log and the resulting pixels establish success.

Every completed capture contains these two scenes:

| Scene | Actual selected entry | Required worker result |
| --- | --- | --- |
| `client/01-text-preview.png` | `Documents/Guide.txt` | Complete UTF-8 text preview with verified CRC32 |
| `client/02-image-preview.png` | `Photos/Color-study.png` | Complete verified preview and decoded 960 × 640 PNG texture |

The fixture is generated with `scripts/generate_ui_fixture.py --images`. Its
text and image are original repository content. The driver invokes normal
Outline filtering, selection, and preview commands. It never inserts a fake
snapshot, preview result, archive status, or rendered workspace.

Readiness depends on the selected entry, snapshot identity, finished worker,
matching preview, CRC verification, and a prepared image texture where relevant.
The harness then waits for two actual renderer screenshot callbacks with the
same ready state; it discards the first frame and saves the second. A UI layout
pass alone does not count as a painted frame. There is no guessed loading sleep.
PNG encoding and acknowledgement waits run on a separate writer thread. The
application deadline is 120 seconds, the external process deadline is 150 seconds,
and each scene acknowledgement is bounded to 45 seconds.

## What the pixels include

The required images have `capture_kind: native-client-area` and
`native_window_chrome: false`. The public
[`egui::Context::request_screenshot` callback](https://github.com/emilk/egui/blob/7c0b40402271e8d40cda69f87cf3c2617c2e4382/crates/egui/src/context.rs)
receives the pixels of a real rendered native viewport.
The pinned [Glow integration](https://github.com/emilk/egui/blob/7c0b40402271e8d40cda69f87cf3c2617c2e4382/crates/eframe/src/native/glow_integration.rs)
reads its framebuffer after painting. This captures the actual UI rendered on that
OS; it does not include native titlebar controls, rounded window clipping, desktop,
or shadows drawn by the window server. PNG encoding converts egui's premultiplied
colors to PNG's straight-alpha representation without compositing a background.

On macOS the driver additionally attempts a native window capture for each scene.
It compiles the repository's small Swift helper, finds the single on-screen
ordinary window belonging to the launched ArcForge PID through CoreGraphics, and
calls `/usr/sbin/screencapture -x -o -l <actual-window-id>`. It does not treat an
eframe or winit identifier as a CoreGraphics window number. The application holds
the same scene until this bounded attempt finishes.

Successful optional files are `01-text-preview-native-window.png` and
`02-image-preview-native-window.png`, beside `capture-run.json`. Their metadata
says `capture_kind: native-window`, includes native chrome, and records that the
window shadow was excluded. A failed attempt is recorded as `unavailable` with
its reason; it is never replaced with a drawn titlebar, a different OS frame, or
a client-area image labelled as a full native window.

The helper checks existing screen-recording permission with
[`CGPreflightScreenCaptureAccess`](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()).
It neither requests permission nor edits TCC, picker approvals, runner policy,
or other access controls. GitHub's
[runner-image provisioning source](https://github.com/actions/runner-images/blob/1ede08b87c78b9a8871f4761a20f9b5fd94796c4/images/macos/scripts/build/configure-tccdb-macos.sh)
describes preconfigured capture access, but the deployed image and actual result
remain authoritative. Missing permission, WindowServer visibility, or capture
failure does not invalidate separately successful client-area pixels; the
limitation stays explicit in the artifact.

## Provenance and validation

`capture-run.json` records the native target, source commit and dirty state,
build command/profile/feature, executable hash before and after capture, fixture
and generator hashes, actual OS version, selected runner environment fields,
scene hashes, and optional native-window outcomes. Each scene's JSON records the
process ID, observed entry and snapshot identity, metadata revision, CRC status,
decoded image dimensions, actual pixel dimensions, scale, theme, viewport state,
and the chosen font paths/names/weights/variation coordinates.

Source archives are hashed again after capture. The driver rejects changed
fixtures or executables, wrong process/OS/architecture metadata, missing scene
state, invalid PNG dimensions, and incomplete runs. A uniform renderer image is
rejected before publication. These checks do not replace visual inspection:
review every actual PNG for clipping, legibility, font selection, geometry and
preview content. A dirty or uncommitted source checkout is rejected before the
capture build; a changed revision or dirty state after capture also fails the run.

`app.stdout.log` and `app.stderr.log` are retained even when native startup fails.
The macOS helper has separate build/capture logs. Read `capture-run.json` before
using any image from a failed run. Small `.ready` and `.continue` files are the
driver's scene handshake, not application data.

## CI coverage and limits

The existing six native CI jobs capture both architectures of Linux, Windows and
macOS after their unchanged release package checks. A separate capture-only
`macos-26` ARM64 job supplies current macOS appearance without changing the six
targets or exact release asset inventory. Runner labels are defined by the
[GitHub-hosted runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners);
the artifact records the OS/image version actually used.

Artifacts are named `native-gui-<target>` and
`native-gui-macos-26-aarch64-apple-darwin`. They are kept outside `dist/` and outside
the `arcforge-*` artifact pattern used to assemble release bundles. CI uploads
available diagnostics on failure, but a missing mandatory scene fails its job.
Unit tests and `--version` do not establish a native GUI capture.

The initial workflow deliberately uses the production Glow context on Windows.
A hosted desktop may lack a suitable OpenGL driver; the workflow must surface
that startup failure instead of silently substituting an offscreen renderer or
another backend. A runner label, installed SDK, or successful build alone does
not establish this capability. Do not describe a platform as visually verified
until its actual artifacts have been downloaded and inspected.

After review, keep selected original images and their provenance under the
[repository screenshot convention](../screenshots/README.md). Capture metadata
supports comparison; it does not assert BetterZip fidelity by itself.
