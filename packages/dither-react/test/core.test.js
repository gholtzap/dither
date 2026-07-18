import assert from "node:assert/strict";
import test from "node:test";

import { parseDitherSpec } from "../src/core.js";

test("parses the declarative image syntax", () => {
  assert.deepEqual(
    parseDitherSpec("bayer2x2,brightness=0.1,invert=true,grain-scale=2"),
    {
      algorithm: "bayer2x2",
      brightness: 0.1,
      invert: true,
      grainScale: 2,
    },
  );
  assert.throws(() => parseDitherSpec("bayer2x2,unknown=1"));
  assert.throws(() => parseDitherSpec("bayer2x2,brightness=bright"));
  assert.deepEqual(parseDitherSpec("preset=CMYK print"), {
    preset: "CMYK print",
  });
  assert.deepEqual(
    parseDitherSpec('{"recipe":{"separation":{"mode":"tri-tone"}}}'),
    { recipe: { separation: { mode: "tri-tone" } } },
  );
});
