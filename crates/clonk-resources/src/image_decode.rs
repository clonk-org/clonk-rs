use image::error::{
    DecodingError, ParameterError, ParameterErrorKind, UnsupportedError, UnsupportedErrorKind,
};
use image::{ColorType, DynamicImage, ImageBuffer, ImageError, ImageFormat, Limits, Luma};
use jpeg_decoder::{Decoder, PixelFormat};
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

/// Decode an image after identifying its format from its file signature.
pub fn load_image_from_memory(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let format = image::guess_format(bytes)?;
    load_image_from_memory_with_format(bytes, format)
}

/// Decode an image with the same JPEG pixels produced before image 0.25.
///
/// Image 0.25 replaced `jpeg-decoder` with `zune-jpeg`, whose inverse DCT and
/// color conversion produce different bytes for shipped assets. Loader and
/// texture pixels are deterministic inputs in this port, so keep the previous
/// decoder explicitly while using image 0.25 for every other codec and API.
pub fn load_image_from_memory_with_format(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<DynamicImage, ImageError> {
    if format == ImageFormat::Jpeg {
        decode_jpeg(Cursor::new(bytes))
    } else {
        image::load_from_memory_with_format(bytes, format)
    }
}

/// Open an image whose format is selected from its path extension.
pub fn open_image(path: impl AsRef<Path>) -> Result<DynamicImage, ImageError> {
    let path = path.as_ref();
    let reader = BufReader::new(File::open(path)?);
    let format = ImageFormat::from_path(path)?;
    if format == ImageFormat::Jpeg {
        decode_jpeg(reader)
    } else {
        image::load(reader, format)
    }
}

fn decode_jpeg(reader: impl Read) -> Result<DynamicImage, ImageError> {
    let mut decoder = Decoder::new(reader);
    decoder.read_info().map_err(jpeg_error)?;
    let info = decoder.info().ok_or_else(|| {
        ImageError::Decoding(DecodingError::from_format_hint(ImageFormat::Jpeg.into()))
    })?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let color_type = match info.pixel_format {
        PixelFormat::L8 => ColorType::L8,
        PixelFormat::L16 => ColorType::L16,
        PixelFormat::RGB24 | PixelFormat::CMYK32 => ColorType::Rgb8,
    };
    let mut limits = Limits::default();
    limits.reserve_buffer(width, height, color_type)?;
    let pixels = decoder.decode().map_err(jpeg_error)?;

    match info.pixel_format {
        PixelFormat::L8 => image::GrayImage::from_raw(width, height, pixels)
            .map(DynamicImage::ImageLuma8)
            .ok_or_else(dimension_error),
        PixelFormat::L16 => {
            let samples = pixels
                .chunks_exact(2)
                .map(|sample| u16::from_ne_bytes([sample[0], sample[1]]))
                .collect();
            ImageBuffer::<Luma<u16>, Vec<u16>>::from_raw(width, height, samples)
                .map(DynamicImage::ImageLuma16)
                .ok_or_else(dimension_error)
        }
        PixelFormat::RGB24 => image::RgbImage::from_raw(width, height, pixels)
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(dimension_error),
        PixelFormat::CMYK32 => image::RgbImage::from_raw(width, height, cmyk_to_rgb(&pixels))
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(dimension_error),
    }
}

fn cmyk_to_rgb(input: &[u8]) -> Vec<u8> {
    input
        .chunks_exact(4)
        .flat_map(|pixel| {
            let c = 255 - u16::from(pixel[0]);
            let m = 255 - u16::from(pixel[1]);
            let y = 255 - u16::from(pixel[2]);
            let k = 255 - u16::from(pixel[3]);
            [
                ((k * c) / 255) as u8,
                ((k * m) / 255) as u8,
                ((k * y) / 255) as u8,
            ]
        })
        .collect()
}

fn jpeg_error(error: jpeg_decoder::Error) -> ImageError {
    match error {
        jpeg_decoder::Error::Io(error) => ImageError::IoError(error),
        jpeg_decoder::Error::Unsupported(feature) => {
            ImageError::Unsupported(UnsupportedError::from_format_and_kind(
                ImageFormat::Jpeg.into(),
                UnsupportedErrorKind::GenericFeature(format!("{feature:?}")),
            ))
        }
        error => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), error)),
    }
}

fn dimension_error() -> ImageError {
    ImageError::Parameter(ParameterError::from_kind(
        ParameterErrorKind::DimensionMismatch,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_jpeg_is_rejected_by_the_image_allocation_limit() {
        // A complete SOF0 header is enough for jpeg-decoder to expose the
        // dimensions without decoding scan data. At 65,535 square, RGB output
        // would need far more than image's default 512 MiB allocation limit.
        let oversized_header = [
            0xff, 0xd8, // SOI
            0xff, 0xc0, 0x00, 0x11, // SOF0 and segment length
            0x08, 0xff, 0xff, 0xff, 0xff, // precision, height, width
            0x03, // components
            0x01, 0x11, 0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00,
        ];

        let error = load_image_from_memory_with_format(&oversized_header, ImageFormat::Jpeg)
            .expect_err("oversized JPEG must be rejected before pixel allocation");

        assert!(matches!(error, ImageError::Limits(_)), "{error:?}");
    }
}
