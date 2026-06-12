//! Raw 8-bit indexed BMP reading. The legacy engine loads `Landscape.bmp`
//! and `Map.bmp` as palette-INDEX surfaces (the index byte is the
//! material+texture key; the palette colors are presentation only), so the
//! port must read the indices verbatim — generic image decoding to RGBA
//! destroys them.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitmapError {
    #[error("bitmap data truncated ({0})")]
    Truncated(&'static str),
    #[error("not a BMP file (missing BM signature)")]
    Signature,
    #[error("unsupported BMP bit depth {0} (only 8-bit indexed is read)")]
    BitDepth(u16),
    #[error("unsupported BMP compression {0} (only uncompressed is read)")]
    Compression(u32),
    #[error("invalid BMP dimensions {width}x{height}")]
    Dimensions { width: i32, height: i32 },
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
    /// Decode an uncompressed 8-bit BMP. Bottom-up rows (positive height)
    /// are flipped to top-down; top-down files (negative height) are read
    /// as-is. Row padding to 4-byte boundaries is skipped.
    pub fn decode(bytes: &[u8]) -> Result<Self, BitmapError> {
        if bytes.len() < 54 {
            return Err(BitmapError::Truncated("header"));
        }
        if &bytes[0..2] != b"BM" {
            return Err(BitmapError::Signature);
        }
        let data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
        let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        let bit_count = u16::from_le_bytes([bytes[28], bytes[29]]);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);

        if bit_count != 8 {
            return Err(BitmapError::BitDepth(bit_count));
        }
        if compression != 0 {
            return Err(BitmapError::Compression(compression));
        }
        let bottom_up = raw_height >= 0;
        let height = raw_height.unsigned_abs();
        if width <= 0 || height == 0 {
            return Err(BitmapError::Dimensions {
                width,
                height: raw_height,
            });
        }
        let width = width as u32;
        let row_stride = ((width as usize) + 3) & !3;
        let needed = data_offset + row_stride * height as usize;
        if bytes.len() < needed {
            return Err(BitmapError::Truncated("pixel data"));
        }

        let mut indices = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height as usize {
            let source_row = if bottom_up {
                height as usize - 1 - row
            } else {
                row
            };
            let start = data_offset + source_row * row_stride;
            indices.extend_from_slice(&bytes[start..start + width as usize]);
        }
        Ok(Self {
            width,
            height,
            indices,
        })
    }

    pub fn index_at(&self, x: u32, y: u32) -> Option<u8> {
        (x < self.width && y < self.height)
            .then(|| self.indices[(y * self.width + x) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn decodes_top_down_indices_as_is() {
        let bytes = encode_bmp(&[&[9, 8], &[7, 6], &[5, 4]], false);
        let bitmap = IndexedBitmap::decode(&bytes).expect("decodes");
        assert_eq!(bitmap.indices, vec![9, 8, 7, 6, 5, 4]);
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
}
