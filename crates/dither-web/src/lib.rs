use std::{num::NonZeroU32, path::PathBuf};

use dither_core::{
    AssetKind, CrtPhase, DitherAlgorithm, Document, FourColor, HalftoneShape, Ink, MapPattern,
    Metadata, Monochrome, PaletteSettings, Recipe, Resampling, Separation, SourceImage, SourceInfo,
    StylizeEffect, Texture, ThreeColor, ToneBand, TriTone, built_in_presets, linear_to_srgb,
    srgb_to_linear,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebOptions {
    preset: Option<String>,
    recipe: Option<WebRecipe>,
    algorithm: Option<String>,
    strength: Option<f32>,
    seed: Option<u64>,
    brightness: Option<f32>,
    contrast: Option<f32>,
    gamma: Option<f32>,
    blur: Option<f32>,
    sharpen: Option<f32>,
    black_point: Option<f32>,
    white_point: Option<f32>,
    denoise: Option<f32>,
    invert: Option<bool>,
    threshold: Option<f32>,
    softness: Option<f32>,
    grain: Option<f32>,
    grain_scale: Option<f32>,
    paper: Option<f32>,
    paper_scale: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebRecipe {
    bypass: Option<bool>,
    separation: Option<WebSeparation>,
    dither: Option<WebDither>,
    resampling: Option<String>,
    preprocess: Option<WebPreprocess>,
    stylize: Option<WebStylize>,
    print: Option<WebPrint>,
    glow: Option<WebGlow>,
    displacement: Option<WebDisplacement>,
    crt: Option<WebCrt>,
    grain: Option<WebTexture>,
    paper: Option<WebTexture>,
    paper_color: Option<[f32; 3]>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebStylize {
    effect: Option<String>,
    cell_size: Option<f32>,
    amount: Option<f32>,
    seed: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebDither {
    algorithm: Option<String>,
    strength: Option<f32>,
    seed: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebPreprocess {
    brightness: Option<f32>,
    contrast: Option<f32>,
    gamma: Option<f32>,
    blur: Option<f32>,
    sharpen: Option<f32>,
    black_point: Option<f32>,
    white_point: Option<f32>,
    denoise: Option<f32>,
    invert: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebPrint {
    dpi: Option<f32>,
    lpi: Option<f32>,
    bleed_pixels: Option<u8>,
    trapping_pixels: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebGlow {
    enabled: Option<bool>,
    threshold: Option<f32>,
    radius: Option<f32>,
    falloff: Option<f32>,
    intensity: Option<f32>,
    tint: Option<[f32; 3]>,
    gamma: Option<f32>,
    saturation: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebDisplacement {
    enabled: Option<bool>,
    x_strength: Option<f32>,
    y_strength: Option<f32>,
    distress_amount: Option<f32>,
    pattern: Option<String>,
    pattern_scale: Option<f32>,
    seed: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebCrt {
    enabled: Option<bool>,
    phase: Option<String>,
    wave_strength: Option<f32>,
    wave_frequency: Option<f32>,
    scanlines: Option<f32>,
    rgb_bleed: Option<f32>,
    sync_tearing: Option<f32>,
    phosphor_mask: Option<f32>,
    bloom: Option<f32>,
    seed: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebTexture {
    amount: Option<f32>,
    scale: Option<f32>,
    seed: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebInk {
    enabled: Option<bool>,
    color: Option<[f32; 3]>,
    offset: Option<[i32; 2]>,
    angle_degrees: Option<f32>,
    bleed_pixels: Option<u8>,
    trapping_pixels: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct WebToneBand {
    range: Option<[f32; 2]>,
    ink: Option<WebInk>,
    intensity: Option<f32>,
    grain: Option<WebTexture>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
enum WebSeparation {
    Monochrome {
        #[serde(default)]
        threshold: Option<f32>,
        #[serde(default)]
        softness: Option<f32>,
        #[serde(default)]
        ink: Option<WebInk>,
    },
    #[serde(alias = "three-color")]
    Cmy {
        #[serde(default)]
        threshold: Option<f32>,
        #[serde(default)]
        softness: Option<f32>,
        #[serde(default)]
        inks: Vec<WebInk>,
    },
    Rgb {
        #[serde(default)]
        threshold: Option<f32>,
        #[serde(default)]
        softness: Option<f32>,
        #[serde(default)]
        inks: Vec<WebInk>,
    },
    Cmyk {
        #[serde(default)]
        inks: Vec<WebInk>,
    },
    Tonal {
        #[serde(default)]
        colors: Option<Vec<[f32; 3]>>,
        #[serde(default)]
        inks: Vec<WebInk>,
        #[serde(default)]
        size: Option<u8>,
    },
    Indexed {
        #[serde(default)]
        colors: Option<Vec<[f32; 3]>>,
        #[serde(default)]
        inks: Vec<WebInk>,
        #[serde(default)]
        size: Option<u8>,
    },
    Custom {
        #[serde(default)]
        colors: Option<Vec<[f32; 3]>>,
        #[serde(default)]
        inks: Vec<WebInk>,
        #[serde(default)]
        size: Option<u8>,
    },
    TriTone {
        #[serde(default)]
        shadows: Option<Box<WebToneBand>>,
        #[serde(default)]
        midtones: Option<Box<WebToneBand>>,
        #[serde(default)]
        highlights: Option<Box<WebToneBand>>,
    },
}

impl WebRecipe {
    fn apply(self, recipe: &mut Recipe) -> Result<(), String> {
        if let Some(value) = self.bypass {
            recipe.bypass = value;
        }
        if let Some(dither) = self.dither {
            if let Some(algorithm) = dither.algorithm {
                recipe.dither.algorithm = parse_algorithm(&algorithm)?;
            }
            if let Some(value) = dither.strength {
                recipe.dither.strength = value;
            }
            if let Some(value) = dither.seed {
                recipe.dither.seed = value;
            }
        }
        if let Some(value) = self.resampling {
            recipe.resampling = parse_resampling(&value)?;
        }
        if let Some(settings) = self.preprocess {
            if let Some(value) = settings.brightness {
                recipe.preprocess.brightness = value;
            }
            if let Some(value) = settings.contrast {
                recipe.preprocess.contrast = value;
            }
            if let Some(value) = settings.gamma {
                recipe.preprocess.gamma = value;
            }
            if let Some(value) = settings.blur {
                recipe.preprocess.blur_radius = value;
            }
            if let Some(value) = settings.sharpen {
                recipe.preprocess.sharpen = value;
            }
            if let Some(value) = settings.black_point {
                recipe.preprocess.black_point = value;
            }
            if let Some(value) = settings.white_point {
                recipe.preprocess.white_point = value;
            }
            if let Some(value) = settings.denoise {
                recipe.preprocess.denoise = value;
            }
            if let Some(value) = settings.invert {
                recipe.preprocess.invert = value;
            }
        }
        if let Some(settings) = self.stylize {
            if let Some(value) = settings.effect {
                recipe.stylize.effect = parse_stylize_effect(&value)?;
            }
            if let Some(value) = settings.cell_size {
                recipe.stylize.cell_size = value;
            }
            if let Some(value) = settings.amount {
                recipe.stylize.amount = value;
            }
            if let Some(value) = settings.seed {
                recipe.stylize.seed = value;
            }
        }
        if let Some(settings) = self.print {
            if let Some(value) = settings.dpi {
                recipe.print.dpi = value;
            }
            if let Some(value) = settings.lpi {
                recipe.print.lpi = value;
            }
            if let Some(value) = settings.bleed_pixels {
                recipe.print.bleed_pixels = value;
            }
            if let Some(value) = settings.trapping_pixels {
                recipe.print.trapping_pixels = value;
            }
        }
        if let Some(settings) = self.glow {
            if let Some(value) = settings.enabled {
                recipe.glow.enabled = value;
            }
            if let Some(value) = settings.threshold {
                recipe.glow.threshold = value;
            }
            if let Some(value) = settings.radius {
                recipe.glow.radius = value;
            }
            if let Some(value) = settings.falloff {
                recipe.glow.falloff = value;
            }
            if let Some(value) = settings.intensity {
                recipe.glow.intensity = value;
            }
            if let Some(value) = settings.tint {
                recipe.glow.tint = value;
            }
            if let Some(value) = settings.gamma {
                recipe.glow.gamma = value;
            }
            if let Some(value) = settings.saturation {
                recipe.glow.saturation = value;
            }
        }
        if let Some(settings) = self.displacement {
            if let Some(value) = settings.enabled {
                recipe.displacement.enabled = value;
            }
            if let Some(value) = settings.x_strength {
                recipe.displacement.x_strength = value;
            }
            if let Some(value) = settings.y_strength {
                recipe.displacement.y_strength = value;
            }
            if let Some(value) = settings.distress_amount {
                recipe.displacement.distress_amount = value;
            }
            if let Some(value) = settings.pattern {
                recipe.displacement.pattern = parse_map_pattern(&value)?;
            }
            if let Some(value) = settings.pattern_scale {
                recipe.displacement.pattern_scale = value;
            }
            if let Some(value) = settings.seed {
                recipe.displacement.seed = value;
            }
        }
        if let Some(settings) = self.crt {
            if let Some(value) = settings.enabled {
                recipe.crt.enabled = value;
            }
            if let Some(value) = settings.phase {
                recipe.crt.phase = parse_crt_phase(&value)?;
            }
            if let Some(value) = settings.wave_strength {
                recipe.crt.wave_strength = value;
            }
            if let Some(value) = settings.wave_frequency {
                recipe.crt.wave_frequency = value;
            }
            if let Some(value) = settings.scanlines {
                recipe.crt.scanlines = value;
            }
            if let Some(value) = settings.rgb_bleed {
                recipe.crt.rgb_bleed = value;
            }
            if let Some(value) = settings.sync_tearing {
                recipe.crt.sync_tearing = value;
            }
            if let Some(value) = settings.phosphor_mask {
                recipe.crt.phosphor_mask = value;
            }
            if let Some(value) = settings.bloom {
                recipe.crt.bloom = value;
            }
            if let Some(value) = settings.seed {
                recipe.crt.seed = value;
            }
        }
        if let Some(settings) = self.grain {
            settings.apply(&mut recipe.grain);
        }
        if let Some(settings) = self.paper {
            settings.apply(&mut recipe.paper);
        }
        if let Some(value) = self.paper_color {
            recipe.paper_color = value;
        }
        if let Some(separation) = self.separation {
            recipe.separation = separation.into_core();
        }
        Ok(())
    }
}

impl WebTexture {
    fn apply(self, texture: &mut Texture) {
        if let Some(value) = self.amount {
            texture.amount = value;
        }
        if let Some(value) = self.scale {
            texture.scale = value;
        }
        if let Some(value) = self.seed {
            texture.seed = value;
        }
    }
}

impl WebInk {
    fn apply(self, ink: &mut Ink) {
        if let Some(value) = self.enabled {
            ink.enabled = value;
        }
        if let Some(value) = self.color {
            ink.color = value;
        }
        if let Some(value) = self.offset {
            ink.offset = value;
        }
        if let Some(value) = self.angle_degrees {
            ink.angle_degrees = value;
        }
        if let Some(value) = self.bleed_pixels {
            ink.bleed_pixels = value;
        }
        if let Some(value) = self.trapping_pixels {
            ink.trapping_pixels = value;
        }
    }
}

impl WebToneBand {
    fn apply(self, band: &mut ToneBand) {
        if let Some(value) = self.range {
            band.range = value;
        }
        if let Some(ink) = self.ink {
            ink.apply(&mut band.ink);
        }
        if let Some(value) = self.intensity {
            band.intensity = value;
        }
        if let Some(grain) = self.grain {
            grain.apply(&mut band.grain);
        }
    }
}

impl WebSeparation {
    fn into_core(self) -> Separation {
        match self {
            Self::Monochrome {
                threshold,
                softness,
                ink,
            } => {
                let mut settings = Monochrome::default();
                if let Some(value) = threshold {
                    settings.threshold = value;
                }
                if let Some(value) = softness {
                    settings.softness = value;
                }
                if let Some(ink) = ink {
                    ink.apply(&mut settings.ink);
                }
                Separation::Monochrome(settings)
            }
            Self::Cmy {
                threshold,
                softness,
                inks,
            } => Separation::ThreeColor(three_color_settings(threshold, softness, inks)),
            Self::Rgb {
                threshold,
                softness,
                inks,
            } => Separation::Rgb(three_color_settings(threshold, softness, inks)),
            Self::Cmyk { inks } => {
                let mut settings = FourColor::default();
                apply_inks(
                    inks,
                    [
                        &mut settings.cyan,
                        &mut settings.magenta,
                        &mut settings.yellow,
                        &mut settings.black,
                    ],
                );
                Separation::Cmyk(settings)
            }
            Self::Tonal { colors, inks, size } => {
                Separation::Tonal(palette_settings(colors, inks, size, false))
            }
            Self::Indexed { colors, inks, size } => {
                Separation::Indexed(palette_settings(colors, inks, size, true))
            }
            Self::Custom { colors, inks, size } => {
                Separation::Custom(palette_settings(colors, inks, size, false))
            }
            Self::TriTone {
                shadows,
                midtones,
                highlights,
            } => {
                let mut settings = TriTone::default();
                if let Some(band) = shadows {
                    (*band).apply(&mut settings.shadows);
                }
                if let Some(band) = midtones {
                    (*band).apply(&mut settings.midtones);
                }
                if let Some(band) = highlights {
                    (*band).apply(&mut settings.highlights);
                }
                Separation::TriTone(settings)
            }
        }
    }
}

fn apply_inks<const N: usize>(inks: Vec<WebInk>, targets: [&mut Ink; N]) {
    for (ink, target) in inks.into_iter().zip(targets) {
        ink.apply(target);
    }
}

fn three_color_settings(
    threshold: Option<f32>,
    softness: Option<f32>,
    inks: Vec<WebInk>,
) -> ThreeColor {
    let mut settings = ThreeColor::default();
    if let Some(value) = threshold {
        settings.threshold = value;
    }
    if let Some(value) = softness {
        settings.softness = value;
    }
    apply_inks(
        inks,
        [
            &mut settings.cyan,
            &mut settings.magenta,
            &mut settings.yellow,
        ],
    );
    settings
}

fn palette_settings(
    colors: Option<Vec<[f32; 3]>>,
    inks: Vec<WebInk>,
    size: Option<u8>,
    extract_by_default: bool,
) -> PaletteSettings {
    let mut settings = PaletteSettings::default();
    if let Some(colors) = colors {
        settings.inks = colors
            .iter()
            .enumerate()
            .map(|(index, color)| Ink::new(*color, [0, 0], [45.0, 15.0, 75.0, 0.0][index % 4]))
            .collect();
        settings.colors = colors;
    } else if extract_by_default {
        settings.colors.clear();
        settings.inks.clear();
    }
    if let Some(value) = size {
        settings.size = value;
    }
    for (index, ink) in inks.into_iter().enumerate() {
        if index >= settings.inks.len() {
            settings.inks.push(Ink::default());
        }
        ink.apply(&mut settings.inks[index]);
    }
    settings
}

impl WebOptions {
    fn apply(self, recipe: &mut Recipe) -> Result<(), String> {
        if let Some(preset) = self.preset {
            *recipe = built_in_presets()
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(preset.trim()))
                .map(|(_, recipe)| recipe.clone())
                .ok_or_else(|| format!("unknown preset: {preset}"))?;
        }
        if let Some(settings) = self.recipe {
            settings.apply(recipe)?;
        }
        if let Some(algorithm) = self.algorithm {
            recipe.dither.algorithm = parse_algorithm(&algorithm)?;
        }
        set_bounded(
            "strength",
            self.strength,
            0.0,
            1.0,
            &mut recipe.dither.strength,
        )?;
        if let Some(seed) = self.seed {
            recipe.dither.seed = seed;
        }
        set_bounded(
            "brightness",
            self.brightness,
            -1.0,
            1.0,
            &mut recipe.preprocess.brightness,
        )?;
        set_bounded(
            "contrast",
            self.contrast,
            0.0,
            3.0,
            &mut recipe.preprocess.contrast,
        )?;
        set_bounded("gamma", self.gamma, 0.1, 4.0, &mut recipe.preprocess.gamma)?;
        set_bounded(
            "blur",
            self.blur,
            0.0,
            32.0,
            &mut recipe.preprocess.blur_radius,
        )?;
        set_bounded(
            "sharpen",
            self.sharpen,
            0.0,
            3.0,
            &mut recipe.preprocess.sharpen,
        )?;
        set_bounded(
            "blackPoint",
            self.black_point,
            0.0,
            0.99,
            &mut recipe.preprocess.black_point,
        )?;
        set_bounded(
            "whitePoint",
            self.white_point,
            0.01,
            1.5,
            &mut recipe.preprocess.white_point,
        )?;
        set_bounded(
            "denoise",
            self.denoise,
            0.0,
            1.0,
            &mut recipe.preprocess.denoise,
        )?;
        if let Some(invert) = self.invert {
            recipe.preprocess.invert = invert;
        }
        if let Separation::Monochrome(monochrome) = &mut recipe.separation {
            set_bounded(
                "threshold",
                self.threshold,
                0.0,
                1.0,
                &mut monochrome.threshold,
            )?;
            set_bounded(
                "softness",
                self.softness,
                0.01,
                1.0,
                &mut monochrome.softness,
            )?;
        }
        set_bounded("grain", self.grain, 0.0, 0.8, &mut recipe.grain.amount)?;
        set_bounded(
            "grainScale",
            self.grain_scale,
            0.25,
            12.0,
            &mut recipe.grain.scale,
        )?;
        set_bounded("paper", self.paper, 0.0, 0.5, &mut recipe.paper.amount)?;
        set_bounded(
            "paperScale",
            self.paper_scale,
            0.5,
            24.0,
            &mut recipe.paper.scale,
        )?;
        validate_recipe(recipe)
    }
}

#[wasm_bindgen]
pub fn dither_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    options_json: &str,
) -> Result<Vec<u8>, JsValue> {
    render_document(
        rgba,
        width,
        height,
        options_json,
        (&[], 0, 0),
        (&[], 0, 0),
        (&[], 0, 0),
    )
    .map(|rendered| rendered.composite_rgba)
    .map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub struct WebRender {
    width: u32,
    height: u32,
    composite_rgba: Vec<u8>,
    plate_metadata_json: String,
    plate_coverages: Vec<u8>,
}

#[wasm_bindgen]
impl WebRender {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn composite_rgba(&self) -> Vec<u8> {
        self.composite_rgba.clone()
    }

    pub fn plate_metadata_json(&self) -> String {
        self.plate_metadata_json.clone()
    }

    pub fn plate_coverages(&self) -> Vec<u8> {
        self.plate_coverages.clone()
    }
}

#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub fn dither_document_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    options_json: &str,
    paper_rgba: &[u8],
    paper_width: u32,
    paper_height: u32,
    displacement_rgba: &[u8],
    displacement_width: u32,
    displacement_height: u32,
    distress_rgba: &[u8],
    distress_width: u32,
    distress_height: u32,
) -> Result<WebRender, JsValue> {
    render_document(
        rgba,
        width,
        height,
        options_json,
        (paper_rgba, paper_width, paper_height),
        (displacement_rgba, displacement_width, displacement_height),
        (distress_rgba, distress_width, distress_height),
    )
    .map_err(|error| JsValue::from_str(&error))
}

fn render_document(
    rgba: &[u8],
    width: u32,
    height: u32,
    options_json: &str,
    paper: (&[u8], u32, u32),
    displacement: (&[u8], u32, u32),
    distress: (&[u8], u32, u32),
) -> Result<WebRender, String> {
    let source = source_from_rgba(rgba, width, height, "browser-rgba")?
        .ok_or("source image cannot be empty")?;
    let options: WebOptions = serde_json::from_str(options_json)
        .map_err(|error| format!("invalid dither options: {error}"))?;
    let mut document = Document::new(source);
    options.apply(&mut document.recipe)?;
    for (kind, (bytes, width, height), name) in [
        (AssetKind::PaperTexture, paper, "paper texture"),
        (AssetKind::DisplacementMap, displacement, "displacement map"),
        (AssetKind::DistressMask, distress, "distress mask"),
    ] {
        if let Some(asset) = source_from_rgba(bytes, width, height, name)? {
            document.set_asset(kind, asset);
        }
    }

    let rendered = document.render_document();
    let plate_metadata: Vec<_> = rendered
        .plates
        .iter()
        .map(|plate| {
            serde_json::json!({
                "name": plate.name,
                "ink": {
                    "enabled": plate.ink.enabled,
                    "color": plate.ink.color,
                    "offset": plate.ink.offset,
                    "angleDegrees": plate.ink.angle_degrees,
                    "bleedPixels": plate.ink.bleed_pixels,
                    "trappingPixels": plate.ink.trapping_pixels,
                }
            })
        })
        .collect();
    Ok(WebRender {
        width: rendered.composite.width(),
        height: rendered.composite.height(),
        composite_rgba: encode_pixels(rendered.composite.pixels()),
        plate_metadata_json: serde_json::to_string(&plate_metadata)
            .map_err(|error| error.to_string())?,
        plate_coverages: rendered
            .plates
            .iter()
            .flat_map(|plate| plate.coverage().iter().copied().map(encode))
            .collect(),
    })
}

fn source_from_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: &str,
) -> Result<Option<SourceImage>, String> {
    if rgba.is_empty() && width == 0 && height == 0 {
        return Ok(None);
    }
    let width = NonZeroU32::new(width).ok_or("width must be greater than zero")?;
    let height = NonZeroU32::new(height).ok_or("height must be greater than zero")?;
    let expected = width
        .get()
        .checked_mul(height.get())
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("image dimensions are too large")? as usize;
    if rgba.len() != expected {
        return Err(format!(
            "expected {expected} RGBA bytes, received {}",
            rgba.len()
        ));
    }

    let pixels = rgba
        .chunks_exact(4)
        .map(|pixel| {
            [
                srgb_to_linear(pixel[0] as f32 / 255.0),
                srgb_to_linear(pixel[1] as f32 / 255.0),
                srgb_to_linear(pixel[2] as f32 / 255.0),
                pixel[3] as f32 / 255.0,
            ]
        })
        .collect();
    SourceImage::new(
        width,
        height,
        pixels,
        SourceInfo {
            path: PathBuf::new(),
            format: format.into(),
            bit_depth: 8,
            color_profile: Vec::new(),
            metadata: Metadata::default(),
        },
    )
    .map(Some)
    .map_err(|error| error.to_string())
}

fn encode_pixels(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|pixel| {
            [
                encode(linear_to_srgb(pixel[0])),
                encode(linear_to_srgb(pixel[1])),
                encode(linear_to_srgb(pixel[2])),
                encode(pixel[3]),
            ]
        })
        .collect()
}

fn encode(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn set_bounded(
    name: &str,
    value: Option<f32>,
    minimum: f32,
    maximum: f32,
    target: &mut f32,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    *target = value;
    Ok(())
}

fn validate_recipe(recipe: &Recipe) -> Result<(), String> {
    validate_bounded("dither.strength", recipe.dither.strength, 0.0, 1.0)?;
    if let DitherAlgorithm::Bayer { matrix_size } = recipe.dither.algorithm
        && !matches!(matrix_size, 2 | 4 | 8)
    {
        return Err("Bayer matrix size must be 2, 4, or 8".into());
    }
    validate_bounded(
        "preprocess.brightness",
        recipe.preprocess.brightness,
        -1.0,
        1.0,
    )?;
    validate_bounded("preprocess.contrast", recipe.preprocess.contrast, 0.0, 3.0)?;
    validate_bounded("preprocess.gamma", recipe.preprocess.gamma, 0.1, 4.0)?;
    validate_bounded("preprocess.blur", recipe.preprocess.blur_radius, 0.0, 32.0)?;
    validate_bounded("preprocess.sharpen", recipe.preprocess.sharpen, 0.0, 3.0)?;
    validate_bounded(
        "preprocess.blackPoint",
        recipe.preprocess.black_point,
        0.0,
        0.99,
    )?;
    validate_bounded(
        "preprocess.whitePoint",
        recipe.preprocess.white_point,
        0.01,
        1.5,
    )?;
    if recipe.preprocess.black_point >= recipe.preprocess.white_point {
        return Err("preprocess.blackPoint must be below whitePoint".into());
    }
    validate_bounded("preprocess.denoise", recipe.preprocess.denoise, 0.0, 1.0)?;
    validate_bounded("stylize.cellSize", recipe.stylize.cell_size, 4.0, 256.0)?;
    validate_bounded("stylize.amount", recipe.stylize.amount, 0.0, 2.0)?;

    match &recipe.separation {
        Separation::Monochrome(settings) => {
            validate_thresholds(settings.threshold, settings.softness)?;
            validate_ink("separation.ink", settings.ink)?;
        }
        Separation::ThreeColor(settings) | Separation::Rgb(settings) => {
            validate_thresholds(settings.threshold, settings.softness)?;
            validate_ink("separation.inks[0]", settings.cyan)?;
            validate_ink("separation.inks[1]", settings.magenta)?;
            validate_ink("separation.inks[2]", settings.yellow)?;
        }
        Separation::Cmyk(settings) => {
            validate_ink("separation.inks[0]", settings.cyan)?;
            validate_ink("separation.inks[1]", settings.magenta)?;
            validate_ink("separation.inks[2]", settings.yellow)?;
            validate_ink("separation.inks[3]", settings.black)?;
        }
        Separation::Tonal(settings)
        | Separation::Indexed(settings)
        | Separation::Custom(settings) => validate_palette(settings)?,
        Separation::TriTone(settings) => {
            validate_tone_band("separation.shadows", settings.shadows)?;
            validate_tone_band("separation.midtones", settings.midtones)?;
            validate_tone_band("separation.highlights", settings.highlights)?;
        }
    }

    validate_bounded("print.dpi", recipe.print.dpi, 36.0, 2400.0)?;
    validate_bounded("print.lpi", recipe.print.lpi, 5.0, 300.0)?;
    if recipe.print.bleed_pixels > 16 || recipe.print.trapping_pixels > 16 {
        return Err("print bleedPixels and trappingPixels must be at most 16".into());
    }

    validate_bounded("glow.threshold", recipe.glow.threshold, 0.0, 1.0)?;
    validate_bounded("glow.radius", recipe.glow.radius, 0.0, 64.0)?;
    validate_bounded("glow.falloff", recipe.glow.falloff, 1.0, 4.0)?;
    validate_bounded("glow.intensity", recipe.glow.intensity, 0.0, 4.0)?;
    validate_color("glow.tint", recipe.glow.tint)?;
    validate_bounded("glow.gamma", recipe.glow.gamma, 0.1, 4.0)?;
    validate_bounded("glow.saturation", recipe.glow.saturation, 0.0, 3.0)?;

    validate_bounded(
        "displacement.xStrength",
        recipe.displacement.x_strength,
        -128.0,
        128.0,
    )?;
    validate_bounded(
        "displacement.yStrength",
        recipe.displacement.y_strength,
        -128.0,
        128.0,
    )?;
    validate_bounded(
        "displacement.distressAmount",
        recipe.displacement.distress_amount,
        0.0,
        1.0,
    )?;
    validate_bounded(
        "displacement.patternScale",
        recipe.displacement.pattern_scale,
        2.0,
        256.0,
    )?;

    validate_bounded("crt.waveStrength", recipe.crt.wave_strength, 0.0, 128.0)?;
    validate_bounded("crt.waveFrequency", recipe.crt.wave_frequency, 0.1, 64.0)?;
    validate_bounded("crt.scanlines", recipe.crt.scanlines, 0.0, 1.0)?;
    validate_bounded("crt.rgbBleed", recipe.crt.rgb_bleed, 0.0, 24.0)?;
    validate_bounded("crt.syncTearing", recipe.crt.sync_tearing, 0.0, 128.0)?;
    validate_bounded("crt.phosphorMask", recipe.crt.phosphor_mask, 0.0, 1.0)?;
    validate_bounded("crt.bloom", recipe.crt.bloom, 0.0, 2.0)?;

    validate_texture("grain", recipe.grain, 0.8, 0.25, 12.0)?;
    validate_texture("paper", recipe.paper, 0.5, 0.5, 24.0)?;
    validate_color("paperColor", recipe.paper_color)
}

fn validate_thresholds(threshold: f32, softness: f32) -> Result<(), String> {
    validate_bounded("separation.threshold", threshold, 0.0, 1.0)?;
    validate_bounded("separation.softness", softness, 0.01, 1.0)
}

fn validate_palette(settings: &PaletteSettings) -> Result<(), String> {
    if settings.colors.len() == 1 || settings.colors.len() > 64 {
        return Err("palette colors must be empty for extraction or contain 2 to 64 colors".into());
    }
    for (index, color) in settings.colors.iter().enumerate() {
        validate_color(&format!("palette.colors[{index}]"), *color)?;
    }
    if settings.inks.len() > 64 {
        return Err("palette inks must contain at most 64 plates".into());
    }
    for (index, ink) in settings.inks.iter().enumerate() {
        validate_ink(&format!("palette.inks[{index}]"), *ink)?;
    }
    if !(2..=64).contains(&settings.size) {
        return Err("palette size must be between 2 and 64".into());
    }
    Ok(())
}

fn validate_tone_band(name: &str, band: ToneBand) -> Result<(), String> {
    validate_bounded(&format!("{name}.range[0]"), band.range[0], 0.0, 1.0)?;
    validate_bounded(&format!("{name}.range[1]"), band.range[1], 0.0, 1.0)?;
    if band.range[0] > band.range[1] {
        return Err(format!("{name} range start must not exceed its end"));
    }
    validate_bounded(&format!("{name}.intensity"), band.intensity, 0.0, 2.0)?;
    validate_texture(&format!("{name}.grain"), band.grain, 1.0, 0.25, 12.0)?;
    validate_ink(&format!("{name}.ink"), band.ink)
}

fn validate_ink(name: &str, ink: Ink) -> Result<(), String> {
    validate_color(&format!("{name}.color"), ink.color)?;
    if ink.offset.iter().any(|value| !(-128..=128).contains(value)) {
        return Err(format!("{name}.offset values must be between -128 and 128"));
    }
    validate_bounded(
        &format!("{name}.angleDegrees"),
        ink.angle_degrees,
        -180.0,
        180.0,
    )?;
    if ink.bleed_pixels > 16 || ink.trapping_pixels > 16 {
        return Err(format!(
            "{name} bleedPixels and trappingPixels must be at most 16"
        ));
    }
    Ok(())
}

fn validate_texture(
    name: &str,
    texture: Texture,
    maximum_amount: f32,
    minimum_scale: f32,
    maximum_scale: f32,
) -> Result<(), String> {
    validate_bounded(
        &format!("{name}.amount"),
        texture.amount,
        0.0,
        maximum_amount,
    )?;
    validate_bounded(
        &format!("{name}.scale"),
        texture.scale,
        minimum_scale,
        maximum_scale,
    )
}

fn validate_color(name: &str, color: [f32; 3]) -> Result<(), String> {
    for (index, value) in color.into_iter().enumerate() {
        validate_bounded(&format!("{name}[{index}]"), value, 0.0, 1.0)?;
    }
    Ok(())
}

fn validate_bounded(name: &str, value: f32, minimum: f32, maximum: f32) -> Result<(), String> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(())
}

fn parse_resampling(value: &str) -> Result<Resampling, String> {
    match normalize_name(value).as_str() {
        "nearest" => Ok(Resampling::Nearest),
        "bilinear" => Ok(Resampling::Bilinear),
        "supersample2x" | "supersample-2x" => Ok(Resampling::Supersample2x),
        _ => Err(format!("unsupported resampling mode: {value}")),
    }
}

fn parse_stylize_effect(value: &str) -> Result<StylizeEffect, String> {
    match normalize_name(value).as_str() {
        "none" => Ok(StylizeEffect::None),
        "pixelate" => Ok(StylizeEffect::Pixelate),
        "ascii" => Ok(StylizeEffect::Ascii),
        "dot-matrix" | "dotmatrix" => Ok(StylizeEffect::DotMatrix),
        "mosaic" => Ok(StylizeEffect::Mosaic),
        "bricks" => Ok(StylizeEffect::Bricks),
        "pointillism" => Ok(StylizeEffect::Pointillism),
        "heatmap" => Ok(StylizeEffect::Heatmap),
        "outline" => Ok(StylizeEffect::Outline),
        _ => Err(format!("unsupported effect: {value}")),
    }
}

fn parse_map_pattern(value: &str) -> Result<MapPattern, String> {
    match normalize_name(value).as_str() {
        "imported" => Ok(MapPattern::Imported),
        "grain" => Ok(MapPattern::Grain),
        "halftone" => Ok(MapPattern::Halftone),
        "grunge" => Ok(MapPattern::Grunge),
        "splatter" => Ok(MapPattern::Splatter),
        _ => Err(format!("unsupported displacement pattern: {value}")),
    }
}

fn parse_crt_phase(value: &str) -> Result<CrtPhase, String> {
    match normalize_name(value).as_str() {
        "waveform" => Ok(CrtPhase::Waveform),
        "linear" => Ok(CrtPhase::Linear),
        "flux" => Ok(CrtPhase::Flux),
        _ => Err(format!("unsupported CRT phase: {value}")),
    }
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

fn parse_algorithm(value: &str) -> Result<DitherAlgorithm, String> {
    let name = normalize_name(value);
    match name.as_str() {
        "bayer2x2" | "bayer-2x2" | "bayer2" => Ok(DitherAlgorithm::Bayer { matrix_size: 2 }),
        "bayer4x4" | "bayer-4x4" | "bayer4" => Ok(DitherAlgorithm::Bayer { matrix_size: 4 }),
        "bayer8x8" | "bayer-8x8" | "bayer8" => Ok(DitherAlgorithm::Bayer { matrix_size: 8 }),
        "floyd-steinberg" | "floydsteinberg" | "floyd" => Ok(DitherAlgorithm::FloydSteinberg),
        "atkinson" => Ok(DitherAlgorithm::Atkinson),
        "sierra-lite" | "sierralite" => Ok(DitherAlgorithm::SierraLite),
        "sierra-two-row" | "sierratworow" => Ok(DitherAlgorithm::SierraTwoRow),
        "sierra" => Ok(DitherAlgorithm::Sierra),
        "stucki" => Ok(DitherAlgorithm::Stucki),
        "burkes" => Ok(DitherAlgorithm::Burkes),
        "jarvis-judice-ninke" | "jarvisjudiceninke" | "jjn" => {
            Ok(DitherAlgorithm::JarvisJudiceNinke)
        }
        "blue-noise" | "bluenoise" => Ok(DitherAlgorithm::BlueNoise),
        "modulation" => Ok(DitherAlgorithm::Modulation),
        "dot" | "halftone-dot" => Ok(DitherAlgorithm::Halftone {
            shape: HalftoneShape::Dot,
        }),
        "line" | "halftone-line" => Ok(DitherAlgorithm::Halftone {
            shape: HalftoneShape::Line,
        }),
        "cross" | "halftone-cross" => Ok(DitherAlgorithm::Halftone {
            shape: HalftoneShape::Cross,
        }),
        "diamond" | "halftone-diamond" => Ok(DitherAlgorithm::Halftone {
            shape: HalftoneShape::Diamond,
        }),
        "clustered-dot" | "clustereddot" => Ok(DitherAlgorithm::Halftone {
            shape: HalftoneShape::ClusteredDot,
        }),
        _ => Err(format!("unsupported dither algorithm: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(input: &[u8], width: u32, height: u32, options: &str) -> Result<WebRender, String> {
        render_document(
            input,
            width,
            height,
            options,
            (&[], 0, 0),
            (&[], 0, 0),
            (&[], 0, 0),
        )
    }

    #[test]
    fn renders_rgba_and_validates_browser_options() {
        let input = [0, 0, 0, 255, 255, 255, 255, 128];
        let output = render(&input, 2, 1, r#"{"algorithm":"bayer2x2","brightness":0.1}"#).unwrap();
        assert_eq!(output.composite_rgba.len(), input.len());
        assert_eq!(output.composite_rgba[3], 255);
        assert_eq!(output.composite_rgba[7], 128);
        assert!(render(&input, 2, 1, r#"{"brightness":2}"#).is_err());
        assert!(render(&input, 2, 1, r#"{"algorithm":"unknown"}"#).is_err());
        assert!(render(&input[..4], 2, 1, "{}").is_err());
    }

    #[test]
    fn renders_complete_recipes_assets_and_separated_plates() {
        let input = [32, 96, 160, 255, 220, 120, 40, 255];
        let paper = [128, 128, 128, 255];
        let output = render_document(
            &input,
            2,
            1,
            r#"{
                "recipe": {
                    "separation": {"mode":"cmyk"},
                    "resampling": "nearest",
                    "print": {"dpi":600,"lpi":75,"bleedPixels":1,"trappingPixels":1},
                    "glow": {"enabled":true,"threshold":0.5,"radius":2,"intensity":0.2},
                    "displacement": {"enabled":true,"pattern":"grain","xStrength":1},
                    "crt": {"enabled":true,"phase":"linear","scanlines":0.2},
                    "grain": {"amount":0.1,"scale":2,"seed":8},
                    "paper": {"amount":0.1,"scale":3,"seed":9},
                    "paperColor": [0.9,0.8,0.7]
                }
            }"#,
            (&paper, 1, 1),
            (&paper, 1, 1),
            (&paper, 1, 1),
        )
        .unwrap();
        assert_eq!(output.composite_rgba.len(), input.len());
        assert_eq!(output.plate_coverages.len(), 4 * 2);
        assert!(output.plate_metadata_json.contains("magenta"));
        assert!(render(&input, 2, 1, r#"{"recipe":{"print":{"dpi":0}}}"#).is_err());
    }

    #[test]
    fn exposes_stylize_settings_and_the_bypass_preset() {
        let input = [32, 96, 160, 255, 220, 120, 40, 255];
        let bypass = render(&input, 2, 1, r#"{"preset":"None"}"#).unwrap();
        assert_eq!(bypass.composite_rgba, input);
        assert!(bypass.plate_coverages.is_empty());

        assert!(
            render(
                &input,
                2,
                1,
                r#"{"recipe":{"stylize":{"effect":"pixelate","cellSize":4,"amount":1}}}"#,
            )
            .is_ok()
        );
        assert!(
            render(
                &input,
                2,
                1,
                r#"{"recipe":{"stylize":{"effect":"unknown"}}}"#,
            )
            .is_err()
        );
    }
}
