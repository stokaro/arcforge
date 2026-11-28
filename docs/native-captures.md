# Native GUI captures

`cargo xtask gui-capture` starts the real ArcForge desktop on its native host,
opens the original UI fixture through the ordinary archive worker, records two
finished previews, stages a deletion, saves a verified ZIP copy, then closes the
archive. On macOS it also captures actual Open and Save As panels. This is maintainer tooling behind
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

The local default directory is `target/native-gui/<target>`. CI writes to a fresh
`RUNNER_TEMP/arcforge-native-gui-<run_id>-<run_attempt>/<target>` directory, with a
`macos-26-` target prefix for the additional macOS job. This keeps evidence outside
the Rust cache: even an empty restored directory would correctly fail the helper's
create-new check. The artifact upload reads the same per-run location.

On Linux, CI starts a real X11 window in Xvfb and requests Mesa software OpenGL:

The X11 runtime also needs `libxkbcommon-x11-0` on Ubuntu. The general
`libxkbcommon-dev` build dependency does not supply that dynamically loaded X11
library; a successful headless build or `--version` check cannot detect its absence.

```sh
LIBGL_ALWAYS_SOFTWARE=1 ARCFORGE_CAPTURE_DISPLAY='Xvfb X11; Mesa software OpenGL requested' \
  xvfb-run -a -s '-screen 0 1600x1000x24' \
  cargo xtask gui-capture --directory target/native-gui/linux-review
```

This is an actual X11 client rendered by the shipping backend in a virtual display.
It does not establish behavior on a physical monitor, a particular GPU, Wayland,
or a desktop window manager. Windows and macOS CI launch directly in their hosted
native sessions; the runtime log and the resulting pixels establish success.

On Windows, CI explicitly prepares the pinned Mesa software OpenGL driver
described below. The window and production Glow/WGL backend remain native;
software OpenGL is recorded as part of the runtime evidence.

Every completed capture contains these client scenes, plus two native file panels on macOS.

| Scene | Actual selected entry | Required worker result |
| --- | --- | --- |
| `client/01-text-preview.png` | `Documents/Guide.txt` | Complete UTF-8 text preview with verified CRC32 |
| `client/11-entry-context-menu.png` | `Documents/Guide.txt` | Secondary-button input opens the selected row's popup |
| `client/10-open-in-default-app.png` | `Documents/Guide.txt` | Normal file-open command succeeded; the temporary copy matches the CRC-verified preview |
| `client/02-image-preview.png` | `Photos/Color-study.png` | Complete verified preview and decoded 960 × 640 PNG texture |
| `client/03-empty-window.png` | None | Closed document, cleared selection/preview, no pending picker |
| `04-open-panel-native-window.png` (macOS) | None | Actual owned `NSOpenPanel`, active native picker and verified window identity |
| `client/05-staged-delete.png` | None | Original document with the image removed, one staged deletion |
| `client/06-discard-prompt.png` | None | Attempt to close opens the unsaved-changes prompt |
| `07-save-panel-native-window.png` (macOS) | None | Actual owned `NSSavePanel` opened for the edited ZIP |
| `client/08-save-collision.png` | None | Save to the original path rejected with the deletion still staged |
| `client/09-saved-copy.png` | None | Clean copy open, only the selected image missing |

The file-open scene uses the same controller action as a double-click. On macOS and Windows the developer harness uses the machine's current text-file association. On Linux it supplies a process-local XDG text handler that copies the file it receives into the evidence directory. The gate compares that copy independently against the fixture with Python's `zipfile`. The capture records the ArcForge workspace after the handoff.

The capture parent gives the ArcForge child a dedicated temporary directory and observes two separate copies of Guide.txt. Both copies must still exist after Close Archive. The parent then waits for the real desktop process to exit normally and fails the run if any ArcForge open-copy directory remains or a neighboring sentinel directory was modified. The `temporary_open_cleanup` fields in `capture-run.json` record the observations.

The harness cancels the inspected Save As panel and submits the collision and
success destinations through the workspace's save controller. It verifies the
new ZIP with Python's independent `zipfile` reader and checks the input SHA-256.

The fixture is generated with `scripts/generate_ui_fixture.py --images`. Its
text and image are original repository content. The driver invokes normal
Outline filtering, selection, and preview commands. It never inserts a fake
snapshot, preview result, archive status, or rendered workspace.

Readiness depends on the selected entry, snapshot identity, finished worker,
matching preview, CRC verification, and a prepared image texture where relevant.
The discard prompt also waits for its rendered area's fade-in to reach full
opacity. The capture feature parks the pointer over blank workspace padding.
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

On macOS the native AppKit shell must attach successfully. The driver additionally
attempts a native window capture for each client scene. It compiles the
repository's small Swift helper, verifies the actual AppKit window number against
on-screen CoreGraphics windows belonging to the launched ArcForge PID, and
calls `/usr/sbin/screencapture -x -o -l <actual-window-id>`. It does not treat an
eframe or winit identifier as a CoreGraphics window number. The application holds
the same scene until this bounded attempt finishes.

Successful workspace files are `01-text-preview-native-window.png`,
`02-image-preview-native-window.png`, and `03-empty-window-native-window.png`,
beside `capture-run.json`. Their metadata
says `capture_kind: native-window`, includes native chrome, and records that the
window shadow was excluded. A failed attempt is recorded as `unavailable` with
its reason; it is never replaced with a drawn titlebar, a different OS frame, or
a client-area image labelled as a full native window.

The helper also accepts an explicit CoreGraphics window ID supplied by the
application for a native dialog. It still requires the requested window to be
on-screen and owned by the launched application's PID. An identified sheet can
be smaller than the minimum document window size, as with the compact Save As
panel on macOS 15. The same ownership, opacity and stable-bounds checks apply.
Without an explicit ID,
the ordinary workspace capture continues to require one unambiguous ordinary
window. A native panel must be captured as an OS window; a client framebuffer
cannot establish its appearance. Each file-panel scene saves a readiness record
without an egui PNG, waits for the actual panel capture, and cancels the owned
panel before advancing. A missing native panel image fails the macOS
capture run. No native dialog image is inferred from a renderer frame.

The helper checks existing screen-recording permission with
[`CGPreflightScreenCaptureAccess`](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()).
It neither requests permission nor edits TCC, picker approvals, runner policy,
or other access controls. GitHub's
[runner-image provisioning source](https://github.com/actions/runner-images/blob/1ede08b87c78b9a8871f4761a20f9b5fd94796c4/images/macos/scripts/build/configure-tccdb-macos.sh)
describes preconfigured capture access, but the deployed image and actual result
remain authoritative. Missing permission, WindowServer visibility, or capture
failure does not invalidate separately successful client-area pixels; the
limitation stays explicit in the artifact.

### macOS CI display size

The first native run exposed a 1024 × 768 macOS runner desktop, which constrained
the actual application window. The helper now queries the main display's
supported modes through
[`CGDisplayCopyAllDisplayModes`](https://developer.apple.com/documentation/coregraphics/quartz-display-services)
and, **only inside GitHub Actions**, requests a larger desktop-usable mode through
`CGBeginDisplayConfiguration`, `CGConfigureDisplayWithDisplayMode`, and
`CGCompleteDisplayConfiguration` with
[`forSession`](https://developer.apple.com/documentation/coregraphics/cgconfigureoption/forsession).
This scope survives the helper process and lasts only for the ephemeral runner's
login session; it does not save permanent display preferences. It prefers at least 1600 × 1000, with a 1440 × 900
minimum, leaving room for the 1280 × 800 application scene and real system UI.
It selects only an advertised mode; it does not create a virtual display, enlarge
a screenshot, or change screen-recording permissions. Local `gui-capture` leaves
the desktop mode alone.

The earlier `CGDisplaySetDisplayMode` attempt reported a larger mode while its
helper was running, but the app still opened at the old size. Apple's
[API contract](https://developer.apple.com/documentation/coregraphics/cgdisplaysetdisplaymode(_:_:_:))
explains why: that change reverts when the calling program exits.

`display_configuration` records the available modes, original mode/bounds,
requested scope/mode, CoreGraphics result, and observed final mode/bounds. The
transition has a two-second bound. Before starting ArcForge, xtask launches a
separate read-only `--describe-desktop` process after the setter has exited and
records `after_helper_exit` plus `persists_after_helper_exit`. A reverted mode or
smaller bounds cannot remain labelled as a successful configuration. If the host exposes no suitable mode or declines
the change, that outcome remains explicit and the actual smaller capture remains
labelled with its true dimensions. The scene PNG and viewport metadata remain the
evidence for the window size actually rendered.

### Windows CI software OpenGL

The first Windows x64 and ARM64 runs both failed native startup with
`egui_glow requires opengl 2.0+`. The headless tests and package verification had
passed because they do not create an OpenGL context. CI now prepares a software
driver for this hosted-machine limitation, without replacing Glow or using an
offscreen UI renderer.

[`prepare_windows_capture_driver.ps1`](../scripts/prepare_windows_capture_driver.ps1)
uses the producer's
[`mmozeiko/build-mesa` source revision](https://github.com/mmozeiko/build-mesa/tree/73f7cfb6d2ec725d1c036c33eba62f420f87818e)
and [Mesa 26.3.1 release](https://github.com/mmozeiko/build-mesa/releases/tag/26.3.1),
which builds llvmpipe with LLVM 23.1.5. The producer documents static WGL DLLs for
both native architectures. The driver archives are pinned by SHA-256:

| Architecture | Release asset | SHA-256 |
| --- | --- | --- |
| x64 | `mesa-llvmpipe-x64-26.3.1.7z` | `08145a3717e0c1be0d97b74b53070f14eb39cb2f7de22f84fe20d1a581299c12` |
| ARM64 | `mesa-llvmpipe-arm64-26.3.1.7z` | `c74c960c0416f3fa7f55dd877c34bbb1408f097cdf20a5638cd10ac1c90ae3c5` |

The script verifies the package digest and the DLL's PE architecture, then places
only `opengl32.dll` beside the debug capture executable. This follows Mesa's
[documented per-application WGL deployment](https://docs.mesa3d.org/drivers/llvmpipe.html#windows).
It changes no system library, driver registration, or permission. Package
verification and upload happen first. Downloaded archives stay in `RUNNER_TEMP`,
the deployed DLL is removed before the Rust cache is saved, and no Mesa binary is
included in `dist/`, release bundles, or screenshot artifacts.

Runtime metadata explicitly records the software driver, upstream source commit,
archive SHA-256, and deployed library path/SHA-256. The recorded library identity
describes the prepared runtime; successful native pixels still require the actual
window/context to start. This evidence does not establish hardware GPU behavior.

The external runtime has its own licenses. Mesa's
[license overview](https://docs.mesa3d.org/license.html) identifies its core as MIT
and directs readers to component SPDX identifiers and the source `licenses/`
directory; it does not describe every component as identically licensed. LLVM's
[23.1.5 license](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.5/llvm/LICENSE.TXT)
is Apache 2.0 with LLVM exceptions and retains its additional component notices.
These terms apply to the CI toolchain runtime, independently of ArcForge's MIT
license. A future decision to distribute that runtime with the product requires
its own complete source/notice inventory.

## Provenance and validation

`capture-run.json` records the native target, source commit and dirty state,
build command/profile/feature, executable hash before and after capture, fixture
and generator hashes, actual OS version, selected runner environment fields,
scene hashes, and native-window outcomes. Scene JSON uses schema version 2.
Preview records include process ID, observed entry and snapshot identity, metadata
revision, CRC status and decoded image dimensions. Empty records explicitly have
no document, selection or pending picker. Edit records identify the archive,
the pending deletion count, an open prompt or picker, and the observed error.
Native file-panel records identify the actual AppKit class, panel and parent window.
Client records include actual pixel dimensions, scale, theme, viewport state and
font paths/names/weights/variation coordinates.

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

The workflow deliberately retains the production Glow context on every OS.
Runtime failures remain visible; a runner label, installed SDK, successful build,
or prepared driver alone does not establish window rendering. Do not describe a
platform as visually verified until its actual artifacts have been downloaded
and inspected.

After review, keep selected original images and their provenance under the
[repository screenshot convention](../screenshots/README.md). Capture metadata
supports comparison; it does not assert BetterZip fidelity by itself.
