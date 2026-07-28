mod storage;
mod stored_render;

use std::{
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};

pub use stored_render::{RenderError, StoredImage};

/// Linear-light, unassociated RGBA. RGB may be HDR; alpha is always in 0...1.
pub type Pixel = [f32; 4];

pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: PathBuf,
    pub format: String,
    pub bit_depth: u8,
    pub color_profile: Vec<u8>,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub exif: Vec<u8>,
    pub xmp: Vec<u8>,
    pub iptc: Vec<u8>,
    pub camera: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceImage {
    width: NonZeroU32,
    height: NonZeroU32,
    pixels: Vec<Pixel>,
    pub info: SourceInfo,
}

impl SourceImage {
    pub fn new(
        width: NonZeroU32,
        height: NonZeroU32,
        pixels: Vec<Pixel>,
        info: SourceInfo,
    ) -> Result<Self, ImageError> {
        let expected = width.get() as usize * height.get() as usize;
        if pixels.len() != expected {
            return Err(ImageError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        if pixels.iter().flatten().any(|value| !value.is_finite()) {
            return Err(ImageError::NonFinitePixel);
        }
        Ok(Self {
            width,
            height,
            pixels: pixels
                .into_iter()
                .map(|mut pixel| {
                    pixel[..3]
                        .iter_mut()
                        .for_each(|channel| *channel = channel.max(0.0));
                    pixel[3] = pixel[3].clamp(0.0, 1.0);
                    pixel
                })
                .collect(),
            info,
        })
    }

    pub fn width(&self) -> u32 {
        self.width.get()
    }

    pub fn height(&self) -> u32 {
        self.height.get()
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    fn pixel(&self, x: i32, y: i32) -> Pixel {
        let x = x.clamp(0, self.width() as i32 - 1) as usize;
        let y = y.clamp(0, self.height() as i32 - 1) as usize;
        self.pixels[y * self.width() as usize + x]
    }

    fn sample(&self, x: f32, y: f32) -> Pixel {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let tx = x - x.floor();
        let ty = y - y.floor();
        let mut output = [0.0; 4];
        for (pixel, weight) in [
            (self.pixel(x0, y0), (1.0 - tx) * (1.0 - ty)),
            (self.pixel(x0 + 1, y0), tx * (1.0 - ty)),
            (self.pixel(x0, y0 + 1), (1.0 - tx) * ty),
            (self.pixel(x0 + 1, y0 + 1), tx * ty),
        ] {
            for channel in 0..4 {
                output[channel] += pixel[channel] * weight;
            }
        }
        output
    }

    fn sample_nearest(&self, x: f32, y: f32) -> Pixel {
        self.pixel(x.round() as i32, y.round() as i32)
    }
}

fn default_angle() -> f32 {
    45.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ink {
    pub enabled: bool,
    pub color: [f32; 3],
    pub offset: [i32; 2],
    pub angle_degrees: f32,
    pub bleed_pixels: u8,
    pub trapping_pixels: u8,
}

impl Default for Ink {
    fn default() -> Self {
        Self::new([0.02; 3], [0, 0], default_angle())
    }
}

impl Ink {
    pub const fn new(color: [f32; 3], offset: [i32; 2], angle_degrees: f32) -> Self {
        Self {
            enabled: true,
            color,
            offset,
            angle_degrees,
            bleed_pixels: 0,
            trapping_pixels: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Texture {
    pub amount: f32,
    pub scale: f32,
    pub seed: u64,
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            amount: 0.12,
            scale: 3.0,
            seed: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HalftoneShape {
    Dot,
    Line,
    Cross,
    Diamond,
    ClusteredDot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum DitherAlgorithm {
    Bayer {
        matrix_size: u8,
    },
    #[default]
    FloydSteinberg,
    Atkinson,
    SierraLite,
    SierraTwoRow,
    Sierra,
    Stucki,
    Burkes,
    JarvisJudiceNinke,
    BlueNoise,
    Modulation,
    Halftone {
        shape: HalftoneShape,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resampling {
    Nearest,
    #[default]
    Bilinear,
    Supersample2x,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    /// Normalized source bounds: left, top, right, bottom.
    pub crop: [f32; 4],
    /// Clockwise quarter turns in the range 0...3.
    pub quarter_turns: u8,
    pub straighten_degrees: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            crop: [0.0, 0.0, 1.0, 1.0],
            quarter_turns: 0,
            straighten_degrees: 0.0,
        }
    }
}

impl Transform {
    pub fn normalized(self) -> Self {
        let left = self.crop[0].clamp(0.0, 0.99);
        let top = self.crop[1].clamp(0.0, 0.99);
        let right = self.crop[2].clamp(left + 0.01, 1.0);
        let bottom = self.crop[3].clamp(top + 0.01, 1.0);
        Self {
            crop: [left, top, right, bottom],
            quarter_turns: self.quarter_turns % 4,
            straighten_degrees: self.straighten_degrees.clamp(-45.0, 45.0),
        }
    }

    pub fn is_identity(self) -> bool {
        self.normalized() == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DitherSettings {
    pub algorithm: DitherAlgorithm,
    pub strength: f32,
    pub seed: u64,
}

impl Default for DitherSettings {
    fn default() -> Self {
        Self {
            algorithm: DitherAlgorithm::default(),
            strength: 1.0,
            seed: 7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preprocess {
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
    pub blur_radius: f32,
    pub sharpen: f32,
    pub black_point: f32,
    pub white_point: f32,
    pub denoise: f32,
    pub invert: bool,
}

impl Default for Preprocess {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            blur_radius: 0.0,
            sharpen: 0.0,
            black_point: 0.0,
            white_point: 1.0,
            denoise: 0.0,
            invert: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monochrome {
    pub ink: Ink,
    pub threshold: f32,
    pub softness: f32,
}

impl Default for Monochrome {
    fn default() -> Self {
        Self {
            ink: Ink::default(),
            threshold: 0.5,
            softness: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreeColor {
    pub cyan: Ink,
    pub magenta: Ink,
    pub yellow: Ink,
    pub threshold: f32,
    pub softness: f32,
}

impl Default for ThreeColor {
    fn default() -> Self {
        Self {
            cyan: Ink::new([0.0, 0.55, 0.65], [-1, 0], 15.0),
            magenta: Ink::new([0.85, 0.05, 0.35], [1, 0], 75.0),
            yellow: Ink::new([0.95, 0.72, 0.05], [0, 1], 0.0),
            threshold: 0.5,
            softness: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FourColor {
    pub cyan: Ink,
    pub magenta: Ink,
    pub yellow: Ink,
    pub black: Ink,
}

impl Default for FourColor {
    fn default() -> Self {
        Self {
            cyan: Ink::new([0.0, 0.65, 0.78], [0, 0], 15.0),
            magenta: Ink::new([0.88, 0.03, 0.4], [0, 0], 75.0),
            yellow: Ink::new([0.98, 0.78, 0.02], [0, 0], 0.0),
            black: Ink::new([0.02; 3], [0, 0], 45.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaletteSettings {
    pub colors: Vec<[f32; 3]>,
    pub inks: Vec<Ink>,
    pub size: u8,
}

impl Default for PaletteSettings {
    fn default() -> Self {
        let colors = vec![[0.02; 3], [0.94, 0.92, 0.84]];
        Self {
            inks: colors
                .iter()
                .enumerate()
                .map(|(index, color)| Ink::new(*color, [0, 0], plate_angle(index)))
                .collect(),
            colors,
            size: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneBand {
    pub range: [f32; 2],
    pub ink: Ink,
    pub intensity: f32,
    pub grain: Texture,
}

impl Default for ToneBand {
    fn default() -> Self {
        Self {
            range: [0.0, 1.0],
            ink: Ink::default(),
            intensity: 1.0,
            grain: Texture::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriTone {
    pub shadows: ToneBand,
    pub midtones: ToneBand,
    pub highlights: ToneBand,
}

impl Default for TriTone {
    fn default() -> Self {
        Self {
            shadows: ToneBand {
                range: [0.0, 0.42],
                ink: Ink::new([0.03, 0.03, 0.04], [-1, 0], 45.0),
                ..ToneBand::default()
            },
            midtones: ToneBand {
                range: [0.25, 0.75],
                ink: Ink::new([0.65, 0.08, 0.14], [1, 0], 75.0),
                grain: Texture {
                    seed: 2,
                    ..Texture::default()
                },
                ..ToneBand::default()
            },
            highlights: ToneBand {
                range: [0.58, 1.0],
                ink: Ink::new([0.95, 0.74, 0.12], [0, 1], 15.0),
                grain: Texture {
                    seed: 3,
                    ..Texture::default()
                },
                ..ToneBand::default()
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Separation {
    Monochrome(Monochrome),
    /// Kept as the legacy serialized name for CMY process separation.
    ThreeColor(ThreeColor),
    Tonal(PaletteSettings),
    Indexed(PaletteSettings),
    Custom(PaletteSettings),
    Rgb(ThreeColor),
    Cmyk(FourColor),
    TriTone(TriTone),
}

impl Default for Separation {
    fn default() -> Self {
        Self::Monochrome(Monochrome::default())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintSettings {
    pub dpi: f32,
    pub lpi: f32,
    pub bleed_pixels: u8,
    pub trapping_pixels: u8,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            dpi: 300.0,
            lpi: 45.0,
            bleed_pixels: 0,
            trapping_pixels: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glow {
    pub enabled: bool,
    pub threshold: f32,
    pub radius: f32,
    pub falloff: f32,
    pub intensity: f32,
    pub tint: [f32; 3],
    pub gamma: f32,
    pub saturation: f32,
}

impl Default for Glow {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.7,
            radius: 12.0,
            falloff: 2.0,
            intensity: 0.5,
            tint: [1.0, 0.8, 0.55],
            gamma: 1.0,
            saturation: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Displacement {
    pub enabled: bool,
    pub x_strength: f32,
    pub y_strength: f32,
    pub distress_amount: f32,
    pub pattern: MapPattern,
    pub pattern_scale: f32,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapPattern {
    #[default]
    Imported,
    Grain,
    Halftone,
    Grunge,
    Splatter,
}

impl Default for Displacement {
    fn default() -> Self {
        Self {
            enabled: false,
            x_strength: 0.0,
            y_strength: 0.0,
            distress_amount: 0.0,
            pattern: MapPattern::Imported,
            pattern_scale: 18.0,
            seed: 19,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrtPhase {
    #[default]
    Waveform,
    Linear,
    Flux,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrtEffect {
    pub enabled: bool,
    pub phase: CrtPhase,
    pub wave_strength: f32,
    pub wave_frequency: f32,
    pub scanlines: f32,
    pub rgb_bleed: f32,
    pub sync_tearing: f32,
    pub phosphor_mask: f32,
    pub bloom: f32,
    pub seed: u64,
}

impl Default for CrtEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            phase: CrtPhase::default(),
            wave_strength: 0.0,
            wave_frequency: 8.0,
            scanlines: 0.0,
            rgb_bleed: 0.0,
            sync_tearing: 0.0,
            phosphor_mask: 0.0,
            bloom: 0.0,
            seed: 13,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetPaths {
    pub paper_texture: Option<PathBuf>,
    pub displacement_map: Option<PathBuf>,
    pub distress_mask: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recipe {
    pub separation: Separation,
    pub dither: DitherSettings,
    pub resampling: Resampling,
    pub transform: Transform,
    pub preprocess: Preprocess,
    pub print: PrintSettings,
    pub glow: Glow,
    pub displacement: Displacement,
    pub crt: CrtEffect,
    pub grain: Texture,
    pub paper: Texture,
    pub paper_color: [f32; 3],
    pub assets: AssetPaths,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            separation: Separation::default(),
            dither: DitherSettings::default(),
            resampling: Resampling::default(),
            transform: Transform::default(),
            preprocess: Preprocess::default(),
            print: PrintSettings::default(),
            glow: Glow::default(),
            displacement: Displacement::default(),
            crt: CrtEffect::default(),
            grain: Texture {
                amount: 0.08,
                scale: 1.2,
                seed: 2,
            },
            paper: Texture::default(),
            paper_color: [0.94, 0.92, 0.84],
            assets: AssetPaths::default(),
        }
    }
}

pub fn built_in_presets() -> &'static [(&'static str, Recipe)] {
    static PRESETS: OnceLock<Vec<(&'static str, Recipe)>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        let classic = Recipe::default();

        let mut newspaper = Recipe::default();
        newspaper.dither.algorithm = DitherAlgorithm::Halftone {
            shape: HalftoneShape::ClusteredDot,
        };
        newspaper.print.lpi = 55.0;

        let mut xerox = Recipe::default();
        xerox.dither.algorithm = DitherAlgorithm::Atkinson;
        xerox.grain.amount = 0.28;
        xerox.paper.amount = 0.22;

        let mut modulation = Recipe::default();
        modulation.dither.algorithm = DitherAlgorithm::Modulation;
        modulation.preprocess.contrast = 1.25;

        let mut retro = Recipe::default();
        retro.dither.algorithm = DitherAlgorithm::Bayer { matrix_size: 4 };
        retro.separation = Separation::Custom(preset_palette(&[
            [0.04, 0.04, 0.08],
            [0.25, 0.18, 0.45],
            [0.82, 0.25, 0.34],
            [0.98, 0.78, 0.32],
            [0.92, 0.92, 0.78],
        ]));

        let warm = Recipe {
            separation: Separation::Custom(preset_palette(&[
                [0.05, 0.03, 0.03],
                [0.55, 0.08, 0.05],
                [0.95, 0.70, 0.20],
                [0.96, 0.92, 0.78],
            ])),
            ..Recipe::default()
        };

        let mut dream_glow = Recipe::default();
        dream_glow.glow.enabled = true;
        dream_glow.glow.radius = 24.0;
        dream_glow.glow.intensity = 0.8;
        dream_glow.glow.falloff = 2.0;

        let mut crt_waveform = Recipe::default();
        crt_waveform.crt.enabled = true;
        crt_waveform.crt.phase = CrtPhase::Waveform;
        crt_waveform.crt.wave_strength = 14.0;
        crt_waveform.crt.scanlines = 0.35;
        crt_waveform.crt.rgb_bleed = 3.0;

        let mut crt_linear = crt_waveform.clone();
        crt_linear.crt.phase = CrtPhase::Linear;
        crt_linear.crt.wave_strength = 22.0;

        let mut crt_flux = crt_waveform.clone();
        crt_flux.crt.phase = CrtPhase::Flux;
        crt_flux.crt.wave_strength = 32.0;
        crt_flux.crt.sync_tearing = 18.0;
        crt_flux.crt.bloom = 0.25;

        let mut grunge = Recipe::default();
        grunge.displacement.enabled = true;
        grunge.displacement.pattern = MapPattern::Grunge;
        grunge.displacement.x_strength = 12.0;
        grunge.displacement.y_strength = 5.0;
        grunge.displacement.distress_amount = 0.35;

        let mut cmyk = Recipe {
            separation: Separation::Cmyk(FourColor::default()),
            ..Recipe::default()
        };
        cmyk.dither.algorithm = DitherAlgorithm::Halftone {
            shape: HalftoneShape::Dot,
        };
        cmyk.print.lpi = 45.0;

        vec![
            ("Classic diffusion", classic),
            ("Newspaper screen", newspaper),
            ("Dry Xerox", xerox),
            ("Modulated bitmap", modulation),
            ("Retro five-color", retro),
            ("Warm poster", warm),
            ("Dream glow", dream_glow),
            ("CRT waveform", crt_waveform),
            ("CRT linear", crt_linear),
            ("CRT flux", crt_flux),
            ("Grunge displacement", grunge),
            ("CMYK print", cmyk),
        ]
    })
}

fn preset_palette(colors: &[[f32; 3]]) -> PaletteSettings {
    PaletteSettings {
        colors: colors.to_vec(),
        inks: colors
            .iter()
            .enumerate()
            .map(|(index, color)| Ink::new(*color, [0, 0], plate_angle(index)))
            .collect(),
        size: colors.len() as u8,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    PaperTexture,
    DisplacementMap,
    DistressMask,
}

#[derive(Clone, Debug, Default)]
struct Assets {
    paper_texture: Option<Arc<SourceImage>>,
    displacement_map: Option<Arc<SourceImage>>,
    distress_mask: Option<Arc<SourceImage>>,
}

/// An immutable source plus editable instructions and immutable user-supplied assets.
#[derive(Clone, Debug)]
pub struct Document {
    source: Arc<SourceImage>,
    assets: Assets,
    pub recipe: Recipe,
}

impl Document {
    pub fn new(source: SourceImage) -> Self {
        Self {
            source: Arc::new(source),
            assets: Assets::default(),
            recipe: Recipe::default(),
        }
    }

    pub fn source(&self) -> &SourceImage {
        &self.source
    }

    pub fn set_asset(&mut self, kind: AssetKind, image: SourceImage) {
        let image = Some(Arc::new(image));
        match kind {
            AssetKind::PaperTexture => self.assets.paper_texture = image,
            AssetKind::DisplacementMap => self.assets.displacement_map = image,
            AssetKind::DistressMask => self.assets.distress_mask = image,
        }
    }

    pub fn clear_asset(&mut self, kind: AssetKind) {
        match kind {
            AssetKind::PaperTexture => self.assets.paper_texture = None,
            AssetKind::DisplacementMap => self.assets.displacement_map = None,
            AssetKind::DistressMask => self.assets.distress_mask = None,
        }
    }

    pub fn output_dimensions(&self) -> (NonZeroU32, NonZeroU32) {
        transformed_dimensions(&self.source, self.recipe.transform)
    }

    pub fn plate_names(&self) -> Vec<String> {
        match &self.recipe.separation {
            Separation::Monochrome(settings) => settings
                .ink
                .enabled
                .then(|| "black".to_owned())
                .into_iter()
                .collect(),
            Separation::ThreeColor(settings) => enabled_named_inks(&[
                ("cyan", settings.cyan),
                ("magenta", settings.magenta),
                ("yellow", settings.yellow),
            ]),
            Separation::Rgb(settings) => enabled_named_inks(&[
                ("red", settings.cyan),
                ("green", settings.magenta),
                ("blue", settings.yellow),
            ]),
            Separation::Cmyk(settings) => enabled_named_inks(&[
                ("cyan", settings.cyan),
                ("magenta", settings.magenta),
                ("yellow", settings.yellow),
                ("black", settings.black),
            ]),
            Separation::TriTone(settings) => enabled_named_inks(&[
                ("shadows", settings.shadows.ink),
                ("midtones", settings.midtones.ink),
                ("highlights", settings.highlights.ink),
            ]),
            Separation::Tonal(settings) => palette_plate_names("tone", settings),
            Separation::Indexed(settings) => palette_plate_names("index", settings),
            Separation::Custom(settings) => palette_plate_names("color", settings),
        }
    }

    pub fn render(&self) -> RenderedImage {
        let (width, height) = self.output_dimensions();
        self.render_document_to(width, height).composite
    }

    pub fn render_document(&self) -> RenderedDocument {
        let (width, height) = self.output_dimensions();
        self.render_document_to(width, height)
    }

    pub fn render_preview(&self, max_dimension: NonZeroU32) -> RenderedImage {
        let (width, height) = preview_dimensions(self.output_dimensions(), max_dimension);
        self.render_document_to(width, height).composite
    }

    pub fn render_document_preview(&self, max_dimension: NonZeroU32) -> RenderedDocument {
        let (width, height) = preview_dimensions(self.output_dimensions(), max_dimension);
        self.render_document_to(width, height)
    }

    pub fn render_source_preview(&self, max_dimension: NonZeroU32) -> RenderedImage {
        let (width, height) = preview_dimensions(self.output_dimensions(), max_dimension);
        let width_usize = width.get() as usize;
        let height_usize = height.get() as usize;
        let pixels = (0..height_usize)
            .flat_map(|y| {
                (0..width_usize).map(move |x| {
                    self.sample_transformed(
                        x as f32,
                        y as f32,
                        width_usize,
                        height_usize,
                        Resampling::Bilinear,
                    )
                })
            })
            .collect();
        RenderedImage::new(width, height, pixels)
    }

    pub fn extract_palette(&self, count: u8) -> Vec<[f32; 3]> {
        let (width, height) = self.output_dimensions();
        let (width, height) = (width.get() as usize, height.get() as usize);
        let mut pixels = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    self.sample_transformed(x as f32, y as f32, width, height, Resampling::Bilinear)
                })
            })
            .collect::<Vec<_>>();
        preprocess(&mut pixels, width, height, self.recipe.preprocess);
        extract_palette(&pixels, count.clamp(2, 64) as usize)
    }

    fn render_document_to(
        &self,
        output_width: NonZeroU32,
        output_height: NonZeroU32,
    ) -> RenderedDocument {
        if self.recipe.resampling == Resampling::Supersample2x
            && let (Some(width), Some(height)) = (
                output_width.get().checked_mul(2),
                output_height.get().checked_mul(2),
            )
        {
            let mut high_resolution = self.clone();
            high_resolution.recipe.resampling = Resampling::Bilinear;
            return downsample_document(
                high_resolution.render_document_to(
                    NonZeroU32::new(width).unwrap(),
                    NonZeroU32::new(height).unwrap(),
                ),
                output_width,
                output_height,
            );
        }
        let width = output_width.get() as usize;
        let height = output_height.get() as usize;
        let (full_width, full_height) = self.output_dimensions();
        let scale =
            (width as f32 / full_width.get() as f32).min(height as f32 / full_height.get() as f32);
        let mut pixels = self.resample_effects(width, height, scale);
        let mut preprocess_settings = self.recipe.preprocess;
        preprocess_settings.blur_radius *= scale;
        preprocess(&mut pixels, width, height, preprocess_settings);
        let mut glow = self.recipe.glow;
        glow.radius *= scale;
        apply_glow(&mut pixels, width, height, glow);
        apply_crt_surface(&mut pixels, width, height, self.recipe.crt);

        let mut plates: Vec<_> = self
            .make_plates(&pixels, width, height)
            .into_iter()
            .filter(|plate| plate.ink.enabled)
            .collect();
        let multiple_plates = plates.len() > 1;
        for plate in &mut plates {
            for (coverage, pixel) in plate.coverage.iter_mut().zip(&pixels) {
                *coverage *= pixel[3];
            }
            let _ = apply_distress(
                &mut plate.coverage,
                width,
                height,
                self.assets.distress_mask.as_deref(),
                self.recipe.displacement,
                scale,
                None,
            );
            let expansion = ((self.recipe.print.bleed_pixels as f32
                + plate.ink.bleed_pixels as f32
                + if multiple_plates {
                    self.recipe.print.trapping_pixels as f32 + plate.ink.trapping_pixels as f32
                } else {
                    0.0
                })
                * scale)
                .round() as usize;
            if expansion > 0 {
                plate.coverage = dilate(&plate.coverage, width, height, expansion);
            }
            plate.coverage = shift_mask(
                &plate.coverage,
                width,
                height,
                (plate.ink.offset[0] as f32 * scale).round() as i32,
                (plate.ink.offset[1] as f32 * scale).round() as i32,
            );
        }

        let alpha: Vec<f32> = pixels.iter().map(|pixel| pixel[3]).collect();
        let paper = self.paper(width, height);
        let composite = compose(output_width, output_height, &paper, &alpha, &plates);
        RenderedDocument { composite, plates }
    }

    fn resample_effects(&self, width: usize, height: usize, scale: f32) -> Vec<Pixel> {
        (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let mut ox = 0.0;
                    let mut oy = 0.0;
                    if self.recipe.displacement.enabled
                        && let Some(sample) = displacement_sample(
                            self.assets.displacement_map.as_deref(),
                            x,
                            y,
                            width,
                            height,
                            self.recipe.displacement,
                            scale,
                        )
                    {
                        ox += (sample[0] * 2.0 - 1.0) * self.recipe.displacement.x_strength * scale;
                        oy += (sample[1] * 2.0 - 1.0) * self.recipe.displacement.y_strength * scale;
                    }
                    if self.recipe.crt.enabled {
                        let phase = y as f32 / height.max(1) as f32
                            * self.recipe.crt.wave_frequency
                            * std::f32::consts::TAU;
                        match self.recipe.crt.phase {
                            CrtPhase::Waveform => {
                                ox += phase.sin() * self.recipe.crt.wave_strength * scale;
                            }
                            CrtPhase::Linear => {
                                ox += ((phase / std::f32::consts::TAU).fract() * 2.0 - 1.0)
                                    * self.recipe.crt.wave_strength
                                    * scale;
                            }
                            CrtPhase::Flux => {
                                let turbulence =
                                    random(x as i32 / 8, y as i32 / 8, self.recipe.crt.seed) - 0.5;
                                ox += (phase.sin() + (phase * 0.37).sin() + turbulence)
                                    * self.recipe.crt.wave_strength
                                    * scale
                                    * 0.6;
                                oy += (phase * 0.53).cos()
                                    * self.recipe.crt.wave_strength
                                    * scale
                                    * 0.12;
                            }
                        }
                        let tear = random(0, y as i32 / 3, self.recipe.crt.seed);
                        if tear > 0.88 {
                            ox += tear * self.recipe.crt.sync_tearing * scale;
                        }
                    }
                    let mut pixel = self.sample_transformed(
                        x as f32 + ox,
                        y as f32 + oy,
                        width,
                        height,
                        self.recipe.resampling,
                    );
                    if self.recipe.crt.enabled && self.recipe.crt.rgb_bleed > 0.0 {
                        let bleed = self.recipe.crt.rgb_bleed * scale;
                        pixel[0] = self.sample_transformed(
                            x as f32 + ox + bleed,
                            y as f32 + oy,
                            width,
                            height,
                            self.recipe.resampling,
                        )[0];
                        pixel[2] = self.sample_transformed(
                            x as f32 + ox - bleed,
                            y as f32 + oy,
                            width,
                            height,
                            self.recipe.resampling,
                        )[2];
                    }
                    pixel
                })
            })
            .collect()
    }

    fn sample_transformed(
        &self,
        x: f32,
        y: f32,
        width: usize,
        height: usize,
        resampling: Resampling,
    ) -> Pixel {
        let Some((source_x, source_y, source_scale_x, source_scale_y)) =
            transformed_source_coordinates(
                &self.source,
                self.recipe.transform,
                x,
                y,
                width,
                height,
            )
        else {
            return [0.0; 4];
        };
        sample_source(
            &self.source,
            source_x,
            source_y,
            source_scale_x,
            source_scale_y,
            resampling,
        )
    }

    fn paper(&self, width: usize, height: usize) -> Vec<[f32; 3]> {
        (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let procedural = noise(
                        x as f32 / self.recipe.paper.scale.max(0.01),
                        y as f32 / self.recipe.paper.scale.max(0.01),
                        self.recipe.paper.seed,
                    );
                    let imported = self
                        .assets
                        .paper_texture
                        .as_deref()
                        .map(|image| {
                            luminance(sample_normalized(image, x, y, width, height)) * 2.0 - 1.0
                        })
                        .unwrap_or(0.0);
                    self.recipe.paper_color.map(|channel| {
                        (channel * (1.0 + (procedural + imported) * self.recipe.paper.amount))
                            .max(0.0)
                    })
                })
            })
            .collect()
    }

    fn make_plates(&self, pixels: &[Pixel], width: usize, height: usize) -> Vec<RenderedPlate> {
        match &self.recipe.separation {
            Separation::Monochrome(settings) => vec![self.scalar_plate(
                "black",
                settings.ink,
                pixels.iter().map(|pixel| 1.0 - luminance(*pixel)).collect(),
                settings.threshold,
                settings.softness,
                width,
                height,
                0,
            )],
            Separation::ThreeColor(settings) => self.cmy_plates(*settings, pixels, width, height),
            Separation::Rgb(settings) => [
                ("red", settings.cyan, 0),
                ("green", settings.magenta, 1),
                ("blue", settings.yellow, 2),
            ]
            .into_iter()
            .map(|(name, ink, channel)| {
                self.scalar_plate(
                    name,
                    ink,
                    pixels.iter().map(|pixel| pixel[channel]).collect(),
                    settings.threshold,
                    settings.softness,
                    width,
                    height,
                    channel as u64,
                )
            })
            .collect(),
            Separation::Cmyk(settings) => {
                let mut values = vec![[0.0; 4]; pixels.len()];
                for (output, pixel) in values.iter_mut().zip(pixels) {
                    let c = 1.0 - pixel[0].clamp(0.0, 1.0);
                    let m = 1.0 - pixel[1].clamp(0.0, 1.0);
                    let y = 1.0 - pixel[2].clamp(0.0, 1.0);
                    let k = c.min(m).min(y);
                    let denominator = (1.0 - k).max(0.001);
                    *output = [
                        (c - k) / denominator,
                        (m - k) / denominator,
                        (y - k) / denominator,
                        k,
                    ];
                }
                [
                    ("cyan", settings.cyan, 0),
                    ("magenta", settings.magenta, 1),
                    ("yellow", settings.yellow, 2),
                    ("black", settings.black, 3),
                ]
                .into_iter()
                .map(|(name, ink, channel)| {
                    self.scalar_plate(
                        name,
                        ink,
                        values.iter().map(|value| value[channel]).collect(),
                        0.5,
                        0.5,
                        width,
                        height,
                        channel as u64,
                    )
                })
                .collect()
            }
            Separation::TriTone(settings) => [
                ("shadows", settings.shadows),
                ("midtones", settings.midtones),
                ("highlights", settings.highlights),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (name, band))| {
                let values = pixels
                    .iter()
                    .enumerate()
                    .map(|(pixel_index, pixel)| {
                        let tone = luminance(*pixel);
                        let feather = 0.08;
                        let inside =
                            smoothstep(band.range[0] - feather, band.range[0] + feather, tone)
                                * (1.0
                                    - smoothstep(
                                        band.range[1] - feather,
                                        band.range[1] + feather,
                                        tone,
                                    ));
                        let x = pixel_index % width;
                        let y = pixel_index / width;
                        let grain = noise(
                            x as f32 / band.grain.scale.max(0.01),
                            y as f32 / band.grain.scale.max(0.01),
                            band.grain.seed,
                        ) * band.grain.amount;
                        (inside * band.intensity + grain).clamp(0.0, 1.0)
                    })
                    .collect();
                self.scalar_plate(
                    name,
                    band.ink,
                    values,
                    0.5,
                    0.5,
                    width,
                    height,
                    index as u64,
                )
            })
            .collect(),
            Separation::Tonal(settings) => {
                self.palette_plates("tone", settings, pixels, width, height, true)
            }
            Separation::Indexed(settings) => {
                self.palette_plates("index", settings, pixels, width, height, false)
            }
            Separation::Custom(settings) => {
                self.palette_plates("color", settings, pixels, width, height, false)
            }
        }
    }

    fn cmy_plates(
        &self,
        settings: ThreeColor,
        pixels: &[Pixel],
        width: usize,
        height: usize,
    ) -> Vec<RenderedPlate> {
        [
            ("cyan", settings.cyan, 0),
            ("magenta", settings.magenta, 1),
            ("yellow", settings.yellow, 2),
        ]
        .into_iter()
        .map(|(name, ink, channel)| {
            self.scalar_plate(
                name,
                ink,
                pixels.iter().map(|pixel| 1.0 - pixel[channel]).collect(),
                settings.threshold,
                settings.softness,
                width,
                height,
                channel as u64,
            )
        })
        .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn scalar_plate(
        &self,
        name: &str,
        ink: Ink,
        mut values: Vec<f32>,
        threshold: f32,
        softness: f32,
        width: usize,
        height: usize,
        plate: u64,
    ) -> RenderedPlate {
        for (index, value) in values.iter_mut().enumerate() {
            let x = index % width;
            let y = index / width;
            let grain = noise(
                x as f32 / self.recipe.grain.scale.max(0.01),
                y as f32 / self.recipe.grain.scale.max(0.01),
                self.recipe.grain.seed.wrapping_add(plate),
            ) * self.recipe.grain.amount;
            *value = (adjust_coverage(*value, threshold, softness) + grain).clamp(0.0, 1.0);
        }
        let _ = dither_scalar(
            &mut values,
            width,
            self.recipe.dither,
            self.scaled_print(width, height),
            ink.angle_degrees,
            plate,
            None,
        );
        RenderedPlate {
            name: name.into(),
            ink,
            coverage: values,
        }
    }

    fn palette_plates(
        &self,
        prefix: &str,
        settings: &PaletteSettings,
        pixels: &[Pixel],
        width: usize,
        height: usize,
        tonal: bool,
    ) -> Vec<RenderedPlate> {
        let mut colors = if settings.colors.len() >= 2 {
            valid_palette(&settings.colors)
        } else {
            extract_palette(pixels, settings.size.clamp(2, 64) as usize)
        };
        if tonal {
            colors.sort_by(|a, b| luminance3(*a).total_cmp(&luminance3(*b)));
        }
        let (dither_pixels, dither_colors) = if tonal {
            (
                pixels
                    .iter()
                    .map(|pixel| {
                        let value = luminance(*pixel);
                        [value, value, value, pixel[3]]
                    })
                    .collect::<Vec<_>>(),
                colors
                    .iter()
                    .map(|color| [luminance3(*color); 3])
                    .collect::<Vec<_>>(),
            )
        } else {
            (pixels.to_vec(), colors.clone())
        };
        let assignments = dither_palette(
            &dither_pixels,
            &dither_colors,
            width,
            height,
            self.recipe.dither,
            self.scaled_print(width, height),
        );
        colors
            .iter()
            .enumerate()
            .map(|(color_index, color)| RenderedPlate {
                name: format!("{prefix}-{:02}", color_index + 1),
                ink: settings
                    .inks
                    .get(color_index)
                    .copied()
                    .map(|mut ink| {
                        ink.color = *color;
                        ink
                    })
                    .unwrap_or_else(|| Ink::new(*color, [0, 0], plate_angle(color_index))),
                coverage: assignments
                    .iter()
                    .map(|assigned| usize::from(*assigned) == color_index)
                    .map(u8::from)
                    .map(f32::from)
                    .collect(),
            })
            .collect()
    }

    fn scaled_print(&self, width: usize, height: usize) -> PrintSettings {
        let (full_width, full_height) = self.output_dimensions();
        let scale =
            (width as f32 / full_width.get() as f32).min(height as f32 / full_height.get() as f32);
        PrintSettings {
            dpi: self.recipe.print.dpi * scale,
            ..self.recipe.print
        }
    }
}

fn enabled_named_inks(inks: &[(&str, Ink)]) -> Vec<String> {
    inks.iter()
        .filter(|(_, ink)| ink.enabled)
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

fn palette_plate_names(prefix: &str, settings: &PaletteSettings) -> Vec<String> {
    let count = if settings.colors.len() >= 2 {
        settings.colors.len().min(64)
    } else {
        settings.size.clamp(2, 64) as usize
    };
    (0..count)
        .filter(|index| settings.inks.get(*index).is_none_or(|ink| ink.enabled))
        .map(|index| format!("{prefix}-{:02}", index + 1))
        .collect()
}

fn downsample_document(
    rendered: RenderedDocument,
    width: NonZeroU32,
    height: NonZeroU32,
) -> RenderedDocument {
    let high_width = rendered.composite.width() as usize;
    let sample = |values: &[Pixel], x: usize, y: usize| {
        let mut output = [0.0; 4];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let pixel = values[(y * 2 + dy) * high_width + x * 2 + dx];
            for channel in 0..4 {
                output[channel] += pixel[channel] * 0.25;
            }
        }
        output
    };
    let mut composite_pixels = Vec::with_capacity(width.get() as usize * height.get() as usize);
    for y in 0..height.get() as usize {
        for x in 0..width.get() as usize {
            composite_pixels.push(sample(rendered.composite.pixels(), x, y));
        }
    }
    let plates = rendered
        .plates
        .into_iter()
        .map(|plate| {
            let mut coverage = Vec::with_capacity(width.get() as usize * height.get() as usize);
            for y in 0..height.get() as usize {
                for x in 0..width.get() as usize {
                    coverage.push(
                        [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .into_iter()
                            .map(|(dx, dy)| plate.coverage[(y * 2 + dy) * high_width + x * 2 + dx])
                            .sum::<f32>()
                            * 0.25,
                    );
                }
            }
            RenderedPlate { coverage, ..plate }
        })
        .collect();
    RenderedDocument {
        composite: RenderedImage::new(width, height, composite_pixels),
        plates,
    }
}

fn plate_angle(index: usize) -> f32 {
    [45.0, 15.0, 75.0, 0.0][index % 4]
}

fn valid_palette(colors: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if colors.len() >= 2 {
        colors.iter().take(64).copied().collect()
    } else {
        PaletteSettings::default().colors
    }
}

fn transformed_dimensions(source: &SourceImage, transform: Transform) -> (NonZeroU32, NonZeroU32) {
    let transform = transform.normalized();
    let crop_width =
        (source.width() as f32 * (transform.crop[2] - transform.crop[0])).round() as u32;
    let crop_height =
        (source.height() as f32 * (transform.crop[3] - transform.crop[1])).round() as u32;
    let crop_width = NonZeroU32::new(crop_width).unwrap_or(NonZeroU32::MIN);
    let crop_height = NonZeroU32::new(crop_height).unwrap_or(NonZeroU32::MIN);
    if transform.quarter_turns.is_multiple_of(2) {
        (crop_width, crop_height)
    } else {
        (crop_height, crop_width)
    }
}

fn transformed_source_coordinates(
    source: &SourceImage,
    transform: Transform,
    x: f32,
    y: f32,
    width: usize,
    height: usize,
) -> Option<(f32, f32, f32, f32)> {
    let transform = transform.normalized();
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let center_x = x + 0.5 - width * 0.5;
    let center_y = y + 0.5 - height * 0.5;
    let radians = -transform.straighten_degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let rotated_x = center_x * cos - center_y * sin;
    let rotated_y = center_x * sin + center_y * cos;
    let u = (rotated_x + width * 0.5) / width;
    let v = (rotated_y + height * 0.5) / height;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    let (crop_u, crop_v) = match transform.quarter_turns {
        0 => (u, v),
        1 => (v, 1.0 - u),
        2 => (1.0 - u, 1.0 - v),
        3 => (1.0 - v, u),
        _ => unreachable!(),
    };
    let crop_width = transform.crop[2] - transform.crop[0];
    let crop_height = transform.crop[3] - transform.crop[1];
    let source_u = transform.crop[0] + crop_u * crop_width;
    let source_v = transform.crop[1] + crop_v * crop_height;
    let source_x = source_u * source.width() as f32 - 0.5;
    let source_y = source_v * source.height() as f32 - 0.5;
    let oriented_source_width = if transform.quarter_turns.is_multiple_of(2) {
        source.width() as f32 * crop_width
    } else {
        source.height() as f32 * crop_height
    };
    let oriented_source_height = if transform.quarter_turns.is_multiple_of(2) {
        source.height() as f32 * crop_height
    } else {
        source.width() as f32 * crop_width
    };
    Some((
        source_x,
        source_y,
        oriented_source_width / width,
        oriented_source_height / height,
    ))
}

fn preview_dimensions(
    dimensions: (NonZeroU32, NonZeroU32),
    max_dimension: NonZeroU32,
) -> (NonZeroU32, NonZeroU32) {
    let (source_width, source_height) = dimensions;
    let scale =
        (max_dimension.get() as f32 / source_width.get().max(source_height.get()) as f32).min(1.0);
    let width = NonZeroU32::new((source_width.get() as f32 * scale).round() as u32)
        .unwrap_or(NonZeroU32::MIN);
    let height = NonZeroU32::new((source_height.get() as f32 * scale).round() as u32)
        .unwrap_or(NonZeroU32::MIN);
    (width, height)
}

fn sample_normalized(
    source: &SourceImage,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Pixel {
    source.sample(
        (x as f32 + 0.5) * source.width() as f32 / width as f32 - 0.5,
        (y as f32 + 0.5) * source.height() as f32 / height as f32 - 0.5,
    )
}

fn sample_source(
    source: &SourceImage,
    x: f32,
    y: f32,
    scale_x: f32,
    scale_y: f32,
    resampling: Resampling,
) -> Pixel {
    match resampling {
        Resampling::Nearest => source.sample_nearest(x, y),
        Resampling::Bilinear => source.sample(x, y),
        Resampling::Supersample2x => {
            let mut output = [0.0; 4];
            for (dx, dy) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                let pixel = source.sample(x + dx * scale_x, y + dy * scale_y);
                for channel in 0..4 {
                    output[channel] += pixel[channel] * 0.25;
                }
            }
            output
        }
    }
}

fn displacement_sample(
    imported: Option<&SourceImage>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    settings: Displacement,
    render_scale: f32,
) -> Option<[f32; 2]> {
    if settings.pattern == MapPattern::Imported {
        return imported
            .map(|image| sample_normalized(image, x, y, width, height))
            .map(|pixel| [pixel[0], pixel[1]]);
    }
    let scale = (settings.pattern_scale * render_scale).max(0.5);
    Some([
        pattern_value(settings.pattern, x as f32, y as f32, scale, settings.seed),
        pattern_value(
            settings.pattern,
            x as f32,
            y as f32,
            scale,
            settings.seed ^ 0x9E37_79B9,
        ),
    ])
}

fn pattern_value(pattern: MapPattern, x: f32, y: f32, scale: f32, seed: u64) -> f32 {
    match pattern {
        MapPattern::Imported => 0.5,
        MapPattern::Grain => 0.5 + noise(x / scale, y / scale, seed) * 0.5,
        MapPattern::Halftone => {
            let phase = std::f32::consts::TAU / scale;
            0.5 + 0.5 * (x * phase).sin() * (y * phase).sin()
        }
        MapPattern::Grunge => {
            let broad = noise(x / scale, y / scale, seed);
            let detail = noise(x / (scale * 0.27), y / (scale * 0.27), seed ^ 0xA511_E9B3);
            ((broad * 0.75 + detail * 0.25) * 1.8 + 0.5).clamp(0.0, 1.0)
        }
        MapPattern::Splatter => splatter(x, y, scale, seed),
    }
}

fn splatter(x: f32, y: f32, scale: f32, seed: u64) -> f32 {
    let cell_x = (x / scale).floor() as i32;
    let cell_y = (y / scale).floor() as i32;
    for cy in cell_y - 1..=cell_y + 1 {
        for cx in cell_x - 1..=cell_x + 1 {
            let unit = |variation| random(cx, cy, variation) * 0.5 + 0.5;
            let center_x = (cx as f32 + unit(seed)) * scale;
            let center_y = (cy as f32 + unit(seed ^ 0xC2B2_AE35)) * scale;
            let radius = scale * (0.08 + unit(seed ^ 0x27D4_EB2F) * 0.32);
            if (x - center_x).hypot(y - center_y) <= radius {
                return 0.0;
            }
        }
    }
    1.0
}

fn preprocess(pixels: &mut [Pixel], width: usize, height: usize, settings: Preprocess) {
    if settings.denoise > 0.0 {
        let median = median_filter(pixels, width, height);
        blend_pixels(pixels, &median, settings.denoise);
    }
    if settings.blur_radius > 0.0 {
        let blurred = gaussian_blur(pixels, width, height, settings.blur_radius);
        pixels.copy_from_slice(&blurred);
    }
    if settings.sharpen > 0.0 {
        let blurred = gaussian_blur(pixels, width, height, 1.25);
        for (pixel, low) in pixels.iter_mut().zip(blurred) {
            for (channel, value) in pixel[..3].iter_mut().enumerate() {
                *value = (*value + (*value - low[channel]) * settings.sharpen).max(0.0);
            }
        }
    }
    let black_point = settings.black_point.min(settings.white_point - 0.001);
    let white_point = settings.white_point.max(settings.black_point + 0.001);
    let span = white_point - black_point;
    for pixel in pixels {
        for channel in &mut pixel[..3] {
            let mut value = ((*channel - black_point) / span).max(0.0);
            value = ((value - 0.5) * settings.contrast + 0.5 + settings.brightness).max(0.0);
            value = value.powf(1.0 / settings.gamma.max(0.01));
            if settings.invert {
                value = 1.0 - value;
            }
            *channel = value.max(0.0);
        }
    }
}

fn median_filter(pixels: &[Pixel], width: usize, height: usize) -> Vec<Pixel> {
    let mut output = pixels.to_vec();
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                let mut values = [0.0; 9];
                let mut next = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let px = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
                        let py = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
                        values[next] = pixels[py * width + px][channel];
                        next += 1;
                    }
                }
                values.sort_by(f32::total_cmp);
                output[y * width + x][channel] = values[4];
            }
        }
    }
    output
}

fn gaussian_blur(pixels: &[Pixel], width: usize, height: usize, radius: f32) -> Vec<Pixel> {
    let radius = radius.clamp(0.1, 64.0);
    let extent = (radius * 2.5).ceil() as i32;
    let sigma2 = 2.0 * radius * radius;
    let kernel: Vec<f32> = (-extent..=extent)
        .map(|offset| (-(offset * offset) as f32 / sigma2).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.into_iter().map(|weight| weight / sum).collect();
    let mut horizontal = vec![[0.0; 4]; pixels.len()];
    let mut output = vec![[0.0; 4]; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let px = (x as i32 + index as i32 - extent).clamp(0, width as i32 - 1) as usize;
                for channel in 0..4 {
                    horizontal[y * width + x][channel] += pixels[y * width + px][channel] * weight;
                }
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let py = (y as i32 + index as i32 - extent).clamp(0, height as i32 - 1) as usize;
                for channel in 0..4 {
                    output[y * width + x][channel] += horizontal[py * width + x][channel] * weight;
                }
            }
        }
    }
    output
}

fn power_blur(
    pixels: &[Pixel],
    width: usize,
    height: usize,
    radius: f32,
    falloff: f32,
) -> Vec<Pixel> {
    let extent = radius.clamp(1.0, 64.0).ceil() as i32;
    let scale = (radius / 4.0).max(0.25);
    let exponent = falloff.clamp(1.0, 4.0) * 0.5;
    let kernel: Vec<f32> = (-extent..=extent)
        .map(|offset| 1.0 / (1.0 + (offset as f32 / scale).powi(2)).powf(exponent))
        .collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.into_iter().map(|weight| weight / sum).collect();
    let mut horizontal = vec![[0.0; 4]; pixels.len()];
    let mut output = vec![[0.0; 4]; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let px = (x as i32 + index as i32 - extent).clamp(0, width as i32 - 1) as usize;
                for channel in 0..4 {
                    horizontal[y * width + x][channel] += pixels[y * width + px][channel] * weight;
                }
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let py = (y as i32 + index as i32 - extent).clamp(0, height as i32 - 1) as usize;
                for channel in 0..4 {
                    output[y * width + x][channel] += horizontal[py * width + x][channel] * weight;
                }
            }
        }
    }
    output
}

fn blend_pixels(destination: &mut [Pixel], source: &[Pixel], amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    for (destination, source) in destination.iter_mut().zip(source) {
        for channel in 0..3 {
            destination[channel] += (source[channel] - destination[channel]) * amount;
        }
    }
}

fn apply_glow(pixels: &mut [Pixel], width: usize, height: usize, settings: Glow) {
    if !settings.enabled || settings.intensity <= 0.0 || settings.radius <= 0.0 {
        return;
    }
    let mut highlights = pixels.to_vec();
    for pixel in &mut highlights {
        let weight = smoothstep(settings.threshold, 1.0, luminance(*pixel));
        let gray = luminance(*pixel);
        for (channel, value) in pixel[..3].iter_mut().enumerate() {
            let saturated = gray + (*value - gray) * settings.saturation;
            *value =
                saturated.max(0.0).powf(settings.gamma.max(0.01)) * settings.tint[channel] * weight;
        }
    }
    let glow = power_blur(
        &highlights,
        width,
        height,
        settings.radius,
        settings.falloff,
    );
    for (pixel, glow) in pixels.iter_mut().zip(glow) {
        for channel in 0..3 {
            pixel[channel] += glow[channel] * settings.intensity;
        }
    }
}

fn apply_crt_surface(pixels: &mut [Pixel], width: usize, height: usize, settings: CrtEffect) {
    if !settings.enabled {
        return;
    }
    if settings.bloom > 0.0 {
        let bloom = gaussian_blur(pixels, width, height, 3.0);
        for (pixel, bloom) in pixels.iter_mut().zip(bloom) {
            for (channel, value) in pixel[..3].iter_mut().enumerate() {
                *value += bloom[channel] * settings.bloom;
            }
        }
    }
    for y in 0..height {
        let scanline =
            1.0 - settings.scanlines * (0.5 + 0.5 * (y as f32 * std::f32::consts::PI).cos());
        for x in 0..width {
            let mask_channel = x % 3;
            let pixel = &mut pixels[y * width + x];
            for (channel, value) in pixel[..3].iter_mut().enumerate() {
                let phosphor = if channel == mask_channel {
                    1.0
                } else {
                    1.0 - settings.phosphor_mask
                };
                *value *= scanline.max(0.0) * phosphor.max(0.0);
            }
        }
    }
}

fn adjust_coverage(value: f32, threshold: f32, softness: f32) -> f32 {
    let width = softness.clamp(0.01, 1.0);
    smoothstep(threshold - width, threshold + width, value)
}

fn dither_scalar(
    values: &mut [f32],
    width: usize,
    settings: DitherSettings,
    print: PrintSettings,
    angle: f32,
    plate: u64,
    cancel: Option<&AtomicBool>,
) -> bool {
    let height = values.len() / width;
    if let Some(kernel) = diffusion_kernel(settings.algorithm) {
        return diffuse_scalar(values, width, height, kernel, settings.strength, cancel);
    }
    match settings.algorithm {
        DitherAlgorithm::Bayer { matrix_size } => {
            let size = match matrix_size {
                2 | 4 | 8 => matrix_size as usize,
                _ => 8,
            };
            for y in 0..height {
                if render_cancelled(cancel, y) {
                    return false;
                }
                for x in 0..width {
                    let threshold =
                        (bayer_value(x % size, y % size, size) as f32 + 0.5) / (size * size) as f32;
                    values[y * width + x] = f32::from(values[y * width + x] >= threshold);
                }
            }
        }
        DitherAlgorithm::BlueNoise => {
            let map = blue_noise();
            for y in 0..height {
                if render_cancelled(cancel, y) {
                    return false;
                }
                for x in 0..width {
                    let offset = ((settings.seed.wrapping_add(plate) as usize) * 17) & 63;
                    let rank = map[((y + offset) & 63) * 64 + ((x + offset * 3) & 63)];
                    values[y * width + x] =
                        f32::from(values[y * width + x] >= (rank as f32 + 0.5) / 4096.0);
                }
            }
        }
        DitherAlgorithm::Modulation => {
            for y in 0..height {
                if render_cancelled(cancel, y) {
                    return false;
                }
                for x in 0..width {
                    let index = y * width + x;
                    let value = values[index].clamp(0.0, 1.0);
                    let carrier =
                        (x as f32 * 0.55 + y as f32 * 0.17 + value * std::f32::consts::TAU * 1.5)
                            .sin();
                    values[index] = f32::from(value >= 0.5 + carrier * 0.42);
                }
            }
        }
        DitherAlgorithm::Halftone { shape } => {
            let cell = (print.dpi / print.lpi.max(1.0)).max(1.0);
            let radians = angle.to_radians();
            let (sin, cos) = radians.sin_cos();
            for y in 0..height {
                if render_cancelled(cancel, y) {
                    return false;
                }
                for x in 0..width {
                    let rx = x as f32 * cos - y as f32 * sin;
                    let ry = x as f32 * sin + y as f32 * cos;
                    let nx = (rx / cell).rem_euclid(1.0) - 0.5;
                    let ny = (ry / cell).rem_euclid(1.0) - 0.5;
                    let threshold = match shape {
                        HalftoneShape::Dot => (nx * nx + ny * ny) * std::f32::consts::PI,
                        HalftoneShape::Line => ny.abs() * 2.0,
                        HalftoneShape::Cross => nx.abs().min(ny.abs()) * 2.0,
                        HalftoneShape::Diamond => nx.abs() + ny.abs(),
                        HalftoneShape::ClusteredDot => nx.abs().max(ny.abs()) * 2.0,
                    };
                    values[y * width + x] = f32::from(values[y * width + x] >= threshold);
                }
            }
        }
        DitherAlgorithm::FloydSteinberg
        | DitherAlgorithm::Atkinson
        | DitherAlgorithm::SierraLite
        | DitherAlgorithm::SierraTwoRow
        | DitherAlgorithm::Sierra
        | DitherAlgorithm::Stucki
        | DitherAlgorithm::Burkes
        | DitherAlgorithm::JarvisJudiceNinke => unreachable!(),
    }
    true
}

fn diffusion_kernel(algorithm: DitherAlgorithm) -> Option<&'static [(i32, i32, f32)]> {
    match algorithm {
        DitherAlgorithm::FloydSteinberg => Some(&[
            (1, 0, 7.0 / 16.0),
            (-1, 1, 3.0 / 16.0),
            (0, 1, 5.0 / 16.0),
            (1, 1, 1.0 / 16.0),
        ]),
        DitherAlgorithm::Atkinson => Some(&[
            (1, 0, 1.0 / 8.0),
            (2, 0, 1.0 / 8.0),
            (-1, 1, 1.0 / 8.0),
            (0, 1, 1.0 / 8.0),
            (1, 1, 1.0 / 8.0),
            (0, 2, 1.0 / 8.0),
        ]),
        DitherAlgorithm::SierraLite => Some(&[(1, 0, 0.5), (-1, 1, 0.25), (0, 1, 0.25)]),
        DitherAlgorithm::SierraTwoRow => Some(&[
            (1, 0, 4.0 / 16.0),
            (2, 0, 3.0 / 16.0),
            (-2, 1, 1.0 / 16.0),
            (-1, 1, 2.0 / 16.0),
            (0, 1, 3.0 / 16.0),
            (1, 1, 2.0 / 16.0),
            (2, 1, 1.0 / 16.0),
        ]),
        DitherAlgorithm::Sierra => Some(&[
            (1, 0, 5.0 / 32.0),
            (2, 0, 3.0 / 32.0),
            (-2, 1, 2.0 / 32.0),
            (-1, 1, 4.0 / 32.0),
            (0, 1, 5.0 / 32.0),
            (1, 1, 4.0 / 32.0),
            (2, 1, 2.0 / 32.0),
            (-1, 2, 2.0 / 32.0),
            (0, 2, 3.0 / 32.0),
            (1, 2, 2.0 / 32.0),
        ]),
        DitherAlgorithm::Stucki => Some(&[
            (1, 0, 8.0 / 42.0),
            (2, 0, 4.0 / 42.0),
            (-2, 1, 2.0 / 42.0),
            (-1, 1, 4.0 / 42.0),
            (0, 1, 8.0 / 42.0),
            (1, 1, 4.0 / 42.0),
            (2, 1, 2.0 / 42.0),
            (-2, 2, 1.0 / 42.0),
            (-1, 2, 2.0 / 42.0),
            (0, 2, 4.0 / 42.0),
            (1, 2, 2.0 / 42.0),
            (2, 2, 1.0 / 42.0),
        ]),
        DitherAlgorithm::Burkes => Some(&[
            (1, 0, 8.0 / 32.0),
            (2, 0, 4.0 / 32.0),
            (-2, 1, 2.0 / 32.0),
            (-1, 1, 4.0 / 32.0),
            (0, 1, 8.0 / 32.0),
            (1, 1, 4.0 / 32.0),
            (2, 1, 2.0 / 32.0),
        ]),
        DitherAlgorithm::JarvisJudiceNinke => Some(&[
            (1, 0, 7.0 / 48.0),
            (2, 0, 5.0 / 48.0),
            (-2, 1, 3.0 / 48.0),
            (-1, 1, 5.0 / 48.0),
            (0, 1, 7.0 / 48.0),
            (1, 1, 5.0 / 48.0),
            (2, 1, 3.0 / 48.0),
            (-2, 2, 1.0 / 48.0),
            (-1, 2, 3.0 / 48.0),
            (0, 2, 5.0 / 48.0),
            (1, 2, 3.0 / 48.0),
            (2, 2, 1.0 / 48.0),
        ]),
        _ => None,
    }
}

fn diffuse_scalar(
    values: &mut [f32],
    width: usize,
    height: usize,
    kernel: &[(i32, i32, f32)],
    strength: f32,
    cancel: Option<&AtomicBool>,
) -> bool {
    let strength = strength.clamp(0.0, 1.0);
    for y in 0..height {
        if render_cancelled(cancel, y) {
            return false;
        }
        for x in 0..width {
            let index = y * width + x;
            let old = values[index].clamp(0.0, 1.0);
            let new = f32::from(old >= 0.5);
            let error = (old - new) * strength;
            values[index] = new;
            for &(dx, dy, weight) in kernel {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
                    values[ny as usize * width + nx as usize] += error * weight;
                }
            }
        }
    }
    true
}

fn render_cancelled(cancel: Option<&AtomicBool>, row: usize) -> bool {
    row.is_multiple_of(8) && cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed))
}

fn bayer_value(x: usize, y: usize, size: usize) -> usize {
    if size == 1 {
        return 0;
    }
    let half = size / 2;
    let quadrant = [[0, 2], [3, 1]][y / half][x / half];
    4 * bayer_value(x % half, y % half, half) + quadrant
}

fn blue_noise() -> &'static [u16; 4096] {
    static MAP: OnceLock<[u16; 4096]> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut ranks = [0_u16; 4096];
        let mut distance = [f32::MAX; 4096];
        let mut selected = [false; 4096];
        let mut current = 2113;
        for rank in 0..4096_u16 {
            selected[current] = true;
            ranks[current] = rank;
            let cx = current % 64;
            let cy = current / 64;
            let mut next = 0;
            let mut best = -1.0_f32;
            for index in 0..4096 {
                if selected[index] {
                    continue;
                }
                let x = index % 64;
                let y = index / 64;
                let dx = x.abs_diff(cx).min(64 - x.abs_diff(cx)) as f32;
                let dy = y.abs_diff(cy).min(64 - y.abs_diff(cy)) as f32;
                distance[index] = distance[index].min(dx * dx + dy * dy);
                let score = distance[index] + random(x as i32, y as i32, 0xB10E) * 0.01;
                if score > best {
                    best = score;
                    next = index;
                }
            }
            current = next;
        }
        ranks
    })
}

fn dither_palette(
    pixels: &[Pixel],
    colors: &[[f32; 3]],
    width: usize,
    height: usize,
    settings: DitherSettings,
    print: PrintSettings,
) -> Vec<u8> {
    let mut work: Vec<[f32; 3]> = pixels
        .iter()
        .map(|pixel| pixel[..3].try_into().unwrap())
        .collect();
    if let Some(kernel) = diffusion_kernel(settings.algorithm) {
        let mut output = vec![0; work.len()];
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let nearest = nearest_color(work[index], colors);
                output[index] = nearest as u8;
                let error = [
                    work[index][0] - colors[nearest][0],
                    work[index][1] - colors[nearest][1],
                    work[index][2] - colors[nearest][2],
                ];
                for &(dx, dy, weight) in kernel {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
                        let target = &mut work[ny as usize * width + nx as usize];
                        for channel in 0..3 {
                            target[channel] += error[channel] * weight * settings.strength;
                        }
                    }
                }
            }
        }
        return output;
    }

    let mut output = vec![0; work.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let (first, second) = nearest_two(work[index], colors);
            let a = colors[first];
            let b = colors[second];
            let direction = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let denominator = direction
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .max(0.0001);
            let mix = ((work[index][0] - a[0]) * direction[0]
                + (work[index][1] - a[1]) * direction[1]
                + (work[index][2] - a[2]) * direction[2])
                / denominator;
            let threshold = ordered_threshold(x, y, settings, print, 45.0);
            output[index] = if mix.clamp(0.0, 1.0) >= threshold {
                second as u8
            } else {
                first as u8
            };
        }
    }
    output
}

fn ordered_threshold(
    x: usize,
    y: usize,
    settings: DitherSettings,
    print: PrintSettings,
    angle: f32,
) -> f32 {
    match settings.algorithm {
        DitherAlgorithm::Bayer { matrix_size } => {
            let size = match matrix_size {
                2 | 4 | 8 => matrix_size as usize,
                _ => 8,
            };
            (bayer_value(x % size, y % size, size) as f32 + 0.5) / (size * size) as f32
        }
        DitherAlgorithm::BlueNoise => {
            (blue_noise()[(y & 63) * 64 + (x & 63)] as f32 + 0.5) / 4096.0
        }
        DitherAlgorithm::Halftone { shape } => {
            let cell = (print.dpi / print.lpi.max(1.0)).max(1.0);
            let radians = angle.to_radians();
            let (sin, cos) = radians.sin_cos();
            let rx = x as f32 * cos - y as f32 * sin;
            let ry = x as f32 * sin + y as f32 * cos;
            let nx = (rx / cell).rem_euclid(1.0) - 0.5;
            let ny = (ry / cell).rem_euclid(1.0) - 0.5;
            match shape {
                HalftoneShape::Dot => ((nx * nx + ny * ny) * std::f32::consts::PI).clamp(0.0, 1.0),
                HalftoneShape::Line => (ny.abs() * 2.0).clamp(0.0, 1.0),
                HalftoneShape::Cross => (nx.abs().min(ny.abs()) * 2.0).clamp(0.0, 1.0),
                HalftoneShape::Diamond => (nx.abs() + ny.abs()).clamp(0.0, 1.0),
                HalftoneShape::ClusteredDot => (nx.abs().max(ny.abs()) * 2.0).clamp(0.0, 1.0),
            }
        }
        DitherAlgorithm::Modulation => 0.5 + (x as f32 * 0.55 + y as f32 * 0.17).sin() * 0.42,
        _ => 0.5,
    }
}

fn nearest_color(pixel: [f32; 3], colors: &[[f32; 3]]) -> usize {
    colors
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| color_distance(pixel, **a).total_cmp(&color_distance(pixel, **b)))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn nearest_two(pixel: [f32; 3], colors: &[[f32; 3]]) -> (usize, usize) {
    let mut distances: Vec<_> = colors
        .iter()
        .enumerate()
        .map(|(index, color)| (color_distance(pixel, *color), index))
        .collect();
    distances.sort_by(|a, b| a.0.total_cmp(&b.0));
    (distances[0].1, distances.get(1).unwrap_or(&distances[0]).1)
}

fn color_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) * 0.2126 + (a[1] - b[1]).powi(2) * 0.7152 + (a[2] - b[2]).powi(2) * 0.0722
}

fn extract_palette(pixels: &[Pixel], count: usize) -> Vec<[f32; 3]> {
    let samples: Vec<[f32; 3]> = pixels
        .iter()
        .step_by((pixels.len() / 32768).max(1))
        .filter(|pixel| pixel[3] > 0.0)
        .map(|pixel| pixel[..3].try_into().unwrap())
        .collect();
    if samples.is_empty() {
        return PaletteSettings::default().colors;
    }
    let count = count.clamp(2, 64).min(samples.len());
    let mut centers: Vec<[f32; 3]> = (0..count)
        .map(|index| samples[index * samples.len() / count])
        .collect();
    for _ in 0..12 {
        let mut sums = vec![[0.0; 3]; count];
        let mut totals = vec![0_u32; count];
        for sample in &samples {
            let cluster = nearest_color(*sample, &centers);
            for channel in 0..3 {
                sums[cluster][channel] += sample[channel];
            }
            totals[cluster] += 1;
        }
        for index in 0..count {
            if totals[index] > 0 {
                for channel in 0..3 {
                    centers[index][channel] = sums[index][channel] / totals[index] as f32;
                }
            }
        }
    }
    centers.sort_by(|a, b| luminance3(*a).total_cmp(&luminance3(*b)));
    centers
}

fn apply_distress(
    coverage: &mut [f32],
    width: usize,
    height: usize,
    mask: Option<&SourceImage>,
    settings: Displacement,
    render_scale: f32,
    cancel: Option<&AtomicBool>,
) -> bool {
    let amount = settings.distress_amount.clamp(0.0, 1.0);
    if amount == 0.0 || (settings.pattern == MapPattern::Imported && mask.is_none()) {
        return true;
    }
    let scale = (settings.pattern_scale * render_scale).max(0.5);
    for y in 0..height {
        if render_cancelled(cancel, y) {
            return false;
        }
        for x in 0..width {
            let texture = if settings.pattern == MapPattern::Imported {
                luminance(sample_normalized(mask.unwrap(), x, y, width, height))
            } else {
                pattern_value(settings.pattern, x as f32, y as f32, scale, settings.seed)
            };
            coverage[y * width + x] *= 1.0 - amount * (1.0 - texture);
        }
    }
    true
}

fn dilate(mask: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; mask.len()];
    for y in 0..height {
        for x in 0..width {
            let mut maximum = 0.0_f32;
            for dy in -(radius as i32)..=radius as i32 {
                for dx in -(radius as i32)..=radius as i32 {
                    if dx * dx + dy * dy > (radius * radius) as i32 {
                        continue;
                    }
                    let px = x as i32 + dx;
                    let py = y as i32 + dy;
                    if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                        maximum = maximum.max(mask[py as usize * width + px as usize]);
                    }
                }
            }
            output[y * width + x] = maximum;
        }
    }
    output
}

fn shift_mask(mask: &[f32], width: usize, height: usize, dx: i32, dy: i32) -> Vec<f32> {
    let mut output = vec![0.0; mask.len()];
    for y in 0..height {
        for x in 0..width {
            let sx = x as i32 - dx;
            let sy = y as i32 - dy;
            if sx >= 0 && sx < width as i32 && sy >= 0 && sy < height as i32 {
                output[y * width + x] = mask[sy as usize * width + sx as usize];
            }
        }
    }
    output
}

fn compose(
    width: NonZeroU32,
    height: NonZeroU32,
    paper: &[[f32; 3]],
    alpha: &[f32],
    plates: &[RenderedPlate],
) -> RenderedImage {
    let mut pixels = Vec::with_capacity(paper.len());
    for index in 0..paper.len() {
        let mut rgb = paper[index];
        for plate in plates {
            composite_ink(
                &mut rgb,
                paper[index],
                plate.ink.color,
                plate.coverage[index],
            );
        }
        pixels.push([
            rgb[0].max(0.0),
            rgb[1].max(0.0),
            rgb[2].max(0.0),
            alpha[index],
        ]);
    }
    RenderedImage::new(width, height, pixels)
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedPlate {
    pub name: String,
    pub ink: Ink,
    coverage: Vec<f32>,
}

impl RenderedPlate {
    pub fn coverage(&self) -> &[f32] {
        &self.coverage
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedDocument {
    pub composite: RenderedImage,
    pub plates: Vec<RenderedPlate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedImage {
    width: NonZeroU32,
    height: NonZeroU32,
    pixels: Vec<Pixel>,
}

impl RenderedImage {
    fn new(width: NonZeroU32, height: NonZeroU32, pixels: Vec<Pixel>) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn width(&self) -> u32 {
        self.width.get()
    }

    pub fn height(&self) -> u32 {
        self.height.get()
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    PixelCount { expected: usize, actual: usize },
    NonFinitePixel,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PixelCount { expected, actual } => {
                write!(formatter, "expected {expected} pixels, received {actual}")
            }
            Self::NonFinitePixel => formatter.write_str("pixels must contain finite values"),
        }
    }
}

impl std::error::Error for ImageError {}

fn luminance(pixel: Pixel) -> f32 {
    luminance3([pixel[0], pixel[1], pixel[2]])
}

fn luminance3(pixel: [f32; 3]) -> f32 {
    pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722
}

fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let value = ((value - low) / (high - low).max(0.0001)).clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn composite_ink(rgb: &mut [f32; 3], paper: [f32; 3], ink: [f32; 3], coverage: f32) {
    for channel in 0..3 {
        rgb[channel] *= 1.0 - coverage + coverage * ink[channel] / paper[channel].max(0.001);
    }
}

fn noise(x: f32, y: f32, seed: u64) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let a = random(x0, y0, seed);
    let b = random(x0 + 1, y0, seed);
    let c = random(x0, y0 + 1, seed);
    let d = random(x0 + 1, y0 + 1, seed);
    (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy
}

fn random(x: i32, y: i32, seed: u64) -> f32 {
    let mut value = seed
        ^ (x as u32 as u64).wrapping_mul(0x9e37_79b1)
        ^ (y as u32 as u64).wrapping_mul(0x85eb_ca77);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value as u32 as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU8};

    fn source() -> SourceImage {
        SourceImage::new(
            NonZeroU32::new(8).unwrap(),
            NonZeroU32::new(4).unwrap(),
            (0..32)
                .map(|index| {
                    let value = index as f32 / 31.0;
                    [value, value * 0.7, 1.0 - value, 1.0]
                })
                .collect(),
            SourceInfo {
                path: "source.test".into(),
                format: "test".into(),
                bit_depth: 32,
                color_profile: vec![1, 2, 3],
                metadata: Metadata {
                    exif: vec![4, 5, 6],
                    ..Metadata::default()
                },
            },
        )
        .unwrap()
    }

    #[test]
    fn srgb_transfer_functions_round_trip_display_values() {
        for encoded in [0.0, 0.02, 0.18, 0.5, 1.0] {
            let round_trip = linear_to_srgb(srgb_to_linear(encoded));
            assert!((round_trip - encoded).abs() < 1.0e-6);
        }
    }

    #[test]
    fn every_algorithm_is_deterministic_binary_and_non_destructive() {
        let algorithms = [
            DitherAlgorithm::Bayer { matrix_size: 8 },
            DitherAlgorithm::FloydSteinberg,
            DitherAlgorithm::Atkinson,
            DitherAlgorithm::SierraLite,
            DitherAlgorithm::SierraTwoRow,
            DitherAlgorithm::Sierra,
            DitherAlgorithm::Stucki,
            DitherAlgorithm::Burkes,
            DitherAlgorithm::JarvisJudiceNinke,
            DitherAlgorithm::BlueNoise,
            DitherAlgorithm::Modulation,
            DitherAlgorithm::Halftone {
                shape: HalftoneShape::Dot,
            },
            DitherAlgorithm::Halftone {
                shape: HalftoneShape::Line,
            },
            DitherAlgorithm::Halftone {
                shape: HalftoneShape::Cross,
            },
            DitherAlgorithm::Halftone {
                shape: HalftoneShape::Diamond,
            },
            DitherAlgorithm::Halftone {
                shape: HalftoneShape::ClusteredDot,
            },
        ];
        for algorithm in algorithms {
            let mut document = Document::new(source());
            let original = document.source().clone();
            document.recipe.dither.algorithm = algorithm;
            let first = document.render_document();
            let second = document.render_document();
            assert_eq!(first, second);
            assert!(
                first
                    .plates
                    .iter()
                    .flat_map(|plate| plate.coverage())
                    .all(|value| *value == 0.0 || *value == 1.0)
            );
            assert_eq!(document.source(), &original);
        }
    }

    #[test]
    fn all_color_modes_produce_named_plates() {
        let modes = [
            Separation::Monochrome(Monochrome::default()),
            Separation::ThreeColor(ThreeColor::default()),
            Separation::Tonal(PaletteSettings::default()),
            Separation::Indexed(PaletteSettings {
                colors: Vec::new(),
                size: 4,
                ..PaletteSettings::default()
            }),
            Separation::Custom(PaletteSettings::default()),
            Separation::Rgb(ThreeColor::default()),
            Separation::Cmyk(FourColor::default()),
            Separation::TriTone(TriTone::default()),
        ];
        for mode in modes {
            let mut document = Document::new(source());
            document.recipe.separation = mode;
            let rendered = document.render_document();
            assert!(!rendered.plates.is_empty());
            assert!(rendered.plates.iter().all(|plate| !plate.name.is_empty()));
        }
    }

    #[test]
    fn preprocessing_effects_and_assets_change_output() {
        let mut document = Document::new(source());
        let plain = document.render();
        document.recipe.preprocess.brightness = 0.2;
        document.recipe.glow.enabled = true;
        document.recipe.crt.enabled = true;
        document.recipe.crt.scanlines = 0.5;
        document.recipe.displacement.enabled = true;
        document.recipe.displacement.x_strength = 2.0;
        document.set_asset(AssetKind::DisplacementMap, source());
        assert_ne!(document.render(), plain);
    }

    #[test]
    fn new_sampling_glow_maps_and_presets_are_effective_and_deterministic() {
        let image = source();
        assert_ne!(
            sample_source(&image, 1.4, 1.2, 2.0, 2.0, Resampling::Nearest),
            sample_source(&image, 1.4, 1.2, 2.0, 2.0, Resampling::Bilinear)
        );
        for pattern in [
            MapPattern::Grain,
            MapPattern::Halftone,
            MapPattern::Grunge,
            MapPattern::Splatter,
        ] {
            let first = pattern_value(pattern, 13.0, 7.0, 9.0, 5);
            assert_eq!(first, pattern_value(pattern, 13.0, 7.0, 9.0, 5));
            assert!((0.0..=1.0).contains(&first));
        }
        let mut pixels = vec![[0.0; 4]; 25];
        pixels[12] = [1.0; 4];
        let glow = power_blur(&pixels, 5, 5, 3.0, 2.0);
        assert!(glow[11][0] > 0.0);
        assert!(glow[12][0] > glow[0][0]);
        assert!(built_in_presets().len() >= 10);
    }

    #[test]
    fn palette_extraction_is_bounded_and_deterministic() {
        let document = Document::new(source());
        let first = document.extract_palette(6);
        assert_eq!(first, document.extract_palette(6));
        assert!((2..=6).contains(&first.len()));
    }

    #[test]
    fn preview_preserves_dimensions() {
        let document = Document::new(source());
        let preview = document.render_preview(NonZeroU32::new(4).unwrap());
        assert_eq!((preview.width(), preview.height()), (4, 2));
    }

    fn assert_stored_render_matches(document: &Document) {
        let expected = document.render_document();
        let mut plates = Vec::new();
        let stored = document
            .render_stored(
                &AtomicBool::new(false),
                &AtomicU8::new(0),
                |name, ink, width, height, coverage| {
                    plates.push((name.to_owned(), ink, width, height, coverage.to_vec()));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            (stored.width(), stored.height()),
            (expected.composite.width(), expected.composite.height())
        );
        assert_eq!(
            stored.scratch_byte_len(),
            std::mem::size_of_val(stored.pixels())
        );
        for (actual, expected) in stored.pixels().iter().zip(expected.composite.pixels()) {
            for (actual, expected) in actual.iter().zip(expected) {
                assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
            }
        }
        assert_eq!(plates.len(), expected.plates.len());
        for ((name, ink, width, height, coverage), expected) in plates.iter().zip(&expected.plates)
        {
            assert_eq!(name, &expected.name);
            assert_eq!(ink, &expected.ink);
            assert_eq!((*width, *height), (stored.width(), stored.height()));
            for (actual, expected) in coverage.iter().zip(expected.coverage()) {
                assert!((actual - expected).abs() < 1.0e-6);
            }
        }
    }

    #[test]
    fn stored_renderer_matches_all_presets_and_color_modes() {
        for (_, recipe) in built_in_presets() {
            let mut document = Document::new(source());
            document.recipe = recipe.clone();
            assert_stored_render_matches(&document);
        }
        for separation in [
            Separation::Monochrome(Monochrome::default()),
            Separation::ThreeColor(ThreeColor::default()),
            Separation::Tonal(PaletteSettings::default()),
            Separation::Indexed(PaletteSettings::default()),
            Separation::Custom(PaletteSettings::default()),
            Separation::Rgb(ThreeColor::default()),
            Separation::Cmyk(FourColor::default()),
            Separation::TriTone(TriTone::default()),
        ] {
            let mut document = Document::new(source());
            document.recipe.separation = separation;
            document.recipe.preprocess.blur_radius = 1.5;
            document.recipe.preprocess.sharpen = 0.4;
            document.recipe.preprocess.denoise = 0.3;
            document.recipe.glow.enabled = true;
            document.recipe.glow.radius = 2.0;
            document.recipe.crt.enabled = true;
            document.recipe.crt.bloom = 0.2;
            document.recipe.crt.rgb_bleed = 1.0;
            document.recipe.transform = Transform {
                crop: [0.125, 0.0, 0.875, 1.0],
                quarter_turns: 1,
                straighten_degrees: 2.0,
            };
            assert_stored_render_matches(&document);
        }
    }

    #[test]
    fn stored_renderer_stops_before_writing_when_cancelled() {
        let mut wrote_plate = false;
        let result = Document::new(source()).render_stored(
            &AtomicBool::new(true),
            &AtomicU8::new(0),
            |_, _, _, _, _| {
                wrote_plate = true;
                Ok(())
            },
        );
        assert!(matches!(result, Err(RenderError::Cancelled)));
        assert!(!wrote_plate);
    }

    #[test]
    fn geometry_is_non_destructive_and_persists_in_recipe_json() {
        let mut document = Document::new(source());
        let original = document.source().clone();
        document.recipe.transform = Transform {
            crop: [0.25, 0.0, 0.75, 1.0],
            quarter_turns: 1,
            straighten_degrees: 3.5,
        };
        assert_eq!(
            document.output_dimensions(),
            (NonZeroU32::new(4).unwrap(), NonZeroU32::new(4).unwrap())
        );
        assert_eq!(document.source(), &original);
        let bytes = serde_json::to_vec(&document.recipe).unwrap();
        let restored: Recipe = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.transform, document.recipe.transform);
    }

    #[test]
    fn recipe_round_trips_for_persistence() {
        let recipe = Recipe {
            separation: Separation::Cmyk(FourColor::default()),
            assets: AssetPaths {
                paper_texture: Some("paper.tif".into()),
                ..AssetPaths::default()
            },
            ..Recipe::default()
        };
        let bytes = serde_json::to_vec(&recipe).unwrap();
        assert_eq!(serde_json::from_slice::<Recipe>(&bytes).unwrap(), recipe);
    }

    #[test]
    fn rejects_invalid_source_buffers() {
        let error = SourceImage::new(
            NonZeroU32::new(2).unwrap(),
            NonZeroU32::new(2).unwrap(),
            vec![[0.0; 4]],
            SourceInfo {
                path: "source.test".into(),
                format: "test".into(),
                bit_depth: 8,
                color_profile: vec![],
                metadata: Metadata::default(),
            },
        )
        .unwrap_err();
        assert_eq!(
            error,
            ImageError::PixelCount {
                expected: 4,
                actual: 1
            }
        );
    }
}
