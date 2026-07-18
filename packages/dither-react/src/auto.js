import {
  ditherImage,
  ditherImageWithPlates,
  parseDitherSpec,
} from "./core.js";

const selector = "img[dither], img[data-dither]";
const states = new WeakMap();
let observer;

export function enhanceDitherImages(root = document) {
  const images = root.matches?.(selector)
    ? [root]
    : Array.from(root.querySelectorAll(selector));
  for (const image of images) enhanceImage(image);
}

export function observeDitherImages(root = document) {
  if (observer) return observer;
  enhanceDitherImages(root);
  observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      if (mutation.type === "attributes") enhanceImage(mutation.target);
      for (const node of mutation.addedNodes) {
        if (node.nodeType === Node.ELEMENT_NODE) enhanceDitherImages(node);
      }
      for (const node of mutation.removedNodes) {
        if (node.nodeType === Node.ELEMENT_NODE) cleanupDitherImages(node);
      }
    }
  });
  observer.observe(root, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: [
      "src",
      "dither",
      "data-dither",
      "data-dither-paper",
      "data-dither-displacement",
      "data-dither-distress",
      "data-dither-plates",
    ],
  });
  return observer;
}

function cleanupDitherImages(root) {
  const images = root.matches?.(selector)
    ? [root]
    : Array.from(root.querySelectorAll(selector));
  for (const image of images) {
    const state = states.get(image);
    state?.controller.abort();
    if (state?.url) URL.revokeObjectURL(state.url);
    states.delete(image);
  }
}

async function enhanceImage(image) {
  const spec = image.getAttribute("dither") ?? image.dataset.dither;
  const assets = {
    paperTexture: image.dataset.ditherPaper,
    displacementMap: image.dataset.ditherDisplacement,
    distressMask: image.dataset.ditherDistress,
  };
  const includePlates = image.hasAttribute("data-dither-plates");
  const settingsKey = JSON.stringify({ spec, assets, includePlates });
  const current = states.get(image);
  if (!spec) {
    if (current) {
      current.controller.abort();
      if (current.url) URL.revokeObjectURL(current.url);
      image.src = current.source;
      states.delete(image);
    }
    return;
  }

  const displayedSource = image.getAttribute("src");
  if (!displayedSource) return;
  if (displayedSource === current?.url && current.settingsKey === settingsKey) return;
  const source = current?.url === displayedSource ? current.source : displayedSource;
  if (current?.source === source && current.settingsKey === settingsKey) return;
  current?.controller.abort();
  if (current?.url === displayedSource) image.src = source;
  if (current?.url) URL.revokeObjectURL(current.url);
  const controller = new AbortController();
  const state = { source, settingsKey, controller, url: undefined };
  states.set(image, state);
  image.setAttribute("aria-busy", "true");
  try {
    const render = includePlates ? ditherImageWithPlates : ditherImage;
    const result = await render(source, parseDitherSpec(spec), {
      signal: controller.signal,
      assets,
    });
    if (states.get(image) !== state) return;
    const blob = includePlates ? result.composite : result;
    state.url = URL.createObjectURL(blob);
    image.src = state.url;
    image.removeAttribute("aria-busy");
    image.dispatchEvent(
      new CustomEvent("ditherload", {
        detail: { blob, plates: includePlates ? result.plates : [] },
      }),
    );
  } catch (error) {
    if (error?.name === "AbortError") return;
    if (states.get(image) === state) states.delete(image);
    image.removeAttribute("aria-busy");
    image.dispatchEvent(new CustomEvent("dithererror", { detail: { error } }));
  }
}

if (typeof document !== "undefined") {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => observeDitherImages(), {
      once: true,
    });
  } else {
    observeDitherImages();
  }
}
