# Desktop typography and native macOS window

## Actual regular and semibold faces

The default egui proportional font is Ubuntu Light. ArcForge replaces that
primary face with an upright regular font and a separate, genuine semibold
face. `typography::regular(size)` uses the proportional family;
`typography::semibold(size)` uses the named `ArcForge Semibold` family. The latter
is an internal egui family identifier, not a renamed font asset or a synthetic
stroke applied to a regular face.

The desktop calls `typography::prepare()` before starting the native event loop.
It reads a fixed, small list of platform font locations. `install()` applies the
prepared definitions without filesystem work in a UI frame. Library-only callers
that do not prepare system fonts use the embedded pair.

| Platform | Preferred installed family | Selection |
| --- | --- | --- |
| macOS | San Francisco from `SFNS.ttf`, `SFNS.ttc`, `SFNSText.ttf`, or `SFNSDisplay.ttf` in `/System/Library/Fonts` | Upright regular/semibold faces or variable weight instances; actual TTC face indices are preserved |
| Windows | Segoe UI from `segoeui.ttf` and `seguisb.ttf` in the system Fonts directory | Upright weights 400 and 600 |
| Linux and other desktop targets | Noto Sans, Open Sans, or Cantarell from the recorded common locations in `typography.rs` | A complete upright regular/semibold pair; this is a bounded candidate list, not a fontconfig preference resolver |
| Any platform with unavailable or unusable candidates | Embedded Open Sans Regular and Semibold | Verified static weights 400 and 600, including Cyrillic coverage |

System files are read on the user's machine; Apple and Microsoft font files are
not embedded, copied into this repository, or redistributed. The loader requires
both weights from the same candidate family before replacing the embedded pair.
It rejects italic/condensed candidates and incorrect static weights. It inspects
at most 64 faces per file and 256 named instances, and bounds each file read to
32 MiB, including a file that grows while being read. Missing, malformed, or
oversized files fall back to the embedded pair.

For a variable font, the loader uses supported `wght` values 400 and 600. If the
font uses a different coordinate range, it looks for its named Regular/Semibold
instances instead of guessing a face index or a weight coordinate. The `opsz`
axis, when available, is set to 14 within the supported range for this compact
text interface. This is an explicit egui adaptation; it does not claim AppKit's
automatic optical sizing, tracking, or accessibility font selection.

`typography::selection()` exposes each primary font's source path, embedded/system
status, own full-name record, collection index, selected weight, and variation
coordinates for native-capture metadata. A variable font can retain a Regular
full-name record while its selected semibold instance is established by its
weight coordinates. Existing egui symbol fallbacks and the regular monospace
font remain available for their original roles.

The implementation uses the already locked `skrifa 0.44.0` font parser directly;
it does not introduce a second font parser, a system-font enumeration service,
or an unsafe platform bridge. Regression tests check the embedded weights and
Cyrillic glyphs, reject malformed/wrong-weight inputs, and select a semibold face
at a nonzero TTC index without copying the collection's bytes.

## Native macOS titlebar

`apps/arcforge/src/platform.rs` uses the pinned egui/eframe 0.37 viewport API:
full-size content, transparent native titlebar, hidden duplicated native title,
visible native titlebar buttons, and native decorations. egui-winit maps these
settings to the corresponding winit macOS window attributes. The workspace
reserves leading space for the real close/minimize/zoom buttons and owns only its
content and explicit header drag area. Background dragging is disabled so that
ordinary content gestures retain their own meaning.

Windows and Linux retain their native window decorations. ArcForge does not draw
macOS traffic lights on those platforms. Transparent titlebar integration alone
does not implement AppKit Liquid Glass/vibrancy, native toolbar customization,
or every BetterZip window behavior. A renderer capture establishes client pixels;
native macOS window chrome still requires an actual full-window macOS capture.

References:

- [Apple typography guidance](https://developer.apple.com/design/human-interface-guidelines/typography): system fonts and readable regular/semibold weights.
- [egui 0.37 ViewportBuilder](https://docs.rs/egui/0.37.0/egui/struct.ViewportBuilder.html): the native titlebar/full-size-content settings.
- [egui 0.37 FontData](https://docs.rs/egui/0.37.0/egui/struct.FontData.html): collection indices and variable font data.
- [BetterZip 6 reference evidence](betterzip-6.md): the required workspace hierarchy and remaining parity differences.

## Embedded Open Sans provenance

These two unmodified files are the legacy **Open Sans Version 1.10**, distributed
in Debian source package **fonts-open-sans 1.11-2**. The package version and the
font's internal version are different. The font files' own name records identify
their copyright and **Apache License 2.0** terms. A modern Open Sans release's
different license is not used to establish the terms of these files.

Source retrieved and verified on 2026-11-28:
[fixed source package](https://sources.debian.org/src/fonts-open-sans/1.11-2/).
The downloaded font bytes were compared byte-for-byte with the local runtime
copies before inclusion. The original complete license was downloaded from the
same fixed source package. No font outlines, metadata, names, or glyph coverage
were changed.

| Repository file | Original source | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `crates/ui-egui/assets/fonts/OpenSans-Regular.ttf` | [OpenSans-Regular.ttf](https://sources.debian.org/data/main/f/fonts-open-sans/1.11-2/OpenSans-Regular.ttf) | 217360 | `e64e508b2aa2880f907e470c4550980ec4c0694d103a43f36150ac3f93189bee` |
| `crates/ui-egui/assets/fonts/OpenSans-Semibold.ttf` | [OpenSans-Semibold.ttf](https://sources.debian.org/data/main/f/fonts-open-sans/1.11-2/OpenSans-Semibold.ttf) | 221328 | `aa3b0ef53db12e3d45094030cac0e69d384e44cc5978643dd4390041cad546e2` |
| `licenses/fonts/OpenSans-Apache-2.0.txt` | [Apache License.txt](https://sources.debian.org/data/main/f/fonts-open-sans/1.11-2/Apache%20License.txt) | 11323 | `283ea6cc2997a1a70da0049e09adf9317bb60ca1b51279b65196b83a69e1996b` |

Original copyright records:

- Regular: Digitized data copyright © 2010-2011, Google Corporation.
- Semibold: Digitized data copyright © 2011, Google Corporation.

The full license and original copyright notice accompany source and binary
distributions in [licenses/fonts/OpenSans-Apache-2.0.txt](../../licenses/fonts/OpenSans-Apache-2.0.txt)
and [licenses/fonts/OpenSans-NOTICE.txt](../../licenses/fonts/OpenSans-NOTICE.txt).
The font assets retain their Apache-2.0 terms; ArcForge's own code remains MIT.
