use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use super::{
    Document, FourColor, Ink, PaletteSettings, Pixel, Resampling, Separation, StylizeEffect,
    ThreeColor, ToneBand, adjust_coverage, apply_distress, apply_stylize, blend_pixels,
    composite_ink, diffusion_kernel, dither_scalar, extract_palette, luminance, luminance3,
    nearest_color, nearest_two, noise, ordered_threshold, smoothstep,
};
use crate::storage::Scratch;

#[derive(Debug)]
pub enum RenderError {
    Cancelled,
    Storage(String),
    Target(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("render cancelled"),
            Self::Storage(error) => write!(formatter, "render storage failed: {error}"),
            Self::Target(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<std::io::Error> for RenderError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

pub struct StoredImage {
    width: NonZeroU32,
    height: NonZeroU32,
    pixels: Scratch<Pixel>,
}

impl StoredImage {
    pub fn width(&self) -> u32 {
        self.width.get()
    }

    pub fn height(&self) -> u32 {
        self.height.get()
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn rows(&self, start_y: u32, row_count: u32) -> &[Pixel] {
        let start = start_y.min(self.height()) as usize * self.width() as usize;
        let end_y = start_y.saturating_add(row_count).min(self.height());
        let end = end_y as usize * self.width() as usize;
        &self.pixels[start..end]
    }

    #[cfg(test)]
    pub(crate) fn scratch_byte_len(&self) -> usize {
        self.pixels.byte_len()
    }
}

impl Document {
    pub fn render_stored<F>(
        &self,
        cancel: &AtomicBool,
        progress: &AtomicU8,
        mut write_plate: F,
    ) -> Result<StoredImage, RenderError>
    where
        F: FnMut(&str, Ink, u32, u32, &[f32]) -> Result<(), RenderError>,
    {
        let (width, height) = self.output_dimensions();
        if self.recipe.resampling != Resampling::Supersample2x {
            return self.render_stored_at(width, height, cancel, progress, &mut write_plate);
        }

        let high_width = NonZeroU32::new(
            width
                .get()
                .checked_mul(2)
                .ok_or_else(|| RenderError::Storage("render width overflow".into()))?,
        )
        .unwrap();
        let high_height = NonZeroU32::new(
            height
                .get()
                .checked_mul(2)
                .ok_or_else(|| RenderError::Storage("render height overflow".into()))?,
        )
        .unwrap();
        let mut high_document = self.clone();
        high_document.recipe.resampling = Resampling::Bilinear;
        let high = high_document.render_stored_at(
            high_width,
            high_height,
            cancel,
            progress,
            &mut |name, ink, _, _, coverage| {
                let downsampled =
                    downsample_mask(coverage, high_width.get() as usize, width, height, cancel)?;
                write_plate(name, ink, width.get(), height.get(), &downsampled)
            },
        )?;
        check_cancel(cancel)?;
        downsample_image(&high, width, height, cancel)
    }

    fn render_stored_at<F>(
        &self,
        width: NonZeroU32,
        height: NonZeroU32,
        cancel: &AtomicBool,
        progress: &AtomicU8,
        write_plate: &mut F,
    ) -> Result<StoredImage, RenderError>
    where
        F: FnMut(&str, Ink, u32, u32, &[f32]) -> Result<(), RenderError>,
    {
        let width_usize = width.get() as usize;
        let height_usize = height.get() as usize;
        let len = width_usize
            .checked_mul(height_usize)
            .ok_or_else(|| RenderError::Storage("render dimensions overflow".into()))?;
        let (full_width, full_height) = self.output_dimensions();
        let scale = (width.get() as f32 / full_width.get() as f32)
            .min(height.get() as f32 / full_height.get() as f32);

        let mut pixels = Scratch::<Pixel>::new(len)?;
        self.resample_stored(&mut pixels, width_usize, height_usize, scale, cancel)?;
        progress.store(15, Ordering::Relaxed);
        if self.recipe.bypass {
            progress.store(75, Ordering::Relaxed);
            return Ok(StoredImage {
                width,
                height,
                pixels,
            });
        }

        let preprocess_a = self.recipe.preprocess.denoise > 0.0
            || self.recipe.preprocess.blur_radius > 0.0
            || self.recipe.preprocess.sharpen > 0.0;
        let preprocess_b =
            self.recipe.preprocess.blur_radius > 0.0 || self.recipe.preprocess.sharpen > 0.0;
        let glow_work = self.recipe.glow.enabled
            && self.recipe.glow.intensity > 0.0
            && self.recipe.glow.radius > 0.0;
        let crt_work = self.recipe.crt.enabled && self.recipe.crt.bloom > 0.0;
        let stylize_work = self.recipe.stylize.effect != StylizeEffect::None;
        let mut work_a =
            Scratch::<Pixel>::new(if preprocess_a || glow_work || crt_work || stylize_work {
                len
            } else {
                1
            })?;
        let mut work_b = Scratch::<Pixel>::new(if preprocess_b || glow_work || crt_work {
            len
        } else {
            1
        })?;
        let mut work_c = Scratch::<Pixel>::new(if glow_work { len } else { 1 })?;
        preprocess_stored(
            &mut pixels,
            &mut work_a,
            &mut work_b,
            width_usize,
            self.recipe.preprocess,
            scale,
            cancel,
        )?;
        if stylize_work
            && !apply_stylize(
                &mut pixels,
                &mut work_a,
                width_usize,
                height_usize,
                self.recipe.stylize,
                scale,
                Some(cancel),
            )
        {
            return Err(RenderError::Cancelled);
        }
        apply_glow_stored(
            &mut pixels,
            &mut work_a,
            &mut work_b,
            &mut work_c,
            width_usize,
            height_usize,
            self.recipe.glow,
            scale,
            cancel,
        )?;
        apply_crt_stored(
            &mut pixels,
            &mut work_a,
            &mut work_b,
            width_usize,
            height_usize,
            self.recipe.crt,
            cancel,
        )?;
        drop(work_a);
        drop(work_b);
        drop(work_c);
        progress.store(40, Ordering::Relaxed);

        let mut composite = Scratch::<Pixel>::new(len)?;
        for y in 0..height_usize {
            check_row(cancel, y)?;
            for x in 0..width_usize {
                let index = y * width_usize + x;
                let paper = self.paper_at(x, y, width_usize, height_usize);
                composite[index] = [paper[0], paper[1], paper[2], pixels[index][3]];
            }
        }

        let enabled_count = self.plate_names().len();
        let mut mask = Scratch::<f32>::new(len)?;
        let mut mask_work = Scratch::<f32>::new(if masks_need_work(self) { len } else { 1 })?;
        let mut completed = 0_usize;
        {
            let mut emit = |name: &str,
                            ink: Ink,
                            scalar: bool,
                            threshold: f32,
                            softness: f32,
                            plate_index: u64,
                            fill: &mut dyn FnMut(usize, Pixel) -> f32|
             -> Result<(), RenderError> {
                if !ink.enabled {
                    return Ok(());
                }
                for y in 0..height_usize {
                    check_row(cancel, y)?;
                    for x in 0..width_usize {
                        let index = y * width_usize + x;
                        mask[index] = fill(index, pixels[index]);
                    }
                }
                if scalar {
                    prepare_scalar_mask(
                        self,
                        &mut mask,
                        width_usize,
                        threshold,
                        softness,
                        plate_index,
                        cancel,
                    )?;
                    if !dither_scalar(
                        &mut mask,
                        width_usize,
                        self.recipe.dither,
                        self.scaled_print(width_usize, height_usize),
                        ink.angle_degrees,
                        plate_index,
                        Some(cancel),
                    ) {
                        return Err(RenderError::Cancelled);
                    }
                }
                finish_mask(
                    self,
                    &pixels,
                    &mut mask,
                    &mut mask_work,
                    width_usize,
                    height_usize,
                    scale,
                    enabled_count > 1,
                    ink,
                    cancel,
                )?;
                for y in 0..height_usize {
                    check_row(cancel, y)?;
                    for x in 0..width_usize {
                        let index = y * width_usize + x;
                        let paper = self.paper_at(x, y, width_usize, height_usize);
                        let mut rgb = [
                            composite[index][0],
                            composite[index][1],
                            composite[index][2],
                        ];
                        composite_ink(&mut rgb, paper, ink.color, mask[index]);
                        composite[index][..3].copy_from_slice(&rgb);
                    }
                }
                write_plate(name, ink, width.get(), height.get(), &mask)?;
                completed += 1;
                let plate_progress = if enabled_count == 0 {
                    75
                } else {
                    40 + (completed * 35 / enabled_count) as u8
                };
                progress.store(plate_progress, Ordering::Relaxed);
                Ok(())
            };

            match &self.recipe.separation {
                Separation::Monochrome(settings) => emit(
                    "black",
                    settings.ink,
                    true,
                    settings.threshold,
                    settings.softness,
                    0,
                    &mut |_, pixel| 1.0 - luminance(pixel),
                )?,
                Separation::ThreeColor(settings) => emit_cmy(*settings, &mut emit)?,
                Separation::Rgb(settings) => {
                    for (name, ink, channel) in [
                        ("red", settings.cyan, 0),
                        ("green", settings.magenta, 1),
                        ("blue", settings.yellow, 2),
                    ] {
                        emit(
                            name,
                            ink,
                            true,
                            settings.threshold,
                            settings.softness,
                            channel as u64,
                            &mut |_, pixel| pixel[channel],
                        )?;
                    }
                }
                Separation::Cmyk(settings) => emit_cmyk(*settings, &mut emit)?,
                Separation::TriTone(settings) => {
                    for (index, (name, band)) in [
                        ("shadows", settings.shadows),
                        ("midtones", settings.midtones),
                        ("highlights", settings.highlights),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        emit(
                            name,
                            band.ink,
                            true,
                            0.5,
                            0.5,
                            index as u64,
                            &mut |pixel_index, pixel| {
                                tone_value(band, pixel_index, width_usize, pixel)
                            },
                        )?;
                    }
                }
                Separation::Tonal(settings) => render_palette_stored(
                    self,
                    settings,
                    &pixels,
                    width_usize,
                    height_usize,
                    true,
                    "tone",
                    cancel,
                    &mut emit,
                )?,
                Separation::Indexed(settings) => render_palette_stored(
                    self,
                    settings,
                    &pixels,
                    width_usize,
                    height_usize,
                    false,
                    "index",
                    cancel,
                    &mut emit,
                )?,
                Separation::Custom(settings) => render_palette_stored(
                    self,
                    settings,
                    &pixels,
                    width_usize,
                    height_usize,
                    false,
                    "color",
                    cancel,
                    &mut emit,
                )?,
            }
        }
        drop(mask_work);
        drop(mask);
        drop(pixels);

        for y in 0..height_usize {
            check_row(cancel, y)?;
            for pixel in &mut composite[y * width_usize..(y + 1) * width_usize] {
                for channel in &mut pixel[..3] {
                    *channel = channel.max(0.0);
                }
            }
        }
        progress.store(75, Ordering::Relaxed);
        Ok(StoredImage {
            width,
            height,
            pixels: composite,
        })
    }

    fn resample_stored(
        &self,
        output: &mut [Pixel],
        width: usize,
        height: usize,
        scale: f32,
        cancel: &AtomicBool,
    ) -> Result<(), RenderError> {
        for y in 0..height {
            check_row(cancel, y)?;
            for x in 0..width {
                let mut ox = 0.0;
                let mut oy = 0.0;
                if !self.recipe.bypass
                    && self.recipe.displacement.enabled
                    && let Some(sample) = super::displacement_sample(
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
                if !self.recipe.bypass && self.recipe.crt.enabled {
                    let phase = y as f32 / height.max(1) as f32
                        * self.recipe.crt.wave_frequency
                        * std::f32::consts::TAU;
                    match self.recipe.crt.phase {
                        super::CrtPhase::Waveform => {
                            ox += phase.sin() * self.recipe.crt.wave_strength * scale;
                        }
                        super::CrtPhase::Linear => {
                            ox += ((phase / std::f32::consts::TAU).fract() * 2.0 - 1.0)
                                * self.recipe.crt.wave_strength
                                * scale;
                        }
                        super::CrtPhase::Flux => {
                            let turbulence =
                                super::random(x as i32 / 8, y as i32 / 8, self.recipe.crt.seed)
                                    - 0.5;
                            ox += (phase.sin() + (phase * 0.37).sin() + turbulence)
                                * self.recipe.crt.wave_strength
                                * scale
                                * 0.6;
                            oy +=
                                (phase * 0.53).cos() * self.recipe.crt.wave_strength * scale * 0.12;
                        }
                    }
                    let tear = super::random(0, y as i32 / 3, self.recipe.crt.seed);
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
                if !self.recipe.bypass && self.recipe.crt.enabled && self.recipe.crt.rgb_bleed > 0.0
                {
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
                output[y * width + x] = pixel;
            }
        }
        Ok(())
    }

    fn paper_at(&self, x: usize, y: usize, width: usize, height: usize) -> [f32; 3] {
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
                luminance(super::sample_normalized(image, x, y, width, height)) * 2.0 - 1.0
            })
            .unwrap_or(0.0);
        self.recipe.paper_color.map(|channel| {
            (channel * (1.0 + (procedural + imported) * self.recipe.paper.amount)).max(0.0)
        })
    }
}

fn emit_cmy(
    settings: ThreeColor,
    emit: &mut impl FnMut(
        &str,
        Ink,
        bool,
        f32,
        f32,
        u64,
        &mut dyn FnMut(usize, Pixel) -> f32,
    ) -> Result<(), RenderError>,
) -> Result<(), RenderError> {
    for (name, ink, channel) in [
        ("cyan", settings.cyan, 0),
        ("magenta", settings.magenta, 1),
        ("yellow", settings.yellow, 2),
    ] {
        emit(
            name,
            ink,
            true,
            settings.threshold,
            settings.softness,
            channel as u64,
            &mut |_, pixel| 1.0 - pixel[channel],
        )?;
    }
    Ok(())
}

fn emit_cmyk(
    settings: FourColor,
    emit: &mut impl FnMut(
        &str,
        Ink,
        bool,
        f32,
        f32,
        u64,
        &mut dyn FnMut(usize, Pixel) -> f32,
    ) -> Result<(), RenderError>,
) -> Result<(), RenderError> {
    for (name, ink, channel) in [
        ("cyan", settings.cyan, 0),
        ("magenta", settings.magenta, 1),
        ("yellow", settings.yellow, 2),
        ("black", settings.black, 3),
    ] {
        emit(
            name,
            ink,
            true,
            0.5,
            0.5,
            channel as u64,
            &mut |_, pixel| {
                let c = 1.0 - pixel[0].clamp(0.0, 1.0);
                let m = 1.0 - pixel[1].clamp(0.0, 1.0);
                let y = 1.0 - pixel[2].clamp(0.0, 1.0);
                let k = c.min(m).min(y);
                (if channel == 3 {
                    k
                } else {
                    [c, m, y][channel] - k
                }) / if channel == 3 {
                    1.0
                } else {
                    (1.0 - k).max(0.001)
                }
            },
        )?;
    }
    Ok(())
}

fn tone_value(band: ToneBand, index: usize, width: usize, pixel: Pixel) -> f32 {
    let tone = luminance(pixel);
    let feather = 0.08;
    let inside = smoothstep(band.range[0] - feather, band.range[0] + feather, tone)
        * (1.0 - smoothstep(band.range[1] - feather, band.range[1] + feather, tone));
    let x = index % width;
    let y = index / width;
    let grain = noise(
        x as f32 / band.grain.scale.max(0.01),
        y as f32 / band.grain.scale.max(0.01),
        band.grain.seed,
    ) * band.grain.amount;
    (inside * band.intensity + grain).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
fn render_palette_stored(
    document: &Document,
    settings: &PaletteSettings,
    pixels: &[Pixel],
    width: usize,
    height: usize,
    tonal: bool,
    prefix: &str,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(
        &str,
        Ink,
        bool,
        f32,
        f32,
        u64,
        &mut dyn FnMut(usize, Pixel) -> f32,
    ) -> Result<(), RenderError>,
) -> Result<(), RenderError> {
    let mut colors = if settings.colors.len() >= 2 {
        super::valid_palette(&settings.colors)
    } else {
        extract_palette(pixels, settings.size.clamp(2, 64) as usize)
    };
    if tonal {
        colors.sort_by(|a, b| luminance3(*a).total_cmp(&luminance3(*b)));
    }
    let dither_colors = if tonal {
        colors
            .iter()
            .map(|color| [luminance3(*color); 3])
            .collect::<Vec<_>>()
    } else {
        colors.clone()
    };
    let len = width * height;
    let mut work = Scratch::<[f32; 3]>::new(len)?;
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            let index = y * width + x;
            work[index] = if tonal {
                [luminance(pixels[index]); 3]
            } else {
                [pixels[index][0], pixels[index][1], pixels[index][2]]
            };
        }
    }
    let mut assignments = Scratch::<u8>::new(len)?;
    dither_palette_into(
        &mut work,
        &mut assignments,
        &dither_colors,
        width,
        height,
        document.recipe.dither,
        document.scaled_print(width, height),
        cancel,
    )?;
    drop(work);
    for (color_index, color) in colors.iter().enumerate() {
        let mut ink = settings
            .inks
            .get(color_index)
            .copied()
            .unwrap_or_else(|| Ink::new(*color, [0, 0], super::plate_angle(color_index)));
        ink.color = *color;
        let name = format!("{prefix}-{:02}", color_index + 1);
        emit(
            &name,
            ink,
            false,
            0.5,
            0.5,
            color_index as u64,
            &mut |index, _| f32::from(usize::from(assignments[index]) == color_index),
        )?;
    }
    Ok(())
}

fn masks_need_work(document: &Document) -> bool {
    if document.recipe.print.bleed_pixels > 0 || document.recipe.print.trapping_pixels > 0 {
        return true;
    }
    let needs_work = |ink: Ink| {
        ink.bleed_pixels > 0 || ink.trapping_pixels > 0 || ink.offset[0] != 0 || ink.offset[1] != 0
    };
    match &document.recipe.separation {
        Separation::Monochrome(settings) => needs_work(settings.ink),
        Separation::ThreeColor(settings) | Separation::Rgb(settings) => {
            [settings.cyan, settings.magenta, settings.yellow]
                .into_iter()
                .any(needs_work)
        }
        Separation::Cmyk(settings) => [
            settings.cyan,
            settings.magenta,
            settings.yellow,
            settings.black,
        ]
        .into_iter()
        .any(needs_work),
        Separation::TriTone(settings) => [
            settings.shadows.ink,
            settings.midtones.ink,
            settings.highlights.ink,
        ]
        .into_iter()
        .any(needs_work),
        Separation::Tonal(settings)
        | Separation::Indexed(settings)
        | Separation::Custom(settings) => settings.inks.iter().copied().any(needs_work),
    }
}

#[allow(clippy::too_many_arguments)]
fn dither_palette_into(
    work: &mut [[f32; 3]],
    output: &mut [u8],
    colors: &[[f32; 3]],
    width: usize,
    height: usize,
    settings: super::DitherSettings,
    print: super::PrintSettings,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    if let Some(kernel) = diffusion_kernel(settings.algorithm) {
        for y in 0..height {
            check_row(cancel, y)?;
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
        return Ok(());
    }
    for y in 0..height {
        check_row(cancel, y)?;
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
            output[index] = if mix.clamp(0.0, 1.0) >= ordered_threshold(x, y, settings, print, 45.0)
            {
                second as u8
            } else {
                first as u8
            };
        }
    }
    Ok(())
}

fn prepare_scalar_mask(
    document: &Document,
    values: &mut [f32],
    width: usize,
    threshold: f32,
    softness: f32,
    plate: u64,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    let height = values.len() / width;
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            let index = y * width + x;
            let grain = noise(
                x as f32 / document.recipe.grain.scale.max(0.01),
                y as f32 / document.recipe.grain.scale.max(0.01),
                document.recipe.grain.seed.wrapping_add(plate),
            ) * document.recipe.grain.amount;
            values[index] =
                (adjust_coverage(values[index], threshold, softness) + grain).clamp(0.0, 1.0);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_mask(
    document: &Document,
    pixels: &[Pixel],
    mask: &mut [f32],
    work: &mut [f32],
    width: usize,
    height: usize,
    scale: f32,
    multiple_plates: bool,
    ink: Ink,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            let index = y * width + x;
            mask[index] *= pixels[index][3];
        }
    }
    if !apply_distress(
        mask,
        width,
        height,
        document.assets.distress_mask.as_deref(),
        document.recipe.displacement,
        scale,
        Some(cancel),
    ) {
        return Err(RenderError::Cancelled);
    }
    let expansion = ((document.recipe.print.bleed_pixels as f32
        + ink.bleed_pixels as f32
        + if multiple_plates {
            document.recipe.print.trapping_pixels as f32 + ink.trapping_pixels as f32
        } else {
            0.0
        })
        * scale)
        .round() as usize;
    if expansion > 0 {
        dilate_into(mask, work, width, height, expansion, cancel)?;
        copy_rows(mask, work, width, cancel)?;
    }
    let dx = (ink.offset[0] as f32 * scale).round() as i32;
    let dy = (ink.offset[1] as f32 * scale).round() as i32;
    if dx != 0 || dy != 0 {
        shift_into(mask, work, width, height, dx, dy, cancel)?;
        copy_rows(mask, work, width, cancel)?;
    }
    Ok(())
}

fn preprocess_stored(
    pixels: &mut [Pixel],
    work_a: &mut [Pixel],
    work_b: &mut [Pixel],
    width: usize,
    mut settings: super::Preprocess,
    scale: f32,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    let height = pixels.len() / width;
    settings.blur_radius *= scale;
    if settings.denoise > 0.0 {
        median_into(pixels, work_a, width, height, cancel)?;
        for y in 0..height {
            check_row(cancel, y)?;
            let range = y * width..(y + 1) * width;
            blend_pixels(&mut pixels[range.clone()], &work_a[range], settings.denoise);
        }
    }
    if settings.blur_radius > 0.0 {
        blur_into(
            pixels,
            work_a,
            work_b,
            width,
            height,
            &gaussian_kernel(settings.blur_radius),
            cancel,
        )?;
        copy_rows(pixels, work_b, width, cancel)?;
    }
    if settings.sharpen > 0.0 {
        blur_into(
            pixels,
            work_a,
            work_b,
            width,
            height,
            &gaussian_kernel(1.25),
            cancel,
        )?;
        for y in 0..height {
            check_row(cancel, y)?;
            for x in 0..width {
                let index = y * width + x;
                for (channel, value) in pixels[index][..3].iter_mut().enumerate() {
                    *value =
                        (*value + (*value - work_b[index][channel]) * settings.sharpen).max(0.0);
                }
            }
        }
    }
    let black_point = settings.black_point.min(settings.white_point - 0.001);
    let white_point = settings.white_point.max(settings.black_point + 0.001);
    let span = white_point - black_point;
    for y in 0..height {
        check_row(cancel, y)?;
        for pixel in &mut pixels[y * width..(y + 1) * width] {
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
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_glow_stored(
    pixels: &mut [Pixel],
    work_a: &mut [Pixel],
    work_b: &mut [Pixel],
    work_c: &mut [Pixel],
    width: usize,
    height: usize,
    mut settings: super::Glow,
    scale: f32,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    settings.radius *= scale;
    if !settings.enabled || settings.intensity <= 0.0 || settings.radius <= 0.0 {
        return Ok(());
    }
    copy_rows(work_a, pixels, width, cancel)?;
    for y in 0..height {
        check_row(cancel, y)?;
        for pixel in &mut work_a[y * width..(y + 1) * width] {
            let weight = smoothstep(settings.threshold, 1.0, luminance(*pixel));
            let gray = luminance(*pixel);
            for (channel, value) in pixel[..3].iter_mut().enumerate() {
                let saturated = gray + (*value - gray) * settings.saturation;
                *value = saturated.max(0.0).powf(settings.gamma.max(0.01))
                    * settings.tint[channel]
                    * weight;
            }
        }
    }
    let kernel = power_kernel(settings.radius, settings.falloff);
    blur_into(work_a, work_b, work_c, width, height, &kernel, cancel)?;
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            let index = y * width + x;
            for channel in 0..3 {
                pixels[index][channel] += work_c[index][channel] * settings.intensity;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_crt_stored(
    pixels: &mut [Pixel],
    work_a: &mut [Pixel],
    work_b: &mut [Pixel],
    width: usize,
    height: usize,
    settings: super::CrtEffect,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    if !settings.enabled {
        return Ok(());
    }
    if settings.bloom > 0.0 {
        blur_into(
            pixels,
            work_a,
            work_b,
            width,
            height,
            &gaussian_kernel(3.0),
            cancel,
        )?;
        for (pixel, bloom) in pixels.iter_mut().zip(work_b.iter()) {
            for (channel, value) in pixel[..3].iter_mut().enumerate() {
                *value += bloom[channel] * settings.bloom;
            }
        }
    }
    for y in 0..height {
        check_row(cancel, y)?;
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
    Ok(())
}

fn median_into(
    input: &[Pixel],
    output: &mut [Pixel],
    width: usize,
    height: usize,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    output.copy_from_slice(input);
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            for channel in 0..3 {
                let mut values = [0.0; 9];
                let mut next = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let px = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
                        let py = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
                        values[next] = input[py * width + px][channel];
                        next += 1;
                    }
                }
                values.sort_by(f32::total_cmp);
                output[y * width + x][channel] = values[4];
            }
        }
    }
    Ok(())
}

fn gaussian_kernel(radius: f32) -> Vec<f32> {
    let radius = radius.clamp(0.1, 64.0);
    let extent = (radius * 2.5).ceil() as i32;
    let sigma2 = 2.0 * radius * radius;
    normalize_kernel(
        (-extent..=extent)
            .map(|offset| (-(offset * offset) as f32 / sigma2).exp())
            .collect(),
    )
}

fn power_kernel(radius: f32, falloff: f32) -> Vec<f32> {
    let extent = radius.clamp(1.0, 64.0).ceil() as i32;
    let scale = (radius / 4.0).max(0.25);
    let exponent = falloff.clamp(1.0, 4.0) * 0.5;
    normalize_kernel(
        (-extent..=extent)
            .map(|offset| 1.0 / (1.0 + (offset as f32 / scale).powi(2)).powf(exponent))
            .collect(),
    )
}

fn normalize_kernel(mut kernel: Vec<f32>) -> Vec<f32> {
    let sum: f32 = kernel.iter().sum();
    for value in &mut kernel {
        *value /= sum;
    }
    kernel
}

#[allow(clippy::too_many_arguments)]
fn blur_into(
    input: &[Pixel],
    horizontal: &mut [Pixel],
    output: &mut [Pixel],
    width: usize,
    height: usize,
    kernel: &[f32],
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    horizontal.fill([0.0; 4]);
    output.fill([0.0; 4]);
    let extent = (kernel.len() / 2) as i32;
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let px = (x as i32 + index as i32 - extent).clamp(0, width as i32 - 1) as usize;
                for channel in 0..4 {
                    horizontal[y * width + x][channel] += input[y * width + px][channel] * weight;
                }
            }
        }
    }
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            for (index, weight) in kernel.iter().enumerate() {
                let py = (y as i32 + index as i32 - extent).clamp(0, height as i32 - 1) as usize;
                for channel in 0..4 {
                    output[y * width + x][channel] += horizontal[py * width + x][channel] * weight;
                }
            }
        }
    }
    Ok(())
}

fn dilate_into(
    input: &[f32],
    output: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    output.fill(0.0);
    for y in 0..height {
        check_row(cancel, y)?;
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
                        maximum = maximum.max(input[py as usize * width + px as usize]);
                    }
                }
            }
            output[y * width + x] = maximum;
        }
    }
    Ok(())
}

fn shift_into(
    input: &[f32],
    output: &mut [f32],
    width: usize,
    height: usize,
    dx: i32,
    dy: i32,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    output.fill(0.0);
    for y in 0..height {
        check_row(cancel, y)?;
        for x in 0..width {
            let source_x = x as i32 - dx;
            let source_y = y as i32 - dy;
            if source_x >= 0 && source_x < width as i32 && source_y >= 0 && source_y < height as i32
            {
                output[y * width + x] = input[source_y as usize * width + source_x as usize];
            }
        }
    }
    Ok(())
}

fn copy_rows<T: Copy>(
    destination: &mut [T],
    source: &[T],
    width: usize,
    cancel: &AtomicBool,
) -> Result<(), RenderError> {
    for (y, (destination, source)) in destination
        .chunks_mut(width)
        .zip(source.chunks(width))
        .enumerate()
    {
        check_row(cancel, y)?;
        destination.copy_from_slice(source);
    }
    Ok(())
}

fn downsample_mask(
    input: &[f32],
    high_width: usize,
    width: NonZeroU32,
    height: NonZeroU32,
    cancel: &AtomicBool,
) -> Result<Scratch<f32>, RenderError> {
    let mut output = Scratch::<f32>::new(width.get() as usize * height.get() as usize)?;
    for y in 0..height.get() as usize {
        check_row(cancel, y)?;
        for x in 0..width.get() as usize {
            output[y * width.get() as usize + x] = [
                input[(y * 2) * high_width + x * 2],
                input[(y * 2) * high_width + x * 2 + 1],
                input[(y * 2 + 1) * high_width + x * 2],
                input[(y * 2 + 1) * high_width + x * 2 + 1],
            ]
            .into_iter()
            .sum::<f32>()
                * 0.25;
        }
    }
    Ok(output)
}

fn downsample_image(
    input: &StoredImage,
    width: NonZeroU32,
    height: NonZeroU32,
    cancel: &AtomicBool,
) -> Result<StoredImage, RenderError> {
    let high_width = input.width() as usize;
    let mut output = Scratch::<Pixel>::new(width.get() as usize * height.get() as usize)?;
    for y in 0..height.get() as usize {
        check_row(cancel, y)?;
        for x in 0..width.get() as usize {
            let mut pixel = [0.0; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sample = input.pixels()[(y * 2 + dy) * high_width + x * 2 + dx];
                for channel in 0..4 {
                    pixel[channel] += sample[channel] * 0.25;
                }
            }
            output[y * width.get() as usize + x] = pixel;
        }
    }
    Ok(StoredImage {
        width,
        height,
        pixels: output,
    })
}

fn check_row(cancel: &AtomicBool, row: usize) -> Result<(), RenderError> {
    if row.is_multiple_of(8) {
        check_cancel(cancel)?;
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), RenderError> {
    if cancel.load(Ordering::Relaxed) {
        Err(RenderError::Cancelled)
    } else {
        Ok(())
    }
}
