import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

let onMessage;
let resolveMessage;
globalThis.self = {
  addEventListener(type, listener) {
    if (type === "message") onMessage = listener;
  },
  postMessage(message) {
    resolveMessage(message);
  },
};
globalThis.fetch = async (source) =>
  new Response(readFileSync(source), {
    headers: { "content-type": "application/wasm" },
  });

await import("../src/worker.js");

test("worker renders transferable RGBA output", async () => {
  const result = new Promise((resolve) => {
    resolveMessage = resolve;
  });
  const input = Uint8Array.from([0, 0, 0, 255, 255, 255, 255, 128]);
  onMessage({
    data: {
      id: 7,
      rgba: input.buffer,
      width: 2,
      height: 1,
      optionsJson: JSON.stringify({ algorithm: "atkinson" }),
      assets: {
        paperTexture: { rgba: new ArrayBuffer(0), width: 0, height: 0 },
        displacementMap: { rgba: new ArrayBuffer(0), width: 0, height: 0 },
        distressMask: { rgba: new ArrayBuffer(0), width: 0, height: 0 },
      },
    },
  });
  const message = await result;
  assert.equal(message.id, 7);
  assert.equal(message.error, undefined);
  assert.equal(new Uint8Array(message.rgba).length, input.length);
  assert.equal(message.plateMetadata.length, 1);
  assert.equal(new Uint8Array(message.plateCoverages).length, 2);
});
