# Dither

<img width="1512" height="962" alt="Screenshot 2026-07-18 at 7 17 45 PM" src="https://github.com/user-attachments/assets/1fa6eeab-a3b0-4023-9537-8ea359fafe1d" />

**Dither is a free alternative to spending hundreds of dollars assembling comparable
print effects, plugins, and texture tools from shops like [Doron Supply](https://www.doronsupply.com/shop?category=assets%3Aall-assets).**

It is a desktop-first, non-destructive image dithering and print-separation editor
with a shared Rust rendering engine.

## Desktop app

Dither is for preparing photographs and artwork for risograph, screen-print,
halftone, photocopy, and deliberately low-fidelity digital output. Open an image,
adjust a live preview, inspect the generated ink plates, then export the result at
the source image's full resolution. Edits are stored as recipes; the source file is
never overwritten.

The editor provides:

- ordered, error-diffusion, blue-noise, modulation, and shape-based halftone dithering;
- monochrome, tonal, indexed/custom-palette, RGB, CMY, CMYK, and tri-tone separations;
- brightness, contrast, gamma, levels, blur, sharpen, denoise, and inversion controls;
- print controls for DPI, LPI, bleed, trapping, ink colors, and individual plate setup;
- optional paper texture, displacement, distress, glow, CRT, grain, and surface effects;
- tabs, undo/redo, comparison snapshots, reusable recipe files, and recoverable projects;
- a folder browser with thumbnails, recent/favorite folders, watched-folder refresh, and
  batch export; and
- lossless 16-bit PNG/TIFF or 32-bit OpenEXR export, with optional separate plate files.

Common raster formats and many camera RAW formats can be opened. The Plates panel
shows the composite plus each grayscale mask and ink preview. The Output panel names
and previews export destinations, renders from the original full-resolution image,
tracks export history, and asks before any required metadata loss or bit-depth
reduction.

On macOS, run the development build with:

```sh
./rebuild-run.sh
```

## Web

The web package runs the Rust engine in WebAssembly inside a Web Worker. Images are
processed locally in the browser; consumers do not need Rust or an image-processing
server.

### Install

Install the published package:

```sh
npm install @gavinholtzapple/dither
```

### React component

```jsx
import { DitherImage } from "@gavinholtzapple/dither/react";

<DitherImage
  src="/photo.jpg"
  alt="Dithered artwork"
  options={{
    recipe: {
      separation: { mode: "cmyk" },
      dither: { algorithm: "dot", strength: 1 },
      preprocess: { brightness: 0.1, contrast: 1.2 },
      print: { dpi: 600, lpi: 75 },
    },
  }}
/>
```

### Declarative image attribute

Import the automatic enhancer once, then use the `dither` attribute on normal images:

```jsx
import "@gavinholtzapple/dither/auto";

<img
  src="/photo.jpg"
  alt="Dithered artwork"
  dither="bayer2x2,brightness=0.1,contrast=1.2"
/>
```

### Programmatic rendering and plates

```js
import { ditherImageWithPlates } from "@gavinholtzapple/dither";

const { composite, plates } = await ditherImageWithPlates(file, {
  recipe: {
    separation: { mode: "cmyk" },
    dither: { algorithm: "dot" },
  },
});

const url = URL.createObjectURL(composite);
image.src = url;
// Call URL.revokeObjectURL(url) when the image is no longer needed.
```

`plates` contains one grayscale `Blob` per enabled ink. Imported paper textures,
displacement maps, and distress masks are passed through the third argument's `assets`
property. Remote source images must permit CORS.
