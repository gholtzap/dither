import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  dither_document_rgba,
  dither_rgba,
  initSync,
} from "../wasm/dither_web.js";

initSync({
  module: readFileSync(new URL("../wasm/dither_web_bg.wasm", import.meta.url)),
});

test("browser WebAssembly renders RGBA with the shared engine", () => {
  const input = Uint8Array.from([0, 0, 0, 255, 255, 255, 255, 128]);
  const output = dither_rgba(
    input,
    2,
    1,
    JSON.stringify({ algorithm: "bayer2x2", brightness: 0.1 }),
  );
  assert.equal(output.length, input.length);
  assert.equal(output[3], 255);
  assert.equal(output[7], 128);
});

test("browser WebAssembly renders full recipes, assets, and plates", () => {
  const input = Uint8Array.from([32, 96, 160, 255, 220, 120, 40, 255]);
  const asset = Uint8Array.from([128, 128, 128, 255]);
  const rendered = dither_document_rgba(
    input,
    2,
    1,
    JSON.stringify({
      recipe: {
        separation: { mode: "cmyk" },
        resampling: "supersample2x",
        glow: { enabled: true, radius: 2 },
        displacement: { enabled: true, pattern: "imported", xStrength: 1 },
        crt: { enabled: true, phase: "flux", scanlines: 0.2 },
      },
    }),
    asset,
    1,
    1,
    asset,
    1,
    1,
    asset,
    1,
    1,
  );
  assert.equal(rendered.composite_rgba().length, input.length);
  assert.equal(rendered.plate_coverages().length, 8);
  assert.equal(JSON.parse(rendered.plate_metadata_json()).length, 4);
  rendered.free();
});
