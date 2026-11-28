# Interface screenshots

Real ArcForge application captures are grouped by feature. This organization follows the topic folders, numbered scenes, and reproduction notes in [stokaro/kc](https://github.com/stokaro/kc/blob/6c7cb63743c4b043fd48bae71ffe20bd99deeae2/screenshots/README.md). KC images and scripts are not copied.

| Topic | Evidence |
| --- | --- |
| [ZIP extraction](zip-extraction/README.md) | Earlier Linux workspace and extraction states; known visual differences from BetterZip 6 |

## Add or update a scene

1. Create `screenshots/<topic>/README.md` and number captures `01-scene.png`, `02-scene.png`, and so on. For multiple platforms use `screenshots/<topic>/<target>/01-scene.png`. Prefer PNGs from the running native application; preserve the original JPEG format when the capture tool supplies JPEG, and record that encoding. Do not substitute mockups, generated images, composited screens, or drawn reconstructions.
2. Record the source commit or content tree, any local changes, build command, fixture and its creation, OS/architecture, toolkit/backend when known, window dimensions, scale, and theme. Do not claim another platform from a matching build result.
3. Give the actions and visible readiness condition for each scene. Record unstable values instead of hiding them. If capture tooling is added, keep it development-only and ensure it records the actual rendered native application frame after loading, selection and layout settle. Label framebuffer/client-area images as excluding system chrome. Label screenshots including actual OS chrome separately. Neither kind may have another platform's frame composited onto it.
4. Use synthetic or redistributable archives without credentials or private paths. Keep reference-app images as source links unless their redistribution is separately permitted.
5. Capture macOS, Windows and Linux; prioritize macOS when comparing against BetterZip 6. A missing display/session or failed capture makes that platform unverified. Keep the failure evidence and describe the required environment instead of substituting another OS or a headless reconstruction.
6. Inspect every image at native size for readable text, clipping, focus/selection, pane boundaries, overlays, and intended state. Compare toolbar/titlebar, typography, proportions, outline rows, inspector and preview separately against the latest supplied reference. Document actual failures and remaining differences; a broad three-pane match is not visual acceptance.
7. Link the evidence from the PR and [reference inventory](../docs/reference/betterzip-6.md). Screenshots do not prove archive correctness, cancellation, accessibility, native integration, or visual parity on untested platforms.

Keep useful earlier scenes when they explain a behavior change; update the index and notes when replacing obsolete images.
