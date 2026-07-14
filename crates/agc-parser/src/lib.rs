#![forbid(unsafe_code)]
//! Diagnostic, loss-preserving parser for the historical yaYUL source dialect.

use agc_ast::{LineKind, SourceLine, SourceUnit, Span, SpannedText, Statement, StatementKind};
use agc_source::{SourceFile, SourceId};
use thiserror::Error;

/// Severity of a parser diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// Input cannot be represented as a meaningful source construct.
    Error,
    /// Input is retained but merits inspection.
    Warning,
}

/// Structured parser diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Source span.
    pub span: Span,
    /// One-based line number.
    pub line: u32,
}

/// Successful parse with non-fatal diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOutput {
    /// Parsed source unit.
    pub unit: SourceUnit,
    /// Diagnostics in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Fatal parser entry-point errors.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Input bytes are not UTF-8.
    #[error("source {source_id} is not UTF-8: {error}")]
    Utf8 {
        /// Source identity.
        source_id: SourceId,
        /// UTF-8 failure.
        error: std::str::Utf8Error,
    },
}

/// Parses a loaded source file without normalizing its bytes.
pub fn parse_file(file: &SourceFile) -> Result<ParseOutput, ParseError> {
    let text = std::str::from_utf8(&file.bytes).map_err(|error| ParseError::Utf8 {
        source_id: file.id.clone(),
        error,
    })?;
    Ok(parse_str(file.id.clone(), text))
}

/// Parses one UTF-8 source string.
pub fn parse_str(source: SourceId, text: &str) -> ParseOutput {
    let mut lines = Vec::new();
    let mut diagnostics = Vec::new();
    let bytes = text.as_bytes();
    let mut offset = 0;
    let mut line_number = 1_u32;

    while offset < bytes.len() {
        let newline = bytes[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|relative| offset + relative);
        let physical_end = newline.unwrap_or(bytes.len());
        let content_end = if physical_end > offset && bytes[physical_end - 1] == b'\r' {
            physical_end - 1
        } else {
            physical_end
        };
        let terminator = match newline {
            Some(_) if content_end < physical_end => "\r\n",
            Some(_) => "\n",
            None => "",
        };
        let raw = &text[offset..content_end];
        let span = Span::new(offset, content_end);
        let kind = parse_line(raw, offset, line_number, &mut diagnostics);
        lines.push(SourceLine {
            number: line_number,
            span,
            terminator: terminator.to_owned(),
            kind,
        });
        offset = newline.map_or(bytes.len(), |position| position + 1);
        line_number += 1;
    }
    if text.is_empty() {
        // An empty file has no physical lines and still round-trips exactly.
    }

    let unit = SourceUnit {
        source,
        text: text.to_owned(),
        lines,
    };
    debug_assert_eq!(unit.reconstruct(), text);
    ParseOutput { unit, diagnostics }
}

fn parse_line(raw: &str, base: usize, line: u32, diagnostics: &mut Vec<Diagnostic>) -> LineKind {
    let trimmed_start = raw.trim_start_matches([' ', '\t']);
    let leading = raw.len() - trimmed_start.len();
    if trimmed_start.is_empty() {
        return LineKind::Blank;
    }
    if is_page_marker(trimmed_start) {
        return LineKind::PageMarker {
            text: spanned(trimmed_start, base + leading),
        };
    }
    if let Some(comment) = trimmed_start.strip_prefix('#') {
        return LineKind::Comment {
            text: spanned(comment, base + leading + 1),
        };
    }
    if let Some(include) = trimmed_start.strip_prefix('$') {
        let include_base = base + leading + 1;
        let (body, comment) = split_comment(include, include_base);
        let path = body.trim();
        let path_start = include_base + body.len() - body.trim_start().len();
        if path.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "AGCP001",
                message: "include directive has no path".to_owned(),
                span: Span::new(include_base, include_base + include.len()),
                line,
            });
        }
        return LineKind::Include {
            path: spanned(path, path_start),
            comment,
        };
    }

    let (code, comment) = split_comment(raw, base);
    let fields = fields(code, base);
    if fields.is_empty() {
        return LineKind::Comment {
            text: comment.unwrap_or_else(|| spanned("", base + raw.len())),
        };
    }
    let starts_in_column_zero = !raw.starts_with([' ', '\t']);
    let (label_index, operation_index) = classify_fields(&fields, starts_in_column_zero);
    let Some(operation) = fields.get(operation_index).cloned() else {
        return LineKind::Label {
            label: fields[0].clone(),
            comment,
            raw: raw.to_owned(),
        };
    };
    let operand = operand_after(code, base, operation.span.end);
    let kind = statement_kind(&operation.text);
    LineKind::Statement(Statement {
        label: label_index.map(|index| fields[index].clone()),
        operation,
        operand,
        comment,
        kind,
        raw: raw.to_owned(),
    })
}

fn classify_fields(fields: &[SpannedText], starts_in_column_zero: bool) -> (Option<usize>, usize) {
    if starts_in_column_zero {
        (Some(0), 1)
    } else if fields.len() >= 2
        && is_relative_label(&fields[0].text)
        && is_known_operation(&fields[1].text)
    {
        (Some(0), 1)
    } else {
        (None, 0)
    }
}

fn split_comment(input: &str, base: usize) -> (&str, Option<SpannedText>) {
    input.find('#').map_or((input, None), |index| {
        let comment_text = &input[index + 1..];
        (
            &input[..index],
            Some(spanned(comment_text, base + index + 1)),
        )
    })
}

fn fields(input: &str, base: usize) -> Vec<SpannedText> {
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in input.char_indices() {
        if character.is_whitespace() {
            if let Some(field_start) = start.take() {
                result.push(spanned(&input[field_start..index], base + field_start));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(field_start) = start {
        result.push(spanned(&input[field_start..], base + field_start));
    }
    result
}

fn operand_after(code: &str, base: usize, operation_end: usize) -> Option<SpannedText> {
    let local = operation_end - base;
    let remainder = &code[local..];
    let trimmed = remainder.trim();
    if trimmed.is_empty() {
        None
    } else {
        let start = operation_end + remainder.len() - remainder.trim_start().len();
        Some(spanned(trimmed, start))
    }
}

fn spanned(text: &str, start: usize) -> SpannedText {
    SpannedText {
        text: text.to_owned(),
        span: Span::new(start, start + text.len()),
    }
}

fn is_page_marker(input: &str) -> bool {
    input.starts_with("## Page ")
        || input.starts_with("# Page ")
        || (input.len() >= 8
            && input
                .chars()
                .all(|character| matches!(character, '*' | '-' | '=')))
}

fn is_relative_label(input: &str) -> bool {
    let unsigned = input.strip_prefix(['+', '-']).unwrap_or(input);
    !unsigned.is_empty() && unsigned.chars().all(|character| character.is_ascii_digit())
}

fn statement_kind(operation: &str) -> StatementKind {
    let upper = operation.trim_end_matches('*').to_ascii_uppercase();
    if BASIC_OPERATIONS.contains(&upper.as_str()) {
        StatementKind::Instruction
    } else if DIRECTIVES.contains(&upper.as_str()) {
        StatementKind::Directive
    } else if INTERPRETIVE_OPERATIONS.contains(&upper.as_str()) {
        StatementKind::Interpretive
    } else {
        StatementKind::Unknown
    }
}

fn is_known_operation(operation: &str) -> bool {
    statement_kind(operation) != StatementKind::Unknown
}

const BASIC_OPERATIONS: &[&str] = &[
    "AD", "ADS", "AUG", "BZF", "BZMF", "CA", "CAF", "CCS", "COM", "CS", "DAS", "DCA", "DCOM",
    "DCS", "DIM", "DOUBLE", "DV", "DXCH", "EDRUPT", "EXTEND", "INCR", "INDEX", "LXCH", "MASK",
    "MP", "MSU", "NDX", "NOOP", "OVSK", "QXCH", "RAND", "READ", "RELINT", "RESUME", "RETURN",
    "ROR", "RXOR", "SQUARE", "SU", "TC", "TCR", "TS", "WAND", "WOR", "WRITE", "XCH", "XLQ",
    "XXALQ", "ZL", "ZQ",
];

const DIRECTIVES: &[&str] = &[
    "=", "1DNADR", "2BCADR", "2CADR", "2DEC", "2OCT", "ADRES", "BANK", "BBCON", "BLOCK", "BNKSUM",
    "CADR", "CHECK=", "COUNT", "DEC", "DNCHAN", "ECADR", "EQUALS", "ERASE", "GENADR", "INCLUDE",
    "MEMORY", "OCT", "REMADR", "SETLOC", "SUBRO", "VN", "XCADR",
];

const INTERPRETIVE_OPERATIONS: &[&str] = &[
    "ABS", "ACOS", "ARCCOS", "ARCSIN", "ASIN", "AXC", "BDDV", "BDDV*", "BDSU", "BMN", "BOF",
    "BOFF", "BON", "BOV", "BOVB", "BPL", "BZE", "BZE/GOTO", "CALL", "CCALL", "CGOTO", "CLEAR",
    "COS", "DAD", "DCOMP", "DDV", "DLOAD", "DMPR", "DMPR*", "DOT", "DSQ", "DSU", "EXIT", "GOTO",
    "INCR", "ITCQ", "LXA", "MXV", "NORM", "PDDL", "PDVL", "PUSH", "ROUND", "RTB", "SET", "SIGN",
    "SIN", "SL", "SL1", "SLOAD", "SQRT", "SR", "SR1", "STADR", "STCALL", "STODL", "STORE", "STOVL",
    "STQ", "TAD", "TIX", "TLOAD", "TP", "TSLC", "V/SC", "VAD", "VCOMP", "VDEF", "VLOAD", "VPROJ",
    "VSL", "VSR", "VSQ", "VSU", "VXV", "VXSC", "XAD",
];

#[cfg(test)]
mod tests {
    use super::*;
    use agc_source::Program;

    fn id() -> SourceId {
        SourceId::new(Program::Luminary099, "TEST.agc")
    }

    #[test]
    fn round_trip_retains_crlf_comments_and_final_line() {
        let input = "# heading\r\nSTART\tTC\tNEXT # go\r\n\t$CHILD.agc\n";
        let parsed = parse_str(id(), input);
        assert_eq!(parsed.unit.reconstruct(), input);
        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.unit.statements().count(), 1);
        assert_eq!(parsed.unit.includes().count(), 1);
    }

    #[test]
    fn detects_relative_location_labels() {
        let parsed = parse_str(id(), "  -1 OCT 12345\n");
        let (_, statement) = parsed.unit.statements().next().unwrap();
        assert_eq!(statement.label.as_ref().unwrap().text, "-1");
        assert_eq!(statement.operation.text, "OCT");
    }
}
