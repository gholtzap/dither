use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Cursor, Seek, SeekFrom, Write},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use dither_core::{
    Document, Metadata, Pixel, RenderError, SourceImage, SourceInfo, StoredImage, linear_to_srgb,
    srgb_to_linear,
};
use exr::{
    meta::attribute::AttributeValue,
    prelude::{
        Encoding, Image, Layer, LayerAttributes, MetaData, SmallVec, SpecificChannels, Text, Vec2,
        WritableImage, attribute::Chromaticities,
    },
};
use image::{
    DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, ImageReader, Rgba,
    codecs::tiff::TiffDecoder, metadata::Orientation,
};
use lcms2::{ColorSpaceSignature, Intent, PixelFormat, Profile, Transform};
use rawler::{
    decoders::{Orientation as RawOrientation, RawDecodeParams, RawMetadata},
    formats::tiff::{
        DirectoryWriter, GenericTiffReader, Rational, TiffWriter, Value, reader::TiffReader,
    },
    imgop::{
        Dim2,
        develop::{Intermediate, ProcessingStep, RawDevelop},
    },
    rawsource::RawSource,
    tags::{ExifTag, TiffCommonTag},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Png16,
    #[default]
    Tiff16,
    OpenExr32,
}

impl ExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png16 => "png",
            Self::Tiff16 => "tif",
            Self::OpenExr32 => "exr",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Png16 => "PNG 16-bit",
            Self::Tiff16 => "TIFF 16-bit",
            Self::OpenExr32 => "OpenEXR 32-bit",
        }
    }

    pub const fn bit_depth(self) -> u8 {
        match self {
            Self::Png16 | Self::Tiff16 => 16,
            Self::OpenExr32 => 32,
        }
    }

    pub const fn embeds_icc_profile(self) -> bool {
        matches!(self, Self::Png16 | Self::Tiff16)
    }

    pub fn preserves_all_metadata(self, metadata: &Metadata) -> bool {
        !matches!(self, Self::Png16) || (metadata.iptc.is_empty() && metadata.camera.is_empty())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    pub overwrite: bool,
    pub allow_metadata_loss: bool,
    pub allow_bit_depth_reduction: bool,
}

pub const RASTER_EXTENSIONS: &[&str] = &["png", "tif", "tiff", "exr", "jpg", "jpeg", "webp", "bmp"];
pub const RAW_EXTENSIONS: &[&str] = &[
    "ari", "arw", "cr2", "cr3", "crw", "dcr", "dcs", "dng", "erf", "iiq", "kdc", "mef", "mos",
    "mrw", "nef", "nrw", "orf", "pef", "qtk", "raf", "raw", "rw2", "srw", "tfr", "x3f",
];

pub fn supported_extensions() -> impl Iterator<Item = &'static str> {
    RASTER_EXTENSIONS.iter().chain(RAW_EXTENSIONS).copied()
}

pub fn open(path: impl AsRef<Path>) -> Result<SourceImage, IoError> {
    let path = path.as_ref();
    if is_raw(path) {
        return open_raw(path);
    }
    open_raster(path)
}

fn open_raster(path: &Path) -> Result<SourceImage, IoError> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader.format().ok_or(IoError::UnsupportedFormat)?;
    if format == ImageFormat::Tiff {
        return decode_raster(
            TiffDecoder::new(BufReader::new(File::open(path)?))?,
            path,
            format,
        );
    }
    decode_raster(reader.into_decoder()?, path, format)
}

fn decode_raster(
    mut decoder: impl ImageDecoder,
    path: &Path,
    format: ImageFormat,
) -> Result<SourceImage, IoError> {
    let color_type = decoder.original_color_type();
    if matches!(
        color_type,
        image::ExtendedColorType::Cmyk8 | image::ExtendedColorType::Cmyk16
    ) {
        return Err(IoError::UnsupportedColorSpace(
            "CMYK raster input requires a decoder that exposes the original CMYK samples".into(),
        ));
    }
    let bit_depth = color_type.bits_per_pixel() / u16::from(color_type.channel_count());
    if bit_depth > 32 {
        return Err(IoError::UnsupportedBitDepth(bit_depth));
    }

    let mut color_profile = decoder.icc_profile()?.unwrap_or_default();
    let raw_exif = decoder.exif_metadata()?.unwrap_or_default();
    let orientation = decoder.orientation()?;
    if orientation != Orientation::NoTransforms {
        // Orientation is applied to the pixels below and intentionally omitted
        // from the normalized metadata archive.
    }
    let (exif, iptc) = if format == ImageFormat::Tiff {
        read_tiff_metadata(path)?
    } else {
        (
            normalize_exif(&raw_exif)?,
            normalize_iptc(decoder.iptc_metadata()?.unwrap_or_default()),
        )
    };
    let mut metadata = Metadata {
        xmp: decoder.xmp_metadata()?.unwrap_or_default(),
        iptc,
        exif,
        camera: Vec::new(),
    };

    let mut decoded = DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba32f();
    let (width, height) = rgba.dimensions();
    let mut pixels: Vec<[f32; 4]> = rgba.pixels().map(|pixel| pixel.0).collect();
    if format == ImageFormat::OpenExr {
        read_exr_metadata(path, &mut metadata)?;
    } else if to_linear_srgb(&mut pixels, &color_profile)? {
        color_profile = Profile::new_srgb().icc()?;
    }

    SourceImage::new(
        NonZeroU32::new(width).ok_or(IoError::EmptyImage)?,
        NonZeroU32::new(height).ok_or(IoError::EmptyImage)?,
        pixels,
        SourceInfo {
            path: path.to_path_buf(),
            format: format_name(format).into(),
            bit_depth: bit_depth as u8,
            color_profile,
            metadata,
        },
    )
    .map_err(|error| IoError::InvalidImage(error.to_string()))
}

fn open_raw(path: &Path) -> Result<SourceImage, IoError> {
    let raw_source = RawSource::new(path)?;
    let decoder = rawler::get_decoder(&raw_source)?;
    let params = RawDecodeParams::default();
    let raw = decoder.raw_image(&raw_source, &params, false)?;
    let metadata = decoder.raw_metadata(&raw_source, &params)?;
    let bit_depth =
        u8::try_from(raw.bps).map_err(|_| IoError::UnsupportedBitDepth(raw.bps as u16))?;
    let orientation = raw_orientation(raw.orientation);
    let mut developer = RawDevelop::default();
    developer.steps.retain(|step| *step != ProcessingStep::SRgb);
    let intermediate = developer.develop_intermediate(&raw)?;
    let dimensions = intermediate.dim();
    let pixels: Vec<[f32; 4]> = match intermediate {
        Intermediate::Monochrome(image) => image
            .data
            .into_iter()
            .map(|value| [value, value, value, 1.0])
            .collect(),
        Intermediate::ThreeColor(image) => image
            .data
            .into_iter()
            .map(|pixel| [pixel[0], pixel[1], pixel[2], 1.0])
            .collect(),
        Intermediate::FourColor(_) => {
            return Err(IoError::UnsupportedColorSpace(
                "four-channel camera space without a calibration matrix".into(),
            ));
        }
    };
    let data: Vec<f32> = pixels.into_iter().flatten().collect();
    let buffer = ImageBuffer::<Rgba<f32>, Vec<f32>>::from_vec(
        dimensions.w as u32,
        dimensions.h as u32,
        data,
    )
    .ok_or_else(|| IoError::InvalidImage("RAW developer returned an invalid buffer".into()))?;
    let mut image = DynamicImage::ImageRgba32F(buffer);
    image.apply_orientation(orientation);
    let image = image.into_rgba32f();
    let (width, height) = image.dimensions();

    SourceImage::new(
        NonZeroU32::new(width).ok_or(IoError::EmptyImage)?,
        NonZeroU32::new(height).ok_or(IoError::EmptyImage)?,
        image.pixels().map(|pixel| pixel.0).collect(),
        SourceInfo {
            path: path.to_path_buf(),
            format: format!("{} {} RAW", raw.clean_make, raw.clean_model),
            bit_depth,
            color_profile: Profile::new_srgb().icc()?,
            metadata: Metadata {
                camera: serde_json::to_vec(&metadata)
                    .map_err(|error| IoError::InvalidImage(error.to_string()))?,
                ..Metadata::default()
            },
        },
    )
    .map_err(|error| IoError::InvalidImage(error.to_string()))
}

pub fn export(
    document: &Document,
    destination: impl AsRef<Path>,
    format: ExportFormat,
    options: ExportOptions,
) -> Result<(), IoError> {
    export_cancellable(
        document,
        destination,
        format,
        options,
        &AtomicBool::new(false),
        &AtomicU8::new(0),
    )
}

pub fn export_cancellable(
    document: &Document,
    destination: impl AsRef<Path>,
    format: ExportFormat,
    options: ExportOptions,
    cancel: &AtomicBool,
    progress: &AtomicU8,
) -> Result<(), IoError> {
    let source = document.source();
    let destination = destination.as_ref();
    preflight(source, destination, format, options)?;
    progress.store(5, Ordering::Relaxed);
    if cancel.load(Ordering::Relaxed) {
        return Err(IoError::Cancelled);
    }

    let temporary = temporary_path(destination);
    let result = (|| {
        let rendered = document
            .render_stored(cancel, progress, |_, _, _, _, _| Ok(()))
            .map_err(render_error)?;
        write_stored_export(
            source,
            &rendered,
            &temporary,
            format,
            document.recipe.print.dpi,
            cancel,
            progress,
        )?;
        check_cancel(cancel)?;
        commit_export_set(
            &[(temporary.clone(), destination.to_path_buf())],
            options.overwrite,
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return result;
    }
    progress.store(100, Ordering::Relaxed);
    Ok(())
}

/// Exports the full-resolution composite and one true 16-bit grayscale PNG per ink.
pub fn export_with_plates(
    document: &Document,
    destination: impl AsRef<Path>,
    format: ExportFormat,
    options: ExportOptions,
) -> Result<Vec<PathBuf>, IoError> {
    export_with_plates_cancellable(
        document,
        destination,
        format,
        options,
        &AtomicBool::new(false),
        &AtomicU8::new(0),
    )
}

pub fn export_with_plates_cancellable(
    document: &Document,
    destination: impl AsRef<Path>,
    format: ExportFormat,
    options: ExportOptions,
    cancel: &AtomicBool,
    progress: &AtomicU8,
) -> Result<Vec<PathBuf>, IoError> {
    let source = document.source();
    let destination = destination.as_ref();
    preflight(source, destination, format, options)?;
    progress.store(5, Ordering::Relaxed);
    let composite_temporary = temporary_path(destination);
    let mut plate_outputs = Vec::new();
    let mut target_error = None;
    let result = (|| {
        let rendered =
            document.render_stored(cancel, progress, |name, _, width, height, coverage| {
                let destination_path = plate_path(destination, name);
                let duplicate = destination_path == destination
                    || plate_outputs
                        .iter()
                        .any(|(_, path)| path == &destination_path);
                if duplicate {
                    let error = IoError::InvalidImage(format!(
                        "more than one export output uses {}",
                        destination_path.display()
                    ));
                    let message = error.to_string();
                    target_error = Some(error);
                    return Err(RenderError::Target(message));
                }
                if let Err(error) =
                    preflight(source, &destination_path, ExportFormat::Png16, options)
                {
                    let message = error.to_string();
                    target_error = Some(error);
                    return Err(RenderError::Target(message));
                }
                let temporary = temporary_path(&destination_path);
                plate_outputs.push((temporary.clone(), destination_path));
                let result = File::create(&temporary)
                    .map_err(IoError::from)
                    .and_then(|file| {
                        write_plate_png(
                            source,
                            coverage,
                            width,
                            height,
                            document.recipe.print.dpi,
                            BufWriter::new(CancellableWriter::new(file, cancel)),
                            cancel,
                        )
                    });
                if let Err(error) = result {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(RenderError::Cancelled);
                    }
                    let message = error.to_string();
                    target_error = Some(error);
                    return Err(RenderError::Target(message));
                }
                Ok(())
            });
        let rendered = match rendered {
            Ok(rendered) => rendered,
            Err(_) if target_error.is_some() => return Err(target_error.take().unwrap()),
            Err(error) => return Err(render_error(error)),
        };
        write_stored_export(
            source,
            &rendered,
            &composite_temporary,
            format,
            document.recipe.print.dpi,
            cancel,
            progress,
        )?;
        check_cancel(cancel)?;
        let outputs: Vec<_> =
            std::iter::once((composite_temporary.clone(), destination.to_path_buf()))
                .chain(plate_outputs.iter().cloned())
                .collect();
        commit_export_set(&outputs, options.overwrite)
    })();
    if let Err(error) = result {
        cleanup_temporaries(
            std::iter::once(&composite_temporary)
                .chain(plate_outputs.iter().map(|(temporary, _)| temporary)),
        );
        return Err(error);
    }
    progress.store(100, Ordering::Relaxed);
    Ok(std::iter::once(destination.to_path_buf())
        .chain(plate_outputs.into_iter().map(|(_, path)| path))
        .collect())
}

pub fn preflight(
    source: &SourceImage,
    destination: impl AsRef<Path>,
    format: ExportFormat,
    options: ExportOptions,
) -> Result<(), IoError> {
    protect_destination(source, destination.as_ref(), options)?;
    validate_export(source, format, options)
}

fn write_stored_export(
    source: &SourceImage,
    rendered: &StoredImage,
    path: &Path,
    format: ExportFormat,
    dpi: f32,
    cancel: &AtomicBool,
    progress: &AtomicU8,
) -> Result<(), IoError> {
    let file = File::create(path)?;
    let writer = BufWriter::new(CancellableWriter::new(file, cancel));
    let result = match format {
        ExportFormat::Png16 => write_png(source, rendered, dpi, writer, cancel, progress),
        ExportFormat::Tiff16 => write_tiff(source, rendered, dpi, writer, cancel, progress),
        ExportFormat::OpenExr32 => write_exr(source, rendered, writer, progress),
    };
    if cancel.load(Ordering::Relaxed) && result.is_err() {
        return Err(IoError::Cancelled);
    }
    result
}

const EXPORT_ROWS: u32 = 256;

fn write_png<W: Write>(
    source: &SourceImage,
    rendered: &StoredImage,
    dpi: f32,
    writer: W,
    cancel: &AtomicBool,
    progress: &AtomicU8,
) -> Result<(), IoError> {
    let pixel_encoder = PixelEncoder::new(&source.info.color_profile)?;
    let mut info = png::Info::with_size(rendered.width(), rendered.height());
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Sixteen;
    info.pixel_dims = Some(pixel_dimensions(dpi));
    info.icc_profile = Some(Cow::Owned(pixel_encoder.profile.clone()));
    if !source.info.metadata.exif.is_empty() {
        info.exif_metadata = Some(Cow::Owned(exif_blob(
            &source.info.metadata.exif,
            rendered.width(),
            rendered.height(),
        )?));
    }
    if !source.info.metadata.xmp.is_empty() {
        info.utf8_text.push(png::text_metadata::ITXtChunk::new(
            "XML:com.adobe.xmp",
            String::from_utf8(source.info.metadata.xmp.clone())
                .map_err(|error| IoError::InvalidImage(error.to_string()))?,
        ));
    }
    let mut encoder = png::Encoder::with_info(writer, info)?;
    encoder.set_compression(png::Compression::High);
    let mut png = encoder.write_header()?;
    let mut writer = png.stream_writer()?;
    let chunks = rendered
        .pixels()
        .chunks(EXPORT_ROWS as usize * rendered.width() as usize);
    let chunk_count = chunks.len().max(1);
    for (index, rows) in chunks.enumerate() {
        check_cancel(cancel)?;
        writer.write_all(&u16_bytes(&pixel_encoder.encode(rows), u16::to_be_bytes))?;
        progress.store(
            75 + ((index + 1) * 20 / chunk_count) as u8,
            Ordering::Relaxed,
        );
    }
    writer.finish()?;
    Ok(())
}

fn write_plate_png<W: Write>(
    source: &SourceImage,
    coverage: &[f32],
    width: u32,
    height: u32,
    dpi: f32,
    writer: W,
    cancel: &AtomicBool,
) -> Result<(), IoError> {
    let mut info = png::Info::with_size(width, height);
    info.color_type = png::ColorType::Grayscale;
    info.bit_depth = png::BitDepth::Sixteen;
    info.pixel_dims = Some(pixel_dimensions(dpi));
    if !source.info.metadata.exif.is_empty() {
        info.exif_metadata = Some(Cow::Owned(exif_blob(
            &source.info.metadata.exif,
            width,
            height,
        )?));
    }
    if !source.info.metadata.xmp.is_empty() {
        info.utf8_text.push(png::text_metadata::ITXtChunk::new(
            "XML:com.adobe.xmp",
            String::from_utf8(source.info.metadata.xmp.clone())
                .map_err(|error| IoError::InvalidImage(error.to_string()))?,
        ));
    }
    let mut encoder = png::Encoder::with_info(writer, info)?;
    encoder.set_compression(png::Compression::High);
    let mut png = encoder.write_header()?;
    let mut writer = png.stream_writer()?;
    for rows in coverage.chunks(EXPORT_ROWS as usize * width as usize) {
        check_cancel(cancel)?;
        let bytes: Vec<u8> = rows
            .iter()
            .flat_map(|value| quantize(*value).to_be_bytes())
            .collect();
        writer.write_all(&bytes)?;
    }
    writer.finish()?;
    Ok(())
}

fn pixel_dimensions(dpi: f32) -> png::PixelDimensions {
    let pixels_per_meter = (dpi.max(1.0) / 0.0254).round() as u32;
    png::PixelDimensions {
        xppu: pixels_per_meter,
        yppu: pixels_per_meter,
        unit: png::Unit::Meter,
    }
}

fn plate_path(composite: &Path, name: &str) -> PathBuf {
    let stem = composite
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("dithered");
    composite.with_file_name(format!("{stem}-plate-{name}.png"))
}

fn protect_destination(
    source: &SourceImage,
    destination: &Path,
    options: ExportOptions,
) -> Result<(), IoError> {
    if same_path(&source.info.path, destination)? {
        return Err(IoError::SourceOverwrite);
    }
    if destination.exists() && !options.overwrite {
        return Err(IoError::DestinationExists);
    }
    if destination.exists() && !destination.is_file() {
        return Err(IoError::DestinationNotFile);
    }
    if destination.parent().is_some_and(|parent| !parent.exists()) {
        return Err(IoError::MissingDestinationDirectory);
    }
    Ok(())
}

fn validate_export(
    source: &SourceImage,
    format: ExportFormat,
    options: ExportOptions,
) -> Result<(), IoError> {
    if source.info.bit_depth > format.bit_depth() && !options.allow_bit_depth_reduction {
        return Err(IoError::BitDepthReduction {
            source: source.info.bit_depth,
            output: format.bit_depth(),
        });
    }

    if !format.preserves_all_metadata(&source.info.metadata) && !options.allow_metadata_loss {
        return Err(IoError::MetadataLossRequiresConfirmation);
    }
    Ok(())
}

fn write_tiff<W: Write + Seek>(
    source: &SourceImage,
    rendered: &StoredImage,
    dpi: f32,
    writer: W,
    cancel: &AtomicBool,
    progress: &AtomicU8,
) -> Result<(), IoError> {
    let pixel_encoder = PixelEncoder::new(&source.info.color_profile)?;
    let mut tiff = TiffWriter::new(writer).map_err(tiff_error)?;
    let mut root = DirectoryWriter::new();

    if !source.info.metadata.exif.is_empty() {
        add_exif_directories(
            &mut tiff,
            &mut root,
            &source.info.metadata.exif,
            rendered.width(),
            rendered.height(),
        )?;
    }
    if !source.info.metadata.iptc.is_empty() {
        match decode_iptc(&source.info.metadata.iptc)? {
            IptcArchive::Bytes(bytes) => root.add_tag_undefined(ExifTag::IptcNaa, bytes),
            IptcArchive::Tiff(value) => root.add_value(ExifTag::IptcNaa, value),
        }
    }

    if !source.info.metadata.camera.is_empty() {
        let metadata: RawMetadata = serde_json::from_slice(&source.info.metadata.camera)
            .map_err(|error| IoError::InvalidImage(error.to_string()))?;
        let mut exif = DirectoryWriter::new();
        exif.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);
        metadata
            .write_exif_tags(&mut tiff, &mut root, &mut exif)
            .map_err(tiff_error)?;
        root.add_tag(TiffCommonTag::Make, metadata.make.as_str());
        root.add_tag(TiffCommonTag::Model, metadata.model.as_str());
        let offset = exif.build(&mut tiff).map_err(tiff_error)?;
        root.add_tag(TiffCommonTag::ExifIFDPointer, offset);
    }

    let mut strips = Vec::new();
    let chunks = rendered
        .pixels()
        .chunks(EXPORT_ROWS as usize * rendered.width() as usize);
    let chunk_count = chunks.len().max(1);
    for (index, rendered_rows) in chunks.enumerate() {
        check_cancel(cancel)?;
        let rows = rendered_rows.len() / rendered.width() as usize;
        let data: Vec<u16> = pixel_encoder
            .encode(rendered_rows)
            .into_iter()
            .flatten()
            .collect();
        let (_, mut strip) = tiff
            .write_strips_lzw(&data, 4, Dim2::new(rendered.width() as usize, rows), rows)
            .map_err(tiff_error)?;
        strips.append(&mut strip);
        progress.store(
            75 + ((index + 1) * 20 / chunk_count) as u8,
            Ordering::Relaxed,
        );
    }
    let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
    let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
    root.add_tag(TiffCommonTag::Compression, 5_u16);
    root.add_tag(TiffCommonTag::Predictor, 1_u16);
    root.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
    root.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
    root.add_tag(TiffCommonTag::BitsPerSample, [16_u16; 4]);
    root.add_tag(TiffCommonTag::SamplesPerPixel, 4_u16);
    root.add_tag(TiffCommonTag::PhotometricInt, 2_u16);
    root.add_tag(TiffCommonTag::RowsPerStrip, EXPORT_ROWS);
    root.add_tag(TiffCommonTag::ImageWidth, rendered.width());
    root.add_tag(TiffCommonTag::ImageLength, rendered.height());
    let resolution = Rational {
        n: (dpi.max(1.0) * 1000.0).round() as u32,
        d: 1000,
    };
    root.add_value(
        TiffCommonTag::XResolution,
        Value::Rational(vec![resolution]),
    );
    root.add_value(
        TiffCommonTag::YResolution,
        Value::Rational(vec![resolution]),
    );
    root.add_tag(TiffCommonTag::ResolutionUnit, 2_u16);
    root.add_tag(TiffCommonTag::Orientation, 1_u16);
    root.add_tag(ExifTag::PlanarConfiguration, 1_u16);
    root.add_tag(ExifTag::ExtraSamples, 2_u16);
    if !root.contains(TiffCommonTag::Software) {
        root.add_tag(TiffCommonTag::Software, "Dither");
    }
    root.add_tag_undefined(ExifTag::IccProfile, pixel_encoder.profile);
    if !source.info.metadata.xmp.is_empty() {
        root.add_tag_undefined(TiffCommonTag::Xmp, source.info.metadata.xmp.clone());
    }
    tiff.build(root).map_err(tiff_error)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExifArchive {
    root: BTreeMap<u16, Value>,
    exif: BTreeMap<u16, Value>,
    gps: BTreeMap<u16, Value>,
    interop: BTreeMap<u16, Value>,
}

const EXIF_SUB_IFDS: &[u16] = &[ExifTag::ExifOffset as u16, ExifTag::GPSInfo as u16, 0xA005];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum IptcArchive {
    Bytes(Vec<u8>),
    Tiff(Value),
}

fn read_tiff_metadata(path: &Path) -> Result<(Vec<u8>, Vec<u8>), IoError> {
    let mut file = File::open(path)?;
    let reader =
        GenericTiffReader::new(&mut file, 0, 0, Some(1), EXIF_SUB_IFDS).map_err(tiff_error)?;
    let root = reader.root_ifd();
    let exif = encode_exif_archive(root)?;
    let iptc = root
        .get_entry(ExifTag::IptcNaa)
        .map(|entry| serde_json::to_vec(&IptcArchive::Tiff(entry.value.clone())))
        .transpose()
        .map_err(|error| IoError::InvalidImage(error.to_string()))?
        .unwrap_or_default();
    Ok((exif, iptc))
}

fn normalize_exif(raw: &[u8]) -> Result<Vec<u8>, IoError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let raw = raw.strip_prefix(b"Exif\0\0").unwrap_or(raw);
    let mut cursor = Cursor::new(raw);
    let reader =
        GenericTiffReader::new(&mut cursor, 0, 0, Some(1), EXIF_SUB_IFDS).map_err(tiff_error)?;
    encode_exif_archive(reader.root_ifd())
}

fn normalize_iptc(raw: Vec<u8>) -> Vec<u8> {
    if raw.is_empty() {
        Vec::new()
    } else {
        serde_json::to_vec(&IptcArchive::Bytes(raw)).expect("IPTC bytes are serializable")
    }
}

fn encode_exif_archive(root: &rawler::formats::tiff::IFD) -> Result<Vec<u8>, IoError> {
    let exif = root.get_sub_ifd(ExifTag::ExifOffset);
    let archive = ExifArchive {
        root: root
            .entries()
            .iter()
            .filter(|(tag, _)| is_root_metadata_tag(**tag))
            .map(|(tag, entry)| (*tag, entry.value.clone()))
            .collect(),
        exif: exif
            .map(|ifd| {
                ifd.entries()
                    .iter()
                    .filter(|(tag, _)| **tag != 0xA005)
                    .map(|(tag, entry)| (*tag, entry.value.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        gps: root
            .get_sub_ifd(ExifTag::GPSInfo)
            .map(|ifd| {
                ifd.entries()
                    .iter()
                    .map(|(tag, entry)| (*tag, entry.value.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        interop: exif
            .and_then(|ifd| ifd.get_sub_ifd(0xA005_u16))
            .map(|ifd| {
                ifd.entries()
                    .iter()
                    .map(|(tag, entry)| (*tag, entry.value.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    };
    if archive.root.is_empty()
        && archive.exif.is_empty()
        && archive.gps.is_empty()
        && archive.interop.is_empty()
    {
        return Ok(Vec::new());
    }
    serde_json::to_vec(&archive).map_err(|error| IoError::InvalidImage(error.to_string()))
}

fn is_root_metadata_tag(tag: u16) -> bool {
    !matches!(
        tag,
        0x00FE..=0x00FF
            | 0x0100..=0x010A
            | 0x0111..=0x0112
            | 0x0115..=0x0119
            | 0x011C
            | 0x0120..=0x0125
            | 0x012D
            | 0x013D
            | 0x0140..=0x0145
            | 0x014A
            | 0x014C..=0x0156
            | 0x015B
            | 0x0200..=0x0209
            | 0x0211..=0x0214
            | 0x02BC
            | 0x83BB
            | 0x8769
            | 0x8773
            | 0x8825
            | 0xA005
    )
}

fn decode_exif(bytes: &[u8]) -> Result<ExifArchive, IoError> {
    serde_json::from_slice(bytes).map_err(|error| IoError::InvalidImage(error.to_string()))
}

fn decode_iptc(bytes: &[u8]) -> Result<IptcArchive, IoError> {
    serde_json::from_slice(bytes).map_err(|error| IoError::InvalidImage(error.to_string()))
}

fn add_exif_directories<W: std::io::Write + std::io::Seek>(
    tiff: &mut TiffWriter<W>,
    root: &mut DirectoryWriter,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<(), IoError> {
    let archive = decode_exif(bytes)?;
    root.copy(archive.root.iter());
    if !archive.exif.is_empty() || width > 0 {
        let mut directory = DirectoryWriter::new();
        directory.copy(archive.exif.iter());
        directory.add_tag(ExifTag::ExifImageWidth, width);
        directory.add_tag(ExifTag::ExifImageHeight, height);
        if !archive.interop.is_empty() {
            let mut interop = DirectoryWriter::new();
            interop.copy(archive.interop.iter());
            let offset = interop.build(tiff).map_err(tiff_error)?;
            directory.add_tag(0xA005_u16, offset);
        }
        let offset = directory.build(tiff).map_err(tiff_error)?;
        root.add_tag(TiffCommonTag::ExifIFDPointer, offset);
    }
    if !archive.gps.is_empty() {
        let mut directory = DirectoryWriter::new();
        directory.copy(archive.gps.iter());
        let offset = directory.build(tiff).map_err(tiff_error)?;
        root.add_tag(ExifTag::GPSInfo, offset);
    }
    Ok(())
}

fn exif_blob(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>, IoError> {
    let mut output = Vec::new();
    {
        let cursor = Cursor::new(&mut output);
        let mut tiff = TiffWriter::new(cursor).map_err(tiff_error)?;
        let mut root = DirectoryWriter::new();
        add_exif_directories(&mut tiff, &mut root, bytes, width, height)?;
        tiff.build(root).map_err(tiff_error)?;
    }
    Ok(output)
}

fn write_exr<W: Write + Seek>(
    source: &SourceImage,
    rendered: &StoredImage,
    writer: W,
    progress: &AtomicU8,
) -> Result<(), IoError> {
    let width = rendered.width() as usize;
    let mut attributes = LayerAttributes::named("Dither composite");
    attributes.software_name = Some(Text::from("Dither"));
    for (name, bytes) in [
        ("ditherExif", source.info.metadata.exif.as_slice()),
        ("ditherXmp", source.info.metadata.xmp.as_slice()),
        ("ditherIptc", source.info.metadata.iptc.as_slice()),
        ("ditherCamera", source.info.metadata.camera.as_slice()),
    ] {
        if !bytes.is_empty() {
            attributes.other.insert(
                Text::from(name),
                AttributeValue::Bytes {
                    type_hint: Text::from("metadata"),
                    bytes: SmallVec::from_vec(bytes.to_vec()),
                },
            );
        }
    }
    let layer = Layer::new(
        (width, rendered.height() as usize),
        attributes,
        Encoding::SMALL_LOSSLESS,
        SpecificChannels::rgba(|position: Vec2<usize>| {
            let pixel = rendered.pixels()[position.y() * width + position.x()];
            (pixel[0], pixel[1], pixel[2], pixel[3])
        }),
    );
    let mut image = Image::from_layer(layer);
    image.attributes.chromaticities = Some(srgb_chromaticities());
    image
        .write()
        .on_progress(|value| {
            progress.store(75 + (value * 20.0).round() as u8, Ordering::Relaxed);
        })
        .to_buffered(writer)?;
    Ok(())
}

fn read_exr_metadata(path: &Path, metadata: &mut Metadata) -> Result<(), IoError> {
    let file = MetaData::read_from_file(path, false)?;
    let header = file.headers.first().ok_or(IoError::EmptyImage)?;
    if header
        .shared_attributes
        .chromaticities
        .is_some_and(|colors| !chromaticities_are_srgb(colors))
    {
        return Err(IoError::UnsupportedColorSpace(
            "OpenEXR uses non-sRGB chromaticities".into(),
        ));
    }
    for (name, destination) in [
        ("ditherExif", &mut metadata.exif),
        ("ditherXmp", &mut metadata.xmp),
        ("ditherIptc", &mut metadata.iptc),
        ("ditherCamera", &mut metadata.camera),
    ] {
        if let Some(AttributeValue::Bytes { bytes, .. }) =
            header.own_attributes.other.get(&Text::from(name))
        {
            *destination = bytes.to_vec();
        }
    }
    Ok(())
}

fn chromaticities_are_srgb(colors: Chromaticities) -> bool {
    let expected = srgb_chromaticities();
    [
        (colors.red, expected.red),
        (colors.green, expected.green),
        (colors.blue, expected.blue),
        (colors.white, expected.white),
    ]
    .into_iter()
    .all(|(actual, expected)| {
        (actual.0 - expected.0).abs() <= f32::EPSILON
            && (actual.1 - expected.1).abs() <= f32::EPSILON
    })
}

fn srgb_chromaticities() -> Chromaticities {
    Chromaticities {
        red: Vec2(0.64, 0.33),
        green: Vec2(0.30, 0.60),
        blue: Vec2(0.15, 0.06),
        white: Vec2(0.3127, 0.3290),
    }
}

fn tiff_error(error: rawler::formats::tiff::TiffError) -> IoError {
    IoError::InvalidImage(error.to_string())
}

/// Returns true when the source profile cannot describe the RGB result and
/// should be replaced with the sRGB working profile.
fn to_linear_srgb(pixels: &mut [[f32; 4]], profile_bytes: &[u8]) -> Result<bool, IoError> {
    let mut replace_profile = false;
    if !profile_bytes.is_empty() {
        let input = Profile::new_icc(profile_bytes)?;
        let output = Profile::new_srgb();
        match input.color_space() {
            ColorSpaceSignature::RgbData => {
                let transform = Transform::new(
                    &input,
                    PixelFormat::RGB_FLT,
                    &output,
                    PixelFormat::RGB_FLT,
                    Intent::Perceptual,
                )?;
                let encoded: Vec<[f32; 3]> = pixels
                    .iter()
                    .map(|pixel| pixel[..3].try_into().unwrap())
                    .collect();
                let mut converted = vec![[0.0; 3]; pixels.len()];
                transform.transform_pixels(&encoded, &mut converted);
                for (pixel, rgb) in pixels.iter_mut().zip(converted) {
                    pixel[..3].copy_from_slice(&rgb);
                }
            }
            ColorSpaceSignature::GrayData => {
                let transform = Transform::new(
                    &input,
                    PixelFormat::GRAY_FLT,
                    &output,
                    PixelFormat::RGB_FLT,
                    Intent::Perceptual,
                )?;
                let encoded: Vec<f32> = pixels.iter().map(|pixel| pixel[0]).collect();
                let mut converted = vec![[0.0; 3]; pixels.len()];
                transform.transform_pixels(&encoded, &mut converted);
                for (pixel, rgb) in pixels.iter_mut().zip(converted) {
                    pixel[..3].copy_from_slice(&rgb);
                }
                replace_profile = true;
            }
            space => return Err(IoError::UnsupportedColorSpace(format!("{space:?}"))),
        }
    }
    for pixel in pixels {
        for channel in &mut pixel[..3] {
            *channel = srgb_to_linear(*channel);
        }
    }
    Ok(replace_profile)
}

struct PixelEncoder {
    profile: Vec<u8>,
    transform: Transform<[f32; 3], [f32; 3]>,
}

impl PixelEncoder {
    fn new(output_profile: &[u8]) -> Result<Self, IoError> {
        let srgb = Profile::new_srgb();
        let profile = if output_profile.is_empty() {
            Profile::new_srgb()
        } else {
            let profile = Profile::new_icc(output_profile)?;
            if profile.color_space() != ColorSpaceSignature::RgbData {
                return Err(IoError::UnsupportedColorSpace(format!(
                    "{:?}",
                    profile.color_space()
                )));
            }
            profile
        };
        let transform = Transform::new(
            &srgb,
            PixelFormat::RGB_FLT,
            &profile,
            PixelFormat::RGB_FLT,
            Intent::Perceptual,
        )?;
        Ok(Self {
            profile: profile.icc()?,
            transform,
        })
    }

    fn encode(&self, rendered: &[Pixel]) -> Vec<[u16; 4]> {
        let input: Vec<[f32; 3]> = rendered
            .iter()
            .map(|pixel| {
                [
                    linear_to_srgb(pixel[0]),
                    linear_to_srgb(pixel[1]),
                    linear_to_srgb(pixel[2]),
                ]
            })
            .collect();
        let mut output = vec![[0.0; 3]; input.len()];
        self.transform.transform_pixels(&input, &mut output);
        output
            .into_iter()
            .zip(rendered)
            .map(|(rgb, source)| {
                [
                    quantize(rgb[0]),
                    quantize(rgb[1]),
                    quantize(rgb[2]),
                    quantize(source[3]),
                ]
            })
            .collect()
    }
}

fn u16_bytes(pixels: &[[u16; 4]], bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|pixel| pixel.iter().flat_map(|channel| bytes(*channel)))
        .collect()
}

fn quantize(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

fn same_path(left: &Path, right: &Path) -> Result<bool, std::io::Error> {
    let left = left.canonicalize()?;
    let right = if right.exists() {
        right.canonicalize()?
    } else {
        right
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()?
            .join(right.file_name().unwrap_or_default())
    };
    Ok(left == right)
}

fn temporary_path(destination: &Path) -> PathBuf {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let serial = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    destination.with_file_name(format!(
        ".{name}.dither-export-{}-{serial}",
        std::process::id()
    ))
}

fn commit_export_set(outputs: &[(PathBuf, PathBuf)], overwrite: bool) -> Result<(), IoError> {
    if outputs.is_empty() {
        return Ok(());
    }
    if !overwrite {
        let mut committed = Vec::new();
        for (temporary, destination) in outputs {
            if let Err(error) = fs::hard_link(temporary, destination) {
                for path in committed {
                    let _ = fs::remove_file(path);
                }
                return if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Err(IoError::DestinationExists)
                } else {
                    Err(error.into())
                };
            }
            committed.push(destination);
        }
        cleanup_temporaries(outputs.iter().map(|(temporary, _)| temporary));
        return Ok(());
    }

    let mut backups = Vec::with_capacity(outputs.len());
    for (_, destination) in outputs {
        if !destination.exists() {
            backups.push((destination.clone(), None));
            continue;
        }
        let mut backup = temporary_path(destination);
        let mut name = backup.file_name().unwrap_or_default().to_os_string();
        name.push(".backup");
        backup.set_file_name(name);
        if let Err(error) = fs::rename(destination, &backup) {
            restore_backups(&backups)?;
            return Err(error.into());
        }
        backups.push((destination.clone(), Some(backup)));
    }

    let mut committed = Vec::new();
    for (temporary, destination) in outputs {
        if let Err(error) = fs::rename(temporary, destination) {
            for path in &committed {
                let _ = fs::remove_file(path);
            }
            restore_backups(&backups)?;
            return Err(error.into());
        }
        committed.push(destination.clone());
    }
    for (_, backup) in backups {
        if let Some(backup) = backup {
            let _ = fs::remove_file(backup);
        }
    }
    Ok(())
}

fn restore_backups(backups: &[(PathBuf, Option<PathBuf>)]) -> Result<(), IoError> {
    let mut errors = Vec::new();
    for (destination, backup) in backups.iter().rev() {
        if let Some(backup) = backup
            && let Err(error) = fs::rename(backup, destination)
        {
            errors.push(format!(
                "{} could not be restored from {}: {error}",
                destination.display(),
                backup.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(IoError::InvalidImage(format!(
            "export rollback failed: {}",
            errors.join("; ")
        )))
    }
}

fn cleanup_temporaries<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn render_error(error: RenderError) -> IoError {
    match error {
        RenderError::Cancelled => IoError::Cancelled,
        RenderError::Storage(error) | RenderError::Target(error) => IoError::InvalidImage(error),
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), IoError> {
    if cancel.load(Ordering::Relaxed) {
        Err(IoError::Cancelled)
    } else {
        Ok(())
    }
}

struct CancellableWriter<'a> {
    file: File,
    cancel: &'a AtomicBool,
}

impl<'a> CancellableWriter<'a> {
    fn new(file: File, cancel: &'a AtomicBool) -> Self {
        Self { file, cancel }
    }

    fn check(&self) -> std::io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(std::io::Error::other("export cancelled"))
        } else {
            Ok(())
        }
    }
}

impl Write for CancellableWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.check()?;
        self.file.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.check()?;
        self.file.flush()
    }
}

impl Seek for CancellableWriter<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.check()?;
        self.file.seek(position)
    }
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Bmp => "BMP",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::OpenExr => "OpenEXR",
        ImageFormat::Png => "PNG",
        ImageFormat::Tiff => "TIFF",
        ImageFormat::WebP => "WebP",
        _ => "Raster image",
    }
}

fn is_raw(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| RAW_EXTENSIONS.contains(&extension.as_str()))
}

fn raw_orientation(orientation: RawOrientation) -> Orientation {
    match orientation {
        RawOrientation::Normal | RawOrientation::Unknown => Orientation::NoTransforms,
        RawOrientation::HorizontalFlip => Orientation::FlipHorizontal,
        RawOrientation::Rotate180 => Orientation::Rotate180,
        RawOrientation::VerticalFlip => Orientation::FlipVertical,
        RawOrientation::Transpose => Orientation::Rotate90FlipH,
        RawOrientation::Rotate90 => Orientation::Rotate90,
        RawOrientation::Transverse => Orientation::Rotate270FlipH,
        RawOrientation::Rotate270 => Orientation::Rotate270,
    }
}

#[derive(Debug)]
pub enum IoError {
    Io(std::io::Error),
    Codec(image::ImageError),
    Color(lcms2::Error),
    Png(png::EncodingError),
    Exr(exr::error::Error),
    Raw(rawler::RawlerError),
    InvalidImage(String),
    UnsupportedFormat,
    UnsupportedBitDepth(u16),
    UnsupportedColorSpace(String),
    EmptyImage,
    SourceOverwrite,
    DestinationExists,
    DestinationNotFile,
    MissingDestinationDirectory,
    MetadataLossRequiresConfirmation,
    BitDepthReduction { source: u8, output: u8 },
    Cancelled,
}

impl std::fmt::Display for IoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Codec(error) => error.fmt(formatter),
            Self::Color(error) => error.fmt(formatter),
            Self::Png(error) => error.fmt(formatter),
            Self::Exr(error) => error.fmt(formatter),
            Self::Raw(error) => error.fmt(formatter),
            Self::InvalidImage(error) => formatter.write_str(error),
            Self::UnsupportedFormat => formatter.write_str("unsupported image format"),
            Self::UnsupportedBitDepth(depth) => write!(formatter, "unsupported {depth}-bit image"),
            Self::UnsupportedColorSpace(space) => {
                write!(formatter, "unsupported embedded color space: {space}")
            }
            Self::EmptyImage => formatter.write_str("image dimensions must not be zero"),
            Self::SourceOverwrite => formatter.write_str("the source image cannot be overwritten"),
            Self::DestinationExists => formatter.write_str("the destination already exists"),
            Self::DestinationNotFile => formatter.write_str("the destination is not a file"),
            Self::MissingDestinationDirectory => {
                formatter.write_str("the destination directory does not exist")
            }
            Self::MetadataLossRequiresConfirmation => {
                formatter.write_str("this format cannot preserve all source metadata")
            }
            Self::BitDepthReduction { source, output } => {
                write!(
                    formatter,
                    "export reduces bit depth from {source} to {output}"
                )
            }
            Self::Cancelled => formatter.write_str("export cancelled"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<std::io::Error> for IoError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<image::ImageError> for IoError {
    fn from(error: image::ImageError) -> Self {
        Self::Codec(error)
    }
}

impl From<image::error::UnsupportedError> for IoError {
    fn from(error: image::error::UnsupportedError) -> Self {
        Self::Codec(image::ImageError::Unsupported(error))
    }
}

impl From<lcms2::Error> for IoError {
    fn from(error: lcms2::Error) -> Self {
        Self::Color(error)
    }
}

impl From<rawler::RawlerError> for IoError {
    fn from(error: rawler::RawlerError) -> Self {
        Self::Raw(error)
    }
}

impl From<png::EncodingError> for IoError {
    fn from(error: png::EncodingError) -> Self {
        Self::Png(error)
    }
}

impl From<exr::error::Error> for IoError {
    fn from(error: exr::error::Error) -> Self {
        Self::Exr(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dither_core::{Document, FourColor, PaletteSettings, Separation, Transform};
    use image::{ExtendedColorType, ImageEncoder};
    use lcms2::{CIExyY, ToneCurve};
    use std::sync::Arc;

    #[test]
    fn export_format_properties_match_validation_rules() {
        let metadata = Metadata {
            iptc: vec![1],
            ..Metadata::default()
        };
        assert_eq!(ExportFormat::Png16.bit_depth(), 16);
        assert_eq!(ExportFormat::OpenExr32.bit_depth(), 32);
        assert!(ExportFormat::Tiff16.embeds_icc_profile());
        assert!(!ExportFormat::OpenExr32.embeds_icc_profile());
        assert!(!ExportFormat::Png16.preserves_all_metadata(&metadata));
        assert!(ExportFormat::Tiff16.preserves_all_metadata(&metadata));
        assert!(ExportFormat::OpenExr32.preserves_all_metadata(&metadata));
    }

    #[test]
    fn lossless_raster_exports_preserve_dimensions_alpha_profile_and_xmp() {
        let directory = std::env::temp_dir().join(format!("dither-io-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.png");
        let export_path = directory.join("export.png");
        let exif_png_path = directory.join("exif.png");
        let tiff_path = directory.join("export.tif");
        let exr_path = directory.join("export.exr");
        let pixels = [0_u16, 1000, 65535, 32768, 65535, 0, 2000, 65535];
        let mut encoder = image::codecs::png::PngEncoder::new(File::create(&source_path).unwrap());
        encoder
            .set_icc_profile(Profile::new_srgb().icc().unwrap())
            .unwrap();
        encoder
            .write_image(
                &u16_bytes(
                    &[
                        [pixels[0], pixels[1], pixels[2], pixels[3]],
                        [pixels[4], pixels[5], pixels[6], pixels[7]],
                    ],
                    u16::to_ne_bytes,
                ),
                2,
                1,
                ExtendedColorType::Rgba16,
            )
            .unwrap();

        let mut source = open(&source_path).unwrap();
        source.info.metadata.xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#.to_vec();
        let render = Document::new(source.clone()).render();
        export(
            &Document::new(source.clone()),
            &export_path,
            ExportFormat::Png16,
            ExportOptions::default(),
        )
        .unwrap();
        let reopened = open(&export_path).unwrap();

        assert_eq!((reopened.width(), reopened.height()), (2, 1));
        assert_eq!(reopened.info.bit_depth, 16);
        assert!(!reopened.info.color_profile.is_empty());
        assert_eq!(reopened.info.metadata.xmp, source.info.metadata.xmp);
        assert!((reopened.pixels()[0][3] - 32768.0 / 65535.0).abs() < 0.0001);
        assert!(source_path.exists());

        source.info.metadata.exif = sample_exif();
        export(
            &Document::new(source.clone()),
            &exif_png_path,
            ExportFormat::Png16,
            ExportOptions::default(),
        )
        .unwrap();
        let reopened_png = open(&exif_png_path).unwrap();
        assert_exif_preserved_with_dimensions(
            decode_exif(&source.info.metadata.exif).unwrap(),
            decode_exif(&reopened_png.info.metadata.exif).unwrap(),
            2,
            1,
        );

        source.info.metadata.iptc = normalize_iptc(vec![0x1c, 0x02, 0x05, 0x80, 0xff]);
        export(
            &Document::new(source.clone()),
            &tiff_path,
            ExportFormat::Tiff16,
            ExportOptions::default(),
        )
        .unwrap();
        let reopened = open(&tiff_path).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (2, 1));
        assert_eq!(reopened.info.bit_depth, 16);
        assert!(!reopened.info.color_profile.is_empty());
        assert_eq!(reopened.info.metadata.xmp, source.info.metadata.xmp);
        assert!((reopened.pixels()[0][3] - 32768.0 / 65535.0).abs() < 0.0001);
        let original_exif = decode_exif(&source.info.metadata.exif).unwrap();
        let reopened_exif = decode_exif(&reopened.info.metadata.exif).unwrap();
        for (tag, value) in original_exif.root {
            assert_eq!(reopened_exif.root.get(&tag), Some(&value));
        }
        for (tag, value) in original_exif.exif {
            assert_eq!(reopened_exif.exif.get(&tag), Some(&value));
        }
        assert_eq!(
            reopened_exif.exif.get(&(ExifTag::ExifImageWidth as u16)),
            Some(&Value::Long(vec![2]))
        );
        assert_eq!(
            reopened_exif.exif.get(&(ExifTag::ExifImageHeight as u16)),
            Some(&Value::Long(vec![1]))
        );
        assert_eq!(reopened_exif.gps, original_exif.gps);
        assert_eq!(reopened_exif.interop, original_exif.interop);
        assert_eq!(
            iptc_bytes(decode_iptc(&reopened.info.metadata.iptc).unwrap()),
            vec![0x1c, 0x02, 0x05, 0x80, 0xff]
        );

        source.info.metadata.exif = b"exact exif bytes".to_vec();
        source.info.metadata.iptc = b"exact iptc bytes".to_vec();
        source.info.metadata.camera = b"exact camera bytes".to_vec();
        export(
            &Document::new(source.clone()),
            &exr_path,
            ExportFormat::OpenExr32,
            ExportOptions::default(),
        )
        .unwrap();
        let reopened = open(&exr_path).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (2, 1));
        assert_eq!(reopened.info.bit_depth, 32);
        assert_eq!(reopened.info.metadata, source.info.metadata);
        for (actual, expected) in reopened.pixels().iter().zip(render.pixels()) {
            for (actual, expected) in actual.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.000001);
            }
        }

        fs::remove_file(source_path).unwrap();
        fs::remove_file(export_path).unwrap();
        fs::remove_file(exif_png_path).unwrap();
        fs::remove_file(tiff_path).unwrap();
        fs::remove_file(exr_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    fn assert_exif_preserved_with_dimensions(
        original: ExifArchive,
        actual: ExifArchive,
        width: u32,
        height: u32,
    ) {
        for (tag, value) in original.root {
            assert_eq!(actual.root.get(&tag), Some(&value));
        }
        for (tag, value) in original.exif {
            assert_eq!(actual.exif.get(&tag), Some(&value));
        }
        assert_eq!(
            actual.exif.get(&(ExifTag::ExifImageWidth as u16)),
            Some(&Value::Long(vec![width]))
        );
        assert_eq!(
            actual.exif.get(&(ExifTag::ExifImageHeight as u16)),
            Some(&Value::Long(vec![height]))
        );
        assert_eq!(actual.gps, original.gps);
        assert_eq!(actual.interop, original.interop);
    }

    fn sample_exif() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let cursor = Cursor::new(&mut bytes);
            let mut writer = TiffWriter::new(cursor).unwrap();
            let mut root = DirectoryWriter::new();
            root.add_tag(TiffCommonTag::Artist, "Dither test");
            root.add_tag(TiffCommonTag::Software, "Source software");
            root.add_tag(0xC7A1_u16, "private root metadata");
            let mut exif = DirectoryWriter::new();
            exif.add_tag(ExifTag::DateTimeOriginal, "2026:07:18 12:00:00");
            let mut interop = DirectoryWriter::new();
            interop.add_tag(0x0001_u16, "R98");
            let interop_offset = interop.build(&mut writer).unwrap();
            exif.add_tag(0xA005_u16, interop_offset);
            let offset = exif.build(&mut writer).unwrap();
            root.add_tag(TiffCommonTag::ExifIFDPointer, offset);
            writer.build(root).unwrap();
        }
        normalize_exif(&bytes).unwrap()
    }

    fn iptc_bytes(archive: IptcArchive) -> Vec<u8> {
        match archive {
            IptcArchive::Bytes(bytes) => bytes,
            IptcArchive::Tiff(Value::Byte(bytes) | Value::Undefined(bytes)) => bytes,
            IptcArchive::Tiff(value) => panic!("unexpected IPTC value: {value:?}"),
        }
    }

    #[test]
    fn source_path_is_never_an_export_target() {
        let directory = std::env::temp_dir().join(format!("dither-source-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.png");
        image::codecs::png::PngEncoder::new(File::create(&source_path).unwrap())
            .write_image(&[0, 0, 0, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        let source = open(&source_path).unwrap();
        let destination = directory.join("existing.png");
        fs::write(&destination, b"original destination").unwrap();

        assert!(matches!(
            export(
                &Document::new(source.clone()),
                &source_path,
                ExportFormat::Png16,
                ExportOptions {
                    overwrite: true,
                    ..ExportOptions::default()
                }
            ),
            Err(IoError::SourceOverwrite)
        ));
        assert!(matches!(
            export(
                &Document::new(source.clone()),
                &destination,
                ExportFormat::Png16,
                ExportOptions::default()
            ),
            Err(IoError::DestinationExists)
        ));
        assert_eq!(fs::read(&destination).unwrap(), b"original destination");
        export(
            &Document::new(source.clone()),
            &destination,
            ExportFormat::Png16,
            ExportOptions {
                overwrite: true,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            (
                open(&destination).unwrap().width(),
                open(&destination).unwrap().height()
            ),
            (1, 1)
        );
        assert!(source_path.exists());

        fs::remove_file(source_path).unwrap();
        fs::remove_file(destination).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn geometry_exports_full_resolution_with_updated_dimensions() {
        let directory =
            std::env::temp_dir().join(format!("dither-geometry-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.png");
        let export_path = directory.join("geometry.tif");
        let source_pixels = vec![32_000_u16; 8 * 4 * 4];
        image::codecs::png::PngEncoder::new(File::create(&source_path).unwrap())
            .write_image(
                &source_pixels
                    .iter()
                    .flat_map(|value| value.to_ne_bytes())
                    .collect::<Vec<_>>(),
                8,
                4,
                ExtendedColorType::Rgba16,
            )
            .unwrap();
        let source_bytes = fs::read(&source_path).unwrap();
        let mut source = open(&source_path).unwrap();
        source.info.metadata.exif = sample_exif();
        let mut document = Document::new(source);
        document.recipe.transform = Transform {
            crop: [0.125, 0.0, 0.875, 1.0],
            quarter_turns: 1,
            straighten_degrees: 2.0,
        };

        export(
            &document,
            &export_path,
            ExportFormat::Tiff16,
            ExportOptions::default(),
        )
        .unwrap();

        let reopened = open(&export_path).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (4, 6));
        let exif = decode_exif(&reopened.info.metadata.exif).unwrap();
        assert_eq!(
            exif.exif.get(&(ExifTag::ExifImageWidth as u16)),
            Some(&Value::Long(vec![4]))
        );
        assert_eq!(
            exif.exif.get(&(ExifTag::ExifImageHeight as u16)),
            Some(&Value::Long(vec![6]))
        );
        assert_eq!(fs::read(&source_path).unwrap(), source_bytes);

        fs::remove_file(source_path).unwrap();
        fs::remove_file(export_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn streamed_exports_cross_strip_boundaries() {
        let directory = std::env::temp_dir().join(format!("dither-strips-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.test");
        fs::write(&source_path, b"source placeholder").unwrap();
        let source = SourceImage::new(
            NonZeroU32::MIN,
            NonZeroU32::new(EXPORT_ROWS + 1).unwrap(),
            vec![[0.5, 0.5, 0.5, 1.0]; (EXPORT_ROWS + 1) as usize],
            SourceInfo {
                path: source_path.clone(),
                format: "test".into(),
                bit_depth: 16,
                color_profile: Profile::new_srgb().icc().unwrap(),
                metadata: Metadata::default(),
            },
        )
        .unwrap();
        let document = Document::new(source);

        for (name, format) in [
            ("export.png", ExportFormat::Png16),
            ("export.tif", ExportFormat::Tiff16),
        ] {
            let path = directory.join(name);
            export(&document, &path, format, ExportOptions::default()).unwrap();
            let reopened = open(&path).unwrap();
            assert_eq!((reopened.width(), reopened.height()), (1, EXPORT_ROWS + 1));
            fs::remove_file(path).unwrap();
        }

        assert!(supported_extensions().any(|extension| extension == "dcs"));
        assert!(is_raw(Path::new("camera.DCS")));
        fs::remove_file(source_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn separated_export_writes_composite_and_true_grayscale_plates() {
        let directory = std::env::temp_dir().join(format!("dither-plates-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.test");
        fs::write(&source_path, b"source placeholder").unwrap();
        let source = SourceImage::new(
            NonZeroU32::new(2).unwrap(),
            NonZeroU32::new(2).unwrap(),
            vec![
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
            SourceInfo {
                path: source_path.clone(),
                format: "test".into(),
                bit_depth: 16,
                color_profile: Profile::new_srgb().icc().unwrap(),
                metadata: Metadata::default(),
            },
        )
        .unwrap();
        let mut document = Document::new(source);
        document.recipe.separation = Separation::Cmyk(FourColor::default());
        document.recipe.print.dpi = 600.0;
        let composite = directory.join("separated.tif");

        let paths = export_with_plates(
            &document,
            &composite,
            ExportFormat::Tiff16,
            ExportOptions::default(),
        )
        .unwrap();

        assert_eq!(paths.len(), 5);
        assert!(paths.iter().all(|path| path.exists()));
        for path in paths.iter().skip(1) {
            let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
            let reader = decoder.read_info().unwrap();
            assert_eq!(reader.info().color_type, png::ColorType::Grayscale);
            assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
            let dimensions = reader.info().pixel_dims.unwrap();
            assert_eq!(dimensions.unit, png::Unit::Meter);
            assert!((dimensions.xppu as f32 * 0.0254 - 600.0).abs() < 0.1);
        }

        for path in paths {
            fs::remove_file(path).unwrap();
        }
        fs::remove_file(source_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn separated_export_commits_only_palette_plates_that_rendered() {
        let directory = std::env::temp_dir().join(format!("dither-palette-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.test");
        fs::write(&source_path, b"source placeholder").unwrap();
        let source = SourceImage::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            vec![[0.4, 0.5, 0.6, 1.0]],
            SourceInfo {
                path: source_path.clone(),
                format: "test".into(),
                bit_depth: 16,
                color_profile: Profile::new_srgb().icc().unwrap(),
                metadata: Metadata::default(),
            },
        )
        .unwrap();
        let mut document = Document::new(source);
        let mut palette = PaletteSettings::default();
        palette.colors.clear();
        palette.size = 8;
        document.recipe.separation = Separation::Indexed(palette);
        let composite = directory.join("palette.tif");
        let unused_plate = directory.join("palette-plate-index-08.png");
        fs::write(&unused_plate, b"unrelated existing file").unwrap();

        let paths = export_with_plates(
            &document,
            &composite,
            ExportFormat::Tiff16,
            ExportOptions::default(),
        )
        .unwrap();

        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.exists()));
        assert_eq!(fs::read(&unused_plate).unwrap(), b"unrelated existing file");
        assert!(fs::read_dir(&directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".dither-export")
        }));

        let blocked_composite = directory.join("blocked.tif");
        let blocked_plate = directory.join("blocked-plate-index-01.png");
        fs::write(&blocked_plate, b"existing plate").unwrap();
        assert!(matches!(
            export_with_plates(
                &document,
                &blocked_composite,
                ExportFormat::Tiff16,
                ExportOptions::default(),
            ),
            Err(IoError::DestinationExists)
        ));
        assert!(!blocked_composite.exists());
        assert_eq!(fs::read(&blocked_plate).unwrap(), b"existing plate");

        for path in paths {
            fs::remove_file(path).unwrap();
        }
        fs::remove_file(unused_plate).unwrap();
        fs::remove_file(blocked_plate).unwrap();
        fs::remove_file(source_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn export_set_commit_rolls_back_every_destination() {
        let directory = std::env::temp_dir().join(format!("dither-atomic-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();

        let first_temp = directory.join("first.temp");
        let second_temp = directory.join("second.temp");
        let first_destination = directory.join("first.out");
        let missing_parent_destination = directory.join("missing/second.out");
        fs::write(&first_temp, b"first new").unwrap();
        fs::write(&second_temp, b"second new").unwrap();
        assert!(
            commit_export_set(
                &[
                    (first_temp.clone(), first_destination.clone()),
                    (second_temp.clone(), missing_parent_destination),
                ],
                false,
            )
            .is_err()
        );
        assert!(!first_destination.exists());
        assert!(first_temp.exists());
        assert!(second_temp.exists());

        let second_destination = directory.join("second.out");
        fs::write(&first_destination, b"first original").unwrap();
        fs::write(&second_destination, b"second original").unwrap();
        fs::write(&first_temp, b"first replacement").unwrap();
        fs::remove_file(&second_temp).unwrap();
        assert!(
            commit_export_set(
                &[
                    (first_temp.clone(), first_destination.clone()),
                    (second_temp.clone(), second_destination.clone()),
                ],
                true,
            )
            .is_err()
        );
        assert_eq!(fs::read(&first_destination).unwrap(), b"first original");
        assert_eq!(fs::read(&second_destination).unwrap(), b"second original");
        assert!(fs::read_dir(&directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".backup")
        }));

        fs::remove_file(first_temp).ok();
        fs::remove_file(first_destination).unwrap();
        fs::remove_file(second_destination).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn cancellation_removes_all_temporary_outputs_and_preserves_source() {
        let directory = std::env::temp_dir().join(format!("dither-cancel-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("source.test");
        fs::write(&source_path, b"source bytes must stay unchanged").unwrap();
        let source_bytes = fs::read(&source_path).unwrap();
        let source = SourceImage::new(
            NonZeroU32::new(512).unwrap(),
            NonZeroU32::new(512).unwrap(),
            vec![[0.4, 0.5, 0.6, 1.0]; 512 * 512],
            SourceInfo {
                path: source_path.clone(),
                format: "test".into(),
                bit_depth: 16,
                color_profile: Profile::new_srgb().icc().unwrap(),
                metadata: Metadata::default(),
            },
        )
        .unwrap();
        let document = Document::new(source);
        let destination = directory.join("cancelled.tif");
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU8::new(0));
        let cancel_thread = cancel.clone();
        let progress_thread = progress.clone();
        let trigger = std::thread::spawn(move || {
            while progress_thread.load(Ordering::Relaxed) < 15 {
                std::thread::yield_now();
            }
            cancel_thread.store(true, Ordering::Relaxed);
        });

        let result = export_with_plates_cancellable(
            &document,
            &destination,
            ExportFormat::Tiff16,
            ExportOptions::default(),
            &cancel,
            &progress,
        );
        trigger.join().unwrap();

        assert!(matches!(result, Err(IoError::Cancelled)));
        assert!(!destination.exists());
        assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
        assert!(fs::read_dir(&directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".dither-export")
        }));

        fs::remove_file(source_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn encoder_cancellation_is_reported_as_cancellation() {
        let directory =
            std::env::temp_dir().join(format!("dither-encode-cancel-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("cancelled.exr");
        let source = SourceImage::new(
            NonZeroU32::new(2).unwrap(),
            NonZeroU32::new(2).unwrap(),
            vec![[0.4, 0.5, 0.6, 1.0]; 4],
            SourceInfo {
                path: directory.join("source.test"),
                format: "test".into(),
                bit_depth: 16,
                color_profile: Profile::new_srgb().icc().unwrap(),
                metadata: Metadata::default(),
            },
        )
        .unwrap();
        let document = Document::new(source);
        let cancel = AtomicBool::new(false);
        let progress = AtomicU8::new(0);
        let rendered = document
            .render_stored(&cancel, &progress, |_, _, _, _, _| Ok(()))
            .unwrap();
        cancel.store(true, Ordering::Relaxed);

        let result = write_stored_export(
            document.source(),
            &rendered,
            &path,
            ExportFormat::OpenExr32,
            document.recipe.print.dpi,
            &cancel,
            &progress,
        );

        assert!(matches!(result, Err(IoError::Cancelled)));
        fs::remove_file(path).ok();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn grayscale_icc_is_color_managed_into_the_rgb_working_space() {
        let directory = std::env::temp_dir().join(format!("dither-gray-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("gray.png");
        let bytes: Vec<u8> = [0_u16, 32768, 65535]
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .collect();
        let gray_profile = Profile::new_gray(
            &CIExyY {
                x: 0.3457,
                y: 0.3585,
                Y: 1.0,
            },
            &ToneCurve::new(2.2),
        )
        .unwrap();
        let mut encoder = image::codecs::png::PngEncoder::new(File::create(&path).unwrap());
        encoder
            .set_icc_profile(gray_profile.icc().unwrap())
            .unwrap();
        encoder
            .write_image(&bytes, 3, 1, ExtendedColorType::L16)
            .unwrap();

        let image = open(&path).unwrap();

        assert_eq!(image.info.bit_depth, 16);
        assert_eq!(
            Profile::new_icc(&image.info.color_profile)
                .unwrap()
                .color_space(),
            ColorSpaceSignature::RgbData
        );
        assert!(
            image
                .pixels()
                .windows(2)
                .all(|pair| pair[0][0] < pair[1][0])
        );
        assert!(
            image
                .pixels()
                .iter()
                .all(|pixel| (pixel[0] - pixel[1]).abs() < 0.0001
                    && (pixel[1] - pixel[2]).abs() < 0.0001)
        );

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn standards_compliant_dng_develops_and_exports_with_camera_metadata() {
        use rawler::{
            CFA, RawImage,
            cfa::PlaneColor,
            decoders::Camera,
            dng::{
                CropMode, DNG_VERSION_V1_4, DngCompression, DngPhotometricConversion,
                writer::DngWriter,
            },
            imgop::xyz::Illuminant,
            pixarray::PixU16,
            rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel},
        };

        let directory = std::env::temp_dir().join(format!("dither-dng-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let dng_path = directory.join("linear-raw.dng");
        let width = 32_usize;
        let height = 24_usize;
        let pixels = (0..width * height)
            .map(|index| 1024 + ((index * 83) % 60_000) as u16)
            .collect::<Vec<_>>();
        let cfa = CFA::new("RGGB");
        let plane_color = PlaneColor::default();
        let mut camera = Camera {
            make: "Dither".into(),
            model: "CFA RAW test".into(),
            clean_make: "Dither".into(),
            clean_model: "CFA RAW test".into(),
            real_bps: 16,
            cfa: cfa.clone(),
            plane_color: plane_color.clone(),
            ..Camera::default()
        };
        camera.color_matrix.insert(
            Illuminant::D65,
            vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        );
        let raw_image = RawImage::new(
            camera,
            PixU16::new_with(pixels, width, height),
            1,
            [2.0, 1.0, 1.5, f32::NAN],
            RawPhotometricInterpretation::Cfa(CFAConfig::new(&cfa, &plane_color)),
            Some(BlackLevel::new(&[1024_u32; 4], 2, 2, 1)),
            Some(WhiteLevel::new_bits(16, 1)),
            false,
        );
        let mut dng = DngWriter::new(
            BufWriter::new(File::create(&dng_path).unwrap()),
            DNG_VERSION_V1_4,
        )
        .unwrap();
        let mut raw = dng.subframe_on_root(0);
        raw.raw_image(
            &raw_image,
            CropMode::None,
            DngCompression::Uncompressed,
            DngPhotometricConversion::Original,
            1,
        )
        .unwrap();
        raw.finalize().unwrap();
        dng.load_base_tags(&raw_image).unwrap();
        dng.close().unwrap();

        let source = open(&dng_path).unwrap();

        assert_eq!(
            (source.width(), source.height()),
            (width as u32, height as u32)
        );
        assert_eq!(source.info.bit_depth, 16);
        assert!(!source.info.metadata.camera.is_empty());
        assert!(
            source
                .pixels()
                .iter()
                .all(|pixel| pixel.iter().all(|value| value.is_finite()))
        );

        let output = directory.join("linear-raw.exr");
        let tiff_output = directory.join("linear-raw.tif");
        export(
            &Document::new(source.clone()),
            &tiff_output,
            ExportFormat::Tiff16,
            ExportOptions::default(),
        )
        .unwrap();
        export(
            &Document::new(source.clone()),
            &output,
            ExportFormat::OpenExr32,
            ExportOptions::default(),
        )
        .unwrap();
        let reopened = open(&output).unwrap();

        assert_eq!(
            (reopened.width(), reopened.height()),
            (source.width(), source.height())
        );
        assert_eq!(reopened.info.bit_depth, 32);
        assert_eq!(reopened.info.metadata.camera, source.info.metadata.camera);
        let reopened_tiff = open(&tiff_output).unwrap();
        assert_eq!(
            (reopened_tiff.width(), reopened_tiff.height()),
            (source.width(), source.height())
        );
        let tiff_exif = decode_exif(&reopened_tiff.info.metadata.exif).unwrap();
        assert!(tiff_exif.root.contains_key(&(TiffCommonTag::Make as u16)));
        assert!(tiff_exif.root.contains_key(&(TiffCommonTag::Model as u16)));
        assert!(!tiff_exif.exif.is_empty());
        fs::remove_file(output).unwrap();
        fs::remove_file(tiff_output).unwrap();
        fs::remove_file(dng_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
