use crate::error::{ProtocolError, ProtocolResult};

pub const DEFAULT_PACKET_COMPRESSION_LEVEL: u32 = 4;

pub const MAX_PACKET_COMPRESSION_LEVEL: u32 = 9;

fn invalid_level(level: u32) -> ProtocolError {
    ProtocolError::invalid(format!(
        "compression level {level} is outside 0..={MAX_PACKET_COMPRESSION_LEVEL}"
    ))
}

fn checked_level(level: u32) -> ProtocolResult<u32> {
    if level <= MAX_PACKET_COMPRESSION_LEVEL {
        Ok(level)
    } else {
        Err(invalid_level(level))
    }
}

pub trait ZlibCompressor {
    fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()>;
}

pub trait ZlibDecompressor {
    fn decompress(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        expected_size: usize,
    ) -> ProtocolResult<()>;
}

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
pub struct Flate2Compressor {
    level: flate2::Compression,
}

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
impl Flate2Compressor {
    pub fn new(level: u32) -> ProtocolResult<Self> {
        Ok(Self {
            level: flate2::Compression::new(checked_level(level)?),
        })
    }

    pub(crate) const fn with_default_level() -> Self {
        Self {
            level: flate2::Compression::new(DEFAULT_PACKET_COMPRESSION_LEVEL),
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
pub struct Flate2Decompressor;

#[cfg_attr(feature = "libdeflater", allow(dead_code))]
impl Flate2Decompressor {
    pub const fn new() -> Self {
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
        decoder
            .read_exact(output)
            .map_err(|_| ProtocolError::invalid("failed to decompress packet data"))?;
        let mut extra = [0u8; 1];
        let extra_read = decoder
            .read(&mut extra)
            .map_err(|_| ProtocolError::invalid("corrupt zlib stream trailer"))?;
        if extra_read > 0 {
            return Err(ProtocolError::invalid(
                "decompressed data larger than expected size",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "libdeflater")]
pub struct LibdeflateCompressor {
    compressor: libdeflater::Compressor,
}

#[cfg(feature = "libdeflater")]
const DEFAULT_LIBDEFLATE_LEVEL: libdeflater::CompressionLvl =
    match libdeflater::CompressionLvl::new(DEFAULT_PACKET_COMPRESSION_LEVEL as i32) {
        Ok(level) => level,
        Err(_) => panic!("the default packet compression level is not a libdeflate level"),
    };

#[cfg(feature = "libdeflater")]
impl LibdeflateCompressor {
    pub fn new(level: u32) -> ProtocolResult<Self> {
        let lvl = i32::try_from(checked_level(level)?)
            .ok()
            .and_then(|level| libdeflater::CompressionLvl::new(level).ok())
            .ok_or_else(|| invalid_level(level))?;
        Ok(Self {
            compressor: libdeflater::Compressor::new(lvl),
        })
    }

    pub(crate) fn with_default_level() -> Self {
        Self {
            compressor: libdeflater::Compressor::new(DEFAULT_LIBDEFLATE_LEVEL),
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
pub struct LibdeflateDecompressor {
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

pub fn new_compressor(level: u32) -> ProtocolResult<Box<dyn ZlibCompressor + Send + Sync>> {
    #[cfg(feature = "libdeflater")]
    {
        Ok(Box::new(LibdeflateCompressor::new(level)?))
    }
    #[cfg(not(feature = "libdeflater"))]
    {
        Ok(Box::new(Flate2Compressor::new(level)?))
    }
}

pub fn default_compressor() -> Box<dyn ZlibCompressor + Send + Sync> {
    #[cfg(feature = "libdeflater")]
    {
        Box::new(LibdeflateCompressor::with_default_level())
    }
    #[cfg(not(feature = "libdeflater"))]
    {
        Box::new(Flate2Compressor::with_default_level())
    }
}

pub fn new_decompressor() -> Box<dyn ZlibDecompressor + Send + Sync> {
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_compress_decompress_round_trip() {
        let mut compressor = new_compressor(4).unwrap();
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
        let mut compressor = new_compressor(4).unwrap();
        let mut decompressor = new_decompressor();

        let original: Vec<u8> = (0..65536).map(|i: u32| (i % 251) as u8).collect();
        let mut compressed = Vec::new();
        compressor.compress(&original, &mut compressed).unwrap();

        let mut decompressed = Vec::new();
        decompressor
            .decompress(&compressed, &mut decompressed, original.len())
            .unwrap();

        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_out_of_range_compression_level_is_rejected() {
        let err = new_compressor(MAX_PACKET_COMPRESSION_LEVEL + 1)
            .err()
            .expect("a level above the maximum must be rejected");
        assert!(matches!(err, ProtocolError::Invalid { .. }), "{err}");
        assert!(new_compressor(MAX_PACKET_COMPRESSION_LEVEL).is_ok());
        assert!(new_compressor(0).is_ok());
        assert!(new_compressor(DEFAULT_PACKET_COMPRESSION_LEVEL).is_ok());
    }

    #[test]
    fn test_decompress_corrupted_data() {
        let mut decompressor = new_decompressor();
        let corrupted = vec![0x78, 0x9C, 0xFF, 0xFF, 0xFF];
        let mut output = Vec::new();
        let result = decompressor.decompress(&corrupted, &mut output, 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_flate2_corrupt_trailer_rejected() {
        let mut compressor = Flate2Compressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL).unwrap();
        let original = b"trailer integrity check payload";
        let mut compressed = Vec::new();
        compressor.compress(original, &mut compressed).unwrap();

        let last = compressed.len() - 1;
        compressed[last] ^= 0xFF;

        let mut decompressor = Flate2Decompressor::new();
        let mut output = Vec::new();
        let result = decompressor.decompress(&compressed, &mut output, original.len());
        assert!(result.is_err(), "corrupt zlib trailer must be rejected");
    }

    #[test]
    fn test_decompress_empty_input() {
        let mut decompressor = new_decompressor();
        let mut output = Vec::new();
        let result = decompressor.decompress(&[], &mut output, 0);
        let _ = result;
    }

    #[cfg(feature = "libdeflater")]
    #[test]
    fn test_cross_backend_interop() {
        let original: Vec<u8> = (0..4096)
            .map(|i: u32| (i.wrapping_mul(31) % 251) as u8)
            .collect();

        let roundtrip = |mut comp: Box<dyn ZlibCompressor>,
                         mut decomp: Box<dyn ZlibDecompressor>| {
            let mut compressed = Vec::new();
            comp.compress(&original, &mut compressed).unwrap();
            let mut out = Vec::new();
            decomp
                .decompress(&compressed, &mut out, original.len())
                .unwrap();
            assert_eq!(out, original);
        };

        roundtrip(
            Box::new(Flate2Compressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL).unwrap()),
            Box::new(LibdeflateDecompressor::new()),
        );
        roundtrip(
            Box::new(LibdeflateCompressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL).unwrap()),
            Box::new(Flate2Decompressor::new()),
        );
        roundtrip(default_compressor(), Box::new(Flate2Decompressor::new()));
    }
}
