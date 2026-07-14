#![forbid(unsafe_code)]
//! Deterministic symbol definitions, references, and AGC address forms.

use agc_ir::SourceLocation;
use agc_word::AgcWord;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Semantic value assigned to an AGC symbol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SymbolValue {
    /// Fixed-memory location in physical bank order.
    Fixed {
        /// Fixed bank.
        bank: u8,
        /// Offset in the 1024-word bank.
        offset: u16,
    },
    /// Erasable-memory location.
    Erasable {
        /// Erasable bank.
        bank: u8,
        /// Offset in the 256-word bank.
        offset: u16,
    },
    /// One-word constant.
    Constant {
        /// Raw-exact constant.
        value: AgcWord,
    },
    /// Mathematical value used by the assembler.
    Absolute {
        /// Signed integer.
        value: i64,
    },
}

impl SymbolValue {
    /// Converts a memory symbol to the logical address visible from its bank.
    pub const fn logical_address(&self) -> Option<u16> {
        match *self {
            Self::Fixed { bank: 2, offset } => Some(0o4000 | offset),
            Self::Fixed { bank: 3, offset } => Some(0o6000 | offset),
            Self::Fixed { offset, .. } => Some(0o2000 | offset),
            Self::Erasable {
                bank: 0..=2,
                offset,
            } => Some((self.erasable_bank().unwrap() as u16) << 8 | offset),
            Self::Erasable { offset, .. } => Some(0o1400 | offset),
            Self::Constant { value } => Some(value.raw() & 0o7777),
            Self::Absolute { value } if value >= 0 && value <= 0o7777 => Some(value as u16),
            _ => None,
        }
    }

    const fn erasable_bank(&self) -> Option<u8> {
        if let Self::Erasable { bank, .. } = *self {
            Some(bank)
        } else {
            None
        }
    }
}

/// One symbol definition and all discovered uses.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    /// Case-sensitive historical spelling.
    pub name: String,
    /// Assigned semantic value.
    pub value: SymbolValue,
    /// Definition location.
    pub definition: SourceLocation,
    /// Reference locations in expanded source order.
    pub references: Vec<SourceLocation>,
}

/// Insertion-ordered symbol table for reproducible reports.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SymbolTable {
    symbols: IndexMap<String, Symbol>,
}

/// Symbol table failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SymbolError {
    /// Two definitions use the same spelling.
    #[error("symbol {name} is defined more than once")]
    Duplicate {
        /// Duplicated name.
        name: String,
        /// First definition.
        first: SourceLocation,
        /// Conflicting definition.
        second: SourceLocation,
    },
    /// A required symbol has no definition.
    #[error("symbol {name} is unresolved at {location:?}")]
    Unresolved {
        /// Symbol spelling.
        name: String,
        /// Use location.
        location: SourceLocation,
    },
}

impl SymbolTable {
    /// Defines a symbol, rejecting any duplicate explicitly.
    pub fn define(
        &mut self,
        name: impl Into<String>,
        value: SymbolValue,
        definition: SourceLocation,
    ) -> Result<(), SymbolError> {
        let name = name.into();
        if let Some(existing) = self.symbols.get(&name) {
            return Err(SymbolError::Duplicate {
                name,
                first: existing.definition.clone(),
                second: definition,
            });
        }
        self.symbols.insert(
            name.clone(),
            Symbol {
                name,
                value,
                definition,
                references: Vec::new(),
            },
        );
        Ok(())
    }

    /// Resolves without recording a reference.
    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    /// Resolves and records a source reference.
    pub fn reference(
        &mut self,
        name: &str,
        location: SourceLocation,
    ) -> Result<SymbolValue, SymbolError> {
        let symbol = self
            .symbols
            .get_mut(name)
            .ok_or_else(|| SymbolError::Unresolved {
                name: name.to_owned(),
                location: location.clone(),
            })?;
        symbol.references.push(location);
        Ok(symbol.value.clone())
    }

    /// Iterates in deterministic definition order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Symbol)> {
        self.symbols
            .iter()
            .map(|(name, symbol)| (name.as_str(), symbol))
    }

    /// Number of definitions.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Whether no symbols are defined.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_ast::Span;

    fn location(line: u32) -> SourceLocation {
        SourceLocation {
            file: "TEST.agc".to_owned(),
            line,
            span: Span::new(0, 1),
        }
    }

    #[test]
    fn duplicate_definitions_are_not_silently_replaced() {
        let mut table = SymbolTable::default();
        table
            .define("X", SymbolValue::Absolute { value: 1 }, location(1))
            .unwrap();
        assert!(
            table
                .define("X", SymbolValue::Absolute { value: 2 }, location(2))
                .is_err()
        );
    }
}
