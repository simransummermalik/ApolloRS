#![forbid(unsafe_code)]
//! Native Rust include expansion, semantic lowering, symbol resolution, and rope assembly.

use agc_ast::{LineKind, SourceUnit, StatementKind};
use agc_ir::{IrRecord, Operand, ProgramIr, SourceLocation, parse_operand};
use agc_isa::{Mnemonic, encode_with_context};
use agc_overlay::{Overlay, OverlayError, resolve_relative_include};
use agc_parser::{Diagnostic as ParseDiagnostic, Severity as ParseSeverity, parse_file};
use agc_source::{HistoricalCorpus, Program, SourceError, SourceId};
use agc_symbols::{SymbolError, SymbolTable, SymbolValue};
use agc_word::AgcWord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use thiserror::Error;

/// Number of physical words in an Apollo 11 Block II rope.
pub const ROPE_WORDS: usize = 36 * 1024;

/// Conservative lower bound that distinguishes a populated flight rope from
/// yaYUL's bank-identification/checksum-only output after a failed source pass.
pub const MINIMUM_REFERENCE_NONZERO_WORDS: usize = 256;

/// Assembly diagnostic severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Assembly cannot emit a reliable word or artifact.
    Error,
    /// Assembly continued with an explicit, reviewable condition.
    Warning,
}

/// Structured semantic or assembly diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Stable code.
    pub code: String,
    /// Explanation.
    pub message: String,
    /// Source location, when attributable to one record.
    pub location: Option<SourceLocation>,
}

/// Include-expanded parsed source graph.
#[derive(Clone, Debug)]
pub struct ExpandedProgram {
    /// Program identity.
    pub program: Program,
    /// Unique parsed units in first-encounter order.
    pub units: Vec<SourceUnit>,
    /// Include-expanded semantic records.
    pub ir: ProgramIr,
    /// Parser/include diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Deterministic physical rope and semantic metadata.
#[derive(Clone, Debug)]
pub struct ProgramImage {
    /// Physical fixed banks concatenated in numerical bank order.
    pub words: Vec<AgcWord>,
    /// True for every location explicitly emitted by source.
    pub occupied: Vec<bool>,
    /// Fully resolved symbols.
    pub symbols: SymbolTable,
    /// Bank-aware IR with emitted words.
    pub ir: ProgramIr,
    /// Non-fatal diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl ProgramImage {
    /// Serializes words as big-endian 16-bit values with the AGC word in bits 15..1.
    pub fn to_yayul_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.words.len() * 2);
        for raw_bank in 0..36 {
            let bank = if raw_bank < 4 { raw_bank ^ 2 } else { raw_bank };
            for word in &self.words[bank * 1024..(bank + 1) * 1024] {
                bytes.extend_from_slice(&(word.raw() << 1).to_be_bytes());
            }
        }
        bytes
    }

    /// Number of emitted locations.
    pub fn occupied_words(&self) -> usize {
        self.occupied.iter().filter(|&&occupied| occupied).count()
    }
}

/// Configuration for an explicitly identified external reference assembler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceAssemblerConfig {
    /// Path to the yaYUL executable.
    pub executable: PathBuf,
    /// Immutable tool/version identifier recorded in the build report.
    pub toolchain: String,
    /// Request yaYUL's force-output mode. Diagnostics remain fatal to ApolloRS.
    pub force_output: bool,
    /// Minimum populated words accepted as a flight-software image.
    pub minimum_nonzero_words: usize,
}

impl ReferenceAssemblerConfig {
    /// Creates a strict reference configuration.
    pub fn new(executable: impl Into<PathBuf>, toolchain: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            toolchain: toolchain.into(),
            force_output: false,
            minimum_nonzero_words: MINIMUM_REFERENCE_NONZERO_WORDS,
        }
    }
}

/// Auditable result of one isolated reference-assembler invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceAssemblyReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Historical program assembled.
    pub program: Program,
    /// Entry source passed to yaYUL.
    pub entry: String,
    /// Historical checkout commit.
    pub historical_commit: Option<String>,
    /// Exact reference toolchain identity supplied by the caller.
    pub toolchain: String,
    /// Whether yaYUL force-output mode was requested.
    pub force_output: bool,
    /// Process exit code, or `None` when terminated by signal.
    pub exit_code: Option<i32>,
    /// yaYUL unresolved-symbol count.
    pub unresolved_symbols: usize,
    /// yaYUL fatal-error count.
    pub fatal_errors: usize,
    /// yaYUL warning count.
    pub warnings: usize,
    /// yaYUL multiply-defined-symbol count, when reported.
    pub multiply_defined_symbols: usize,
    /// Size of the emitted rope in bytes.
    pub rope_bytes: usize,
    /// Number of nonzero 16-bit words in the emitted rope.
    pub nonzero_words: usize,
    /// SHA-256 of the exact emitted rope bytes.
    pub rope_sha256: String,
    /// Complete captured standard output.
    pub stdout: String,
    /// Complete captured standard error.
    pub stderr: String,
}

/// Validated rope bytes and their reference build report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceAssembly {
    /// Standard yaYUL-order rope image.
    pub rope: Vec<u8>,
    /// Provenance, diagnostics, occupancy, and content digest.
    pub report: ReferenceAssemblyReport,
}

/// Per-bank checksum validation for an independently transcribed rope listing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinsourceBankChecksum {
    /// Physical fixed-bank number.
    pub bank: u8,
    /// One's-complement sum of all 1,024 bank words.
    pub sum: AgcWord,
    /// `positive` or `negative` bank-number checksum convention.
    pub polarity: String,
}

/// Provenance and validation report for a VirtualAGC-style binsource import.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BinsourceAssemblyReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Flight program represented by the listing.
    pub program: Program,
    /// Historical source checkout commit used alongside this reference object.
    pub historical_commit: Option<String>,
    /// Input path supplied by the caller.
    pub binsource: String,
    /// Exact binsource/toolchain revision supplied by the caller.
    pub toolchain: String,
    /// SHA-256 of the binsource text bytes.
    pub binsource_sha256: String,
    /// Number of physical banks parsed.
    pub banks: usize,
    /// Number of rope words parsed.
    pub words: usize,
    /// Number of explicit unused-word markers.
    pub unused_words: usize,
    /// Per-bank checksum evidence.
    pub bank_checksums: Vec<BinsourceBankChecksum>,
    /// Size of the emitted rope in bytes.
    pub rope_bytes: usize,
    /// Number of nonzero emitted words.
    pub nonzero_words: usize,
    /// SHA-256 of the exact emitted rope bytes.
    pub rope_sha256: String,
}

/// Validated rope imported from an independently proofed octal listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinsourceAssembly {
    /// Standard yaYUL-order rope image.
    pub rope: Vec<u8>,
    /// Parse, checksum, and provenance report.
    pub report: BinsourceAssemblyReport,
}

/// Fatal expansion/assembly entry-point failure.
#[derive(Debug, Error)]
pub enum AssemblyError {
    /// Historical source access failed.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// Parser could not decode source bytes.
    #[error("parser failed: {0}")]
    Parser(String),
    /// Overlay applies to a different program.
    #[error("overlay for {actual} cannot be applied to {expected}")]
    OverlayProgram {
        /// Program being assembled.
        expected: Program,
        /// Overlay program.
        actual: Program,
    },
    /// Include cycle.
    #[error("include cycle: {0}")]
    IncludeCycle(String),
    /// Semantic errors prevent a reliable image.
    #[error("assembly produced {0} error diagnostics")]
    Diagnostics(usize),
    /// Compatibility overlay staging failed.
    #[error(transparent)]
    Overlay(#[from] OverlayError),
    /// Reference staging or artifact access failed.
    #[error("reference assembly filesystem error at {path}: {source}")]
    ReferenceIo {
        /// Affected path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Reference assembler output failed strict validation.
    #[error("reference assembly failed: {0}")]
    Reference(String),
}

/// Runs yaYUL in a disposable, writable copy of one historical source tree.
///
/// Historical source is never modified. A compatibility overlay is materialized
/// only in staging, and ApolloRS rejects nonzero diagnostics, malformed rope
/// size, and checksum-only output even when yaYUL itself exits successfully.
pub fn assemble_reference(
    corpus: &HistoricalCorpus,
    program: Program,
    entry: &str,
    overlay: Option<&Overlay>,
    config: &ReferenceAssemblerConfig,
) -> Result<ReferenceAssembly, AssemblyError> {
    if let Some(overlay) = overlay {
        if overlay.program != program {
            return Err(AssemblyError::OverlayProgram {
                expected: program,
                actual: overlay.program,
            });
        }
    }
    let entry_path = validate_reference_entry(entry)?;
    if config.toolchain.trim().is_empty() {
        return Err(AssemblyError::Reference(
            "reference toolchain identity is empty".to_owned(),
        ));
    }
    if config.minimum_nonzero_words == 0 || config.minimum_nonzero_words > ROPE_WORDS {
        return Err(AssemblyError::Reference(format!(
            "invalid minimum nonzero-word threshold {}",
            config.minimum_nonzero_words
        )));
    }

    let staging = TempDir::new().map_err(|source| AssemblyError::ReferenceIo {
        path: std::env::temp_dir(),
        source,
    })?;
    let staged_program = staging.path().join(program.directory());
    let source_program = corpus.program_root(program);
    if let Some(overlay) = overlay {
        overlay.materialize(&source_program, &staged_program)?;
    } else {
        copy_reference_tree(&source_program, &staged_program)?;
    }
    let staged_entry = staged_program.join(&entry_path);
    if !staged_entry.is_file() {
        return Err(AssemblyError::Reference(format!(
            "entry source does not exist: {entry}"
        )));
    }

    let mut command = Command::new(&config.executable);
    command.current_dir(&staged_program);
    if config.force_output {
        command.arg("--force");
    }
    command.arg(&entry_path);
    let output = command
        .output()
        .map_err(|source| AssemblyError::ReferenceIo {
            path: config.executable.clone(),
            source,
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let summary = parse_reference_summary(&stdout);

    if !output.status.success() {
        return Err(AssemblyError::Reference(format!(
            "yaYUL exited with status {}; unresolved={}, fatal={}, multiply-defined={}",
            output.status,
            summary.unresolved_symbols,
            summary.fatal_errors,
            summary.multiply_defined_symbols
        )));
    }
    if summary.unresolved_symbols != 0
        || summary.fatal_errors != 0
        || summary.multiply_defined_symbols != 0
    {
        return Err(AssemblyError::Reference(format!(
            "yaYUL diagnostics are not clean: unresolved={}, fatal={}, multiply-defined={}, warnings={}",
            summary.unresolved_symbols,
            summary.fatal_errors,
            summary.multiply_defined_symbols,
            summary.warnings
        )));
    }

    let rope_path = staged_program.join(format!("{}.bin", entry_path.to_string_lossy()));
    let rope = fs::read(&rope_path).map_err(|source| AssemblyError::ReferenceIo {
        path: rope_path.clone(),
        source,
    })?;
    let expected_bytes = ROPE_WORDS * 2;
    if rope.len() != expected_bytes {
        return Err(AssemblyError::Reference(format!(
            "rope has {} bytes; expected {expected_bytes}",
            rope.len()
        )));
    }
    let nonzero_words = rope
        .chunks_exact(2)
        .filter(|word| word[0] != 0 || word[1] != 0)
        .count();
    if nonzero_words < config.minimum_nonzero_words {
        return Err(AssemblyError::Reference(format!(
            "rope has only {nonzero_words} nonzero words; expected at least {} (likely checksum-only output)",
            config.minimum_nonzero_words
        )));
    }
    let historical_commit = corpus.manifest()?.historical_commit;
    let report = ReferenceAssemblyReport {
        schema_version: 1,
        program,
        entry: entry.to_owned(),
        historical_commit,
        toolchain: config.toolchain.clone(),
        force_output: config.force_output,
        exit_code: output.status.code(),
        unresolved_symbols: summary.unresolved_symbols,
        fatal_errors: summary.fatal_errors,
        warnings: summary.warnings,
        multiply_defined_symbols: summary.multiply_defined_symbols,
        rope_bytes: rope.len(),
        nonzero_words,
        rope_sha256: hex_sha256(&rope),
        stdout,
        stderr,
    };
    Ok(ReferenceAssembly { rope, report })
}

/// Parses and validates a VirtualAGC `*.binsource` octal rope listing in Rust.
///
/// This route exists for historical transcriptions such as Comanche 055 whose
/// `.agc` cards are not yet cleanly reassemblable. Every physical bank must be
/// present exactly once with 1,024 words and a valid positive or negative
/// bank-number checksum before bytes are emitted.
pub fn assemble_binsource_reference(
    corpus: &HistoricalCorpus,
    program: Program,
    binsource: &Path,
    toolchain: &str,
) -> Result<BinsourceAssembly, AssemblyError> {
    if toolchain.trim().is_empty() {
        return Err(AssemblyError::Reference(
            "binsource toolchain identity is empty".to_owned(),
        ));
    }
    let source = fs::read(binsource).map_err(|source| AssemblyError::ReferenceIo {
        path: binsource.to_path_buf(),
        source,
    })?;
    let text = std::str::from_utf8(&source)
        .map_err(|error| AssemblyError::Reference(format!("binsource is not UTF-8: {error}")))?;
    let mut banks = BTreeMap::<u8, Vec<AgcWord>>::new();
    let mut current_bank = None;
    let mut unused_words = 0;
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line
            .split_once(';')
            .map_or(raw_line, |(code, _)| code)
            .trim();
        if line.is_empty() {
            continue;
        }
        if let Some(raw_bank) = line.strip_prefix("BANK=") {
            let bank = u8::from_str_radix(raw_bank.trim(), 8).map_err(|_| {
                AssemblyError::Reference(format!(
                    "invalid octal bank at binsource line {line_number}: {raw_bank}"
                ))
            })?;
            if bank > 0o43 {
                return Err(AssemblyError::Reference(format!(
                    "bank {bank:o} at binsource line {line_number} is outside the Apollo 11 rope"
                )));
            }
            if banks.insert(bank, Vec::new()).is_some() {
                return Err(AssemblyError::Reference(format!(
                    "bank {bank:o} appears more than once"
                )));
            }
            current_bank = Some(bank);
            continue;
        }
        let bank = current_bank.ok_or_else(|| {
            AssemblyError::Reference(format!(
                "rope data precedes the first BANK card at line {line_number}"
            ))
        })?;
        let words = banks
            .get_mut(&bank)
            .expect("current bank was inserted before data");
        for token in line.split_whitespace() {
            let word = if token == "@" {
                unused_words += 1;
                AgcWord::POSITIVE_ZERO
            } else {
                if token.len() != 5 || !token.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
                    return Err(AssemblyError::Reference(format!(
                        "invalid rope word {token:?} at binsource line {line_number}"
                    )));
                }
                let raw = u16::from_str_radix(token, 8).expect("validated octal token");
                AgcWord::try_from_raw(raw).map_err(|error| {
                    AssemblyError::Reference(format!(
                        "invalid rope word at binsource line {line_number}: {error}"
                    ))
                })?
            };
            words.push(word);
            if words.len() > 1024 {
                return Err(AssemblyError::Reference(format!(
                    "bank {bank:o} contains more than 1,024 words"
                )));
            }
        }
    }

    let expected_banks = (0..36).map(|bank| bank as u8).collect::<BTreeSet<_>>();
    let actual_banks = banks.keys().copied().collect::<BTreeSet<_>>();
    if actual_banks != expected_banks {
        let missing = expected_banks
            .difference(&actual_banks)
            .map(|bank| format!("{bank:o}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AssemblyError::Reference(format!(
            "binsource does not contain exactly banks 00..43; missing [{missing}]"
        )));
    }

    let mut bank_checksums = Vec::with_capacity(36);
    for (&bank, words) in &banks {
        if words.len() != 1024 {
            return Err(AssemblyError::Reference(format!(
                "bank {bank:02o} contains {} words; expected 1,024",
                words.len()
            )));
        }
        let sum = words
            .iter()
            .copied()
            .fold(AgcWord::POSITIVE_ZERO, checksum_add);
        let positive = AgcWord::from_raw_truncate(u16::from(bank));
        let negative = positive.complement();
        let polarity = if sum == positive {
            "positive"
        } else if sum == negative {
            "negative"
        } else {
            return Err(AssemblyError::Reference(format!(
                "bank {bank:02o} checksum is {sum}, expected {positive} or {negative}"
            )));
        };
        bank_checksums.push(BinsourceBankChecksum {
            bank,
            sum,
            polarity: polarity.to_owned(),
        });
    }

    let mut rope = Vec::with_capacity(ROPE_WORDS * 2);
    for raw_bank in 0..36_u8 {
        let bank = if raw_bank < 4 { raw_bank ^ 2 } else { raw_bank };
        for word in &banks[&bank] {
            rope.extend_from_slice(&(word.raw() << 1).to_be_bytes());
        }
    }
    let nonzero_words = rope
        .chunks_exact(2)
        .filter(|word| word[0] != 0 || word[1] != 0)
        .count();
    let report = BinsourceAssemblyReport {
        schema_version: 1,
        program,
        historical_commit: corpus.manifest()?.historical_commit,
        binsource: binsource.display().to_string(),
        toolchain: toolchain.to_owned(),
        binsource_sha256: hex_sha256(&source),
        banks: banks.len(),
        words: banks.values().map(Vec::len).sum(),
        unused_words,
        bank_checksums,
        rope_bytes: rope.len(),
        nonzero_words,
        rope_sha256: hex_sha256(&rope),
    };
    Ok(BinsourceAssembly { rope, report })
}

fn checksum_add(left: AgcWord, right: AgcWord) -> AgcWord {
    let mut sum = left.to_i32_lossy_zero() + right.to_i32_lossy_zero();
    if sum > agc_word::MAX_MAGNITUDE {
        sum = sum - (agc_word::MAX_MAGNITUDE + 1) + 1;
    } else if sum < -agc_word::MAX_MAGNITUDE {
        sum = sum + (agc_word::MAX_MAGNITUDE + 1) - 1;
    }
    AgcWord::from_i32(sum).expect("folded checksum remains in one-word range")
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ReferenceSummary {
    unresolved_symbols: usize,
    fatal_errors: usize,
    warnings: usize,
    multiply_defined_symbols: usize,
}

fn parse_reference_summary(output: &str) -> ReferenceSummary {
    let mut summary = ReferenceSummary::default();
    for line in output.lines().map(str::trim) {
        if let Some(value) = summary_count(line, "Unresolved symbols:") {
            summary.unresolved_symbols = value;
        } else if let Some(value) = summary_count(line, "Fatal errors (final):") {
            summary.fatal_errors = value;
        } else if let Some(value) = summary_count(line, "Fatal errors:") {
            summary.fatal_errors = value;
        } else if let Some(value) = summary_count(line, "Warnings:") {
            summary.warnings = value;
        } else if let Some(value) = summary_count(line, "Multiply-defined symbols:") {
            summary.multiply_defined_symbols = value;
        }
    }
    summary
}

fn summary_count(line: &str, label: &str) -> Option<usize> {
    line.strip_prefix(label)?
        .trim()
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn validate_reference_entry(entry: &str) -> Result<PathBuf, AssemblyError> {
    let path = Path::new(entry);
    if entry.trim().is_empty()
        || path.is_absolute()
        || path.extension().is_none_or(|extension| extension != "agc")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AssemblyError::Reference(format!(
            "unsafe or non-AGC entry path: {entry}"
        )));
    }
    Ok(path.to_path_buf())
}

fn copy_reference_tree(source_root: &Path, output_root: &Path) -> Result<(), AssemblyError> {
    fs::create_dir(output_root).map_err(|source| AssemblyError::ReferenceIo {
        path: output_root.to_path_buf(),
        source,
    })?;
    let mut entries = fs::read_dir(source_root)
        .map_err(|source| AssemblyError::ReferenceIo {
            path: source_root.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| AssemblyError::ReferenceIo {
            path: source_root.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let source_path = entry.path();
        let output_path = output_root.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|source| AssemblyError::ReferenceIo {
                path: source_path.clone(),
                source,
            })?;
        if file_type.is_dir() {
            copy_reference_tree(&source_path, &output_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &output_path).map_err(|source| AssemblyError::ReferenceIo {
                path: output_path,
                source,
            })?;
        }
    }
    Ok(())
}

/// Expands source through includes and lowers every statement to typed IR.
pub fn expand_program(
    corpus: &HistoricalCorpus,
    program: Program,
    entry: &str,
    overlay: Option<&Overlay>,
) -> Result<ExpandedProgram, AssemblyError> {
    if let Some(overlay) = overlay {
        if overlay.program != program {
            return Err(AssemblyError::OverlayProgram {
                expected: program,
                actual: overlay.program,
            });
        }
    }
    let mut state = ExpansionState {
        corpus,
        program,
        overlay,
        cache: BTreeMap::new(),
        unit_order: Vec::new(),
        stack: Vec::new(),
        records: Vec::new(),
        diagnostics: Vec::new(),
        hashes: BTreeMap::new(),
    };
    state.expand(&normalize_entry(entry))?;
    let units = state
        .unit_order
        .iter()
        .filter_map(|path| state.cache.get(path).cloned())
        .collect();
    let ir = ProgramIr {
        program: program.directory().to_owned(),
        records: state.records,
        source_hashes: state.hashes.into_iter().collect(),
    };
    Ok(ExpandedProgram {
        program,
        units,
        ir,
        diagnostics: state.diagnostics,
    })
}

struct ExpansionState<'a> {
    corpus: &'a HistoricalCorpus,
    program: Program,
    overlay: Option<&'a Overlay>,
    cache: BTreeMap<String, SourceUnit>,
    unit_order: Vec<String>,
    stack: Vec<String>,
    records: Vec<IrRecord>,
    diagnostics: Vec<Diagnostic>,
    hashes: BTreeMap<String, String>,
}

impl ExpansionState<'_> {
    fn expand(&mut self, relative: &str) -> Result<(), AssemblyError> {
        if let Some(index) = self.stack.iter().position(|path| path == relative) {
            let mut cycle = self.stack[index..].to_vec();
            cycle.push(relative.to_owned());
            return Err(AssemblyError::IncludeCycle(cycle.join(" -> ")));
        }
        if !self.cache.contains_key(relative) {
            let id = SourceId::new(self.program, relative);
            let mut file = self.corpus.read(&id)?;
            let hash = hex_sha256(&file.bytes);
            if let Some(overlay) = self.overlay {
                file.bytes = overlay.apply_to_source(relative, &file.bytes)?;
            }
            let parsed =
                parse_file(&file).map_err(|error| AssemblyError::Parser(error.to_string()))?;
            self.append_parse_diagnostics(relative, &parsed.diagnostics);
            self.hashes.insert(relative.to_owned(), hash);
            self.unit_order.push(relative.to_owned());
            self.cache.insert(relative.to_owned(), parsed.unit);
        }
        self.stack.push(relative.to_owned());
        let unit = self.cache[relative].clone();
        for line in &unit.lines {
            match &line.kind {
                LineKind::Include { path, .. } => {
                    let aliased = self.overlay.map_or(path.text.as_str(), |overlay| {
                        overlay.resolve_include(&path.text)
                    });
                    let resolved =
                        resolve_relative_include(&unit.source, aliased).map_err(|error| {
                            AssemblyError::Parser(format!("{}:{}: {error}", relative, line.number))
                        })?;
                    self.expand(&resolved)?;
                }
                LineKind::Statement(statement) => {
                    let location = SourceLocation {
                        file: relative.to_owned(),
                        line: line.number,
                        span: line.span,
                    };
                    self.records.push(IrRecord {
                        location,
                        label: statement.label.as_ref().map(|label| label.text.clone()),
                        operation: statement.operation.text.to_ascii_uppercase(),
                        operand: parse_operand(
                            statement
                                .operand
                                .as_ref()
                                .map(|operand| operand.text.as_str()),
                        ),
                        kind: statement.kind,
                        bank: None,
                        offset: None,
                        word: None,
                    });
                }
                LineKind::Label { label, .. } => {
                    self.records.push(IrRecord {
                        location: SourceLocation {
                            file: relative.to_owned(),
                            line: line.number,
                            span: line.span,
                        },
                        label: Some(label.text.clone()),
                        operation: String::new(),
                        operand: Operand::None,
                        kind: StatementKind::Directive,
                        bank: None,
                        offset: None,
                        word: None,
                    });
                }
                _ => {}
            }
        }
        self.stack.pop();
        Ok(())
    }

    fn append_parse_diagnostics(&mut self, file: &str, diagnostics: &[ParseDiagnostic]) {
        self.diagnostics
            .extend(diagnostics.iter().map(|diagnostic| Diagnostic {
                severity: match diagnostic.severity {
                    ParseSeverity::Error => Severity::Error,
                    ParseSeverity::Warning => Severity::Warning,
                },
                code: diagnostic.code.to_owned(),
                message: diagnostic.message.clone(),
                location: Some(SourceLocation {
                    file: file.to_owned(),
                    line: diagnostic.line,
                    span: diagnostic.span,
                }),
            }));
    }
}

/// Assembles expanded IR. Any unsupported or ambiguous record becomes an
/// explicit error diagnostic and prevents returning a supposedly valid image.
pub fn assemble(expanded: ExpandedProgram) -> Result<ProgramImage, AssemblyError> {
    let mut ir = expanded.ir;
    let mut diagnostics = expanded.diagnostics;
    let mut symbols = SymbolTable::default();
    let mut location = LocationCounter::Fixed { bank: 2, offset: 0 };
    let mut equates = Vec::new();

    // Pass one: assign locations and memory symbols.
    for (index, record) in ir.records.iter_mut().enumerate() {
        if record.operation.is_empty() {
            if let Some(label) = &record.label {
                let (_, _, value) = location.current_value();
                if let Err(error) = symbols.define(label.clone(), value, record.location.clone()) {
                    symbol_diagnostic(&mut diagnostics, error);
                }
            }
            continue;
        }
        if is_equate(&record.operation) {
            if let Some(label) = &record.label {
                equates.push((index, label.clone()));
            } else {
                push_error(&mut diagnostics, "AGCA001", "equate has no label", record);
            }
            continue;
        }
        if apply_location_directive(record, &mut location, &symbols, &mut diagnostics) {
            continue;
        }
        let emits = emitted_word_count(record, &mut diagnostics);
        if emits == 0 {
            continue;
        }
        let (bank, offset, symbol_value) = location.current_value();
        record.bank = bank;
        record.offset = Some(offset);
        if let Some(label) = &record.label {
            if let Err(error) = symbols.define(label.clone(), symbol_value, record.location.clone())
            {
                symbol_diagnostic(&mut diagnostics, error);
            }
        }
        if let Err(message) = location.advance(emits as u16) {
            push_error(&mut diagnostics, "AGCA002", &message, record);
        }
    }

    // Resolve equates to a fixed point because historical sources chain them.
    let mut pending = equates;
    let mut progress = true;
    while progress && !pending.is_empty() {
        progress = false;
        pending.retain(|(index, label)| {
            let record = &ir.records[*index];
            match evaluate_operand(&record.operand, &symbols) {
                Ok(value) => {
                    let semantic = if (0..=0o77777).contains(&value) {
                        SymbolValue::Constant {
                            value: AgcWord::from_raw_truncate(value as u16),
                        }
                    } else {
                        SymbolValue::Absolute { value }
                    };
                    if let Err(error) =
                        symbols.define(label.clone(), semantic, record.location.clone())
                    {
                        symbol_diagnostic(&mut diagnostics, error);
                    }
                    progress = true;
                    false
                }
                Err(_) => true,
            }
        });
    }
    for (index, label) in pending {
        push_error(
            &mut diagnostics,
            "AGCA003",
            &format!("equate {label} cannot be resolved"),
            &ir.records[index],
        );
    }

    // Pass two: emit into physical bank order.
    let mut words = vec![AgcWord::POSITIVE_ZERO; ROPE_WORDS];
    let mut occupied = vec![false; ROPE_WORDS];
    let mut previous_extend = false;
    for record in &mut ir.records {
        let Some(bank) = record.bank else {
            previous_extend = false;
            continue;
        };
        let offset = record.offset.expect("assigned bank has offset");
        let emitted = emit_record(record, &mut symbols, previous_extend, &mut diagnostics);
        previous_extend = record.operation == "EXTEND";
        for (relative, word) in emitted.into_iter().enumerate() {
            let physical = usize::from(bank) * 1024 + usize::from(offset) + relative;
            if physical >= words.len() {
                push_error(
                    &mut diagnostics,
                    "AGCA004",
                    "emission lies outside installed rope",
                    record,
                );
                continue;
            }
            if occupied[physical] {
                push_error(
                    &mut diagnostics,
                    "AGCA005",
                    "two records emit to the same fixed location",
                    record,
                );
                continue;
            }
            words[physical] = word;
            occupied[physical] = true;
            if relative == 0 {
                record.word = Some(word);
            }
        }
    }

    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    if errors != 0 {
        return Err(AssemblyError::Diagnostics(errors));
    }
    Ok(ProgramImage {
        words,
        occupied,
        symbols,
        ir,
        diagnostics,
    })
}

/// Assembles a standalone parsed unit, useful for instruction-level tests and
/// generated conformance programs.
pub fn assemble_unit(unit: SourceUnit) -> Result<ProgramImage, AssemblyError> {
    let records = unit
        .statements()
        .map(|(line, statement)| IrRecord {
            location: SourceLocation {
                file: unit.source.relative_path.clone(),
                line: line.number,
                span: line.span,
            },
            label: statement.label.as_ref().map(|label| label.text.clone()),
            operation: statement.operation.text.to_ascii_uppercase(),
            operand: parse_operand(
                statement
                    .operand
                    .as_ref()
                    .map(|operand| operand.text.as_str()),
            ),
            kind: statement.kind,
            bank: None,
            offset: None,
            word: None,
        })
        .collect();
    assemble(ExpandedProgram {
        program: unit.source.program,
        units: vec![unit.clone()],
        ir: ProgramIr {
            program: unit.source.program.directory().to_owned(),
            records,
            source_hashes: Default::default(),
        },
        diagnostics: Vec::new(),
    })
}

#[derive(Clone, Copy, Debug)]
enum LocationCounter {
    Fixed { bank: u8, offset: u16 },
    Erasable { bank: u8, offset: u16 },
}

impl LocationCounter {
    fn current_value(self) -> (Option<u8>, u16, SymbolValue) {
        match self {
            Self::Fixed { bank, offset } => {
                (Some(bank), offset, SymbolValue::Fixed { bank, offset })
            }
            Self::Erasable { bank, offset } => {
                (None, offset, SymbolValue::Erasable { bank, offset })
            }
        }
    }

    fn advance(&mut self, amount: u16) -> Result<(), String> {
        match self {
            Self::Fixed { bank, offset } => {
                let next = u32::from(*offset) + u32::from(amount);
                if next > 1024 {
                    return Err(format!(
                        "fixed bank {bank:02o} overflow at offset {next:04o}"
                    ));
                }
                *offset = next as u16;
            }
            Self::Erasable { bank, offset } => {
                let next = u32::from(*offset) + u32::from(amount);
                if next > 256 {
                    return Err(format!(
                        "erasable bank E{bank} overflow at offset {next:04o}"
                    ));
                }
                *offset = next as u16;
            }
        }
        Ok(())
    }
}

fn apply_location_directive(
    record: &IrRecord,
    location: &mut LocationCounter,
    symbols: &SymbolTable,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    match record.operation.as_str() {
        "BANK" => {
            match evaluate_operand(&record.operand, symbols) {
                Ok(value) if (0..36).contains(&value) => {
                    *location = LocationCounter::Fixed {
                        bank: value as u8,
                        offset: 0,
                    };
                }
                _ => push_error(
                    diagnostics,
                    "AGCA010",
                    "BANK requires an installed fixed-bank number",
                    record,
                ),
            }
            true
        }
        "BLOCK" => {
            match evaluate_operand(&record.operand, symbols) {
                Ok(value) if (0..8).contains(&value) => {
                    *location = LocationCounter::Erasable {
                        bank: value as u8,
                        offset: 0,
                    };
                }
                _ => push_error(
                    diagnostics,
                    "AGCA011",
                    "BLOCK requires an erasable-bank number",
                    record,
                ),
            }
            true
        }
        "SETLOC" => {
            match evaluate_operand(&record.operand, symbols) {
                Ok(value) if (0..=0o7777).contains(&value) => {
                    let logical = value as u16;
                    *location = match logical {
                        0..=0o1377 => LocationCounter::Erasable {
                            bank: (logical >> 8) as u8,
                            offset: logical & 0o377,
                        },
                        0o1400..=0o1777 => LocationCounter::Erasable {
                            bank: 0,
                            offset: logical & 0o377,
                        },
                        0o2000..=0o3777 => LocationCounter::Fixed {
                            bank: 0,
                            offset: logical & 0o1777,
                        },
                        0o4000..=0o5777 => LocationCounter::Fixed {
                            bank: 2,
                            offset: logical & 0o1777,
                        },
                        _ => LocationCounter::Fixed {
                            bank: 3,
                            offset: logical & 0o1777,
                        },
                    };
                }
                _ => push_error(
                    diagnostics,
                    "AGCA012",
                    "SETLOC operand is unresolved or out of range",
                    record,
                ),
            }
            true
        }
        "MEMORY" | "COUNT" | "SUBRO" | "CHECK=" => true,
        _ => false,
    }
}

fn emitted_word_count(record: &IrRecord, diagnostics: &mut Vec<Diagnostic>) -> usize {
    if Mnemonic::parse(&record.operation).is_some() {
        1
    } else {
        match record.operation.as_str() {
            "OCT" | "DEC" | "ADRES" | "REMADR" | "ECADR" | "FCADR" | "CADR" | "GENADR"
            | "XCADR" | "BBCON" | "DNCHAN" | "VN" | "BNKSUM" => 1,
            "2OCT" | "2DEC" | "2CADR" | "2BCADR" | "1DNADR" => 2,
            "ERASE" => match &record.operand {
                Operand::Literal(value) if *value >= 0 => *value as usize + 1,
                Operand::None => 1,
                _ => {
                    push_error(
                        diagnostics,
                        "AGCA020",
                        "ERASE size must be a non-negative literal",
                        record,
                    );
                    0
                }
            },
            operation if is_equate(operation) => 0,
            _ => {
                let family = match record.kind {
                    StatementKind::Interpretive => "interpretive operation",
                    StatementKind::Directive => "directive",
                    StatementKind::Instruction => "instruction",
                    StatementKind::Unknown => "operation",
                };
                push_error(
                    diagnostics,
                    "AGCA021",
                    &format!("unsupported {family} {}", record.operation),
                    record,
                );
                0
            }
        }
    }
}

fn emit_record(
    record: &IrRecord,
    symbols: &mut SymbolTable,
    extended_context: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<AgcWord> {
    if let Some(mnemonic) = Mnemonic::parse(&record.operation) {
        let operand = if matches!(
            mnemonic,
            Mnemonic::Extend | Mnemonic::Relint | Mnemonic::Inhint | Mnemonic::Resume
        ) {
            0
        } else {
            match evaluate_and_reference(&record.operand, symbols, &record.location) {
                Ok(value) if (0..=0o7777).contains(&value) => value as u16,
                Ok(value) => {
                    push_error(
                        diagnostics,
                        "AGCA030",
                        &format!("instruction operand {value} is outside twelve bits"),
                        record,
                    );
                    return Vec::new();
                }
                Err(message) => {
                    push_error(diagnostics, "AGCA031", &message, record);
                    return Vec::new();
                }
            }
        };
        if mnemonic.is_extended() && !extended_context {
            push_error(
                diagnostics,
                "AGCA032",
                &format!("{} requires a preceding EXTEND", mnemonic),
                record,
            );
            return Vec::new();
        }
        return match encode_with_context(mnemonic, operand, extended_context) {
            Ok(word) => vec![word],
            Err(error) => {
                push_error(diagnostics, "AGCA033", &error.to_string(), record);
                Vec::new()
            }
        };
    }
    let mut value = || evaluate_and_reference(&record.operand, symbols, &record.location);
    match record.operation.as_str() {
        "OCT" => single_word(value(), record, diagnostics),
        "DEC" => signed_word(value(), record, diagnostics),
        "2OCT" => double_words(value(), false, record, diagnostics),
        "2DEC" => double_words(value(), true, record, diagnostics),
        "ADRES" | "REMADR" | "ECADR" | "FCADR" | "CADR" | "GENADR" | "XCADR" | "BBCON"
        | "DNCHAN" | "VN" => single_word(value(), record, diagnostics),
        "2CADR" | "2BCADR" | "1DNADR" => {
            let first = single_word(value(), record, diagnostics);
            first
                .first()
                .map_or_else(Vec::new, |word| vec![*word, AgcWord::POSITIVE_ZERO])
        }
        "ERASE" => vec![AgcWord::POSITIVE_ZERO; emitted_word_count(record, diagnostics)],
        "BNKSUM" => vec![AgcWord::POSITIVE_ZERO],
        _ => Vec::new(),
    }
}

fn evaluate_operand(operand: &Operand, symbols: &SymbolTable) -> Result<i64, String> {
    match operand {
        Operand::None => Ok(0),
        Operand::Literal(value) => Ok(*value),
        Operand::Symbol { name, offset } => {
            let symbol = symbols
                .get(name)
                .ok_or_else(|| format!("unresolved symbol {name}"))?;
            let base = match symbol.value {
                SymbolValue::Constant { value } => i64::from(value.raw()),
                SymbolValue::Absolute { value } => value,
                _ => i64::from(
                    symbol
                        .value
                        .logical_address()
                        .ok_or_else(|| format!("symbol {name} has no usable logical address"))?,
                ),
            };
            Ok(base + i64::from(*offset))
        }
        Operand::Expression(expression) => evaluate_expression(expression, symbols),
    }
}

fn evaluate_and_reference(
    operand: &Operand,
    symbols: &mut SymbolTable,
    location: &SourceLocation,
) -> Result<i64, String> {
    if let Operand::Symbol { name, offset } = operand {
        let value = symbols
            .reference(name, location.clone())
            .map_err(|error| error.to_string())?;
        let base = match value {
            SymbolValue::Constant { value } => i64::from(value.raw()),
            SymbolValue::Absolute { value } => value,
            _ => i64::from(
                value
                    .logical_address()
                    .ok_or_else(|| format!("symbol {name} has no logical address"))?,
            ),
        };
        Ok(base + i64::from(*offset))
    } else {
        evaluate_operand(operand, symbols)
    }
}

fn evaluate_expression(expression: &str, symbols: &SymbolTable) -> Result<i64, String> {
    let tokens = expression.split_whitespace().collect::<Vec<_>>();
    if tokens.len() == 3 && matches!(tokens[1], "+" | "-") {
        let left = parse_atom(tokens[0], symbols)?;
        let right = parse_atom(tokens[2], symbols)?;
        Ok(if tokens[1] == "+" {
            left + right
        } else {
            left - right
        })
    } else {
        parse_atom(expression.trim(), symbols)
    }
}

fn parse_atom(atom: &str, symbols: &SymbolTable) -> Result<i64, String> {
    if let Some(value) = parse_numeric(atom) {
        Ok(value)
    } else {
        evaluate_operand(
            &Operand::Symbol {
                name: atom.to_owned(),
                offset: 0,
            },
            symbols,
        )
    }
}

fn parse_numeric(input: &str) -> Option<i64> {
    let (negative, digits) = input
        .strip_prefix('-')
        .map_or((false, input), |digits| (true, digits));
    let (radix, digits) = if let Some(value) = digits.strip_suffix('D') {
        (10, value)
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

fn single_word(
    value: Result<i64, String>,
    record: &IrRecord,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<AgcWord> {
    match value {
        Ok(value) if (0..=0o77777).contains(&value) => {
            vec![AgcWord::from_raw_truncate(value as u16)]
        }
        Ok(value) => {
            push_error(
                diagnostics,
                "AGCA040",
                &format!("word value {value} is out of range"),
                record,
            );
            Vec::new()
        }
        Err(message) => {
            push_error(diagnostics, "AGCA041", &message, record);
            Vec::new()
        }
    }
}

fn signed_word(
    value: Result<i64, String>,
    record: &IrRecord,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<AgcWord> {
    match value.and_then(|value| i32::try_from(value).map_err(|error| error.to_string())) {
        Ok(value) => match AgcWord::from_i32(value) {
            Ok(word) => vec![word],
            Err(error) => {
                push_error(diagnostics, "AGCA042", &error.to_string(), record);
                Vec::new()
            }
        },
        Err(message) => {
            push_error(diagnostics, "AGCA043", &message, record);
            Vec::new()
        }
    }
}

fn double_words(
    value: Result<i64, String>,
    signed: bool,
    record: &IrRecord,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<AgcWord> {
    let value = match value {
        Ok(value) => value,
        Err(message) => {
            push_error(diagnostics, "AGCA044", &message, record);
            return Vec::new();
        }
    };
    if signed {
        return match agc_word::AgcDoubleWord::from_i64(value) {
            Ok(double) => vec![double.high, double.low],
            Err(error) => {
                push_error(diagnostics, "AGCA045", &error.to_string(), record);
                Vec::new()
            }
        };
    }
    if !(0..=0x0fff_ffff).contains(&value) {
        push_error(
            diagnostics,
            "AGCA046",
            "double-octal value is out of range",
            record,
        );
        return Vec::new();
    }
    vec![
        AgcWord::from_raw_truncate(((value as u32 >> 14) & 0o37777) as u16),
        AgcWord::from_raw_truncate((value as u32 & 0o37777) as u16),
    ]
}

fn is_equate(operation: &str) -> bool {
    matches!(operation, "=" | "EQUALS")
}

fn push_error(diagnostics: &mut Vec<Diagnostic>, code: &str, message: &str, record: &IrRecord) {
    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        code: code.to_owned(),
        message: message.to_owned(),
        location: Some(record.location.clone()),
    });
}

fn symbol_diagnostic(diagnostics: &mut Vec<Diagnostic>, error: SymbolError) {
    let location = match &error {
        SymbolError::Duplicate { second, .. }
        | SymbolError::Unresolved {
            location: second, ..
        } => Some(second.clone()),
    };
    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        code: "AGCA050".to_owned(),
        message: error.to_string(),
        location,
    });
}

fn normalize_entry(entry: &str) -> String {
    Path::new(entry)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_parser::parse_str;

    #[test]
    fn native_assembler_resolves_forward_transfer() {
        let source = SourceId::new(Program::Luminary099, "TEST.agc");
        let unit = parse_str(
            source,
            "\tBANK\t2\nSTART\tTC\tEND\n\tOCT\t12345\nEND\tTC\tSTART\n",
        )
        .unit;
        let image = assemble_unit(unit).unwrap();
        assert_eq!(image.occupied_words(), 3);
        assert_eq!(image.words[2 * 1024].raw(), 0o4002);
        assert_eq!(image.words[2 * 1024 + 1].raw(), 0o12345);
        assert_eq!(image.words[2 * 1024 + 2].raw(), 0o4000);
    }

    #[test]
    fn reference_summary_uses_final_counts() {
        let output = "\
Unresolved symbols:  0\n\
Fatal errors:  3\n\
Warnings:  1\n\
Multiply-defined symbols:  0\n\
Fatal errors (final):  0\n";
        assert_eq!(
            parse_reference_summary(output),
            ReferenceSummary {
                unresolved_symbols: 0,
                fatal_errors: 0,
                warnings: 1,
                multiply_defined_symbols: 0,
            }
        );
    }

    #[test]
    fn reference_entry_cannot_escape_staging() {
        assert_eq!(
            validate_reference_entry("MAIN.agc").unwrap(),
            PathBuf::from("MAIN.agc")
        );
        assert!(validate_reference_entry("../MAIN.agc").is_err());
        assert!(validate_reference_entry("MAIN.txt").is_err());
    }
}
