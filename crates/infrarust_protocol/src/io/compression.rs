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

#[cfg(feature = "libdeflater")]
pub type Compressor = LibdeflateCompressor;
#[cfg(feature = "libdeflater")]
pub type Decompressor = LibdeflateDecompressor;

#[cfg(not(feature = "libdeflater"))]
pub type Compressor = Flate2Compressor;
#[cfg(not(feature = "libdeflater"))]
pub type Decompressor = Flate2Decompressor;

#[cfg(any(test, not(feature = "libdeflater")))]
pub struct Flate2Compressor {
    level: flate2::Compression,
}

#[cfg(any(test, not(feature = "libdeflater")))]
impl Flate2Compressor {
    pub fn new(level: u32) -> ProtocolResult<Self> {
        Ok(Self {
            level: flate2::Compression::new(checked_level(level)?),
        })
    }

    #[cfg(not(feature = "libdeflater"))]
    pub(crate) const fn with_default_level() -> Self {
        Self {
            level: flate2::Compression::new(DEFAULT_PACKET_COMPRESSION_LEVEL),
        }
    }

    pub fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()> {
        use std::io::Write;

        output.clear();
        let mut encoder = flate2::write::ZlibEncoder::new(output, self.level);
        encoder.write_all(input)?;
        encoder.finish()?;
        Ok(())
    }
}

#[cfg(any(test, not(feature = "libdeflater")))]
fn inflate_failed() -> ProtocolError {
    ProtocolError::invalid("failed to decompress packet data")
}

#[cfg(any(test, not(feature = "libdeflater")))]
pub struct Flate2Decompressor {
    inflate: flate2::Decompress,
}

#[cfg(any(test, not(feature = "libdeflater")))]
impl Flate2Decompressor {
    pub fn new() -> Self {
        Self {
            inflate: flate2::Decompress::new(true),
        }
    }

    pub fn decompress(
        &mut self,
        input: &[u8],
        output: &mut Vec<u8>,
        expected_size: usize,
    ) -> ProtocolResult<()> {
        use flate2::{FlushDecompress, Status};

        output.clear();
        output.reserve_exact(expected_size);
        self.inflate.reset(true);
        let mut status = self
            .inflate
            .decompress_vec(input, output, FlushDecompress::Finish)
            .map_err(|_| inflate_failed())?;

        if status != Status::StreamEnd && output.len() == expected_size {
            let rest = usize::try_from(self.inflate.total_in())
                .ok()
                .and_then(|consumed| input.get(consumed..))
                .ok_or_else(inflate_failed)?;
            let mut overflow = [0u8; 1];
            status = self
                .inflate
                .decompress(rest, &mut overflow, FlushDecompress::Finish)
                .map_err(|_| inflate_failed())?;
        }

        if status != Status::StreamEnd {
            return Err(inflate_failed());
        }
        if self.inflate.total_out() != expected_size as u64 {
            return Err(ProtocolError::invalid(
                "decompressed data does not match the declared size",
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

    pub fn compress(&mut self, input: &[u8], output: &mut Vec<u8>) -> ProtocolResult<()> {
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

    pub fn decompress(
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_compress_decompress_round_trip() {
        let mut compressor = Compressor::new(4).unwrap();
        let mut decompressor = Decompressor::new();

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
        let mut compressor = Compressor::new(4).unwrap();
        let mut decompressor = Decompressor::new();

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
        let err = Compressor::new(MAX_PACKET_COMPRESSION_LEVEL + 1)
            .err()
            .expect("a level above the maximum must be rejected");
        assert!(matches!(err, ProtocolError::Invalid { .. }), "{err}");
        assert!(Compressor::new(MAX_PACKET_COMPRESSION_LEVEL).is_ok());
        assert!(Compressor::new(0).is_ok());
        assert!(Compressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL).is_ok());
    }

    #[test]
    fn test_decompress_corrupted_data() {
        let mut decompressor = Decompressor::new();
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
        let mut decompressor = Decompressor::new();
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

        let mut flate2_out = Vec::new();
        Flate2Compressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL)
            .unwrap()
            .compress(&original, &mut flate2_out)
            .unwrap();
        let mut out = Vec::new();
        LibdeflateDecompressor::new()
            .decompress(&flate2_out, &mut out, original.len())
            .unwrap();
        assert_eq!(out, original);

        for mut compressor in [
            LibdeflateCompressor::new(DEFAULT_PACKET_COMPRESSION_LEVEL).unwrap(),
            LibdeflateCompressor::with_default_level(),
        ] {
            let mut libdeflate_out = Vec::new();
            compressor.compress(&original, &mut libdeflate_out).unwrap();
            let mut out = Vec::new();
            Flate2Decompressor::new()
                .decompress(&libdeflate_out, &mut out, original.len())
                .unwrap();
            assert_eq!(out, original);
        }
    }
}
