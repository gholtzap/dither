const OPTION_NAMES = new Set([
  "preset",
  "algorithm",
  "strength",
  "seed",
  "brightness",
  "contrast",
  "gamma",
  "blur",
  "sharpen",
  "blackPoint",
  "whitePoint",
  "denoise",
  "invert",
  "threshold",
  "softness",
  "grain",
  "grainScale",
  "paper",
  "paperScale",
]);

let worker;
let nextRequestId = 1;
const pending = new Map();

export function parseDitherSpec(spec) {
  const value = String(spec).trim();
  if (value.startsWith("{")) return JSON.parse(value);
  const options = {};
  for (const [index, rawPart] of value.split(",").entries()) {
    const part = rawPart.trim();
    if (!part) continue;
    const separator = part.indexOf("=");
    if (separator === -1) {
      if (index !== 0 || options.algorithm) {
        throw new Error(`Invalid dither option: ${part}`);
      }
      options.algorithm = part;
      continue;
    }
    const rawName = part.slice(0, separator).trim();
    const name = rawName.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    if (!OPTION_NAMES.has(name)) {
      throw new Error(`Unknown dither option: ${rawName}`);
    }
    const rawValue = part.slice(separator + 1).trim();
    if (name === "algorithm" || name === "preset") {
      options[name] = rawValue;
    } else if (rawValue === "true" || rawValue === "false") {
      options[name] = rawValue === "true";
    } else {
      const value = Number(rawValue);
      if (!Number.isFinite(value)) {
        throw new Error(`${rawName} must be a number or boolean`);
      }
      options[name] = value;
    }
  }
  return options;
}

export async function ditherImage(source, options = {}, renderOptions = {}) {
  return (await renderImage(source, options, renderOptions, false)).composite;
}

export async function ditherImageWithPlates(
  source,
  options = {},
  renderOptions = {},
) {
  return renderImage(source, options, renderOptions, true);
}

async function renderImage(source, options, renderOptions, includePlates) {
  if (typeof document === "undefined") {
    throw new Error("ditherImage must run in a browser");
  }
  const { signal, type = "image/png", quality, assets } = renderOptions;
  throwIfAborted(signal);
  ensureWorker();
  const [input, assetPixels] = await Promise.all([
    readPixels(source, signal),
    readAssets(assets, signal),
  ]);
  const output = await processPixels(input, options, assetPixels, signal);
  const composite = await rgbaBlob(
    output.rgba,
    output.width,
    output.height,
    type,
    quality,
    signal,
  );
  const result = { composite, plates: [] };
  if (!includePlates) return result;
  const pixelsPerPlate = output.width * output.height;
  for (const [index, metadata] of output.plateMetadata.entries()) {
    const coverage = new Uint8Array(
      output.plateCoverages,
      index * pixelsPerPlate,
      pixelsPerPlate,
    );
    result.plates.push({
      ...metadata,
      blob: await rgbaBlob(
        coverageToRgba(coverage),
        output.width,
        output.height,
        type,
        quality,
        signal,
      ),
    });
  }
  return result;
}

function ensureWorker() {
  if (worker) return worker;
  if (typeof Worker === "undefined") {
    throw new Error("Web Workers are unavailable in this browser");
  }
  worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
  worker.addEventListener("message", ({ data }) => {
    const request = pending.get(data.id);
    if (!request) return;
    pending.delete(data.id);
    request.signal?.removeEventListener("abort", request.abort);
    if (data.error) request.reject(new Error(data.error));
    else request.resolve(data);
  });
  worker.addEventListener("error", ({ message }) => {
    const error = new Error(message || "Dither worker failed");
    for (const request of pending.values()) {
      request.signal?.removeEventListener("abort", request.abort);
      request.reject(error);
    }
    pending.clear();
    worker?.terminate();
    worker = undefined;
  });
  return worker;
}

function processPixels(input, options, assets, signal) {
  throwIfAborted(signal);
  const id = nextRequestId++;
  const bytes = new Uint8Array(input.rgba);
  const optionsJson = JSON.stringify(options);
  const transfers = [bytes.buffer];
  for (const asset of Object.values(assets)) transfers.push(asset.rgba.buffer);
  return new Promise((resolve, reject) => {
    const abort = () => {
      pending.delete(id);
      reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
    };
    pending.set(id, { resolve, reject, signal, abort });
    signal?.addEventListener("abort", abort, { once: true });
    try {
      ensureWorker().postMessage(
        {
          id,
          rgba: bytes.buffer,
          width: input.width,
          height: input.height,
          optionsJson,
          assets,
        },
        transfers,
      );
    } catch (error) {
      pending.delete(id);
      signal?.removeEventListener("abort", abort);
      reject(error);
    }
  });
}

async function readAssets(assets = {}, signal) {
  const entries = await Promise.all(
    ["paperTexture", "displacementMap", "distressMask"].map(async (name) => [
      name,
      assets[name]
        ? await readPixels(assets[name], signal)
        : { rgba: new Uint8Array(), width: 0, height: 0 },
    ]),
  );
  return Object.fromEntries(entries);
}

async function readPixels(source, signal) {
  const blob =
    source instanceof Blob ? source : await fetchImageBlob(String(source), signal);
  const image = await decodeImage(blob, signal);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("Canvas 2D is unavailable");
    context.drawImage(image, 0, 0);
    return {
      rgba: new Uint8Array(
        context.getImageData(0, 0, canvas.width, canvas.height).data,
      ),
      width: canvas.width,
      height: canvas.height,
    };
  } finally {
    image.close?.();
  }
}

async function fetchImageBlob(source, signal) {
  const response = await fetch(source, { signal });
  if (!response.ok) {
    throw new Error(`Unable to load image: ${response.status} ${response.statusText}`);
  }
  return response.blob();
}

async function decodeImage(blob, signal) {
  throwIfAborted(signal);
  if (typeof createImageBitmap === "function") return createImageBitmap(blob);
  const url = URL.createObjectURL(blob);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    throwIfAborted(signal);
    return image;
  } finally {
    URL.revokeObjectURL(url);
  }
}

function rgbaBlob(rgba, width, height, type, quality, signal) {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return Promise.reject(new Error("Canvas 2D is unavailable"));
  context.putImageData(
    new ImageData(new Uint8ClampedArray(rgba), width, height),
    0,
    0,
  );
  return new Promise((resolve, reject) => {
    throwIfAborted(signal);
    canvas.toBlob((blob) => {
      if (signal?.aborted) {
        reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
      } else if (blob) {
        if (blob.type !== type) {
          reject(new Error(`Browser encoded ${blob.type} instead of ${type}`));
        } else {
          resolve(blob);
        }
      } else {
        reject(new Error(`Browser cannot encode ${type}`));
      }
    }, type, quality);
  });
}

function coverageToRgba(coverage) {
  const rgba = new Uint8ClampedArray(coverage.length * 4);
  for (let index = 0; index < coverage.length; index++) {
    const offset = index * 4;
    rgba[offset] = coverage[index];
    rgba[offset + 1] = coverage[index];
    rgba[offset + 2] = coverage[index];
    rgba[offset + 3] = 255;
  }
  return rgba;
}

function throwIfAborted(signal) {
  if (signal?.aborted) {
    throw signal.reason ?? new DOMException("Aborted", "AbortError");
  }
}
