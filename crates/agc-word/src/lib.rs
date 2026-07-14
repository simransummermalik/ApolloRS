#![forbid(unsafe_code)]
//! Exact finite representations used by the Block II Apollo Guidance Computer.
//!
//! [`AgcWord`] retains both one's-complement zero encodings. Host integers are
//! conversion views, never the authoritative representation.

use core::fmt;
use core::str::FromStr;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Mask containing the fifteen data bits of an AGC word.
pub const WORD_MASK: u16 = 0o77_777;
/// Sign bit of a fifteen-bit AGC word.
pub const WORD_SIGN: u16 = 0o40_000;
/// Largest positive integer magnitude representable in one AGC word.
pub const MAX_MAGNITUDE: i32 = 0o37_777;

/// Errors produced by checked AGC representation conversions.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WordError {
    /// A raw value uses bits outside the target representation.
    #[error("raw value {value:#o} exceeds {bits}-bit AGC representation")]
    RawOutOfRange {
        /// Supplied raw value.
        value: u32,
        /// Width of the target representation.
        bits: u8,
    },
    /// A mathematical integer is outside the one's-complement range.
    #[error("integer {value} is outside AGC one's-complement range [{min}, {max}]")]
    IntegerOutOfRange {
        /// Supplied mathematical integer.
        value: i64,
        /// Minimum representable value.
        min: i64,
        /// Maximum representable value.
        max: i64,
    },
    /// An octal representation is invalid.
    #[error("invalid AGC octal word: {0}")]
    InvalidOctal(String),
}

/// Sign/zero class without discarding either zero encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum SignClass {
    /// Raw `00000`.
    PositiveZero,
    /// A positive non-zero value.
    Positive,
    /// Raw `77777`.
    NegativeZero,
    /// A negative non-zero value.
    Negative,
}

/// Overflow classification for signed one's-complement addition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Overflow {
    /// The mathematical result fits the word.
    None,
    /// Two positive operands produced a negative-sign raw result.
    Positive,
    /// Two negative operands produced a positive-sign raw result.
    Negative,
}

/// Result metadata from an AGC word addition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AddResult {
    /// Raw-exact result after end-around carry.
    pub value: AgcWord,
    /// Whether at least one end-around carry was applied.
    pub end_around_carry: bool,
    /// Signed overflow classification.
    pub overflow: Overflow,
}

/// A fifteen-bit one's-complement AGC data word.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgcWord(u16);

impl AgcWord {
    /// Positive zero.
    pub const POSITIVE_ZERO: Self = Self(0);
    /// Negative zero.
    pub const NEGATIVE_ZERO: Self = Self(WORD_MASK);
    /// Most positive value.
    pub const MAX: Self = Self(0o37_777);
    /// Most negative value.
    pub const MIN: Self = Self(0o40_000);

    /// Constructs a word after verifying that only fifteen bits are present.
    pub const fn try_from_raw(raw: u16) -> Result<Self, WordError> {
        if raw & !WORD_MASK == 0 {
            Ok(Self(raw))
        } else {
            Err(WordError::RawOutOfRange {
                value: raw as u32,
                bits: 15,
            })
        }
    }

    /// Constructs a word by retaining the low fifteen bits.
    pub const fn from_raw_truncate(raw: u16) -> Self {
        Self(raw & WORD_MASK)
    }

    /// Returns the complete fifteen-bit raw representation.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Returns the representation's sign and zero class.
    pub const fn sign_class(self) -> SignClass {
        match self.0 {
            0 => SignClass::PositiveZero,
            WORD_MASK => SignClass::NegativeZero,
            raw if raw & WORD_SIGN == 0 => SignClass::Positive,
            _ => SignClass::Negative,
        }
    }

    /// Returns true for either zero encoding.
    pub const fn is_zero(self) -> bool {
        self.0 == 0 || self.0 == WORD_MASK
    }

    /// Returns true only for raw positive zero.
    pub const fn is_positive_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns true only for raw negative zero.
    pub const fn is_negative_zero(self) -> bool {
        self.0 == WORD_MASK
    }

    /// Returns true when the sign bit is set, including negative zero.
    pub const fn is_negative(self) -> bool {
        self.0 & WORD_SIGN != 0
    }

    /// One's complement, preserving the zero distinction by swapping zeros.
    pub const fn complement(self) -> Self {
        Self((!self.0) & WORD_MASK)
    }

    /// Converts a representable host integer to one's complement.
    pub fn from_i32(value: i32) -> Result<Self, WordError> {
        if !(-MAX_MAGNITUDE..=MAX_MAGNITUDE).contains(&value) {
            return Err(WordError::IntegerOutOfRange {
                value: i64::from(value),
                min: i64::from(-MAX_MAGNITUDE),
                max: i64::from(MAX_MAGNITUDE),
            });
        }
        if value < 0 {
            Ok(Self::from_raw_truncate(!(value.unsigned_abs() as u16)))
        } else {
            Ok(Self(value as u16))
        }
    }

    /// Converts to a host integer, mapping both zero encodings to zero.
    pub const fn to_i32_lossy_zero(self) -> i32 {
        if self.is_negative() {
            -(((!self.0) & WORD_MASK) as i32)
        } else {
            self.0 as i32
        }
    }

    /// Adds with AGC end-around carry and reports signed overflow.
    pub const fn overflowing_add(self, rhs: Self) -> AddResult {
        let sum = self.0 as u32 + rhs.0 as u32;
        let first_carry = sum >> 15;
        let folded = (sum & WORD_MASK as u32) + first_carry;
        let second_carry = folded >> 15;
        let raw = ((folded & WORD_MASK as u32) + second_carry) as u16;
        let value = Self(raw & WORD_MASK);
        let overflow = match (self.is_negative(), rhs.is_negative(), value.is_negative()) {
            (false, false, true) => Overflow::Positive,
            (true, true, false) => Overflow::Negative,
            _ => Overflow::None,
        };
        AddResult {
            value,
            end_around_carry: first_carry != 0 || second_carry != 0,
            overflow,
        }
    }

    /// Adds and returns only the raw-exact word result.
    pub const fn wrapping_add(self, rhs: Self) -> Self {
        self.overflowing_add(rhs).value
    }

    /// Subtracts by adding the one's complement of the right operand.
    pub const fn wrapping_sub(self, rhs: Self) -> Self {
        self.wrapping_add(rhs.complement())
    }

    /// Sign-extends this word to an AGC 16-bit central-register value.
    pub const fn sign_extend(self) -> AgcRegister {
        if self.is_negative() {
            AgcRegister(self.0 | 0o100_000)
        } else {
            AgcRegister(self.0)
        }
    }
}

impl fmt::Debug for AgcWord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AgcWord({:05o})", self.0)
    }
}

impl fmt::Display for AgcWord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:05o}", self.0)
    }
}

impl FromStr for AgcWord {
    type Err = WordError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let cleaned = input.trim().strip_prefix("0o").unwrap_or(input.trim());
        let raw = u16::from_str_radix(cleaned, 8)
            .map_err(|_| WordError::InvalidOctal(input.to_owned()))?;
        Self::try_from_raw(raw)
    }
}

/// A sixteen-bit one's-complement AGC central-register representation.
#[derive(Clone, Copy, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgcRegister(u16);

impl AgcRegister {
    /// Verifies and constructs a full register value.
    pub const fn try_from_raw(raw: u32) -> Result<Self, WordError> {
        if raw <= u16::MAX as u32 {
            Ok(Self(raw as u16))
        } else {
            Err(WordError::RawOutOfRange {
                value: raw,
                bits: 16,
            })
        }
    }

    /// Returns all sixteen raw bits.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Reduces an overflow-capable register to a fifteen-bit memory word.
    pub const fn overflow_correct(self) -> AgcWord {
        let corrected = match self.0 & 0o140_000 {
            0o100_000 => self.0 | 0o140_000,
            0o040_000 => self.0 & 0o037_777,
            _ => self.0,
        };
        AgcWord::from_raw_truncate(corrected)
    }

    /// Returns true when the two sign bits indicate overflow.
    pub const fn has_overflow(self) -> bool {
        matches!(self.0 & 0o140_000, 0o040_000 | 0o100_000)
    }

    /// Adds two register-width one's-complement values with end-around carry.
    pub const fn wrapping_add(self, rhs: Self) -> Self {
        let sum = self.0 as u32 + rhs.0 as u32;
        let folded = (sum & 0xffff) + (sum >> 16);
        Self(((folded & 0xffff) + (folded >> 16)) as u16)
    }
}

impl fmt::Debug for AgcRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AgcRegister({:06o})", self.0)
    }
}

/// Two AGC words containing a 28-bit one's-complement magnitude.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AgcDoubleWord {
    /// Most significant word, including sign.
    pub high: AgcWord,
    /// Least significant word, including the repeated sign bit.
    pub low: AgcWord,
}

impl AgcDoubleWord {
    /// Largest double-precision magnitude.
    pub const MAX_MAGNITUDE: i64 = (1_i64 << 28) - 1;
    /// Positive double-precision zero.
    pub const POSITIVE_ZERO: Self = Self {
        high: AgcWord::POSITIVE_ZERO,
        low: AgcWord::POSITIVE_ZERO,
    };
    /// Negative double-precision zero.
    pub const NEGATIVE_ZERO: Self = Self {
        high: AgcWord::NEGATIVE_ZERO,
        low: AgcWord::NEGATIVE_ZERO,
    };

    /// Constructs from a mathematical integer in the 28-bit magnitude range.
    pub fn from_i64(value: i64) -> Result<Self, WordError> {
        if !(-Self::MAX_MAGNITUDE..=Self::MAX_MAGNITUDE).contains(&value) {
            return Err(WordError::IntegerOutOfRange {
                value,
                min: -Self::MAX_MAGNITUDE,
                max: Self::MAX_MAGNITUDE,
            });
        }
        let negative = value < 0;
        let magnitude = value.unsigned_abs() as u32;
        let encoded = if negative {
            (!magnitude) & 0x0fff_ffff
        } else {
            magnitude
        };
        let sign = if negative { WORD_SIGN } else { 0 };
        Ok(Self {
            high: AgcWord::from_raw_truncate(sign | ((encoded >> 14) as u16 & 0o37_777)),
            low: AgcWord::from_raw_truncate(sign | (encoded as u16 & 0o37_777)),
        })
    }

    /// Converts to a host integer, mapping both double-zero encodings to zero.
    pub const fn to_i64_lossy_zero(self) -> i64 {
        let encoded =
            ((self.high.raw() as u32 & 0o37_777) << 14) | (self.low.raw() as u32 & 0o37_777);
        if self.high.is_negative() {
            -(((!encoded) & 0x0fff_ffff) as i64)
        } else {
            encoded as i64
        }
    }

    /// Returns true for either pair of zero words.
    pub const fn is_zero(self) -> bool {
        self.to_i64_lossy_zero() == 0
    }

    /// Returns true only for the all-ones double-zero representation.
    pub const fn is_negative_zero(self) -> bool {
        self.high.is_negative_zero() && self.low.is_negative_zero()
    }
}

macro_rules! bounded_type {
    ($name:ident, $raw:ty, $max:expr, $label:literal) => {
        #[doc = $label]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name($raw);

        impl $name {
            /// Constructs after checking the architectural range.
            pub fn new(value: $raw) -> Result<Self, WordError> {
                if value <= $max {
                    Ok(Self(value))
                } else {
                    Err(WordError::RawOutOfRange {
                        value: value as u32,
                        bits: ($max as u32).ilog2() as u8 + 1,
                    })
                }
            }

            /// Returns the raw identifier.
            pub const fn get(self) -> $raw {
                self.0
            }
        }
    };
}

bounded_type!(AgcAddress, u16, 0o7777, "A twelve-bit AGC logical address.");
bounded_type!(FixedBank, u8, 0o43, "A Block II fixed-memory bank number.");
bounded_type!(
    ErasableBank,
    u8,
    0o7,
    "A Block II erasable-memory bank number."
);
bounded_type!(
    ChannelAddress,
    u16,
    0o777,
    "A nine-bit AGC I/O channel address."
);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn every_raw_word_round_trips_and_complements_twice() {
        for raw in 0..=WORD_MASK {
            let word = AgcWord::try_from_raw(raw).expect("finite word domain");
            assert_eq!(word.raw(), raw);
            assert_eq!(word.complement().complement(), word);
            assert_eq!(word.to_string(), format!("{raw:05o}"));
        }
    }

    #[test]
    fn signed_zeros_remain_distinct() {
        assert_ne!(AgcWord::POSITIVE_ZERO, AgcWord::NEGATIVE_ZERO);
        assert!(AgcWord::POSITIVE_ZERO.is_zero());
        assert!(AgcWord::NEGATIVE_ZERO.is_zero());
        assert_eq!(AgcWord::POSITIVE_ZERO.complement(), AgcWord::NEGATIVE_ZERO);
        assert_eq!(AgcWord::NEGATIVE_ZERO.to_i32_lossy_zero(), 0);
    }

    #[test]
    fn end_around_carry_examples() {
        assert_eq!(
            AgcWord::try_from_raw(0o77_776)
                .unwrap()
                .wrapping_add(AgcWord::try_from_raw(0o77_776).unwrap()),
            AgcWord::try_from_raw(0o77_775).unwrap()
        );
        let cancellation = AgcWord::from_i32(1234)
            .unwrap()
            .wrapping_add(AgcWord::from_i32(-1234).unwrap());
        assert!(cancellation.is_negative_zero());
    }

    #[test]
    fn all_single_precision_integers_round_trip() {
        for value in -MAX_MAGNITUDE..=MAX_MAGNITUDE {
            let word = AgcWord::from_i32(value).unwrap();
            assert_eq!(word.to_i32_lossy_zero(), value);
        }
    }

    #[test]
    fn representative_double_precision_values_round_trip() {
        let values = [
            -AgcDoubleWord::MAX_MAGNITUDE,
            -16_384,
            -1,
            0,
            1,
            16_384,
            AgcDoubleWord::MAX_MAGNITUDE,
        ];
        for value in values {
            assert_eq!(
                AgcDoubleWord::from_i64(value).unwrap().to_i64_lossy_zero(),
                value
            );
        }
    }

    proptest! {
        #[test]
        fn addition_is_commutative(a in 0_u16..=WORD_MASK, b in 0_u16..=WORD_MASK) {
            let a = AgcWord::try_from_raw(a).unwrap();
            let b = AgcWord::try_from_raw(b).unwrap();
            prop_assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        }

        #[test]
        fn complement_is_additive_inverse(raw in 0_u16..=WORD_MASK) {
            let word = AgcWord::try_from_raw(raw).unwrap();
            prop_assert!(word.wrapping_add(word.complement()).is_zero());
        }
    }
}
