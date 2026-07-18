import {
  createElement,
  forwardRef,
  useEffect,
  useRef,
  useState,
} from "react";

import { ditherImage, ditherImageWithPlates } from "./core.js";

export const DitherImage = forwardRef(function DitherImage(
  {
    src,
    algorithm,
    brightness,
    contrast,
    gamma,
    options = {},
    assets,
    includePlates = false,
    outputType = "image/png",
    outputQuality,
    onDitherLoad,
    onDitherError,
    ...imageProps
  },
  ref,
) {
  const [renderedSrc, setRenderedSrc] = useState(src);
  const [busy, setBusy] = useState(true);
  const loadHandler = useRef(onDitherLoad);
  const errorHandler = useRef(onDitherError);
  loadHandler.current = onDitherLoad;
  errorHandler.current = onDitherError;
  const mergedOptions = { ...options };
  if (algorithm !== undefined) mergedOptions.algorithm = algorithm;
  if (brightness !== undefined) mergedOptions.brightness = brightness;
  if (contrast !== undefined) mergedOptions.contrast = contrast;
  if (gamma !== undefined) mergedOptions.gamma = gamma;
  const optionsKey = JSON.stringify(mergedOptions);

  useEffect(() => {
    const controller = new AbortController();
    let objectUrl;
    setRenderedSrc(src);
    setBusy(true);
    const render = includePlates ? ditherImageWithPlates : ditherImage;
    render(src, JSON.parse(optionsKey), {
      signal: controller.signal,
      type: outputType,
      quality: outputQuality,
      assets,
    })
      .then((result) => {
        if (controller.signal.aborted) return;
        const blob = includePlates ? result.composite : result;
        objectUrl = URL.createObjectURL(blob);
        setRenderedSrc(objectUrl);
        setBusy(false);
        loadHandler.current?.(blob, includePlates ? result.plates : []);
      })
      .catch((error) => {
        if (error?.name === "AbortError") return;
        setBusy(false);
        errorHandler.current?.(error);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [src, optionsKey, assets, includePlates, outputType, outputQuality]);

  return createElement("img", {
    ...imageProps,
    ref,
    src: renderedSrc,
    "aria-busy": busy || undefined,
  });
});
