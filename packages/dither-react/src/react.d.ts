import type { ImgHTMLAttributes, ReactElement, RefAttributes } from "react";
import type {
  DitherAlgorithm,
  DitherAssets,
  DitherOptions,
  DitherPlate,
} from "./core.js";

export interface DitherImageProps
  extends Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> {
  src: string;
  algorithm?: DitherAlgorithm;
  brightness?: number;
  contrast?: number;
  gamma?: number;
  options?: DitherOptions;
  assets?: DitherAssets;
  includePlates?: boolean;
  outputType?: string;
  outputQuality?: number;
  onDitherLoad?: (blob: Blob, plates: DitherPlate[]) => void;
  onDitherError?: (error: Error) => void;
}

export const DitherImage: (
  props: DitherImageProps & RefAttributes<HTMLImageElement>,
) => ReactElement;
