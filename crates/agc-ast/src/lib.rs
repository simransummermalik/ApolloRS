#![forbid(unsafe_code)]
//! Loss-preserving syntax tree for yaYUL-formatted AGC assembly sources.

use agc_source::SourceId;
use serde::{Deserialize, Serialize};

/// Half-open byte span in a source file.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Span {
    /// Inclusive byte offset.
    pub start: usize,
    /// Exclusive byte offset.
    pub end: usize,
}

impl Span {
    /// Constructs a half-open span.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// A token carrying both decoded text and exact source position.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpannedText {
    /// Token text.
    pub text: String,
    /// Token position in the source unit.
    pub span: Span,
}

/// Broad statement family used before semantic lowering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatementKind {
    /// Basic or extended machine instruction.
    Instruction,
    /// yaYUL assembler directive.
    Directive,
    /// Interpretive-language operation.
    Interpretive,
    /// Syntactically valid but not yet classified mnemonic.
    Unknown,
}

/// One assembly statement, retaining the exact original line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    /// Optional location symbol or relative location label.
    pub label: Option<SpannedText>,
    /// Operation or directive mnemonic.
    pub operation: SpannedText,
    /// Unsplit operand text after the operation and before the comment.
    pub operand: Option<SpannedText>,
    /// Trailing comment, including no comment delimiter.
    pub comment: Option<SpannedText>,
    /// Initial syntactic classification.
    pub kind: StatementKind,
    /// Exact line text excluding its line terminator.
    pub raw: String,
}

/// Loss-preserving kind of a physical source line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LineKind {
    /// Empty or whitespace-only line.
    Blank,
    /// Full-line comment.
    Comment {
        /// Comment text after the delimiter.
        text: SpannedText,
    },
    /// Page separator emitted in the historical listings.
    PageMarker {
        /// Exact marker text.
        text: SpannedText,
    },
    /// `$` include directive.
    Include {
        /// Included path exactly as written after trimming field whitespace.
        path: SpannedText,
        /// Optional trailing comment.
        comment: Option<SpannedText>,
    },
    /// A symbol card that assigns the current location without an operation.
    Label {
        /// Location symbol.
        label: SpannedText,
        /// Optional trailing comment.
        comment: Option<SpannedText>,
        /// Exact line text excluding its line terminator.
        raw: String,
    },
    /// Labelled or unlabelled assembly statement.
    Statement(Statement),
}

/// One physical source line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLine {
    /// One-based line number.
    pub number: u32,
    /// Byte span excluding the line terminator.
    pub span: Span,
    /// Exact line terminator (`"\n"`, `"\r\n"`, or empty at EOF).
    pub terminator: String,
    /// Parsed line kind.
    pub kind: LineKind,
}

/// Parsed representation of one historical source file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceUnit {
    /// Stable source identity.
    pub source: SourceId,
    /// Exact decoded source text.
    pub text: String,
    /// Physical lines in source order.
    pub lines: Vec<SourceLine>,
}

impl SourceUnit {
    /// Reconstructs exact source text from line payloads and terminators.
    pub fn reconstruct(&self) -> String {
        let mut result = String::with_capacity(self.text.len());
        for line in &self.lines {
            result.push_str(&self.text[line.span.start..line.span.end]);
            result.push_str(&line.terminator);
        }
        result
    }

    /// Iterates statements without dropping source location.
    pub fn statements(&self) -> impl Iterator<Item = (&SourceLine, &Statement)> {
        self.lines.iter().filter_map(|line| match &line.kind {
            LineKind::Statement(statement) => Some((line, statement)),
            _ => None,
        })
    }

    /// Iterates direct include paths.
    pub fn includes(&self) -> impl Iterator<Item = (&SourceLine, &SpannedText)> {
        self.lines.iter().filter_map(|line| match &line.kind {
            LineKind::Include { path, .. } => Some((line, path)),
            _ => None,
        })
    }

    /// Iterates location-only label cards.
    pub fn labels(&self) -> impl Iterator<Item = (&SourceLine, &SpannedText)> {
        self.lines.iter().filter_map(|line| match &line.kind {
            LineKind::Label { label, .. } => Some((line, label)),
            _ => None,
        })
    }
}
