# Interface screenshots

Real ArcForge application captures are grouped by feature. This organization follows the topic folders, numbered scenes, and reproduction notes in [stokaro/kc](https://github.com/stokaro/kc/blob/6c7cb63743c4b043fd48bae71ffe20bd99deeae2/screenshots/README.md). KC images and scripts are not copied.

| Topic | Evidence |
| --- | --- |
| [ZIP extraction](zip-extraction/README.md) | BetterZip 6 workspace, selection, extraction destination and result states |

## Add or update a scene

1. Create `screenshots/<topic>/README.md` and number captures `01-scene.png`, `02-scene.png`, and so on. Prefer PNGs from the running native application; preserve the original JPEG format when the capture tool supplies JPEG, and record that encoding. Do not substitute mockups, generated images, composited screens, or drawn reconstructions.
2. Record the source commit or content tree, any local changes, build command, fixture and its creation, OS/architecture, toolkit/backend when known, window dimensions, scale, and theme. Do not claim another platform from a matching build result.
3. Give the actions and visible readiness condition for each scene. Record unstable values instead of hiding them. If capture tooling is added, keep it development-only and ensure it records the actual application frame.
4. Use synthetic or redistributable archives without credentials or private paths. Keep reference-app images as source links unless their redistribution is separately permitted.
5. Inspect each image at its native size for readable text, clipping, focus/selection, pane boundaries, overlays, and the intended state. Document relevant limitations and outstanding reference differences.
6. Link the evidence from the PR and [reference inventory](../docs/reference/betterzip-6.md). Screenshots do not prove archive correctness, cancellation, accessibility, native integration, or visual parity on untested platforms.

Keep useful earlier scenes when they explain a behavior change; update the index and notes when replacing obsolete images.
