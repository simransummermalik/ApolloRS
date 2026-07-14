#![forbid(unsafe_code)]
//! Provenance-preserving AGC IR to compilable Rust instruction dispatch.

use agc_ir::ProgramIr;
use agc_symbols::SymbolTable;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Code-generation style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
    /// One dispatch arm per emitted AGC word, preserving instruction boundaries.
    Faithful,
    /// Also emits safe straight-line label ranges as named helper functions.
    Structured,
}

/// Verification status embedded in generated output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationStatus {
    /// Output has not yet been differentially executed.
    Unverified,
    /// Output matched a specified number of trace events.
    TraceMatched {
        /// Number of equal committed trace events.
        events: usize,
    },
    /// Output diverged and carries a concise reason.
    Diverged {
        /// First known divergence reason.
        reason: String,
    },
}

/// Generated Rust and generation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedRust {
    /// Complete standalone Rust source.
    pub source: String,
    /// Number of emitted AGC records.
    pub records: usize,
    /// Generation style.
    pub style: Style,
    /// Verification status embedded in source.
    pub verification: VerificationStatus,
}

/// Rust generation or compile-check failure.
#[derive(Debug, Error)]
pub enum TranspileError {
    /// IR has no emitted words.
    #[error("IR contains no emitted words")]
    Empty,
    /// Generated-source I/O failed.
    #[error("generated Rust I/O error at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// rustc rejected generated output.
    #[error("generated Rust failed to compile: {0}")]
    Compile(String),
}

/// Emits standalone Rust that delegates each raw instruction to a machine
/// trait. The trait boundary makes the generated module compilable without
/// hiding AGC state or substituting host arithmetic.
pub fn generate(
    ir: &ProgramIr,
    symbols: &SymbolTable,
    style: Style,
    verification: VerificationStatus,
) -> Result<GeneratedRust, TranspileError> {
    let records = ir
        .records
        .iter()
        .filter_map(|record| Some((record, record.bank?, record.offset?, record.word?)))
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Err(TranspileError::Empty);
    }
    let verification_text = match &verification {
        VerificationStatus::Unverified => "unverified".to_owned(),
        VerificationStatus::TraceMatched { events } => format!("trace-matched:{events}"),
        VerificationStatus::Diverged { reason } => format!("diverged:{}", sanitize_comment(reason)),
    };
    let mut source = format!(
        "#![forbid(unsafe_code)]\n\
         //! Generated from Apollo Guidance Computer IR.\n\
         //! Program: {}\n\
         //! Verification: {}\n\n\
         #[derive(Clone, Copy, Debug, Eq, PartialEq)]\n\
         pub struct SourceRecord {{\n\
             pub bank: u8,\n\
             pub offset: u16,\n\
             pub word: u16,\n\
             pub file: &'static str,\n\
             pub line: u32,\n\
             pub label: Option<&'static str>,\n\
         }}\n\n\
         pub trait AgcMachine {{\n\
             type Error;\n\
             fn execute_word(&mut self, record: SourceRecord) -> Result<(), Self::Error>;\n\
         }}\n\n\
         pub const VERIFICATION: &str = \"{}\";\n\n\
         pub const RECORDS: &[SourceRecord] = &[\n",
        sanitize_comment(&ir.program),
        verification_text,
        escape_string(&verification_text)
    );
    for (record, bank, offset, word) in &records {
        let label = record.label.as_ref().map_or_else(
            || "None".to_owned(),
            |label| format!("Some(\"{}\")", escape_string(label)),
        );
        let _ = writeln!(
            source,
            "    SourceRecord {{ bank: 0o{:02o}, offset: 0o{:04o}, word: 0o{:05o}, file: \"{}\", line: {}, label: {} }},",
            bank,
            offset,
            word.raw(),
            escape_string(&record.location.file),
            record.location.line,
            label
        );
    }
    source.push_str(
        "];\n\n\
         pub fn dispatch<M: AgcMachine>(machine: &mut M, bank: u8, offset: u16) -> Result<bool, M::Error> {\n\
             let Some(record) = RECORDS.iter().find(|record| record.bank == bank && record.offset == offset).copied() else {\n\
                 return Ok(false);\n\
             };\n\
             machine.execute_word(record)?;\n\
             Ok(true)\n\
         }\n",
    );
    if style == Style::Structured {
        emit_structured_helpers(&mut source, ir, symbols);
    }
    Ok(GeneratedRust {
        source,
        records: records.len(),
        style,
        verification,
    })
}

/// Writes generated source to a chosen path.
pub fn write_generated(
    path: impl AsRef<Path>,
    generated: &GeneratedRust,
) -> Result<(), TranspileError> {
    let path = path.as_ref();
    fs::write(path, &generated.source).map_err(|source| TranspileError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Compile-checks generated standalone source using the selected Rust compiler.
pub fn compile_check(
    rustc: impl AsRef<Path>,
    source_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<(), TranspileError> {
    let output = Command::new(rustc.as_ref())
        .arg("--edition=2024")
        .arg("--crate-type=lib")
        .arg(source_path.as_ref())
        .arg("-o")
        .arg(output_path.as_ref())
        .output()
        .map_err(|source| TranspileError::Io {
            path: source_path.as_ref().to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TranspileError::Compile(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

fn emit_structured_helpers(source: &mut String, ir: &ProgramIr, symbols: &SymbolTable) {
    source
        .push_str("\n// Named straight-line entry points recovered conservatively from labels.\n");
    for (name, symbol) in symbols.iter() {
        let Some(address) = symbol.value.logical_address() else {
            continue;
        };
        let Some(record) = ir
            .records
            .iter()
            .find(|record| record.label.as_deref() == Some(name) && record.word.is_some())
        else {
            continue;
        };
        let (Some(bank), Some(offset)) = (record.bank, record.offset) else {
            continue;
        };
        let _ = write!(
            source,
            "#[doc = \"Original label `{}` at logical address {:04o}.\"]\n\
             pub fn label_{}<M: AgcMachine>(machine: &mut M) -> Result<bool, M::Error> {{\n\
                 dispatch(machine, 0o{:02o}, 0o{:04o})\n\
             }}\n",
            sanitize_comment(name),
            address,
            rust_identifier(name),
            bank,
            offset
        );
    }
}

fn rust_identifier(input: &str) -> String {
    let mut output = String::new();
    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }
    if output.starts_with(|character: char| character.is_ascii_digit()) {
        output.insert(0, '_');
    }
    if output.is_empty() {
        output.push_str("anonymous");
    }
    output
}

fn escape_string(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn sanitize_comment(input: &str) -> String {
    input.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_ast::{Span, StatementKind};
    use agc_ir::{IrRecord, Operand, SourceLocation};
    use agc_word::AgcWord;

    #[test]
    fn faithful_output_is_standalone_and_retains_provenance() {
        let ir = ProgramIr {
            program: "test".to_owned(),
            records: vec![IrRecord {
                location: SourceLocation {
                    file: "MAIN.agc".to_owned(),
                    line: 42,
                    span: Span::new(0, 1),
                },
                label: Some("START".to_owned()),
                operation: "TCF".to_owned(),
                operand: Operand::Literal(0o4000),
                kind: StatementKind::Instruction,
                bank: Some(2),
                offset: Some(0),
                word: Some(AgcWord::from_raw_truncate(0o14000)),
            }],
            source_hashes: Default::default(),
        };
        let generated = generate(
            &ir,
            &SymbolTable::default(),
            Style::Faithful,
            VerificationStatus::Unverified,
        )
        .unwrap();
        assert!(generated.source.contains("MAIN.agc"));
        assert!(generated.source.contains("execute_word"));
    }
}
