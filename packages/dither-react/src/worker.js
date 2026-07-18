import init, { dither_document_rgba } from "../wasm/dither_web.js";

const ready = init();

self.addEventListener("message", async ({ data }) => {
  try {
    await ready;
    const paper = data.assets.paperTexture;
    const displacement = data.assets.displacementMap;
    const distress = data.assets.distressMask;
    const rendered = dither_document_rgba(
      new Uint8Array(data.rgba),
      data.width,
      data.height,
      data.optionsJson,
      new Uint8Array(paper.rgba),
      paper.width,
      paper.height,
      new Uint8Array(displacement.rgba),
      displacement.width,
      displacement.height,
      new Uint8Array(distress.rgba),
      distress.width,
      distress.height,
    );
    const rgba = rendered.composite_rgba();
    const plateCoverages = rendered.plate_coverages();
    const message = {
      id: data.id,
      width: rendered.width,
      height: rendered.height,
      rgba: rgba.buffer,
      plateMetadata: JSON.parse(rendered.plate_metadata_json()),
      plateCoverages: plateCoverages.buffer,
    };
    rendered.free();
    self.postMessage(message, [message.rgba, message.plateCoverages]);
  } catch (error) {
    self.postMessage({ id: data.id, error: String(error?.message ?? error) });
  }
});
