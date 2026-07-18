/* tslint:disable */
/* eslint-disable */

export class WebRender {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    composite_rgba(): Uint8Array;
    plate_coverages(): Uint8Array;
    plate_metadata_json(): string;
    readonly height: number;
    readonly width: number;
}

export function dither_document_rgba(rgba: Uint8Array, width: number, height: number, options_json: string, paper_rgba: Uint8Array, paper_width: number, paper_height: number, displacement_rgba: Uint8Array, displacement_width: number, displacement_height: number, distress_rgba: Uint8Array, distress_width: number, distress_height: number): WebRender;

export function dither_rgba(rgba: Uint8Array, width: number, height: number, options_json: string): Uint8Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_webrender_free: (a: number, b: number) => void;
    readonly dither_document_rgba: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number) => [number, number, number];
    readonly dither_rgba: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly webrender_composite_rgba: (a: number) => [number, number];
    readonly webrender_height: (a: number) => number;
    readonly webrender_plate_coverages: (a: number) => [number, number];
    readonly webrender_plate_metadata_json: (a: number) => [number, number];
    readonly webrender_width: (a: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
