import {
  ditherImage,
  ditherImageWithPlates,
  parseDitherSpec,
} from "../src/core.js";
import "../src/auto.js";
import { DitherImage } from "../src/react.js";

const component = (
  <DitherImage
    src="/photo.jpg"
    alt="Dithered portrait"
    algorithm="bayer2x2"
    brightness={0.1}
    options={{ contrast: 1.2, grain: 0.08 }}
    assets={{ paperTexture: "/paper.png" }}
    includePlates
    onDitherLoad={(blob, plates) => void [blob, plates]}
  />
);
const options = parseDitherSpec("bayer2x2,brightness=0.1");
const rendered = ditherImage("/photo.jpg", options);
const separated = ditherImageWithPlates(
  "/photo.jpg",
  {
    recipe: {
      separation: {
        mode: "cmyk",
        inks: [{ angleDegrees: 15 }, { angleDegrees: 75 }],
      },
      print: { dpi: 600, lpi: 75 },
      glow: { enabled: true, tint: [1, 0.8, 0.4] },
      displacement: { enabled: true, pattern: "imported", xStrength: 4 },
      crt: { enabled: true, phase: "flux", scanlines: 0.2 },
    },
  },
  { assets: { displacementMap: "/map.png" } },
);
const automatic = (
  <img
    src="/photo.jpg"
    alt="Automatically dithered portrait"
    dither="bayer2x2,brightness=0.1"
  />
);

void component;
void rendered;
void separated;
void automatic;
