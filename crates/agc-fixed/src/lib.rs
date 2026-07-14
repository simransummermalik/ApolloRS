#![forbid(unsafe_code)]
//! Scale-aware fixed-point values backed by raw-exact AGC words.

use agc_word::{AgcWord, WordError};
use core::fmt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed-point conversion and arithmetic errors.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FixedError {
    /// The requested fractional precision does not fit an AGC magnitude.
    #[error("fractional precision {0} exceeds the 14 available magnitude bits")]
    InvalidScale(u8),
    /// The scaled integer is not representable by one AGC word.
    #[error(transparent)]
    Word(#[from] WordError),
}

/// One AGC word interpreted with `FRACTION_BITS` binary fractional bits.
///
/// The type retains both zero encodings. Floating point is deliberately kept
/// at the presentation boundary; arithmetic is performed on the raw word.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgcFixed<const FRACTION_BITS: u8>(AgcWord);

impl<const FRACTION_BITS: u8> AgcFixed<FRACTION_BITS> {
    /// Constructs a fixed-point value after validating its compile-time scale.
    pub const fn from_word(word: AgcWord) -> Result<Self, FixedError> {
        if FRACTION_BITS <= 14 {
            Ok(Self(word))
        } else {
            Err(FixedError::InvalidScale(FRACTION_BITS))
        }
    }

    /// Constructs from the signed integer numerator of `value / 2^FRACTION_BITS`.
    pub fn from_scaled_integer(value: i32) -> Result<Self, FixedError> {
        Self::from_word(AgcWord::from_i32(value)?)
    }

    /// Returns the authoritative encoded word.
    pub const fn word(self) -> AgcWord {
        self.0
    }

    /// Returns the signed scaled integer, merging the two zeros.
    pub const fn scaled_integer_lossy_zero(self) -> i32 {
        self.0.to_i32_lossy_zero()
    }

    /// Adds with one's-complement end-around carry.
    pub const fn wrapping_add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }

    /// Subtracts with one's-complement end-around carry.
    pub const fn wrapping_sub(self, rhs: Self) -> Self {
        Self(self.0.wrapping_sub(rhs.0))
    }

    /// Converts to a host floating-point value for display and analysis only.
    pub fn to_f64(self) -> f64 {
        f64::from(self.scaled_integer_lossy_zero()) / 2_f64.powi(i32::from(FRACTION_BITS))
    }
}

impl<const FRACTION_BITS: u8> fmt::Debug for AgcFixed<FRACTION_BITS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgcFixed")
            .field("word", &self.0)
            .field("fraction_bits", &FRACTION_BITS)
            .finish()
    }
}

/// The AGC's common single-precision signed fractional convention.
pub type Fraction = AgcFixed<14>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_type_checked_and_zero_is_preserved() {
        let minus_zero = Fraction::from_word(AgcWord::NEGATIVE_ZERO).unwrap();
        assert!(minus_zero.word().is_negative_zero());
        assert_eq!(minus_zero.to_f64(), 0.0);
        assert!(AgcFixed::<15>::from_word(AgcWord::POSITIVE_ZERO).is_err());
    }

    #[test]
    fn fixed_addition_uses_agc_arithmetic() {
        let quarter = Fraction::from_scaled_integer(0o10_000).unwrap();
        assert_eq!(quarter.wrapping_add(quarter).to_f64(), 0.5);
    }
}
