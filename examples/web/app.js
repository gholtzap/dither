import { ditherImageWithPlates } from "../../packages/dither-react/src/core.js";

const algorithms = [
  "bayer2x2", "bayer4x4", "bayer8x8", "floyd-steinberg", "atkinson",
  "sierra-lite", "sierra-two-row", "sierra", "stucki", "burkes", "jjn",
  "blue-noise", "modulation", "dot", "line", "cross", "diamond", "clustered-dot",
];
const presets = [
  "", "Classic diffusion", "Newspaper screen", "Dry Xerox", "Modulated bitmap",
  "Retro five-color", "Warm poster", "Dream glow", "CRT waveform", "CRT linear",
  "CRT flux", "Grunge displacement", "CMYK print",
];
const groups = {
  recipeControls: [
    { id: "preset", label: "Built-in preset", type: "select", options: presets, value: "" },
  ],
  ditherControls: [
    { id: "algorithm", label: "Algorithm", type: "select", options: algorithms, value: "floyd-steinberg" },
    { id: "resampling", label: "Resampling", type: "select", options: ["nearest", "bilinear", "supersample2x"], value: "bilinear" },
    { id: "ditherStrength", label: "Strength", min: 0, max: 1, step: .05, value: 1 },
    { id: "ditherSeed", label: "Blue-noise seed", type: "number", min: 0, value: 7 },
  ],
  preprocessControls: [
    { id: "brightness", label: "Brightness", min: -1, max: 1, step: .05, value: 0 },
    { id: "contrast", label: "Contrast", min: 0, max: 3, step: .05, value: 1 },
    { id: "gamma", label: "Gamma", min: .1, max: 4, step: .05, value: 1 },
    { id: "blur", label: "Blur radius", min: 0, max: 32, step: .25, value: 0 },
    { id: "sharpen", label: "Sharpen", min: 0, max: 3, step: .05, value: 0 },
    { id: "blackPoint", label: "Black point", min: 0, max: .99, step: .01, value: 0 },
    { id: "whitePoint", label: "White point", min: .01, max: 1.5, step: .01, value: 1 },
    { id: "denoise", label: "Denoise", min: 0, max: 1, step: .05, value: 0 },
    { id: "invert", label: "Invert", type: "checkbox", value: false },
  ],
  printControls: [
    { id: "dpi", label: "DPI", type: "number", min: 36, max: 2400, value: 300 },
    { id: "lpi", label: "LPI", type: "number", min: 5, max: 300, value: 45 },
    { id: "printBleed", label: "Bleed pixels", type: "number", min: 0, max: 16, value: 0 },
    { id: "printTrapping", label: "Trapping pixels", type: "number", min: 0, max: 16, value: 0 },
  ],
  glowControls: [
    { id: "glowEnabled", label: "Enable glow", type: "checkbox", value: false },
    { id: "glowThreshold", label: "Threshold", min: 0, max: 1, step: .01, value: .7 },
    { id: "glowRadius", label: "Radius", min: 0, max: 64, step: .5, value: 12 },
    { id: "glowFalloff", label: "Falloff", min: 1, max: 4, step: .05, value: 2 },
    { id: "glowIntensity", label: "Intensity", min: 0, max: 4, step: .05, value: .5 },
    { id: "glowTint", label: "Tint", type: "color", value: "#ffcc8c" },
    { id: "glowGamma", label: "Gamma", min: .1, max: 4, step: .05, value: 1 },
    { id: "glowSaturation", label: "Saturation", min: 0, max: 3, step: .05, value: 1 },
  ],
  displacementControls: [
    { id: "displacementEnabled", label: "Enable displacement", type: "checkbox", value: false },
    { id: "mapPattern", label: "Pattern", type: "select", options: ["imported", "grain", "halftone", "grunge", "splatter"], value: "imported" },
    { id: "mapScale", label: "Pattern scale", min: 2, max: 256, step: 1, value: 18 },
    { id: "mapSeed", label: "Pattern seed", type: "number", min: 0, value: 19 },
    { id: "xStrength", label: "X strength", min: -128, max: 128, step: 1, value: 0 },
    { id: "yStrength", label: "Y strength", min: -128, max: 128, step: 1, value: 0 },
    { id: "distressAmount", label: "Distress", min: 0, max: 1, step: .01, value: 0 },
  ],
  crtControls: [
    { id: "crtEnabled", label: "Enable CRT", type: "checkbox", value: false },
    { id: "crtPhase", label: "Phase", type: "select", options: ["waveform", "linear", "flux"], value: "waveform" },
    { id: "waveStrength", label: "Wave strength", min: 0, max: 128, step: 1, value: 0 },
    { id: "waveFrequency", label: "Wave frequency", min: .1, max: 64, step: .1, value: 8 },
    { id: "scanlines", label: "Scanlines", min: 0, max: 1, step: .01, value: 0 },
    { id: "rgbBleed", label: "RGB bleed", min: 0, max: 24, step: .1, value: 0 },
    { id: "syncTearing", label: "Sync tearing", min: 0, max: 128, step: 1, value: 0 },
    { id: "phosphorMask", label: "Phosphor mask", min: 0, max: 1, step: .01, value: 0 },
    { id: "crtBloom", label: "Bloom", min: 0, max: 2, step: .01, value: 0 },
    { id: "crtSeed", label: "CRT seed", type: "number", min: 0, value: 13 },
  ],
  surfaceControls: [
    { id: "grainAmount", label: "Grain", min: 0, max: .8, step: .01, value: .08 },
    { id: "grainScale", label: "Grain scale", min: .25, max: 12, step: .05, value: 1.2 },
    { id: "grainSeed", label: "Grain seed", type: "number", min: 0, value: 2 },
    { id: "paperAmount", label: "Paper", min: 0, max: .5, step: .01, value: .12 },
    { id: "paperScale", label: "Paper scale", min: .5, max: 24, step: .1, value: 3 },
    { id: "paperSeed", label: "Paper seed", type: "number", min: 0, value: 1 },
    { id: "paperColor", label: "Paper color", type: "color", value: "#efeadd" },
  ],
  outputControls: [
    { id: "outputType", label: "Format", type: "select", options: [["image/png", "PNG"], ["image/webp", "WebP"], ["image/jpeg", "JPEG"]], value: "image/png" },
    { id: "outputQuality", label: "Quality", min: 0, max: 1, step: .05, value: .92 },
    { id: "showPlates", label: "Show plate masks", type: "checkbox", value: true },
  ],
};

for (const [container, fields] of Object.entries(groups)) {
  addFields(document.querySelector(`#${container}`), fields);
}
addFields(document.querySelector("#separationBase"), [
  { id: "separationMode", label: "Mode", type: "select", options: [["monochrome", "Monochrome"], ["cmy", "CMY plates"], ["rgb", "RGB plates"], ["cmyk", "CMYK plates"], ["tonal", "Tonal gradient"], ["indexed", "Extracted indexed"], ["custom", "Custom palette"], ["tri-tone", "Tri-tone Xerox"]], value: "monochrome" },
  { id: "threshold", label: "Threshold", min: 0, max: 1, step: .01, value: .5 },
  { id: "softness", label: "Tonal width", min: .01, max: 1, step: .01, value: .5 },
  { id: "paletteSize", label: "Palette size", type: "number", min: 2, max: 64, value: 8 },
  { id: "extractPalette", label: "Extract colors", type: "checkbox", value: true },
]);

const form = document.querySelector("#controls");
const original = document.querySelector("#original");
const effect = document.querySelector("#effect");
const status = document.querySelector("#status");
const spec = document.querySelector("#spec");
const plateContainer = document.querySelector("#plates");
const downloadComposite = document.querySelector("#downloadComposite");
const assets = {};
let source;
let sourceUrl;
let outputUrls = [];
let controller;
let timer;
let loadedOptions;

const recipeButtons = document.createElement("div");
recipeButtons.className = "button-row";
recipeButtons.append(
  button("Save JSON", saveRecipe),
  button("Load JSON", () => document.querySelector("#recipeFile").click()),
  button("Randomize grain", randomizeGrain),
  button("Reset all", resetAll),
);
const recipeFile = document.createElement("input");
recipeFile.id = "recipeFile";
recipeFile.type = "file";
recipeFile.accept = "application/json,.json";
recipeFile.hidden = true;
document.querySelector("#recipeControls").append(recipeButtons, recipeFile);

rebuildPlateEditors();
setSource(makeSample());
scheduleRender();

form.addEventListener("input", ({ target }) => {
  if (target.type === "file") return;
  if (target.id === "separationMode" || target.id === "paletteSize") rebuildPlateEditors();
  if (target.id !== "preset" && !target.closest("#outputControls")) {
    document.querySelector("#preset").value = "";
    loadedOptions = undefined;
  }
  if (target.id === "preset") loadedOptions = undefined;
  scheduleRender();
});

document.querySelector("#sourceFile").addEventListener("change", ({ target }) => {
  const [file] = target.files;
  if (file) setSource(file);
});
for (const name of ["paperTexture", "displacementMap", "distressMask"]) {
  document.querySelector(`#${name}`).addEventListener("change", ({ target }) => {
    const [file] = target.files;
    if (file) assets[name] = file;
    else delete assets[name];
    scheduleRender();
  });
}
recipeFile.addEventListener("change", async ({ target }) => {
  const [file] = target.files;
  if (!file) return;
  try {
    const parsed = JSON.parse(await file.text());
    if (!parsed || typeof parsed !== "object") throw new Error("Recipe must be a JSON object");
    loadedOptions = parsed.recipe || parsed.preset ? parsed : { recipe: parsed };
    document.querySelector("#preset").value = "";
    scheduleRender();
  } catch (error) {
    status.textContent = `Error: ${error.message}`;
  } finally {
    target.value = "";
  }
});

function addFields(container, fields) {
  for (const field of fields) container.append(createField(field));
}

function createField({ id, label, type = "range", options, min, max, step, value }) {
  const wrapper = document.createElement("label");
  wrapper.className = `control ${type}`;
  wrapper.dataset.field = id;
  const title = document.createElement("span");
  title.textContent = label;
  let input;
  if (type === "select") {
    input = document.createElement("select");
    for (const option of options) {
      const [optionValue, optionLabel] = Array.isArray(option) ? option : [option, option || "None"];
      input.add(new Option(optionLabel, optionValue));
    }
  } else {
    input = document.createElement("input");
    input.type = type;
    if (min !== undefined) input.min = min;
    if (max !== undefined) input.max = max;
    if (step !== undefined) input.step = step;
  }
  input.id = id;
  if (type === "checkbox") input.checked = value;
  else input.value = value;
  wrapper.append(title, input);
  if (type === "range") {
    const readout = document.createElement("output");
    readout.htmlFor = id;
    readout.value = value;
    input.addEventListener("input", () => { readout.value = input.value; });
    wrapper.append(readout);
  }
  return wrapper;
}

function button(label, action) {
  const element = document.createElement("button");
  element.type = "button";
  element.textContent = label;
  element.addEventListener("click", action);
  return element;
}

function rebuildPlateEditors() {
  const mode = value("separationMode");
  const palette = ["tonal", "indexed", "custom"].includes(mode);
  document.querySelector('[data-field="paletteSize"]').hidden = !palette;
  document.querySelector('[data-field="extractPalette"]').hidden = mode !== "indexed";
  document.querySelector('[data-field="threshold"]').hidden = !["monochrome", "cmy", "rgb"].includes(mode);
  document.querySelector('[data-field="softness"]').hidden = !["monochrome", "cmy", "rgb"].includes(mode);
  const count = mode === "monochrome" ? 1 : ["cmy", "rgb", "tri-tone"].includes(mode) ? 3 : mode === "cmyk" ? 4 : number("paletteSize");
  const labels = plateLabels(mode, count);
  const container = document.querySelector("#plateEditors");
  container.replaceChildren();
  for (let index = 0; index < count; index++) {
    const details = document.createElement("details");
    details.className = "plate-editor";
    if (index === 0) details.open = true;
    const summary = document.createElement("summary");
    summary.textContent = labels[index];
    const controls = document.createElement("div");
    controls.className = "controls";
    const prefix = `plate${index}`;
    const color = defaultPlateColor(mode, index, count);
    addFields(controls, [
      { id: `${prefix}Enabled`, label: "Enabled", type: "checkbox", value: true },
      { id: `${prefix}Color`, label: "Ink / palette color", type: "color", value: color },
      { id: `${prefix}OffsetX`, label: "Offset X", type: "number", min: -128, max: 128, value: 0 },
      { id: `${prefix}OffsetY`, label: "Offset Y", type: "number", min: -128, max: 128, value: 0 },
      { id: `${prefix}Angle`, label: "Screen angle", min: -180, max: 180, step: 1, value: [45, 15, 75, 0][index % 4] },
      { id: `${prefix}Bleed`, label: "Plate bleed", type: "number", min: 0, max: 16, value: 0 },
      { id: `${prefix}Trapping`, label: "Plate trapping", type: "number", min: 0, max: 16, value: 0 },
    ]);
    if (mode === "tri-tone") {
      const ranges = [[0, .42], [.25, .75], [.58, 1]][index];
      addFields(controls, [
        { id: `${prefix}RangeStart`, label: "Range start", min: 0, max: 1, step: .01, value: ranges[0] },
        { id: `${prefix}RangeEnd`, label: "Range end", min: 0, max: 1, step: .01, value: ranges[1] },
        { id: `${prefix}Intensity`, label: "Intensity", min: 0, max: 2, step: .01, value: 1 },
        { id: `${prefix}GrainAmount`, label: "Band grain", min: 0, max: 1, step: .01, value: .12 },
        { id: `${prefix}GrainScale`, label: "Band grain scale", min: .25, max: 12, step: .05, value: 3 },
        { id: `${prefix}GrainSeed`, label: "Band grain seed", type: "number", min: 0, value: index + 1 },
      ]);
    }
    details.append(summary, controls);
    container.append(details);
  }
}

function plateLabels(mode, count) {
  if (mode === "monochrome") return ["Black ink"];
  if (mode === "cmy") return ["Cyan", "Magenta", "Yellow"];
  if (mode === "rgb") return ["Red", "Green", "Blue"];
  if (mode === "cmyk") return ["Cyan", "Magenta", "Yellow", "Black"];
  if (mode === "tri-tone") return ["Shadows", "Midtones", "Highlights"];
  return Array.from({ length: count }, (_, index) => `Palette / plate ${index + 1}`);
}

function defaultPlateColor(mode, index, count) {
  const colors = {
    monochrome: ["#050505"],
    cmy: ["#00788f", "#d90d59", "#f3b80d"],
    rgb: ["#ff2020", "#20ff55", "#2070ff"],
    cmyk: ["#00a6c7", "#e00866", "#fac705", "#050505"],
    "tri-tone": ["#08080a", "#a61424", "#f2bd1f"],
  };
  if (colors[mode]) return colors[mode][index];
  const level = Math.round(index / Math.max(1, count - 1) * 238);
  return `#${level.toString(16).padStart(2, "0").repeat(3)}`;
}

function buildRecipe() {
  return {
    separation: buildSeparation(),
    dither: { algorithm: value("algorithm"), strength: number("ditherStrength"), seed: number("ditherSeed") },
    resampling: value("resampling"),
    preprocess: {
      brightness: number("brightness"), contrast: number("contrast"), gamma: number("gamma"),
      blur: number("blur"), sharpen: number("sharpen"), blackPoint: number("blackPoint"),
      whitePoint: number("whitePoint"), denoise: number("denoise"), invert: checked("invert"),
    },
    print: { dpi: number("dpi"), lpi: number("lpi"), bleedPixels: number("printBleed"), trappingPixels: number("printTrapping") },
    glow: {
      enabled: checked("glowEnabled"), threshold: number("glowThreshold"), radius: number("glowRadius"),
      falloff: number("glowFalloff"), intensity: number("glowIntensity"), tint: rgb("glowTint"),
      gamma: number("glowGamma"), saturation: number("glowSaturation"),
    },
    displacement: {
      enabled: checked("displacementEnabled"), xStrength: number("xStrength"), yStrength: number("yStrength"),
      distressAmount: number("distressAmount"), pattern: value("mapPattern"), patternScale: number("mapScale"), seed: number("mapSeed"),
    },
    crt: {
      enabled: checked("crtEnabled"), phase: value("crtPhase"), waveStrength: number("waveStrength"),
      waveFrequency: number("waveFrequency"), scanlines: number("scanlines"), rgbBleed: number("rgbBleed"),
      syncTearing: number("syncTearing"), phosphorMask: number("phosphorMask"), bloom: number("crtBloom"), seed: number("crtSeed"),
    },
    grain: { amount: number("grainAmount"), scale: number("grainScale"), seed: number("grainSeed") },
    paper: { amount: number("paperAmount"), scale: number("paperScale"), seed: number("paperSeed") },
    paperColor: rgb("paperColor"),
  };
}

function buildSeparation() {
  const mode = value("separationMode");
  const inks = plateLabels(mode, document.querySelectorAll(".plate-editor").length).map((_, index) => readInk(index));
  if (mode === "monochrome") return { mode, threshold: number("threshold"), softness: number("softness"), ink: inks[0] };
  if (["cmy", "rgb"].includes(mode)) return { mode, threshold: number("threshold"), softness: number("softness"), inks };
  if (mode === "cmyk") return { mode, inks };
  if (mode === "tri-tone") {
    const bands = inks.map((ink, index) => ({
      range: [number(`plate${index}RangeStart`), number(`plate${index}RangeEnd`)], ink,
      intensity: number(`plate${index}Intensity`),
      grain: { amount: number(`plate${index}GrainAmount`), scale: number(`plate${index}GrainScale`), seed: number(`plate${index}GrainSeed`) },
    }));
    return { mode, shadows: bands[0], midtones: bands[1], highlights: bands[2] };
  }
  return {
    mode,
    size: number("paletteSize"),
    colors: mode === "indexed" && checked("extractPalette") ? undefined : inks.map((ink) => ink.color),
    inks,
  };
}

function readInk(index) {
  return {
    enabled: checked(`plate${index}Enabled`), color: rgb(`plate${index}Color`),
    offset: [number(`plate${index}OffsetX`), number(`plate${index}OffsetY`)],
    angleDegrees: number(`plate${index}Angle`), bleedPixels: number(`plate${index}Bleed`),
    trappingPixels: number(`plate${index}Trapping`),
  };
}

function value(id) { return document.querySelector(`#${id}`).value; }
function number(id) { return Number(value(id)); }
function checked(id) { return document.querySelector(`#${id}`).checked; }
function rgb(id) {
  const hex = value(id).slice(1);
  return [0, 2, 4].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255);
}

function currentOptions() {
  if (loadedOptions) return loadedOptions;
  const preset = value("preset");
  return preset ? { preset } : { recipe: buildRecipe() };
}

function scheduleRender() {
  clearTimeout(timer);
  timer = setTimeout(render, 120);
}

async function render() {
  controller?.abort();
  controller = new AbortController();
  const active = controller;
  const options = currentOptions();
  status.textContent = "Processing";
  effect.setAttribute("aria-busy", "true");
  const summary = JSON.stringify(options);
  spec.textContent = summary.length > 240 ? `${summary.slice(0, 240)}…` : summary;
  try {
    const result = await ditherImageWithPlates(source, options, {
      signal: active.signal,
      type: value("outputType"),
      quality: number("outputQuality"),
      assets,
    });
    if (active.signal.aborted) return;
    clearOutputUrls();
    const compositeUrl = keepUrl(result.composite);
    effect.src = compositeUrl;
    downloadComposite.href = compositeUrl;
    downloadComposite.download = `dither-composite.${extension()}`;
    plateContainer.replaceChildren();
    if (checked("showPlates")) {
      for (const plate of result.plates) addPlate(plate);
    } else {
      const message = document.createElement("p");
      message.className = "empty";
      message.textContent = `${result.plates.length} plate masks rendered; enable “Show plate masks” to display them.`;
      plateContainer.append(message);
    }
    effect.removeAttribute("aria-busy");
    status.textContent = `Ready · ${result.plates.length} plates`;
  } catch (error) {
    if (error.name === "AbortError") return;
    effect.removeAttribute("aria-busy");
    status.textContent = `Error: ${error.message}`;
  }
}

function addPlate(plate) {
  const figure = document.createElement("figure");
  const caption = document.createElement("figcaption");
  caption.textContent = plate.name;
  const image = new Image();
  image.alt = `${plate.name} grayscale plate mask`;
  const url = keepUrl(plate.blob);
  image.src = url;
  const download = document.createElement("a");
  download.href = url;
  download.download = `dither-plate-${plate.name}.${extension()}`;
  download.textContent = "Save";
  figure.append(caption, image, download);
  plateContainer.append(figure);
}

function extension() {
  return { "image/png": "png", "image/webp": "webp", "image/jpeg": "jpg" }[value("outputType")];
}

function keepUrl(blob) {
  const url = URL.createObjectURL(blob);
  outputUrls.push(url);
  return url;
}

function clearOutputUrls() {
  for (const url of outputUrls) URL.revokeObjectURL(url);
  outputUrls = [];
}

function setSource(nextSource) {
  source = nextSource;
  if (sourceUrl) URL.revokeObjectURL(sourceUrl);
  sourceUrl = nextSource instanceof Blob ? URL.createObjectURL(nextSource) : nextSource;
  original.src = sourceUrl;
  scheduleRender();
}

function makeSample() {
  const canvas = document.createElement("canvas");
  canvas.width = 1200;
  canvas.height = 900;
  const context = canvas.getContext("2d");
  const gradient = context.createLinearGradient(0, 0, canvas.width, canvas.height);
  gradient.addColorStop(0, "#141414");
  gradient.addColorStop(.48, "#e33e2e");
  gradient.addColorStop(1, "#f4edcf");
  context.fillStyle = gradient;
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.fillStyle = "#e5b83b";
  context.beginPath();
  context.arc(850, 290, 190, 0, Math.PI * 2);
  context.fill();
  context.fillStyle = "#132e35";
  context.fillRect(120, 520, 960, 170);
  context.fillStyle = "#f4edcf";
  context.font = "160px Mono";
  context.fillText("DITHER", 145, 660);
  return canvas.toDataURL("image/png");
}

function saveRecipe() {
  const blob = new Blob([JSON.stringify(currentOptions(), null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "dither-recipe.json";
  link.click();
  URL.revokeObjectURL(url);
}

function randomizeGrain() {
  document.querySelector("#grainSeed").value = Math.floor(Math.random() * 1_000_000);
  document.querySelector("#preset").value = "";
  loadedOptions = undefined;
  scheduleRender();
}

function resetAll() {
  form.reset();
  for (const key of Object.keys(assets)) delete assets[key];
  loadedOptions = undefined;
  rebuildPlateEditors();
  setSource(makeSample());
}

addEventListener("beforeunload", () => {
  controller?.abort();
  if (sourceUrl?.startsWith("blob:")) URL.revokeObjectURL(sourceUrl);
  clearOutputUrls();
});
