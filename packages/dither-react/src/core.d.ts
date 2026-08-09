export type RGB = [number, number, number];
export type Offset = [number, number];

export type DitherAlgorithm =
  | "bayer2x2"
  | "bayer4x4"
  | "bayer8x8"
  | "floyd-steinberg"
  | "atkinson"
  | "sierra-lite"
  | "sierra-two-row"
  | "sierra"
  | "stucki"
  | "burkes"
  | "jjn"
  | "blue-noise"
  | "modulation"
  | "dot"
  | "line"
  | "cross"
  | "diamond"
  | "clustered-dot";

export type DitherPreset =
  | "None"
  | "Pixelate"
  | "Dither"
  | "ASCII"
  | "Halftone"
  | "CMYK"
  | "Dot Matrix"
  | "Risograph"
  | "Mosaic"
  | "Bricks"
  | "Pointillism"
  | "Heatmap"
  | "Threshold"
  | "Duotone"
  | "Outline"
  | "Posterize"
  | "Classic diffusion"
  | "Newspaper screen"
  | "Dry Xerox"
  | "Modulated bitmap"
  | "Retro five-color"
  | "Warm poster"
  | "Dream glow"
  | "CRT waveform"
  | "CRT linear"
  | "CRT flux"
  | "Grunge displacement"
  | "CMYK print";

export interface InkOptions {
  enabled?: boolean;
  color?: RGB;
  offset?: Offset;
  angleDegrees?: number;
  bleedPixels?: number;
  trappingPixels?: number;
}

export interface TextureOptions {
  amount?: number;
  scale?: number;
  seed?: number;
}

export interface ToneBandOptions {
  range?: [number, number];
  ink?: InkOptions;
  intensity?: number;
  grain?: TextureOptions;
}

interface ThresholdOptions {
  threshold?: number;
  softness?: number;
}

interface PaletteOptions {
  colors?: RGB[];
  inks?: InkOptions[];
  size?: number;
}

export type SeparationOptions =
  | ({ mode: "monochrome"; ink?: InkOptions } & ThresholdOptions)
  | ({ mode: "cmy" | "three-color" | "rgb"; inks?: InkOptions[] } & ThresholdOptions)
  | { mode: "cmyk"; inks?: InkOptions[] }
  | ({ mode: "tonal" | "indexed" | "custom" } & PaletteOptions)
  | {
      mode: "tri-tone";
      shadows?: ToneBandOptions;
      midtones?: ToneBandOptions;
      highlights?: ToneBandOptions;
    };

export interface DitherRecipe {
  bypass?: boolean;
  separation?: SeparationOptions;
  dither?: {
    algorithm?: DitherAlgorithm;
    strength?: number;
    seed?: number;
  };
  resampling?: "nearest" | "bilinear" | "supersample2x";
  preprocess?: {
    brightness?: number;
    contrast?: number;
    gamma?: number;
    blur?: number;
    sharpen?: number;
    blackPoint?: number;
    whitePoint?: number;
    denoise?: number;
    invert?: boolean;
  };
  stylize?: {
    effect?:
      | "none"
      | "pixelate"
      | "ascii"
      | "dot-matrix"
      | "mosaic"
      | "bricks"
      | "pointillism"
      | "heatmap"
      | "outline";
    cellSize?: number;
    amount?: number;
    seed?: number;
  };
  print?: {
    dpi?: number;
    lpi?: number;
    bleedPixels?: number;
    trappingPixels?: number;
  };
  glow?: {
    enabled?: boolean;
    threshold?: number;
    radius?: number;
    falloff?: number;
    intensity?: number;
    tint?: RGB;
    gamma?: number;
    saturation?: number;
  };
  displacement?: {
    enabled?: boolean;
    xStrength?: number;
    yStrength?: number;
    distressAmount?: number;
    pattern?: "imported" | "grain" | "halftone" | "grunge" | "splatter";
    patternScale?: number;
    seed?: number;
  };
  crt?: {
    enabled?: boolean;
    phase?: "waveform" | "linear" | "flux";
    waveStrength?: number;
    waveFrequency?: number;
    scanlines?: number;
    rgbBleed?: number;
    syncTearing?: number;
    phosphorMask?: number;
    bloom?: number;
    seed?: number;
  };
  grain?: TextureOptions;
  paper?: TextureOptions;
  paperColor?: RGB;
}

export interface DitherOptions {
  preset?: DitherPreset;
  recipe?: DitherRecipe;
  algorithm?: DitherAlgorithm;
  strength?: number;
  seed?: number;
  brightness?: number;
  contrast?: number;
  gamma?: number;
  blur?: number;
  sharpen?: number;
  blackPoint?: number;
  whitePoint?: number;
  denoise?: number;
  invert?: boolean;
  threshold?: number;
  softness?: number;
  grain?: number;
  grainScale?: number;
  paper?: number;
  paperScale?: number;
}

export type ImageSource = string | URL | Blob;

export interface DitherAssets {
  paperTexture?: ImageSource;
  displacementMap?: ImageSource;
  distressMask?: ImageSource;
}

export interface DitherRenderOptions {
  signal?: AbortSignal;
  type?: string;
  quality?: number;
  assets?: DitherAssets;
}

export interface DitherPlate {
  name: string;
  ink: Required<InkOptions>;
  blob: Blob;
}

export interface DitherResult {
  composite: Blob;
  plates: DitherPlate[];
}

export function parseDitherSpec(spec: string): DitherOptions;
export function ditherImage(
  source: ImageSource,
  options?: DitherOptions,
  renderOptions?: DitherRenderOptions,
): Promise<Blob>;
export function ditherImageWithPlates(
  source: ImageSource,
  options?: DitherOptions,
  renderOptions?: DitherRenderOptions,
): Promise<DitherResult>;
