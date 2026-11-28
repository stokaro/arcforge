# macOS presentation boundary

This crate installs a real AppKit `NSToolbar`, `NSSearchToolbarItem`, and application
`NSMenu` on the existing eframe window. AppKit draws and positions the real window
controls, native toolbar materials, grouped controls, search field, menus and
overflow. The renderer draws the archive workspace below that chrome. This is a
native shell adapter, not a claim that the remaining egui table and inspector are
native AppKit controls or that visual parity has passed.

The safe public interface passes owned `Command` values and a `State` snapshot.
The crate does not import egui, the archive provider or the worker controller.
Unsupported archive operations remain disabled. Normal Open is File → Open and
Command+O; the toolbar follows the supplied BetterZip 6 order beginning with
Extract. No BetterZip or Apple image assets are bundled: symbol images are requested
from the installed operating system at runtime.

## Audited unsafe scope

All unsafe code is confined to `src/macos.rs`. Other modules deny unsafe code;
other workspace crates continue to forbid it. The exception is needed for objc2's
explicit declarations of Objective-C callback ABI and target-action selectors.

- The custom class subclasses `NSObject`, has no custom `Drop`, and is
  `MainThreadOnly`. Its toolbar delegate methods have the exact signatures in
  pinned objc2-app-kit 0.3.2. It implements no window or application delegate, so
  winit retains ownership of its event loop, window lifecycle and content view.
- The one `msg_send!` invokes `NSObject.init` on the allocated custom class only
  after Rust ivars have been initialized. No raw window pointer is dereferenced.
  Window discovery compares the borrowed raw NSView address with content views
  obtained through safe AppKit calls, then retains the exact matching NSWindow.
- Target-action setters connect only retained instances of that custom class to
  methods with the documented sender type, or documented AppKit responder-chain
  actions. The toolbar callback safely downcasts its object sender because a
  native overflow representation may supply a menu item. Explicit overflow menu
  representations retain the same command tags. Menu and toolbar tags are checked
  before they become Rust commands.
  Delegates and action targets are weak in AppKit. `NativeShell` retains the target
  until its toolbar is detached and its menus are restored. Rust queue borrows are
  released before invoking the repaint callback or reentering AppKit.
- Standard Edit actions first use AppKit's responder chain, preserving native
  search-field and sheet editing. Only an unhandled action enters the Rust queue
  for egui's clipboard/selection commands.
- Capture-only sheet cancellation checks that the attached window is an
  `NSOpenPanel`, that its sheet parent is this exact retained NSWindow, and that
  the requested window number still matches. It sends the documented `cancel:`
  action to that panel only. It never searches for or cancels unrelated dialogs.

The adapter does no archive processing or filesystem I/O, replaces no native
content view, and creates no background AppKit objects. `NativeShell` is neither
`Send` nor `Sync`. It must live for the desktop window's lifetime and be dropped on
the main thread. The modern shell is installed only on macOS 11 and later; the
portable frontend remains available if attachment is unavailable.

## Evidence and validation

Implementation uses the already pinned objc2 0.6.5 and AppKit/Foundation 0.3.2 API
definitions. Primary API references:

- [NSToolbar](https://developer.apple.com/documentation/appkit/nstoolbar)
- [Unified toolbar style](https://developer.apple.com/documentation/appkit/nswindow/toolbarstyle-swift.enum/unified)
- [NSSearchToolbarItem](https://developer.apple.com/documentation/appkit/nssearchtoolbaritem)
- [NSToolbarDelegate](https://developer.apple.com/documentation/appkit/nstoolbardelegate)
- [objc2 class declaration safety](https://docs.rs/objc2/0.6.5/objc2/macro.define_class.html#safety)

The supplied BetterZip captures remain external reference evidence; see
`docs/reference/betterzip-6.md`. macOS compilation and actual native window/menu/
Open-sheet captures are required to validate this boundary. A Linux test run or a
renderer-only image does not establish native chrome correctness.
