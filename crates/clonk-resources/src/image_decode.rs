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

/// `StdJpeg`'s source manager, as a reader.
///
/// When libjpeg asks for more input than the file holds, `fill_input_buffer`
/// hands it a synthetic `FF D9` end-of-image instead of reporting an error
/// (C4Surface's decoder, StdJpegLibjpeg.cpp:68-76, following libjpeg's own
/// advice for exhausted input). A truncated entropy stream therefore decodes
/// to a complete image, with the missing blocks recovered by the decoder,
/// rather than aborting the whole picture — which matters because this decoder
/// serves portraits, definitions, assets and loader screens alike.
///
/// The marker is re-supplied on every subsequent read, exactly as libjpeg's
/// source manager re-points at the same two bytes each time.
struct SyntheticEndOfImage<R> {
    inner: R,
    exhausted: bool,
    marker: std::iter::Cycle<std::array::IntoIter<u8, 2>>,
}

impl<R: Read> SyntheticEndOfImage<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            exhausted: false,
            marker: [0xff, 0xd9].into_iter().cycle(),
        }
    }
}

impl<R: Read> Read for SyntheticEndOfImage<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if !self.exhausted {
            let read = self.inner.read(buffer)?;
            if read > 0 {
                return Ok(read);
            }
            self.exhausted = true;
        }
        let filled = buffer
            .iter_mut()
            .zip(&mut self.marker)
            .map(|(slot, byte)| *slot = byte)
            .count();
        Ok(filled)
    }
}

fn decode_jpeg(reader: impl Read) -> Result<DynamicImage, ImageError> {
    let mut decoder = Decoder::new(SyntheticEndOfImage::new(reader));
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

    /// `tests/fixtures/jpeg_truncation/`: a 16x16 gradient whose scan data
    /// starts at byte 623, and what libjpeg makes of it. See the README there.
    const GRADIENT16: &[u8] = include_bytes!("../tests/fixtures/jpeg_truncation/gradient16.jpg");
    const GRADIENT16_LIBJPEG_FULL: &[u8] =
        include_bytes!("../tests/fixtures/jpeg_truncation/gradient16_libjpeg_full.rgb");
    const GRADIENT16_LIBJPEG_KEEP640: &[u8] =
        include_bytes!("../tests/fixtures/jpeg_truncation/gradient16_libjpeg_keep640.rgb");
    /// 39 bytes into the entropy stream.
    const GRADIENT16_TRUNCATED: usize = 640;

    fn decode_rgb(bytes: &[u8]) -> image::RgbImage {
        load_image_from_memory_with_format(bytes, ImageFormat::Jpeg)
            .expect("the shared decoder must not fail on this fixture")
            .to_rgb8()
    }

    /// A truncated entropy stream decodes to a complete image.
    ///
    /// `StdJpeg`'s source manager hands libjpeg a synthetic `FF D9` when the
    /// input runs out rather than reporting an error (oracle-src-pinned
    /// src/StdJpegLibjpeg.cpp:68-76), so the decode runs to the last row and
    /// `C4Surface::ReadJPEG` returns a full-size surface
    /// (src/C4Surface.cpp:1029-1072). Both oracle runs behind these fixtures
    /// report `rows_written: 16` and no error. Propagating the underlying
    /// decoder's EOF instead loses the whole image — every portrait,
    /// definition, asset and loader screen goes through here.
    #[test]
    fn a_truncated_entropy_stream_decodes_to_a_complete_image() {
        for keep in [GRADIENT16_TRUNCATED, 650, 660, GRADIENT16.len() - 1] {
            let image = decode_rgb(&GRADIENT16[..keep]);
            assert_eq!(
                (image.width(), image.height()),
                (16, 16),
                "truncating to {keep} bytes must still yield every row"
            );
        }
    }

    /// Truncating the *header* is still an error, not a silent empty image.
    ///
    /// The synthetic end-of-image is what libjpeg's source manager supplies
    /// for exhausted **entropy** input. Cutting a file off inside its Huffman
    /// tables instead makes libjpeg throw before the surface is ever created
    /// (`Bogus Huffman table definition`), and `C4Surface::ReadJPEG` logs it
    /// (src/C4Surface.cpp:1065-1068). The port reports that to its caller
    /// rather than handing back a zero-sized picture.
    #[test]
    fn a_truncated_header_is_still_refused() {
        // Byte 623 is where this fixture's scan data begins; 400 lands inside
        // the Huffman tables.
        let error = load_image_from_memory_with_format(&GRADIENT16[..400], ImageFormat::Jpeg)
            .expect_err("a header that stops mid-table cannot describe an image");
        assert!(
            !matches!(error, ImageError::Limits(_)),
            "the refusal must come from the malformed header, got {error:?}"
        );
    }

    /// The recovered pixels are libjpeg's, up to the decoder difference that
    /// is already there.
    ///
    /// `jpeg-decoder` is not libjpeg: decoding the *whole* fixture already
    /// differs from it, which is a pre-existing inverse-DCT and colour
    /// conversion gap, not something truncation introduces. What this pins is
    /// that recovery adds nothing to that gap — otherwise a synthetic EOI that
    /// silently produced different pixels would read as success.
    #[test]
    fn truncation_recovery_is_no_further_from_libjpeg_than_a_whole_file_is() {
        let deltas = |decoded: &image::RgbImage, oracle: &[u8]| {
            let raw = decoded.as_raw();
            assert_eq!(raw.len(), oracle.len(), "same buffer shape as the oracle");
            raw.iter()
                .zip(oracle)
                .filter(|(ours, theirs)| ours != theirs)
                .map(|(ours, theirs)| u32::from(ours.abs_diff(*theirs)))
                .fold((0usize, 0u32), |(count, worst), delta| {
                    (count + 1, worst.max(delta))
                })
        };

        let (whole_count, whole_worst) = deltas(&decode_rgb(GRADIENT16), GRADIENT16_LIBJPEG_FULL);
        let (recovered_count, recovered_worst) = deltas(
            &decode_rgb(&GRADIENT16[..GRADIENT16_TRUNCATED]),
            GRADIENT16_LIBJPEG_KEEP640,
        );

        assert!(
            recovered_count <= whole_count && recovered_worst <= whole_worst,
            "recovering a truncated stream drifted further from libjpeg \
             ({recovered_count} bytes, worst {recovered_worst}) than decoding \
             the whole file does ({whole_count} bytes, worst {whole_worst})"
        );
    }
}
