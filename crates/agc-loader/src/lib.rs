#![forbid(unsafe_code)]
//! Strict rope-image decoding, parity validation, and memory loading.

use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory, MemoryError};
use agc_word::AgcWord;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Supported 16-bit rope serialization conventions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RopeFormat {
    /// yaYUL/yaAGC order: banks 2,3,0,1,4..43; data in bits 15..1.
    Yayul,
    /// yaYUL `--parity`: same bank/data layout with odd parity in bit 0.
    YayulParity,
    /// Hardware bank order and hardware parity layout.
    Hardware,
    /// Physical bank order 0..43 with a right-aligned 15-bit word.
    PhysicalWords,
}

/// Loaded rope with parity evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RopeImage {
    /// Words in physical fixed-bank order.
    pub words: Vec<AgcWord>,
    /// Input format.
    pub format: RopeFormat,
    /// Number of words whose supplied parity was checked.
    pub parity_words: usize,
}

impl RopeImage {
    /// Installs this rope in a fresh memory map.
    pub fn into_memory(self) -> Result<Memory, MemoryError> {
        Memory::with_rope(self.words)
    }
}

/// Rope loading failure.
#[derive(Debug, Error)]
pub enum LoaderError {
    /// File I/O failure.
    #[error("rope I/O error at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Byte length is not exactly one Apollo 11 rope.
    #[error("rope has {actual} bytes; expected {expected}")]
    Size {
        /// Expected size.
        expected: usize,
        /// Actual size.
        actual: usize,
    },
    /// A non-parity format contains a set reserved bit.
    #[error("reserved bit is set in word {index}: {raw:#06x}")]
    ReservedBit {
        /// File-order word index.
        index: usize,
        /// Raw 16-bit input.
        raw: u16,
    },
    /// Supplied parity is not odd.
    #[error("odd parity failure in word {index}: {raw:#06x}")]
    Parity {
        /// File-order word index.
        index: usize,
        /// Raw 16-bit input.
        raw: u16,
    },
}

/// Loads a rope from disk.
pub fn load_file(path: impl AsRef<Path>, format: RopeFormat) -> Result<RopeImage, LoaderError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| LoaderError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_bytes(&bytes, format)
}

/// Decodes a complete in-memory rope image.
pub fn decode_bytes(bytes: &[u8], format: RopeFormat) -> Result<RopeImage, LoaderError> {
    let words = FIXED_BANKS * FIXED_WORDS_PER_BANK;
    let expected = words * 2;
    if bytes.len() != expected {
        return Err(LoaderError::Size {
            expected,
            actual: bytes.len(),
        });
    }
    let mut physical = vec![AgcWord::POSITIVE_ZERO; words];
    let mut parity_words = 0;
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let raw = u16::from_be_bytes([pair[0], pair[1]]);
        let data = match format {
            RopeFormat::Yayul => {
                if raw & 1 != 0 {
                    return Err(LoaderError::ReservedBit { index, raw });
                }
                raw >> 1
            }
            RopeFormat::YayulParity => {
                if raw.count_ones() % 2 != 1 {
                    return Err(LoaderError::Parity { index, raw });
                }
                parity_words += 1;
                raw >> 1
            }
            RopeFormat::Hardware => {
                if raw.count_ones() % 2 != 1 {
                    return Err(LoaderError::Parity { index, raw });
                }
                parity_words += 1;
                // Hardware parity occupies bit 14; data bit 14 remains bit 15.
                ((raw & 0x8000) >> 1) | (raw & 0x3fff)
            }
            RopeFormat::PhysicalWords => {
                if raw & 0x8000 != 0 {
                    return Err(LoaderError::ReservedBit { index, raw });
                }
                raw
            }
        };
        let file_bank = index / FIXED_WORDS_PER_BANK;
        let bank = match format {
            RopeFormat::Yayul | RopeFormat::YayulParity if file_bank < 4 => file_bank ^ 2,
            _ => file_bank,
        };
        let offset = index % FIXED_WORDS_PER_BANK;
        physical[bank * FIXED_WORDS_PER_BANK + offset] = AgcWord::from_raw_truncate(data);
    }
    Ok(RopeImage {
        words: physical,
        format,
        parity_words,
    })
}

/// Encodes physical words in standard yaYUL bank order without parity.
pub fn encode_yayul(words: &[AgcWord]) -> Result<Vec<u8>, LoaderError> {
    let expected_words = FIXED_BANKS * FIXED_WORDS_PER_BANK;
    if words.len() != expected_words {
        return Err(LoaderError::Size {
            expected: expected_words * 2,
            actual: words.len() * 2,
        });
    }
    let mut bytes = Vec::with_capacity(expected_words * 2);
    for file_bank in 0..FIXED_BANKS {
        let bank = if file_bank < 4 {
            file_bank ^ 2
        } else {
            file_bank
        };
        for word in &words[bank * FIXED_WORDS_PER_BANK..(bank + 1) * FIXED_WORDS_PER_BANK] {
            bytes.extend_from_slice(&(word.raw() << 1).to_be_bytes());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yayul_bank_flip_round_trips() {
        let mut words = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        for bank in 0..FIXED_BANKS {
            words[bank * FIXED_WORDS_PER_BANK] = AgcWord::from_raw_truncate(bank as u16);
        }
        let bytes = encode_yayul(&words).unwrap();
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]) >> 1, 2);
        assert_eq!(
            decode_bytes(&bytes, RopeFormat::Yayul).unwrap().words,
            words
        );
    }

    #[test]
    fn parity_corruption_is_rejected() {
        let mut bytes = vec![0_u8; FIXED_BANKS * FIXED_WORDS_PER_BANK * 2];
        // Zero has even parity and is therefore invalid in parity mode.
        assert!(matches!(
            decode_bytes(&bytes, RopeFormat::YayulParity),
            Err(LoaderError::Parity { .. })
        ));
        bytes[1] = 1;
        assert!(matches!(
            decode_bytes(&bytes, RopeFormat::YayulParity),
            Err(LoaderError::Parity { index: 1, .. })
        ));
    }
}
