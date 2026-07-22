//! Raw 8-bit indexed BMP reading. The legacy engine loads `Landscape.bmp`
//! and `Map.bmp` as palette-INDEX surfaces (the index byte is the
//! material+texture key; the palette colors are presentation only), so the
//! port must read the indices verbatim — generic image decoding to RGBA
//! destroys them.

use thiserror::Error;

const BMP_FILE_HEADER_SIZE: usize = 14;
const BMP_INFO_HEADER_SIZE: usize = 40;
const BMP_PALETTE_ENTRY_COUNT: usize = 256;
const BMP_PALETTE_ENTRY_SIZE: usize = 4;
const BMP_DATA_OFFSET: usize =
    BMP_FILE_HEADER_SIZE + BMP_INFO_HEADER_SIZE + BMP_PALETTE_ENTRY_COUNT * BMP_PALETTE_ENTRY_SIZE;

/// The RGB colors for all 256 entries in an 8-bit indexed BMP palette.
pub type RgbPalette = [[u8; 3]; BMP_PALETTE_ENTRY_COUNT];

#[derive(Debug, Error)]
pub enum BitmapError {
    #[error("bitmap data truncated ({0})")]
    Truncated(&'static str),
    #[error("unsupported BMP bit depth {0} (only 8-bit indexed is read)")]
    BitDepth(u16),
    #[error("8-bit BMP pixel offset {0} precedes the required 256-entry palette")]
    PaletteOffset(u32),
    #[error("invalid BMP dimensions {width}x{height}")]
    Dimensions { width: i32, height: i32 },
    #[error("indexed bitmap dimensions {width}x{height} cannot be encoded as BMP")]
    EncodeDimensions { width: u32, height: u32 },
    #[error(
        "indexed bitmap has {found} bytes, but dimensions {width}x{height} require {expected}"
    )]
    IndexCount {
        width: u32,
        height: u32,
        expected: usize,
        found: usize,
    },
    #[error("encoded BMP exceeds the 32-bit file-size limit")]
    EncodeTooLarge,
}

/// An 8-bit indexed bitmap: palette-index bytes in top-down row-major
/// order, exactly as the C++ engine sees its landscape surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedBitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height` palette indices, row 0 first (top-down).
    pub indices: Vec<u8>,
}

impl IndexedBitmap {
    /// Decode a bottom-up 8-bit BMP with the full 256-entry palette layout
    /// assumed by `CBitmap256Info`. Like `CSurface8::Read`, this interprets
    /// the payload as raw rows regardless of the signature and compression
    /// tag. Rows are exposed top-down; scanline padding is skipped.
    pub fn decode(bytes: &[u8]) -> Result<Self, BitmapError> {
        if bytes.len() < 54 {
            return Err(BitmapError::Truncated("header"));
        }
        let raw_data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        let data_offset = raw_data_offset as usize;
        let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        let bit_count = u16::from_le_bytes([bytes[28], bytes[29]]);

        if bit_count != 8 {
            return Err(BitmapError::BitDepth(bit_count));
        }
        if data_offset < BMP_DATA_OFFSET {
            return Err(BitmapError::PaletteOffset(raw_data_offset));
        }
        if width <= 0 || raw_height <= 0 {
            return Err(BitmapError::Dimensions {
                width,
                height: raw_height,
            });
        }
        let width = width as u32;
        let height = raw_height as u32;
        let row_stride = ((width as usize) + 3) & !3;
        let needed = data_offset + row_stride * height as usize;
        if bytes.len() < needed {
            return Err(BitmapError::Truncated("pixel data"));
        }

        let mut indices = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height as usize {
            let source_row = height as usize - 1 - row;
            let start = data_offset + source_row * row_stride;
            indices.extend_from_slice(&bytes[start..start + width as usize]);
        }
        Ok(Self {
            width,
            height,
            indices,
        })
    }

    /// Decode the index plane together with the source BMP palette. C++'s
    /// `CSurface8` retains these colors even though gameplay addresses only
    /// indices; later `Mat2Pal` overwrites mapped material slots and leaves
    /// every other entry byte-identical.
    pub fn decode_with_palette(bytes: &[u8]) -> Result<(Self, RgbPalette), BitmapError> {
        let bitmap = Self::decode(bytes)?;
        // CSurface8 reads one fixed CBitmap256Info: biSize is ignored and the
        // 256 RGBQUADs always occupy physical bytes 54..1078.
        let palette_start = BMP_FILE_HEADER_SIZE + BMP_INFO_HEADER_SIZE;
        let mut palette = [[0_u8; 3]; BMP_PALETTE_ENTRY_COUNT];
        for (slot, color) in palette.iter_mut().enumerate() {
            let offset = palette_start + slot * BMP_PALETTE_ENTRY_SIZE;
            let entry = &bytes[offset..offset + BMP_PALETTE_ENTRY_SIZE];
            *color = [entry[2], entry[1], entry[0]];
        }
        Ok((bitmap, palette))
    }

    /// Encode the exact palette-index plane as an uncompressed, bottom-up
    /// 8-bit BMP. DiffLandscape.bmp consumes only these indices; its palette
    /// entries are therefore left zeroed.
    pub fn encode(&self) -> Result<Vec<u8>, BitmapError> {
        self.encode_with_palette(&[[0; 3]; BMP_PALETTE_ENTRY_COUNT])
    }

    /// Encode the indexed plane as the bottom-up, uncompressed 8-bit BMP
    /// written by C++ `CSurface8::Save`. Palette colors are supplied as RGB
    /// and stored in BMP's physical BGRA order; scanlines are padded to four
    /// bytes. C++ records the unpadded `width * height` in `biSizeImage`, so
    /// this intentionally does the same even when the file contains padding.
    pub fn encode_with_palette(&self, palette: &RgbPalette) -> Result<Vec<u8>, BitmapError> {
        let width = i32::try_from(self.width).map_err(|_| BitmapError::EncodeDimensions {
            width: self.width,
            height: self.height,
        })?;
        let height = i32::try_from(self.height).map_err(|_| BitmapError::EncodeDimensions {
            width: self.width,
            height: self.height,
        })?;
        if width <= 0 || height <= 0 {
            return Err(BitmapError::EncodeDimensions {
                width: self.width,
                height: self.height,
            });
        }

        let width_usize = self.width as usize;
        let height_usize = self.height as usize;
        let expected = width_usize
            .checked_mul(height_usize)
            .ok_or(BitmapError::EncodeTooLarge)?;
        if self.indices.len() != expected {
            return Err(BitmapError::IndexCount {
                width: self.width,
                height: self.height,
                expected,
                found: self.indices.len(),
            });
        }

        let row_stride = width_usize
            .checked_add(3)
            .map(|width| width & !3)
            .ok_or(BitmapError::EncodeTooLarge)?;
        let pixel_data_size = row_stride
            .checked_mul(height_usize)
            .ok_or(BitmapError::EncodeTooLarge)?;
        let file_size = BMP_DATA_OFFSET
            .checked_add(pixel_data_size)
            .ok_or(BitmapError::EncodeTooLarge)?;
        let file_size = u32::try_from(file_size).map_err(|_| BitmapError::EncodeTooLarge)?;
        let unpadded_image_size =
            u32::try_from(expected).map_err(|_| BitmapError::EncodeTooLarge)?;

        let mut bytes = Vec::with_capacity(file_size as usize);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&file_size.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(BMP_DATA_OFFSET as u32).to_le_bytes());

        bytes.extend_from_slice(&(BMP_INFO_HEADER_SIZE as u32).to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&unpadded_image_size.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&(BMP_PALETTE_ENTRY_COUNT as u32).to_le_bytes());
        bytes.extend_from_slice(&(BMP_PALETTE_ENTRY_COUNT as u32).to_le_bytes());

        for &[red, green, blue] in palette {
            bytes.extend_from_slice(&[blue, green, red, 0]);
        }

        let padding = row_stride - width_usize;
        for row in (0..height_usize).rev() {
            let start = row * width_usize;
            bytes.extend_from_slice(&self.indices[start..start + width_usize]);
            bytes.resize(bytes.len() + padding, 0);
        }
        debug_assert_eq!(bytes.len(), file_size as usize);
        Ok(bytes)
    }

    pub fn index_at(&self, x: u32, y: u32) -> Option<u8> {
        (x < self.width && y < self.height)
            .then(|| self.indices[(y * self.width + x) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn i32_at(bytes: &[u8], offset: usize) -> i32 {
        i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    /// A minimal uncompressed 8-bit BMP with the given top-down rows.
    fn encode_bmp(rows: &[&[u8]], bottom_up: bool) -> Vec<u8> {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let stride = ((width as usize) + 3) & !3;
        let palette_len = 256 * 4;
        let data_offset = 14 + 40 + palette_len;
        let file_size = data_offset + stride * height as usize;

        let mut bytes = Vec::with_capacity(file_size);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data_offset as u32).to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        let raw_height = if bottom_up {
            height as i32
        } else {
            -(height as i32)
        };
        bytes.extend_from_slice(&raw_height.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&256u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.resize(14 + 40 + palette_len, 0);
        let ordered: Vec<&[u8]> = if bottom_up {
            rows.iter().rev().copied().collect()
        } else {
            rows.to_vec()
        };
        for row in ordered {
            bytes.extend_from_slice(row);
            bytes.resize(bytes.len() + (stride - row.len()), 0);
        }
        bytes
    }

    #[test]
    fn decodes_bottom_up_indices_top_down() {
        // The standard positive-height BMP stores rows bottom-up; the
        // engine consumes top-down palette indices (C4Landscape reads the
        // map surface index bytes, not colors).
        let bytes = encode_bmp(&[&[1, 2, 3], &[4, 5, 6]], true);
        let bitmap = IndexedBitmap::decode(&bytes).expect("decodes");
        assert_eq!(bitmap.width, 3);
        assert_eq!(bitmap.height, 2);
        assert_eq!(bitmap.indices, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(bitmap.index_at(2, 1), Some(6));
        assert_eq!(bitmap.index_at(3, 0), None);
    }

    #[test]
    fn rejects_top_down_negative_height_like_cpp_surface8() {
        let bytes = encode_bmp(&[&[9, 8], &[7, 6], &[5, 4]], false);
        assert!(matches!(
            IndexedBitmap::decode(&bytes),
            Err(BitmapError::Dimensions {
                width: 2,
                height: -3
            })
        ));
    }

    #[test]
    fn rejects_pixel_offset_before_the_full_256_entry_palette() {
        let standard = encode_bmp(&[&[1, 2], &[3, 4]], true);
        let compact_offset = 14 + 40 + 16 * 4;
        let mut compact = standard[..compact_offset].to_vec();
        compact[10..14].copy_from_slice(&(compact_offset as u32).to_le_bytes());
        compact.extend_from_slice(&standard[BMP_DATA_OFFSET..]);

        assert!(matches!(
            IndexedBitmap::decode(&compact),
            Err(BitmapError::PaletteOffset(118))
        ));
    }

    #[test]
    fn honors_pixel_offset_after_the_full_256_entry_palette() {
        let standard = encode_bmp(&[&[1, 2], &[3, 4]], true);
        let data_offset = BMP_DATA_OFFSET + 5;
        let mut with_gap = standard[..BMP_DATA_OFFSET].to_vec();
        with_gap.extend_from_slice(&[9; 5]);
        with_gap.extend_from_slice(&standard[BMP_DATA_OFFSET..]);
        with_gap[10..14].copy_from_slice(&(data_offset as u32).to_le_bytes());

        let bitmap = IndexedBitmap::decode(&with_gap).expect("gapped bitmap decodes");
        assert_eq!(bitmap.indices, vec![1, 2, 3, 4]);
    }

    #[test]
    fn indexed_bitmap_ignores_signature_and_compression_like_csurface8() {
        let baseline = encode_bmp(&[&[1, 2, 3], &[4, 5, 6]], true);
        let expected = IndexedBitmap {
            width: 3,
            height: 2,
            indices: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(
            IndexedBitmap::decode(&baseline).expect("baseline decodes"),
            expected
        );

        let mut missing_signature = baseline.clone();
        missing_signature[0..2].copy_from_slice(b"ZZ");
        assert_eq!(
            IndexedBitmap::decode(&missing_signature).expect("signature is ignored"),
            expected
        );

        let mut compressed = baseline;
        compressed[30..34].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            IndexedBitmap::decode(&compressed).expect("compression tag is ignored"),
            expected
        );
    }

    #[test]
    fn palette_decode_ignores_dib_size_like_cpp_surface8() {
        let bitmap = IndexedBitmap {
            width: 1,
            height: 1,
            indices: vec![7],
        };
        let mut palette = [[0; 3]; BMP_PALETTE_ENTRY_COUNT];
        palette[0] = [1, 2, 3];
        palette[1] = [4, 5, 6];
        palette[255] = [7, 8, 9];
        let mut bytes = bitmap
            .encode_with_palette(&palette)
            .expect("bitmap encodes");
        bytes[14..18].copy_from_slice(&44_u32.to_le_bytes());

        let (decoded, decoded_palette) =
            IndexedBitmap::decode_with_palette(&bytes).expect("fixed palette decodes");
        assert_eq!(decoded, bitmap);
        assert_eq!(decoded_palette, palette);
    }

    #[test]
    fn encoded_index_plane_round_trips_with_row_padding() {
        let bitmap = IndexedBitmap {
            width: 3,
            height: 2,
            indices: vec![0xff, 2, 3, 4, 0, 6],
        };
        let encoded = bitmap.encode().expect("encodes");
        assert_eq!(IndexedBitmap::decode(&encoded).expect("decodes"), bitmap);
    }

    #[test]
    fn rejects_non_8bit_bitmaps() {
        let mut bytes = encode_bmp(&[&[1, 2, 3], &[4, 5, 6]], true);
        bytes[28] = 24; // bit count
        assert!(matches!(
            IndexedBitmap::decode(&bytes),
            Err(BitmapError::BitDepth(24))
        ));
    }

    #[test]
    fn encodes_cpp_surface8_layout_palette_and_bottom_up_rows() {
        let bitmap = IndexedBitmap {
            width: 3,
            height: 2,
            indices: vec![1, 2, 3, 4, 5, 6],
        };
        let mut palette = [[0; 3]; 256];
        palette[0] = [192, 196, 252];
        palette[1] = [10, 20, 30];
        palette[255] = [40, 50, 60];

        let bytes = bitmap
            .encode_with_palette(&palette)
            .expect("bitmap encodes");
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(u32_at(&bytes, 2), 1_086);
        assert_eq!(&bytes[6..10], &[0; 4]);
        assert_eq!(u32_at(&bytes, 10), 1_078);
        assert_eq!(u32_at(&bytes, 14), 40);
        assert_eq!(i32_at(&bytes, 18), 3);
        assert_eq!(i32_at(&bytes, 22), 2, "positive height is bottom-up");
        assert_eq!(u16_at(&bytes, 26), 1);
        assert_eq!(u16_at(&bytes, 28), 8);
        assert_eq!(u32_at(&bytes, 30), 0);
        assert_eq!(u32_at(&bytes, 34), 6, "C++ stores the unpadded size");
        assert_eq!((u32_at(&bytes, 38), u32_at(&bytes, 42)), (0, 0));
        assert_eq!((u32_at(&bytes, 46), u32_at(&bytes, 50)), (256, 256));

        assert_eq!(&bytes[54..58], &[252, 196, 192, 0]);
        assert_eq!(&bytes[58..62], &[30, 20, 10, 0]);
        assert_eq!(&bytes[1_074..1_078], &[60, 50, 40, 0]);
        assert_eq!(
            &bytes[1_078..],
            &[4, 5, 6, 0, 1, 2, 3, 0],
            "rows are bottom-up with zero padding"
        );
        let (decoded, decoded_palette) =
            IndexedBitmap::decode_with_palette(&bytes).expect("palette decodes");
        assert_eq!(decoded, bitmap);
        assert_eq!(decoded_palette, palette);
    }

    #[test]
    fn encoded_bitmap_round_trips_indices_with_row_padding() {
        let bitmap = IndexedBitmap {
            width: 5,
            height: 3,
            indices: (0..15).collect(),
        };
        let encoded = bitmap.encode().expect("bitmap encodes");
        let decoded = IndexedBitmap::decode(&encoded).expect("encoded bitmap decodes");
        assert_eq!(decoded, bitmap);
    }

    #[test]
    fn encode_rejects_mismatched_index_plane() {
        let bitmap = IndexedBitmap {
            width: 2,
            height: 2,
            indices: vec![1, 2, 3],
        };
        assert!(matches!(
            bitmap.encode(),
            Err(BitmapError::IndexCount {
                width: 2,
                height: 2,
                expected: 4,
                found: 3
            })
        ));
    }

    #[test]
    fn encode_rejects_zero_and_out_of_range_dimensions() {
        let zero = IndexedBitmap {
            width: 0,
            height: 1,
            indices: Vec::new(),
        };
        assert!(matches!(
            zero.encode(),
            Err(BitmapError::EncodeDimensions {
                width: 0,
                height: 1
            })
        ));

        let out_of_range = IndexedBitmap {
            width: i32::MAX as u32 + 1,
            height: 1,
            indices: Vec::new(),
        };
        assert!(matches!(
            out_of_range.encode(),
            Err(BitmapError::EncodeDimensions { .. })
        ));
    }
}
