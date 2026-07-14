#![forbid(unsafe_code)]
//! Provenance-rich reproducible research artifacts and paper-ready measured tables.

use agc_mission::MissionRun;
use agc_source::{HistoricalCorpus, Program, SourceManifest};
use agc_symbols::{SymbolTable, SymbolValue};
use agc_trace::TraceLog;
use agc_validation::ValidationReport;
use agc_xref::GraphArtifact;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Current report envelope schema.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Provenance required on every generated artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Historical source commit.
    pub historical_commit: String,
    /// Pinned external reference identifier.
    pub reference_toolchain: String,
    /// `ApolloRS` commit or explicit dirty-worktree marker.
    pub apollors_commit: String,
    /// Sorted input `path=sha256` records.
    pub input_hashes: Vec<String>,
    /// Reproduction command.
    pub generation_command: String,
    /// RFC 3339 generation time.
    pub generated_at: String,
    /// Known limitations applying to this artifact.
    pub known_limitations: Vec<String>,
}

impl Provenance {
    /// Creates measured provenance from repository state and manifest.
    pub fn capture(
        repository_root: &Path,
        historical_manifest: &SourceManifest,
        reference_toolchain: impl Into<String>,
        generation_command: impl Into<String>,
        known_limitations: Vec<String>,
    ) -> Self {
        let historical_commit = historical_manifest
            .historical_commit
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let mut input_hashes = historical_manifest
            .entries
            .iter()
            .map(|entry| format!("{}={}", entry.source, entry.sha256))
            .collect::<Vec<_>>();
        input_hashes.sort();
        Self {
            historical_commit,
            reference_toolchain: reference_toolchain.into(),
            apollors_commit: repository_revision(repository_root),
            input_hashes,
            generation_command: generation_command.into(),
            generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            known_limitations,
        }
    }

    /// Adds a content-addressed file input to the provenance record.
    pub fn record_input_file(
        &mut self,
        label: impl AsRef<str>,
        path: impl AsRef<Path>,
    ) -> Result<(), ReportError> {
        self.input_hashes.push(format!(
            "{}={}",
            label.as_ref(),
            file_sha256(path.as_ref())?
        ));
        self.input_hashes.sort();
        self.input_hashes.dedup();
        Ok(())
    }
}

/// Self-describing artifact wrapper.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Envelope schema.
    pub schema_version: u32,
    /// Stable artifact kind.
    pub artifact_kind: String,
    /// Provenance.
    pub provenance: Provenance,
    /// Measured payload.
    pub data: T,
}

impl<T> Envelope<T> {
    /// Wraps measured data.
    pub fn new(kind: impl Into<String>, provenance: Provenance, data: T) -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            artifact_kind: kind.into(),
            provenance,
            data,
        }
    }
}

/// Per-program repository inventory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramInventory {
    /// Program.
    pub program: Program,
    /// Number of source files.
    pub files: usize,
    /// Exact source bytes.
    pub bytes: u64,
    /// Logical source lines.
    pub lines: u64,
}

/// Complete repository inventory derived from the source manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepositoryInventory {
    /// Program summaries.
    pub programs: Vec<ProgramInventory>,
    /// Total `.agc` files.
    pub total_files: usize,
    /// Total bytes.
    pub total_bytes: u64,
    /// Total logical lines.
    pub total_lines: u64,
}

/// One memory-map/symbol row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryMapRow {
    /// Symbol name.
    pub symbol: String,
    /// Region (`fixed`, `erasable`, `constant`, or `absolute`).
    pub region: String,
    /// Bank when applicable.
    pub bank: Option<u8>,
    /// Offset/value.
    pub value: i64,
    /// Source definition.
    pub definition: String,
    /// Number of references.
    pub references: usize,
}

/// Measured evaluation row suitable for a paper table.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvaluationRow {
    /// Scenario.
    pub scenario: String,
    /// Committed instructions.
    pub instructions: u64,
    /// Machine cycles.
    pub cycles: u64,
    /// Sampled visualization frames.
    pub frames: usize,
    /// Applied faults.
    pub faults: usize,
    /// Differential trace result, if paired execution was performed.
    pub trace_equivalent: Option<bool>,
    /// First divergent event, if one exists.
    pub first_divergence: Option<usize>,
}

/// Artifact I/O or validation failure.
#[derive(Debug, Error)]
pub enum ReportError {
    /// Filesystem failure.
    #[error("artifact I/O error at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// JSON serialization failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Artifact schema/provenance failure.
    #[error("invalid artifact: {0}")]
    Invalid(String),
    /// Source inventory failure.
    #[error("source inventory failed: {0}")]
    Source(String),
}

/// Computes inventory totals from exact manifest records.
pub fn repository_inventory(manifest: &SourceManifest) -> RepositoryInventory {
    let programs = [Program::Comanche055, Program::Luminary099]
        .into_iter()
        .map(|program| {
            let entries = manifest
                .entries
                .iter()
                .filter(|entry| entry.source.program == program)
                .collect::<Vec<_>>();
            ProgramInventory {
                program,
                files: entries.len(),
                bytes: entries.iter().map(|entry| entry.bytes).sum(),
                lines: entries.iter().map(|entry| entry.lines).sum(),
            }
        })
        .collect::<Vec<_>>();
    RepositoryInventory {
        total_files: manifest.entries.len(),
        total_bytes: manifest.entries.iter().map(|entry| entry.bytes).sum(),
        total_lines: manifest.entries.iter().map(|entry| entry.lines).sum(),
        programs,
    }
}

/// Generates a fresh source manifest and inventory using only Rust project logic.
pub fn inventory_corpus(
    corpus: &HistoricalCorpus,
) -> Result<(SourceManifest, RepositoryInventory), ReportError> {
    let manifest = corpus
        .manifest()
        .map_err(|error| ReportError::Source(error.to_string()))?;
    let inventory = repository_inventory(&manifest);
    Ok((manifest, inventory))
}

/// Computes a file SHA-256 without retaining the full input in memory.
pub fn file_sha256(path: impl AsRef<Path>) -> Result<String, ReportError> {
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| ReportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let bytes = file.read(&mut buffer).map_err(|source| ReportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if bytes == 0 {
            break;
        }
        digest.update(&buffer[..bytes]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Flattens the symbol table into deterministic memory-map rows.
pub fn memory_map(symbols: &SymbolTable) -> Vec<MemoryMapRow> {
    let mut rows = symbols
        .iter()
        .map(|(name, symbol)| {
            let (region, bank, value) = match symbol.value {
                SymbolValue::Fixed { bank, offset } => ("fixed", Some(bank), i64::from(offset)),
                SymbolValue::Erasable { bank, offset } => {
                    ("erasable", Some(bank), i64::from(offset))
                }
                SymbolValue::Constant { value } => ("constant", None, i64::from(value.raw())),
                SymbolValue::Absolute { value } => ("absolute", None, value),
            };
            MemoryMapRow {
                symbol: name.to_owned(),
                region: region.to_owned(),
                bank,
                value,
                definition: format!("{}:{}", symbol.definition.file, symbol.definition.line),
                references: symbol.references.len(),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    rows
}

/// Produces one measured evaluation row.
pub fn evaluation_row(
    mission: &MissionRun,
    validation: Option<&ValidationReport>,
) -> EvaluationRow {
    EvaluationRow {
        scenario: mission.scenario.clone(),
        instructions: mission.instructions,
        cycles: mission.cycles,
        frames: mission.frames.len(),
        faults: mission.faults_applied,
        trace_equivalent: validation.map(|report| report.equivalent),
        first_divergence: validation
            .and_then(|report| report.first.as_ref().map(|divergence| divergence.event)),
    }
}

/// Renders paper-ready Markdown exclusively from measured rows.
pub fn render_evaluation_markdown(rows: &[EvaluationRow]) -> String {
    let mut output = String::from(
        "| Scenario | Instructions | Cycles | Frames | Faults | Trace equivalent | First divergence |\n\
         |---|---:|---:|---:|---:|:---:|---:|\n",
    );
    for row in rows {
        let _ = writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} |",
            row.scenario.replace('|', "\\|"),
            row.instructions,
            row.cycles,
            row.frames,
            row.faults,
            row.trace_equivalent
                .map_or("not run".to_owned(), |value| value.to_string()),
            row.first_divergence
                .map_or("—".to_owned(), |value| value.to_string())
        );
    }
    output
}

/// Writes a pretty, deterministic JSON envelope and returns its SHA-256.
pub fn write_json<T: Serialize>(
    path: impl AsRef<Path>,
    envelope: &Envelope<T>,
) -> Result<String, ReportError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ReportError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(envelope)?;
    bytes.push(b'\n');
    fs::write(path, &bytes).map_err(|source| ReportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Reads and validates a typed envelope.
pub fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<Envelope<T>, ReportError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| ReportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let envelope: Envelope<T> = serde_json::from_slice(&bytes)?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

/// Validates schema and all mandatory provenance fields.
pub fn validate_envelope<T>(envelope: &Envelope<T>) -> Result<(), ReportError> {
    if envelope.schema_version != REPORT_SCHEMA_VERSION {
        return Err(ReportError::Invalid(format!(
            "schema {} is unsupported",
            envelope.schema_version
        )));
    }
    if envelope.artifact_kind.trim().is_empty()
        || envelope.provenance.historical_commit.trim().is_empty()
        || envelope.provenance.reference_toolchain.trim().is_empty()
        || envelope.provenance.apollors_commit.trim().is_empty()
        || envelope.provenance.generation_command.trim().is_empty()
        || envelope.provenance.generated_at.trim().is_empty()
    {
        return Err(ReportError::Invalid(
            "mandatory provenance field is empty".to_owned(),
        ));
    }
    DateTime::parse_from_rfc3339(&envelope.provenance.generated_at)
        .map_err(|error| ReportError::Invalid(format!("generated_at is invalid: {error}")))?;
    Ok(())
}

/// Generic artifact-file validator used by the CLI.
pub fn validate_artifact_file(path: impl AsRef<Path>) -> Result<(), ReportError> {
    let envelope: Envelope<Value> = read_json(path)?;
    validate_envelope(&envelope)
}

/// Returns a stable summary for a trace without claiming equivalence.
pub fn trace_summary(trace: &TraceLog) -> Value {
    let final_event = trace.events.last();
    serde_json::json!({
        "events": trace.events.len(),
        "cycles": final_event.map_or(0, |event| event.cycle_end),
        "final_pc": final_event.map(|event| event.after.z),
        "memory_accesses": trace.events.iter().map(|event| event.memory.len()).sum::<usize>(),
        "io_operations": trace.events.iter().map(|event| event.io.len()).sum::<usize>(),
        "interrupt_events": trace.events.iter().map(|event| event.interrupts.len()).sum::<usize>()
    })
}

/// Wraps a graph payload with a stable artifact kind.
pub fn graph_envelope(
    kind: &str,
    provenance: Provenance,
    graph: GraphArtifact,
) -> Envelope<GraphArtifact> {
    Envelope::new(kind, provenance, graph)
}

fn repository_revision(root: &Path) -> String {
    let revision = Command::new("git")
        .args(["-C", root.to_string_lossy().as_ref(), "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unversioned".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );
    let dirty = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "status",
            "--porcelain",
        ])
        .output()
        .ok()
        .is_some_and(|output| output.status.success() && !output.stdout.is_empty());
    if dirty {
        format!("{revision}-dirty")
    } else {
        revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_source::{ManifestEntry, SourceId};

    #[test]
    fn inventory_is_computed_not_hard_coded() {
        let manifest = SourceManifest {
            historical_commit: Some("a".repeat(40)),
            entries: vec![ManifestEntry {
                source: SourceId::new(Program::Luminary099, "MAIN.agc"),
                bytes: 10,
                lines: 2,
                sha256: "b".repeat(64),
            }],
        };
        let inventory = repository_inventory(&manifest);
        assert_eq!(inventory.total_bytes, 10);
        assert_eq!(inventory.programs[1].files, 1);
    }

    #[test]
    fn paper_table_marks_unrun_validation_honestly() {
        let row = EvaluationRow {
            scenario: "test".to_owned(),
            instructions: 1,
            cycles: 2,
            frames: 1,
            faults: 0,
            trace_equivalent: None,
            first_divergence: None,
        };
        assert!(render_evaluation_markdown(&[row]).contains("not run"));
    }

    #[test]
    fn file_hashes_are_streamed_and_recorded() {
        let path =
            std::env::temp_dir().join(format!("apollors-report-hash-{}.txt", std::process::id()));
        fs::write(&path, b"ApolloRS\n").unwrap();
        assert_eq!(
            file_sha256(&path).unwrap(),
            "f30bf0eee6eea0186ac80fc8253db29769d5b217b89ad7e30136ef8ca5415497"
        );

        let manifest = SourceManifest {
            historical_commit: Some("a".repeat(40)),
            entries: Vec::new(),
        };
        let mut provenance = Provenance::capture(
            Path::new("."),
            &manifest,
            "reference",
            "command",
            Vec::new(),
        );
        provenance.record_input_file("fixture", &path).unwrap();
        assert_eq!(
            provenance.input_hashes,
            vec![format!("fixture={}", file_sha256(&path).unwrap())]
        );
        fs::remove_file(path).unwrap();
    }
}
