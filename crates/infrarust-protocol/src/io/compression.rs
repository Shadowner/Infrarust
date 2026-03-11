//! Zlib compression abstraction with compile-time backend selection.
//!
//! By default, uses `flate2` (pure Rust via miniz_oxide). With the `libdeflater`
//! feature flag, switches to `libdeflate` for 2-3x better performance.

use crate::error::{ProtocolError, ProtocolResult};

/// Compresses data in zlib format.
pub(crate) trait ZlibCompressor {
    /// Compresses `input` into `output` (zlib format).
    ///
    /// `output` is cleared then filled with compressed data.
    fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()>;
}

/// Decompresses data in zlib format.
pub(crate) trait ZlibDecompressor {
    /// Decompresses `input` (zlib) into `output`.
    ///
    /// `expected_size` is the known decompressed size (from the protocol's
    /// data_len VarInt). `output` is cleared then filled.
    fn decompress(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        expected_size: usize,
    ) -> ProtocolResult<()>;
}

// --- flate2 backend (always available) ---

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
pub(crate) struct Flate2Compressor {
    level: flate2::Compression,
}

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
impl Flate2Compressor {
    pub fn new(level: u32) -> Self {
        Self {
            level: flate2::Compression::new(level),
        }
    }
}

impl ZlibCompressor for Flate2Compressor {
    fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()> {
        use std::io::Write;

        output.clear();
        let mut encoder = flate2::write::ZlibEncoder::new(output, self.level);
        encoder.write_all(input)?;
        encoder.finish()?;
        Ok(())
    }
}

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
pub(crate) struct Flate2Decompressor;

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
impl Flate2Decompressor {
    pub fn new() -> Self {
        Self
    }
}

impl ZlibDecompressor for Flate2Decompressor {
    fn decompress(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        expected_size: usize,
    ) -> ProtocolResult<()> {
        use std::io::Read;

        output.clear();
        output.resize(expected_size, 0);
        let mut decoder = flate2::read::ZlibDecoder::new(input);
        decoder.read_exact(output).map_err(|_| {
            ProtocolError::invalid("failed to decompress packet data")
        })?;
        Ok(())
    }
}

// --- libdeflater backend (behind feature flag) ---

#[cfg(feature = "libdeflater")]
pub(crate) struct LibdeflateCompressor {
    compressor: libdeflater::Compressor,
}

#[cfg(feature = "libdeflater")]
impl LibdeflateCompressor {
    pub fn new(level: u32) -> Self {
        let lvl = libdeflater::CompressionLvl::new(level as i32)
            .unwrap_or_default();
        Self {
            compressor: libdeflater::Compressor::new(lvl),
        }
    }
}

#[cfg(feature = "libdeflater")]
impl ZlibCompressor for LibdeflateCompressor {
    fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()> {
        output.clear();
        let max_size = self.compressor.zlib_compress_bound(input.len());
        output.resize(max_size, 0);
        let actual_size = self
            .compressor
            .zlib_compress(input, output)
            .map_err(|e| ProtocolError::invalid(format!("libdeflate compress error: {e}")))?;
        output.truncate(actual_size);
        Ok(())
    }
}

#[cfg(feature = "libdeflater")]
pub(crate) struct LibdeflateDecompressor {
    decompressor: libdeflater::Decompressor,
}

#[cfg(feature = "libdeflater")]
impl LibdeflateDecompressor {
    pub fn new() -> Self {
        Self {
            decompressor: libdeflater::Decompressor::new(),
        }
    }
}

#[cfg(feature = "libdeflater")]
impl ZlibDecompressor for LibdeflateDecompressor {
    fn decompress(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        expected_size: usize,
    ) -> ProtocolResult<()> {
        output.clear();
        output.resize(expected_size, 0);
        let actual_size = self
            .decompressor
            .zlib_decompress(input, output)
            .map_err(|e| ProtocolError::invalid(format!("libdeflate decompress error: {e}")))?;
        if actual_size != expected_size {
            return Err(ProtocolError::invalid(format!(
                "decompressed size mismatch: expected {expected_size}, got {actual_size}"
            )));
        }
        Ok(())
    }
}

// --- Factory functions ---

/// Creates the default compressor based on enabled features.
pub(crate) fn new_compressor(level: u32) -> Box<dyn ZlibCompressor + Send + Sync> {
    #[cfg(feature = "libdeflater")]
    {
        Box::new(LibdeflateCompressor::new(level))
    }
    #[cfg(not(feature = "libdeflater"))]
    {
        Box::new(Flate2Compressor::new(level))
    }
}

/// Creates the default decompressor based on enabled features.
pub(crate) fn new_decompressor() -> Box<dyn ZlibDecompressor + Send + Sync> {
    #[cfg(feature = "libdeflater")]
    {
        Box::new(LibdeflateDecompressor::new())
    }
    #[cfg(not(feature = "libdeflater"))]
    {
        Box::new(Flate2Decompressor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_round_trip() {
        let mut compressor = new_compressor(4);
        let mut decompressor = new_decompressor();

        let original = b"Hello, Minecraft protocol compression!";
        let mut compressed = Vec::new();
        compressor.compress(original, &mut compressed).unwrap();

        assert_ne!(&compressed[..], &original[..]);

        let mut decompressed = Vec::new();
        decompressor
            .decompress(&compressed, &mut decompressed, original.len())
            .unwrap();

        assert_eq!(&decompressed[..], &original[..]);
    }

    #[test]
    fn test_compress_decompress_large_data() {
        let mut compressor = new_compressor(4);
        let mut decompressor = new_decompressor();

        // 64 KB of patterned data
        let original: Vec<u8> = (0..65536).map(|i| (i % 251) as u8).collect();
        let mut compressed = Vec::new();
        compressor.compress(&original, &mut compressed).unwrap();

        let mut decompressed = Vec::new();
        decompressor
            .decompress(&compressed, &mut decompressed, original.len())
            .unwrap();

        assert_eq!(decompressed, original);
    }
}
