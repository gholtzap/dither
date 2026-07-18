import "react";
import type { DitherPlate } from "./core.js";

declare module "react" {
  interface ImgHTMLAttributes<T> {
    dither?: string;
  }
}

export interface DitherLoadDetail {
  blob: Blob;
  plates: DitherPlate[];
}

declare global {
  interface HTMLElementEventMap {
    ditherload: CustomEvent<DitherLoadDetail>;
    dithererror: CustomEvent<{ error: Error }>;
  }
}

export function enhanceDitherImages(root?: Document | Element): void;
export function observeDitherImages(root?: Document | Element): MutationObserver;
