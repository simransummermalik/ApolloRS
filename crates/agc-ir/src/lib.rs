#![forbid(unsafe_code)]
//! Typed intermediate representation shared by assembly, analysis, and translation.

use agc_ast::{Span, StatementKind};
use agc_word::AgcWord;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Globally stable location in an expanded source graph.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Program-relative source path.
    pub file: String,
    /// One-based line.
    pub line: u32,
    /// Byte span in that file.
    pub span: Span,
}

/// Operand before symbol resolution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum Operand {
    /// No operand.
    None,
    /// Octal or decimal literal represented mathematically.
    Literal(i64),
    /// Symbol name with optional signed offset.
    Symbol {
        /// Historical symbol spelling.
        name: String,
        /// Signed address/value adjustment.
        offset: i32,
    },
    /// Original expression retained when not yet evaluated.
    Expression(String),
}

/// One lowered assembly record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IrRecord {
    /// Source location.
    pub location: SourceLocation,
    /// Optional source label.
    pub label: Option<String>,
    /// Canonical uppercase operation.
    pub operation: String,
    /// Parsed operand.
    pub operand: Operand,
    /// Initial statement family.
    pub kind: StatementKind,
    /// Assigned physical bank, when location assignment has run.
    pub bank: Option<u8>,
    /// Assigned offset in bank.
    pub offset: Option<u16>,
    /// Emitted word after assembly.
    pub word: Option<AgcWord>,
}

/// Expanded and lowered program representation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramIr {
    /// Program identifier.
    pub program: String,
    /// Source records in include-expanded order.
    pub records: Vec<IrRecord>,
    /// Source file text hashes keyed by relative path.
    pub source_hashes: IndexMap<String, String>,
}

impl ProgramIr {
    /// Iterates emitted words with physical locations.
    pub fn emitted_words(&self) -> impl Iterator<Item = (u8, u16, AgcWord)> + '_ {
        self.records
            .iter()
            .filter_map(|record| Some((record.bank?, record.offset?, record.word?)))
    }
}

/// Converts an operand field into a conservative typed representation.
pub fn parse_operand(input: Option<&str>) -> Operand {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Operand::None;
    };
    let first = input.split_whitespace().next().unwrap_or(input);
    if let Some(value) = parse_number(first) {
        return Operand::Literal(value);
    }
    if first
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_.$/+?-".contains(character))
    {
        if let Some((name, offset)) = split_symbol_offset(first) {
            return Operand::Symbol {
                name: name.to_owned(),
                offset,
            };
        }
        return Operand::Symbol {
            name: first.to_owned(),
            offset: 0,
        };
    }
    Operand::Expression(input.to_owned())
}

fn parse_number(input: &str) -> Option<i64> {
    let (negative, digits) = input
        .strip_prefix('-')
        .map_or((false, input), |digits| (true, digits));
    let (radix, digits) = if let Some(digits) = digits.strip_prefix("0o") {
        (8, digits)
    } else if let Some(digits) = digits.strip_suffix('D') {
        (10, digits)
    } else if digits
        .chars()
        .all(|character| matches!(character, '0'..='7'))
    {
        (8, digits)
    } else {
        return None;
    };
    i64::from_str_radix(digits, radix)
        .ok()
        .map(|value| if negative { -value } else { value })
}

fn split_symbol_offset(input: &str) -> Option<(&str, i32)> {
    for (index, character) in input.char_indices().rev().filter(|(index, _)| *index > 0) {
        if matches!(character, '+' | '-') {
            let offset = input[index + 1..].parse::<i32>().ok()?;
            return Some((
                &input[..index],
                if character == '-' { -offset } else { offset },
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operands_distinguish_octal_symbols_and_offsets() {
        assert_eq!(parse_operand(Some("123")), Operand::Literal(0o123));
        assert_eq!(
            parse_operand(Some("TARGET-2")),
            Operand::Symbol {
                name: "TARGET".to_owned(),
                offset: -2
            }
        );
    }
}
