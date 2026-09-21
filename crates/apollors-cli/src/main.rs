#![forbid(unsafe_code)]
//! `ApolloRS` command-line research, execution, validation, and DSKY interface.

use agc_assembler::{
    AssemblyError, ReferenceAssemblerConfig, assemble, assemble_binsource_reference,
    assemble_reference, expand_program,
};
use agc_conformance::{execute_suite, generate_block_ii_suite};
use agc_coverage::analyze_json_lines;
use agc_cpu::Cpu;
use agc_dsky::{DskyState, Key};
use agc_experiments::{FaultMatrixSpec, run_luminary_p63_fault_matrix};
use agc_faults::{Fault, compare_recovery};
use agc_loader::{RopeFormat, encode_yayul, load_file};
use agc_mission::{MissionController, MissionRun, MissionScenario, compare_missions};
use agc_overlay::Overlay;
use agc_reports::{
    Envelope, Provenance, file_sha256, graph_envelope, inventory_corpus, memory_map, trace_summary,
    write_json,
};
use agc_runtime::{Runtime, RuntimeEvent};
use agc_source::{HistoricalCorpus, Program};
use agc_trace::TraceLog;
use agc_transpiler::{Style, VerificationStatus, compile_check, generate, write_generated};
use agc_validation::{YaAgcReferenceTrace, compare_traces, compare_yaagc_reference};
use agc_xref::{call_graph, include_graph};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

const REFERENCE_TOOLCHAIN: &str =
    "VirtualAGC 0b13e5976dbc3c6c76aeab35195135261d7999ff; yaYUL 20260713";

#[derive(Debug, Parser)]
#[command(
    name = "apollors",
    version,
    about = "ApolloRS AGC research and execution system"
)]
struct Cli {
    /// Repository root used for provenance and default paths.
    #[arg(long, global = true, default_value = ".")]
    repository: PathBuf,
    /// Historical Apollo-11 checkout.
    #[arg(long, global = true, default_value = "historical/Apollo-11")]
    historical: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate Rust-native repository/source/include forensics.
    Forensics {
        /// Artifact output directory.
        #[arg(long, default_value = "artifacts/generated")]
        output: PathBuf,
    },
    /// Verify current historical bytes against a JSON source-manifest envelope.
    VerifySource {
        /// Previously generated source-manifest JSON.
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Parse and include-expand one historical flight program.
    Parse {
        #[arg(value_enum)]
        program: ProgramArg,
        /// Entry source relative to the program directory.
        #[arg(long, default_value = "MAIN.agc")]
        entry: String,
        /// Optional compatibility overlay JSON.
        #[arg(long)]
        overlay: Option<PathBuf>,
        /// Write full typed IR JSON here.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Validate or materialize a compatibility overlay outside historical source.
    Overlay {
        #[command(subcommand)]
        command: OverlayCommand,
    },
    /// Assemble with native Rust semantics or an isolated pinned yaYUL reference.
    Assemble {
        #[arg(value_enum)]
        program: ProgramArg,
        #[arg(long, default_value = "MAIN.agc")]
        entry: String,
        #[arg(long)]
        overlay: Option<PathBuf>,
        /// Pinned yaYUL executable; enables strict reference integration.
        #[arg(long, conflicts_with = "reference_binsource")]
        reference_yayul: Option<PathBuf>,
        /// Independently proofed `VirtualAGC` octal listing, parsed and checked in Rust.
        #[arg(long, conflicts_with = "reference_yayul")]
        reference_binsource: Option<PathBuf>,
        /// Exact reference toolchain commit/version recorded in the report.
        #[arg(long)]
        reference_toolchain: Option<String>,
        /// Ask yaYUL to emit despite its internal errors; `ApolloRS` still rejects them.
        #[arg(long, requires = "reference_yayul")]
        force_reference: bool,
        /// Output standard yaYUL-order rope image.
        #[arg(long)]
        output: PathBuf,
        /// Reference build report (defaults beside the rope image).
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Execute a strict rope image and optionally write a JSON-lines trace.
    Execute {
        #[arg(long)]
        rope: PathBuf,
        #[arg(long, value_enum, default_value = "yayul")]
        format: FormatArg,
        #[arg(long, default_value_t = 10_000)]
        instructions: u64,
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Measure dynamic machine coverage from a validated `ApolloRS` trace.
    Coverage {
        /// `ApolloRS` architectural JSON-lines trace.
        #[arg(long)]
        trace: PathBuf,
        /// Provenance-bearing coverage report.
        #[arg(long)]
        output: PathBuf,
    },
    /// Generate and execute the synthetic Block II semantic conformance rope.
    Conformance {
        /// Directory receiving the rope, traces, manifest, logs, and report.
        #[arg(long, default_value = "artifacts/generated/block-ii-conformance")]
        output_dir: PathBuf,
        /// Optional pinned, instrumented yaAGC executable for independent comparison.
        #[arg(long)]
        yaagc: Option<PathBuf>,
    },
    /// Compare two `ApolloRS` JSON-lines traces under the complete trace schema.
    Validate {
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare an `ApolloRS` JSON-lines trace with pinned yaAGC exact-trace TSV.
    ValidateReference {
        /// `ApolloRS` architectural JSON-lines trace.
        #[arg(long)]
        apollors: PathBuf,
        /// Twelve-column exact yaAGC TSV from the documented instrumentation.
        #[arg(long)]
        reference: PathBuf,
        /// Accept either stream as a fully matched but incomplete prefix.
        #[arg(long)]
        allow_prefix: bool,
        /// Optional machine-readable validation report.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Generate standalone provenance-preserving Rust from assembled IR.
    Transpile {
        #[arg(value_enum)]
        program: ProgramArg,
        #[arg(long, default_value = "MAIN.agc")]
        entry: String,
        #[arg(long)]
        overlay: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "faithful")]
        style: StyleArg,
        #[arg(long)]
        output: PathBuf,
        /// Compile-check generated source with this rustc binary.
        #[arg(long)]
        rustc: Option<PathBuf>,
    },
    /// Execute the real Luminary P63-request mission profile.
    Mission {
        #[arg(long)]
        rope: PathBuf,
        #[arg(long, value_enum, default_value = "yayul")]
        format: FormatArg,
        /// Override scenario instruction budget.
        #[arg(long)]
        instructions: Option<u64>,
        /// Optional output JSON.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Optional JSON-lines architectural trace.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Optional rope fault as BANK:OFFSET:MASK in octal.
        #[arg(long)]
        rope_fault: Option<String>,
    },
    /// Run paired nominal/faulted P63 scenarios and report exact divergence.
    FaultCampaign {
        #[arg(long)]
        rope: PathBuf,
        #[arg(long, value_enum, default_value = "yayul")]
        format: FormatArg,
        /// Instruction boundary at which the rope bit flip is applied.
        #[arg(long)]
        at_instruction: u64,
        /// Rope fault as BANK:OFFSET:MASK in octal.
        #[arg(long)]
        rope_fault: String,
        /// Shared mission instruction limit.
        #[arg(long, default_value_t = 300_000)]
        instructions: u64,
        /// Provenance-bearing paired campaign report.
        #[arg(long)]
        output: PathBuf,
    },
    /// Run a declared multiclass paired P63 fault matrix.
    FaultMatrix {
        /// Reference Luminary rope image.
        #[arg(long)]
        rope: PathBuf,
        /// Rope byte ordering.
        #[arg(long, value_enum, default_value = "yayul")]
        format: FormatArg,
        /// Versioned matrix declaration.
        #[arg(long, default_value = "experiments/p63-fault-matrix.json")]
        spec: PathBuf,
        /// Provenance-bearing matrix report.
        #[arg(
            long,
            default_value = "artifacts/generated/luminary099-p63-fault-matrix.json"
        )]
        output: PathBuf,
    },
    /// Run an interactive terminal DSKY/debugger against a real rope.
    Dsky {
        #[arg(long)]
        rope: PathBuf,
        #[arg(long, value_enum, default_value = "yayul")]
        format: FormatArg,
        /// Instructions executed after each keyboard command.
        #[arg(long, default_value_t = 20_000)]
        quantum: u64,
    },
    /// Validate a generated provenance envelope.
    ValidateArtifact {
        #[arg(long)]
        artifact: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum OverlayCommand {
    /// Validate schema, evidence, commit, and target files.
    Verify {
        #[arg(long)]
        overlay: PathBuf,
    },
    /// Copy one program into a new staging directory and apply aliases there.
    Materialize {
        #[arg(long)]
        overlay: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProgramArg {
    Comanche055,
    Luminary099,
}

impl From<ProgramArg> for Program {
    fn from(value: ProgramArg) -> Self {
        match value {
            ProgramArg::Comanche055 => Self::Comanche055,
            ProgramArg::Luminary099 => Self::Luminary099,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FormatArg {
    Yayul,
    YayulParity,
    Hardware,
    Physical,
}

impl From<FormatArg> for RopeFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Yayul => Self::Yayul,
            FormatArg::YayulParity => Self::YayulParity,
            FormatArg::Hardware => Self::Hardware,
            FormatArg::Physical => Self::PhysicalWords,
        }
    }
}

impl FormatArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Yayul => "yayul",
            Self::YayulParity => "yayul-parity",
            Self::Hardware => "hardware",
            Self::Physical => "physical",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum StyleArg {
    Faithful,
    Structured,
}

impl From<StyleArg> for Style {
    fn from(value: StyleArg) -> Self {
        match value {
            StyleArg::Faithful => Self::Faithful,
            StyleArg::Structured => Self::Structured,
        }
    }
}

impl StyleArg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Faithful => "faithful",
            Self::Structured => "structured",
        }
    }
}

const fn program_cli_name(program: Program) -> &'static str {
    match program {
        Program::Comanche055 => "comanche055",
        Program::Luminary099 => "luminary099",
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> Result<()> {
    let repository = absolute_from_current(&cli.repository)?;
    let historical = resolve_from(&repository, &cli.historical);
    let corpus = HistoricalCorpus::new(&historical);
    match cli.command {
        Command::Forensics { output } => {
            let output = resolve_from(&repository, &output);
            run_forensics(&repository, &corpus, &output)
        }
        Command::VerifySource { manifest } => {
            verify_source(&repository, &corpus, manifest.as_deref())
        }
        Command::Parse {
            program,
            entry,
            overlay,
            output,
        } => {
            let program = Program::from(program);
            let overlay = load_program_overlay(&repository, program, overlay.as_deref())?;
            let expanded = expand_program(&corpus, program, &entry, overlay.as_ref())?;
            println!(
                "{}: {} files, {} semantic records, {} diagnostics",
                expanded.program,
                expanded.units.len(),
                expanded.ir.records.len(),
                expanded.diagnostics.len()
            );
            let parse_errors = expanded
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == agc_assembler::Severity::Error)
                .count();
            println!("parser/include errors: {parse_errors}");
            if let Some(path) = output {
                let path = resolve_from(&repository, &path);
                let provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- parse {} --entry {} --output {}",
                        program_cli_name(program),
                        entry,
                        path.display()
                    ),
                    vec![
                        "This artifact is typed, include-expanded source IR; it is not proof that every record can be emitted by the native assembler.".to_owned(),
                    ],
                )?;
                write_json(
                    &path,
                    &Envelope::new(
                        "include-expanded-program-ir",
                        provenance,
                        expanded.ir.clone(),
                    ),
                )?;
            }
            if parse_errors != 0 {
                bail!("source expansion contains {parse_errors} error diagnostics");
            }
            Ok(())
        }
        Command::Overlay { command } => run_overlay(&repository, &corpus, command),
        Command::Assemble {
            program,
            entry,
            overlay,
            reference_yayul,
            reference_binsource,
            reference_toolchain,
            force_reference,
            output,
            report,
        } => {
            let program = Program::from(program);
            let overlay = load_program_overlay(&repository, program, overlay.as_deref())?;
            let output = resolve_from(&repository, &output);
            if let Some(reference_yayul) = reference_yayul {
                let executable = resolve_from(&repository, &reference_yayul);
                let toolchain =
                    reference_toolchain.unwrap_or_else(|| REFERENCE_TOOLCHAIN.to_owned());
                let mut config = ReferenceAssemblerConfig::new(executable, toolchain);
                config.force_output = force_reference;
                let assembly =
                    assemble_reference(&corpus, program, &entry, overlay.as_ref(), &config)?;
                write_bytes(&output, &assembly.rope)?;
                let report = report.map_or_else(
                    || output.with_extension("build.json"),
                    |path| resolve_from(&repository, &path),
                );
                let mut provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- assemble {} --entry {} --reference-yayul {} --output {} --report {}",
                        program_cli_name(program),
                        entry,
                        config.executable.display(),
                        output.display(),
                        report.display()
                    ),
                    vec![
                        "This is an isolated external-reference build, not evidence that ApolloRS's native assembler emits the same rope.".to_owned(),
                    ],
                )?;
                provenance.reference_toolchain.clone_from(&config.toolchain);
                provenance.record_input_file("reference_yayul_executable", &config.executable)?;
                write_json(
                    &report,
                    &Envelope::new(
                        "reference-assembly-report",
                        provenance.clone(),
                        assembly.report.clone(),
                    ),
                )?;
                write_file_sidecar(
                    &output,
                    "reference-rope-image",
                    provenance,
                    serde_json::json!({
                        "program": program,
                        "bytes": assembly.report.rope_bytes,
                        "nonzero_words": assembly.report.nonzero_words,
                        "rope_sha256": assembly.report.rope_sha256,
                    }),
                )?;
                println!(
                    "wrote validated {}-byte reference rope ({} nonzero words, SHA-256 {}) to {}",
                    assembly.report.rope_bytes,
                    assembly.report.nonzero_words,
                    assembly.report.rope_sha256,
                    output.display()
                );
                println!("build report: {}", report.display());
            } else if let Some(reference_binsource) = reference_binsource {
                let binsource = resolve_from(&repository, &reference_binsource);
                let toolchain =
                    reference_toolchain.unwrap_or_else(|| REFERENCE_TOOLCHAIN.to_owned());
                let assembly =
                    assemble_binsource_reference(&corpus, program, &binsource, &toolchain)?;
                write_bytes(&output, &assembly.rope)?;
                let report = report.map_or_else(
                    || output.with_extension("build.json"),
                    |path| resolve_from(&repository, &path),
                );
                let mut provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- assemble {} --reference-binsource {} --output {} --report {}",
                        program_cli_name(program),
                        binsource.display(),
                        output.display(),
                        report.display()
                    ),
                    vec![
                        "This rope is imported from an independently proofed octal binsource after Rust-native bank/checksum validation; it is not a native assembly of the historical .agc transcription.".to_owned(),
                    ],
                )?;
                provenance.reference_toolchain = toolchain;
                provenance.record_input_file("reference_binsource", &binsource)?;
                write_json(
                    &report,
                    &Envelope::new(
                        "binsource-assembly-report",
                        provenance.clone(),
                        assembly.report.clone(),
                    ),
                )?;
                write_file_sidecar(
                    &output,
                    "checksum-validated-rope-image",
                    provenance,
                    serde_json::json!({
                        "program": program,
                        "bytes": assembly.report.rope_bytes,
                        "banks": assembly.report.banks,
                        "rope_sha256": assembly.report.rope_sha256,
                    }),
                )?;
                println!(
                    "wrote checksum-validated {}-bank binsource rope (SHA-256 {}) to {}",
                    assembly.report.banks,
                    assembly.report.rope_sha256,
                    output.display()
                );
                println!("build report: {}", report.display());
            } else {
                let expanded = expand_program(&corpus, program, &entry, overlay.as_ref())?;
                let image = assemble(expanded)?;
                let rope = image.to_yayul_bytes();
                write_bytes(&output, &rope)?;
                let mut provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- assemble {} --entry {} --output {}",
                        program_cli_name(program),
                        entry,
                        output.display()
                    ),
                    vec![
                        "Native assembly support is corpus-driven; equivalence to yaYUL must be established separately for this exact output.".to_owned(),
                    ],
                )?;
                "ApolloRS native assembler".clone_into(&mut provenance.reference_toolchain);
                write_file_sidecar(
                    &output,
                    "native-rope-image",
                    provenance,
                    serde_json::json!({
                        "program": program,
                        "words": image.words.len(),
                        "occupied_words": image.occupied_words(),
                        "rope_sha256": file_sha256(&output)?,
                    }),
                )?;
                println!(
                    "wrote {} words ({} source-occupied) to {}",
                    image.words.len(),
                    image.occupied_words(),
                    output.display()
                );
            }
            Ok(())
        }
        Command::Execute {
            rope,
            format,
            instructions,
            trace,
        } => {
            let rope = resolve_from(&repository, &rope);
            let trace = trace.as_deref().map(|path| resolve_from(&repository, path));
            let provenance = trace
                .as_ref()
                .map(|trace| {
                    let mut provenance = capture_provenance(
                        &repository,
                        &corpus,
                        format!(
                            "cargo run -p apollors-cli -- execute --rope {} --format {} --instructions {} --trace {}",
                            rope.display(),
                            format.as_str(),
                            instructions,
                            trace.display()
                        ),
                        vec![
                            "Execution alone is not a claim of behavioral equivalence; use validate-reference with a pinned yaAGC trace.".to_owned(),
                        ],
                    )?;
                    provenance.record_input_file("rope", &rope)?;
                    Ok::<_, anyhow::Error>(provenance)
                })
                .transpose()?;
            execute_rope(&rope, format.into(), instructions, trace, provenance)
        }
        Command::Coverage { trace, output } => {
            let trace = resolve_from(&repository, &trace);
            let output = resolve_from(&repository, &output);
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- coverage --trace {} --output {}",
                    trace.display(),
                    output.display()
                ),
                vec![
                    "Dynamic coverage describes only events observed in this exact trace and is not proof of unexecuted instruction or mission behavior.".to_owned(),
                    "Rope-fetch coverage uses all 36,864 installed fixed-memory words as its denominator, including constants and unused words that are not executable instructions.".to_owned(),
                    "Physical trace locations are not mapped back to historical source labels in this report.".to_owned(),
                ],
            )?;
            provenance.record_input_file("apollors_trace", &trace)?;
            write_coverage_report(&trace, &output, provenance)
        }
        Command::Conformance { output_dir, yaagc } => {
            let output_dir = resolve_from(&repository, &output_dir);
            let yaagc = yaagc.as_deref().map(|path| resolve_from(&repository, path));
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- conformance --output-dir {}{}",
                    output_dir.display(),
                    yaagc.as_ref().map_or(String::new(), |path| format!(
                        " --yaagc {}",
                        path.display()
                    ))
                ),
                vec![
                    "This is a synthetic Block II semantic rope, not historical Apollo 11 flight software; it complements rather than replaces the Luminary P63 execution evidence.".to_owned(),
                    "Local final-state assertions are specification-derived test vectors, not an independent implementation oracle.".to_owned(),
                    "The yaAGC oracle export compares transition kind, cycle, PC, instruction, A/L/Q, EB/FB/BB, and interrupt vector/number; final memory and channel obligations are checked separately by ApolloRS.".to_owned(),
                ],
            )?;
            if let Some(executable) = &yaagc {
                provenance.record_input_file("yaagc_executable", executable)?;
                provenance.record_input_file(
                    "yaagc_conformance_patch",
                    repository.join("docs/validation/yaagc-conformance-trace.patch"),
                )?;
            }
            run_conformance(&output_dir, yaagc.as_deref(), provenance)
        }
        Command::Validate {
            left,
            right,
            output,
        } => {
            let left = resolve_from(&repository, &left);
            let right = resolve_from(&repository, &right);
            let output = output
                .as_deref()
                .map(|path| resolve_from(&repository, path));
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- validate --left {} --right {}{}",
                    left.display(),
                    right.display(),
                    output
                        .as_ref()
                        .map_or(String::new(), |path| format!(" --output {}", path.display()))
                ),
                vec![
                    "This compares ApolloRS traces under the ApolloRS schema; it is not an independent implementation oracle.".to_owned(),
                ],
            )?;
            provenance.record_input_file("left_trace", &left)?;
            provenance.record_input_file("right_trace", &right)?;
            validate_traces(&left, &right, output, provenance)
        }
        Command::ValidateReference {
            apollors,
            reference,
            allow_prefix,
            output,
        } => {
            let apollors = resolve_from(&repository, &apollors);
            let reference = resolve_from(&repository, &reference);
            let output = output
                .as_deref()
                .map(|path| resolve_from(&repository, path));
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- validate-reference --apollors {} --reference {}{}{}",
                    apollors.display(),
                    reference.display(),
                    if allow_prefix { " --allow-prefix" } else { "" },
                    output
                        .as_ref()
                        .map_or(String::new(), |path| format!(" --output {}", path.display()))
                ),
                vec![
                    "The exact yaAGC instrumentation observes instruction/interrupt kind, cycle, PC, instruction, A/L/Q, EB/FB/BB, and interrupt vector/number; it does not compare every peripheral or memory cell.".to_owned(),
                    "A qualified common-prefix result means every event in the shorter stream matched; it does not claim that the longer stream was exhausted.".to_owned(),
                ],
            )?;
            provenance.record_input_file("apollors_trace", &apollors)?;
            provenance.record_input_file("yaagc_reference_trace", &reference)?;
            validate_reference_trace(&apollors, &reference, allow_prefix, output, provenance)
        }
        Command::Transpile {
            program,
            entry,
            overlay,
            style,
            output,
            rustc,
        } => {
            let program = Program::from(program);
            let overlay = load_program_overlay(&repository, program, overlay.as_deref())?;
            let expanded = expand_program(&corpus, program, &entry, overlay.as_ref())?;
            let image = assemble(expanded)?;
            let generated = generate(
                &image.ir,
                &image.symbols,
                style.into(),
                VerificationStatus::Unverified,
            )?;
            let output = resolve_from(&repository, &output);
            write_generated(&output, &generated)?;
            if let Some(rustc) = rustc {
                let library = output.with_extension("rlib");
                compile_check(rustc, &output, library)?;
            }
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- transpile {} --entry {} --style {} --output {}",
                    program_cli_name(program),
                    entry,
                    style.as_str(),
                    output.display()
                ),
                vec![
                    "Generated instruction dispatch preserves source/word provenance but remains unverified until paired differential execution is recorded.".to_owned(),
                    "The readable typed Pinball V37 model is maintained and tested in agc-dsky rather than generated by this whole-program instruction dispatcher.".to_owned(),
                ],
            )?;
            "ApolloRS native parser/assembler/transpiler"
                .clone_into(&mut provenance.reference_toolchain);
            write_file_sidecar(
                &output,
                "generated-rust-source",
                provenance,
                serde_json::json!({
                    "program": program,
                    "records": generated.records,
                    "style": format!("{:?}", generated.style).to_ascii_lowercase(),
                    "verification": format!("{:?}", generated.verification),
                }),
            )?;
            println!(
                "generated {} records at {}",
                generated.records,
                output.display()
            );
            Ok(())
        }
        Command::Mission {
            rope,
            format,
            instructions,
            output,
            trace,
            rope_fault,
        } => {
            let rope = resolve_from(&repository, &rope);
            let output = output
                .as_deref()
                .map(|path| resolve_from(&repository, path));
            let trace = trace.as_deref().map(|path| resolve_from(&repository, path));
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- mission --rope {} --format {}{}{}{}{}",
                    rope.display(),
                    format.as_str(),
                    instructions.map_or(String::new(), |value| format!(" --instructions {value}")),
                    output.as_ref().map_or(String::new(), |path| format!(" --output {}", path.display())),
                    trace.as_ref().map_or(String::new(), |path| format!(" --trace {}", path.display())),
                    rope_fault.as_ref().map_or(String::new(), |fault| format!(" --rope-fault {fault}"))
                ),
                vec![
                    "The Apollo 11 LM-5 pad-load document explicitly excludes mission-time computed quantities such as state vectors; this scenario applies a documented P63-relevant subset, not a complete mission erasable load.".to_owned(),
                    "REFSMFLG is set as an explicit aligned-flight precondition; ApolloRS does not simulate the preceding platform-alignment procedure.".to_owned(),
                    "No continuous vehicle, IMU, or landing-radar dynamics are coupled to this run, so P63 entry and initial landing-equation writes are demonstrated, not a complete powered-descent trajectory or landing.".to_owned(),
                ],
            )?;
            provenance.record_input_file("luminary_rope", &rope)?;
            run_mission(
                &rope,
                format.into(),
                instructions,
                output,
                trace,
                rope_fault.as_deref(),
                provenance,
            )
        }
        Command::FaultCampaign {
            rope,
            format,
            at_instruction,
            rope_fault,
            instructions,
            output,
        } => {
            let rope = resolve_from(&repository, &rope);
            let output = resolve_from(&repository, &output);
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- fault-campaign --rope {} --format {} --at-instruction {} --rope-fault {} --instructions {} --output {}",
                    rope.display(),
                    format.as_str(),
                    at_instruction,
                    rope_fault,
                    instructions,
                    output.display()
                ),
                vec![
                    "A rope bit flip is an adversarial deterministic experiment, not a claim that the selected fault has a measured Apollo hardware likelihood.".to_owned(),
                    "Recovery is classified only at the configured instruction horizon; a later convergence or divergence is outside this artifact.".to_owned(),
                    "The P63 fixture limitations concerning state vectors and vehicle/sensor dynamics also apply to both campaign arms.".to_owned(),
                ],
            )?;
            provenance.record_input_file("luminary_rope", &rope)?;
            run_fault_campaign(
                &rope,
                format.into(),
                at_instruction,
                &rope_fault,
                instructions,
                &output,
                provenance,
            )
        }
        Command::FaultMatrix {
            rope,
            format,
            spec,
            output,
        } => {
            let rope = resolve_from(&repository, &rope);
            let spec = resolve_from(&repository, &spec);
            let output = resolve_from(&repository, &output);
            let mut provenance = capture_provenance(
                &repository,
                &corpus,
                format!(
                    "cargo run -p apollors-cli -- fault-matrix --rope {} --format {} --spec {} --output {}",
                    rope.display(),
                    format.as_str(),
                    spec.display(),
                    output.display()
                ),
                vec![
                    "Fault cases are deterministic adversarial sensitivity experiments; the matrix does not estimate Apollo component failure probabilities or mission risk.".to_owned(),
                    "Every arm shares the bounded P63 fixture, including its incomplete pad load and absence of coupled vehicle, IMU, and landing-radar dynamics.".to_owned(),
                    "Outcome classes describe observations at the declared common instruction horizon; later recovery or divergence is outside the artifact.".to_owned(),
                    "The baseline is executed once and each fault arm starts from an identical cloned pre-execution controller.".to_owned(),
                ],
            )?;
            provenance.record_input_file("luminary_rope", &rope)?;
            provenance.record_input_file("fault_matrix_spec", &spec)?;
            run_fault_matrix(&rope, format.into(), &spec, &output, provenance)
        }
        Command::Dsky {
            rope,
            format,
            quantum,
        } => interactive_dsky(&resolve_from(&repository, &rope), format.into(), quantum),
        Command::ValidateArtifact { artifact } => {
            let artifact = resolve_from(&repository, &artifact);
            agc_reports::validate_artifact_file(&artifact)?;
            println!("valid artifact: {}", artifact.display());
            Ok(())
        }
    }
}

fn run_forensics(repository: &Path, corpus: &HistoricalCorpus, output: &Path) -> Result<()> {
    let (manifest, inventory) = inventory_corpus(corpus)?;
    let provenance = Provenance::capture(
        repository,
        &manifest,
        REFERENCE_TOOLCHAIN,
        "cargo run -p apollors-cli -- forensics",
        Vec::new(),
    );
    write_json(
        output.join("source-manifest.json"),
        &Envelope::new("source-manifest", provenance.clone(), manifest.clone()),
    )?;
    write_json(
        output.join("repository-inventory.json"),
        &Envelope::new("repository-inventory", provenance.clone(), inventory),
    )?;

    for program in [Program::Comanche055, Program::Luminary099] {
        let overlay = default_overlay(repository, program)?;
        let expanded = expand_program(corpus, program, "MAIN.agc", overlay.as_ref())?;
        let graph = include_graph(&expanded.units);
        write_json(
            output.join(format!(
                "{}-include-graph.json",
                program.directory().to_ascii_lowercase()
            )),
            &graph_envelope("include-graph", provenance.clone(), graph.clone()),
        )?;
        write_text(
            &output.join(format!(
                "{}-include-graph.dot",
                program.directory().to_ascii_lowercase()
            )),
            &graph.to_dot(program.directory()),
        )?;
        let dot_path = output.join(format!(
            "{}-include-graph.dot",
            program.directory().to_ascii_lowercase()
        ));
        write_file_sidecar(
            &dot_path,
            "include-graph-dot",
            provenance.clone(),
            serde_json::json!({"nodes": graph.nodes.len(), "edges": graph.edges.len()}),
        )?;
        write_json(
            output.join(format!(
                "{}-parse-diagnostics.json",
                program.directory().to_ascii_lowercase()
            )),
            &Envelope::new(
                "parse-diagnostics",
                provenance.clone(),
                expanded.diagnostics.clone(),
            ),
        )?;

        match assemble(expanded) {
            Ok(image) => {
                write_json(
                    output.join(format!(
                        "{}-memory-map.json",
                        program.directory().to_ascii_lowercase()
                    )),
                    &Envelope::new("memory-map", provenance.clone(), memory_map(&image.symbols)),
                )?;
                let calls = call_graph(&image.ir, &image.symbols);
                write_json(
                    output.join(format!(
                        "{}-call-graph.json",
                        program.directory().to_ascii_lowercase()
                    )),
                    &graph_envelope("call-graph", provenance.clone(), calls),
                )?;
            }
            Err(AssemblyError::Diagnostics { count, diagnostics }) => {
                let mut by_code = BTreeMap::<String, usize>::new();
                let mut by_message = BTreeMap::<String, usize>::new();
                for diagnostic in &diagnostics {
                    *by_code.entry(diagnostic.code.clone()).or_default() += 1;
                    *by_message.entry(diagnostic.message.clone()).or_default() += 1;
                }
                let mut frequent_messages = by_message.into_iter().collect::<Vec<_>>();
                frequent_messages
                    .sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
                frequent_messages.truncate(100);
                write_json(
                    output.join(format!(
                        "{}-native-assembly-status.json",
                        program.directory().to_ascii_lowercase()
                    )),
                    &Envelope::new(
                        "native-assembly-status",
                        provenance.clone(),
                        serde_json::json!({
                            "success": false,
                            "errors": count,
                            "diagnostics_by_code": by_code,
                            "most_frequent_diagnostics": frequent_messages,
                            "first_diagnostics": diagnostics.into_iter().take(100).collect::<Vec<_>>(),
                        }),
                    ),
                )?;
            }
            Err(error) => {
                write_json(
                    output.join(format!(
                        "{}-native-assembly-status.json",
                        program.directory().to_ascii_lowercase()
                    )),
                    &Envelope::new(
                        "native-assembly-status",
                        provenance.clone(),
                        serde_json::json!({"success": false, "error": error.to_string()}),
                    ),
                )?;
            }
        }
    }
    println!("generated Rust-native forensics under {}", output.display());
    Ok(())
}

fn capture_provenance(
    repository: &Path,
    corpus: &HistoricalCorpus,
    generation_command: String,
    known_limitations: Vec<String>,
) -> Result<Provenance> {
    let (manifest, _) = inventory_corpus(corpus)?;
    Ok(Provenance::capture(
        repository,
        &manifest,
        REFERENCE_TOOLCHAIN,
        generation_command,
        known_limitations,
    ))
}

fn verify_source(
    repository: &Path,
    corpus: &HistoricalCorpus,
    manifest_path: Option<&Path>,
) -> Result<()> {
    let current = corpus.manifest()?;
    if let Some(path) = manifest_path {
        let path = resolve_from(repository, path);
        let expected: Envelope<agc_source::SourceManifest> = agc_reports::read_json(path)?;
        corpus.verify(&expected.data)?;
    }
    println!(
        "verified {} historical .agc files at commit {}",
        current.entries.len(),
        current.historical_commit.as_deref().unwrap_or("unknown")
    );
    Ok(())
}

fn run_overlay(
    repository: &Path,
    corpus: &HistoricalCorpus,
    command: OverlayCommand,
) -> Result<()> {
    match command {
        OverlayCommand::Verify { overlay } => {
            let overlay = Overlay::load(resolve_from(repository, &overlay))?;
            overlay.verify_against(&corpus.program_root(overlay.program))?;
            let commit = corpus.manifest()?.historical_commit.unwrap_or_default();
            if overlay.historical_commit != commit {
                bail!(
                    "overlay commit {} does not match historical commit {}",
                    overlay.historical_commit,
                    commit
                );
            }
            println!(
                "valid {} overlay: {} include aliases",
                overlay.program,
                overlay.include_aliases.len()
            );
            Ok(())
        }
        OverlayCommand::Materialize { overlay, output } => {
            let overlay = Overlay::load(resolve_from(repository, &overlay))?;
            let output = resolve_from(repository, &output);
            overlay.materialize(&corpus.program_root(overlay.program), &output)?;
            println!("materialized {} at {}", overlay.program, output.display());
            Ok(())
        }
    }
}

fn execute_rope(
    rope: &Path,
    format: RopeFormat,
    instructions: u64,
    trace_path: Option<PathBuf>,
    provenance: Option<Provenance>,
) -> Result<()> {
    let image = load_file(rope, format)?;
    let mut runtime = Runtime::new(Cpu::new(image.into_memory()?));
    runtime.run(instructions)?;
    if let Some(path) = trace_path {
        ensure_parent(&path)?;
        let file = fs::File::create(&path).with_context(|| format!("create {}", path.display()))?;
        runtime.trace().write_json_lines(BufWriter::new(file))?;
        let mut provenance = provenance.context("trace output requires provenance")?;
        provenance.record_input_file("trace_jsonl", &path)?;
        write_json(
            trace_provenance_path(&path),
            &Envelope::new(
                "execution-trace-summary",
                provenance,
                trace_summary(runtime.trace()),
            ),
        )?;
    }
    println!(
        "executed {} instructions / {} cycles; PC={:04o}",
        runtime.cpu().instructions(),
        runtime.cpu().cycles(),
        runtime.cpu().program_counter()
    );
    println!("trace summary: {}", trace_summary(runtime.trace()));
    Ok(())
}

fn write_coverage_report(trace: &Path, output: &Path, provenance: Provenance) -> Result<()> {
    let file = fs::File::open(trace).with_context(|| format!("open {}", trace.display()))?;
    let report = analyze_json_lines(BufReader::new(file))?;
    let basis_points = report.instructions.rope_fetch_coverage_ppm / 100;
    println!(
        concat!(
            "coverage: {} events, {} instructions, {} mnemonic/context forms, ",
            "{} unique PCs, {} physical rope words ({}.{:02}% of installed rope), ",
            "{} fixed banks, {} erasable banks, {} channels, {} interrupt entries"
        ),
        report.events.total,
        report.events.instructions,
        report.instructions.unique_mnemonic_forms,
        report.instructions.unique_logical_pcs,
        report.instructions.unique_rope_fetches,
        basis_points / 100,
        basis_points % 100,
        report.instructions.fixed_banks.len(),
        report.memory.erasable_banks.len(),
        report.io.unique_channels,
        report.interrupts.entries,
    );
    write_json(
        output,
        &Envelope::new("execution-trace-coverage", provenance, report),
    )?;
    println!("coverage report: {}", output.display());
    Ok(())
}

fn run_conformance(
    output_dir: &Path,
    yaagc: Option<&Path>,
    mut provenance: Provenance,
) -> Result<()> {
    fs::create_dir_all(output_dir).with_context(|| format!("create {}", output_dir.display()))?;
    let rope_path = output_dir.join("block-ii-conformance.bin");
    let manifest_path = output_dir.join("manifest.json");
    let trace_path = output_dir.join("apollors-trace.jsonl");
    let trace_metadata_path = output_dir.join("apollors-trace.meta.json");
    let report_path = output_dir.join("report.json");

    let suite = generate_block_ii_suite()?;
    write_bytes(&rope_path, &encode_yayul(&suite.rope_words)?)?;
    write_json(
        &manifest_path,
        &Envelope::new(
            "block-ii-conformance-manifest",
            provenance.clone(),
            suite.manifest.clone(),
        ),
    )?;

    let execution = execute_suite(&suite)?;
    let trace_file = fs::File::create(&trace_path)
        .with_context(|| format!("create {}", trace_path.display()))?;
    execution
        .trace
        .write_json_lines(BufWriter::new(trace_file))?;
    write_json(
        &trace_metadata_path,
        &Envelope::new(
            "block-ii-conformance-trace-summary",
            provenance.clone(),
            trace_summary(&execution.trace),
        ),
    )?;

    provenance.record_input_file("generated_conformance_rope", &rope_path)?;
    provenance.record_input_file("apollors_conformance_trace", &trace_path)?;
    provenance.record_input_file("conformance_manifest", &manifest_path)?;

    let oracle = yaagc
        .map(|executable| {
            run_conformance_oracle(
                executable,
                output_dir,
                &rope_path,
                &execution.trace,
                execution.report.cycles,
            )
        })
        .transpose()?;
    if oracle.is_some() {
        provenance.record_input_file(
            "yaagc_conformance_trace",
            output_dir.join("yaagc-trace.tsv"),
        )?;
        provenance.record_input_file("yaagc_command_file", output_dir.join("yaagc-command.txt"))?;
    }

    let oracle_qualified = oracle
        .as_ref()
        .and_then(|value| value.get("qualified"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let overall_passed = execution.report.passed && yaagc.is_none_or(|_| oracle_qualified);
    let data = serde_json::json!({
        "suite": suite.manifest,
        "apollors": execution.report,
        "independent_oracle_requested": yaagc.is_some(),
        "independent_oracle_qualified": oracle_qualified,
        "oracle": oracle,
        "overall_passed": overall_passed,
        "files": {
            "rope": rope_path,
            "manifest": manifest_path,
            "apollors_trace": trace_path,
            "trace_metadata": trace_metadata_path,
        },
    });
    write_json(
        &report_path,
        &Envelope::new("block-ii-semantic-conformance", provenance, data),
    )?;

    println!(
        concat!(
            "conformance: local_passed={}, {} instructions / {} events / {} cycles, ",
            "38 mnemonics / 39 decode forms, oracle_qualified={}, report {}"
        ),
        execution.report.passed,
        execution.report.instructions,
        execution.report.events,
        execution.report.cycles,
        oracle_qualified,
        report_path.display()
    );
    if !overall_passed {
        bail!(
            "Block II semantic conformance failed; inspect {}",
            report_path.display()
        );
    }
    Ok(())
}

fn run_conformance_oracle(
    executable: &Path,
    output_dir: &Path,
    rope: &Path,
    apollors_trace: &TraceLog,
    apollors_cycles: u64,
) -> Result<serde_json::Value> {
    let reference_trace_path = output_dir.join("yaagc-trace.tsv");
    let command_path = output_dir.join("yaagc-command.txt");
    let stdout_path = output_dir.join("yaagc-stdout.log");
    let stderr_path = output_dir.join("yaagc-stderr.log");
    if reference_trace_path.exists() {
        fs::remove_file(&reference_trace_path)
            .with_context(|| format!("remove stale {}", reference_trace_path.display()))?;
    }

    // Run slightly beyond ApolloRS's success breakpoint. The comparator then
    // requires every ApolloRS event to match the independent reference prefix.
    let reference_steps = apollors_cycles.saturating_add(64);
    write_text(
        &command_path,
        &format!("step {reference_steps}\ninfo registers\nquit\n"),
    )?;
    let mut command = ProcessCommand::new(executable);
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    let output = command
        .env("APOLLORS_YAAGC_TRACE", &reference_trace_path)
        .arg("--no-resume")
        .arg(format!("--command={}", command_path.display()))
        .arg(rope)
        .output()
        .with_context(|| format!("run pinned yaAGC at {}", executable.display()))?;
    write_bytes(&stdout_path, &output.stdout)?;
    write_bytes(&stderr_path, &output.stderr)?;
    if !output.status.success() {
        bail!(
            "pinned yaAGC exited with {}; inspect {} and {}",
            output.status,
            stdout_path.display(),
            stderr_path.display()
        );
    }
    let reference_file = fs::File::open(&reference_trace_path)
        .with_context(|| format!("open {}", reference_trace_path.display()))?;
    let reference = YaAgcReferenceTrace::read_tsv(BufReader::new(reference_file))?;
    let comparison = compare_yaagc_reference(apollors_trace, &reference, true);
    let qualified = comparison.equivalent
        && comparison.matched_events == apollors_trace.events.len()
        && reference.events.len() >= apollors_trace.events.len();
    Ok(serde_json::json!({
        "oracle": REFERENCE_TOOLCHAIN,
        "executable": executable,
        "executable_sha256": file_sha256(executable)?,
        "instrumentation_patch": "docs/validation/yaagc-conformance-trace.patch",
        "reference_steps": reference_steps,
        "reference_trace": reference_trace_path,
        "reference_trace_sha256": file_sha256(&reference_trace_path)?,
        "stdout_log": stdout_path,
        "stderr_log": stderr_path,
        "qualified": qualified,
        "qualification_rule": "every ApolloRS event must match a yaAGC event in order; yaAGC may continue beyond the ApolloRS success breakpoint",
        "comparison": comparison,
    }))
}

fn validate_traces(
    left: &Path,
    right: &Path,
    output: Option<PathBuf>,
    provenance: Provenance,
) -> Result<()> {
    let left = read_trace(left)?;
    let right = read_trace(right)?;
    let report = compare_traces(&left, &right);
    if let Some(path) = output {
        write_json(
            &path,
            &Envelope::new("apollors-trace-validation", provenance, report.clone()),
        )?;
    }
    if report.equivalent {
        println!("trace-equivalent across {} events", report.left_events);
        Ok(())
    } else {
        let divergence = report
            .first
            .as_ref()
            .expect("non-equivalent report has divergence");
        bail!(
            "{} divergence at event {} field {}: {}",
            format!("{:?}", divergence.class).to_ascii_lowercase(),
            divergence.event,
            divergence.field,
            divergence.explanation
        )
    }
}

fn validate_reference_trace(
    apollors: &Path,
    reference: &Path,
    allow_prefix: bool,
    output: Option<PathBuf>,
    provenance: Provenance,
) -> Result<()> {
    let apollors_trace = read_trace(apollors)?;
    let reference_file = fs::File::open(reference)
        .with_context(|| format!("open yaAGC reference trace {}", reference.display()))?;
    let reference_trace = YaAgcReferenceTrace::read_tsv(BufReader::new(reference_file))?;
    let report = compare_yaagc_reference(&apollors_trace, &reference_trace, allow_prefix);
    if let Some(path) = output {
        write_json(
            &path,
            &Envelope::new("yaagc-reference-validation", provenance, report.clone()),
        )?;
    }
    if report.equivalent {
        println!(
            "ApolloRS matches yaAGC across {} events ({})",
            report.matched_events,
            if report.complete {
                "complete streams"
            } else {
                "qualified common prefix"
            }
        );
        Ok(())
    } else {
        let divergence = report
            .first
            .as_ref()
            .expect("non-equivalent report has divergence");
        bail!(
            "{} divergence at event {} field {}: ApolloRS={}, yaAGC={} ({})",
            format!("{:?}", divergence.class).to_ascii_lowercase(),
            divergence.event,
            divergence.field,
            divergence.left,
            divergence.right,
            divergence.explanation
        )
    }
}

fn run_mission(
    rope: &Path,
    format: RopeFormat,
    instructions: Option<u64>,
    output: Option<PathBuf>,
    trace: Option<PathBuf>,
    rope_fault: Option<&str>,
    provenance: Provenance,
) -> Result<()> {
    let image = load_file(rope, format)?;
    let mut controller = MissionController::from_rope(image)?;
    if let Some(specification) = rope_fault {
        let (bank, offset, mask) = parse_octal_triplet(specification)?;
        controller.schedule_fault(0, Fault::RopeBitFlip { bank, offset, mask });
    }
    let mut scenario = MissionScenario::luminary_p63_landing();
    if let Some(instructions) = instructions {
        scenario.instruction_limit = instructions;
    }
    let run = controller.run(&scenario)?;
    if let Some(path) = trace {
        ensure_parent(&path)?;
        let file = fs::File::create(&path).with_context(|| format!("create {}", path.display()))?;
        controller
            .runtime()
            .trace()
            .write_json_lines(BufWriter::new(file))?;
        let mut trace_provenance = provenance.clone();
        trace_provenance.record_input_file("trace_jsonl", &path)?;
        write_json(
            trace_provenance_path(&path),
            &Envelope::new(
                "mission-trace-summary",
                trace_provenance,
                trace_summary(controller.runtime().trace()),
            ),
        )?;
    }
    println!("{}", run.final_dsky.render_text());
    println!(
        "mission {}: {} instructions, {} cycles, {} real-state frames, {} faults",
        run.scenario,
        run.instructions,
        run.cycles,
        run.frames.len(),
        run.faults_applied
    );
    if let Some(path) = output {
        write_json(
            &path,
            &Envelope::new("apollo11-luminary099-p63-mission", provenance, run),
        )?;
    }
    Ok(())
}

fn run_fault_campaign(
    rope: &Path,
    format: RopeFormat,
    at_instruction: u64,
    rope_fault: &str,
    instructions: u64,
    output: &Path,
    provenance: Provenance,
) -> Result<()> {
    let image = load_file(rope, format)?;
    let mut baseline = MissionController::from_rope(image.clone())?;
    let mut faulted = MissionController::from_rope(image)?;
    let (bank, offset, mask) = parse_octal_triplet(rope_fault)?;
    faulted.schedule_fault(at_instruction, Fault::RopeBitFlip { bank, offset, mask });
    let mut scenario = MissionScenario::luminary_p63_landing();
    scenario.instruction_limit = instructions;
    let baseline_run = baseline.run(&scenario)?;
    let faulted_run = faulted.run(&scenario)?;
    let trace_comparison = compare_missions(&baseline, &faulted);
    let recovery = compare_recovery(baseline.runtime(), faulted.runtime());
    let report = serde_json::json!({
        "scenario": scenario.name,
        "scheduled_fault": {
            "instruction": at_instruction,
            "fault": {
                "type": "rope-bit-flip",
                "bank": bank,
                "offset": offset,
                "mask": mask,
            }
        },
        "applied_faults": faulted_run.applied_faults,
        "baseline": mission_acceptance_summary(&baseline_run),
        "faulted": mission_acceptance_summary(&faulted_run),
        "trace_comparison": trace_comparison,
        "recovery": recovery,
    });
    write_json(
        output,
        &Envelope::new("paired-p63-fault-campaign", provenance, report),
    )?;
    println!(
        "fault campaign: first divergence {:?}, registers recovered={}, report {}",
        recovery.first_divergence,
        recovery.registers_recovered,
        output.display()
    );
    Ok(())
}

fn run_fault_matrix(
    rope: &Path,
    format: RopeFormat,
    spec_path: &Path,
    output: &Path,
    provenance: Provenance,
) -> Result<()> {
    let specification_bytes = fs::read(spec_path)
        .with_context(|| format!("read fault matrix {}", spec_path.display()))?;
    let specification: FaultMatrixSpec = serde_json::from_slice(&specification_bytes)
        .with_context(|| format!("parse fault matrix {}", spec_path.display()))?;
    specification.validate()?;
    let image = load_file(rope, format)?;
    let report = run_luminary_p63_fault_matrix(image, &specification)?;
    println!(
        "fault matrix: {} cases, {} diverged, {} degraded, {} recovered; report {}",
        report.aggregate.cases,
        report.aggregate.diverged_cases,
        report.aggregate.degraded_cases,
        report.aggregate.recovered_cases,
        output.display()
    );
    write_json(
        output,
        &Envelope::new("paired-p63-fault-matrix", provenance, report),
    )?;
    Ok(())
}

fn mission_acceptance_summary(run: &MissionRun) -> serde_json::Value {
    serde_json::json!({
        "instructions": run.instructions,
        "cycles": run.cycles,
        "faults_applied": run.faults_applied,
        "keyboard_sequence_verified": run.evidence.keyboard_sequence_verified,
        "pinball_reconstruction_matches_rope": run.evidence.pinball_reconstruction_matches_rope,
        "program_63_selected": run.evidence.program_63_selected,
        "p63lm_entry": run.evidence.p63lm_entry,
        "p63_initialization_matches_rope": run.evidence.p63_initialization_matches_rope,
        "landing_guidance_started": run.evidence.landing_guidance_started,
        "p63spot_entry": run.evidence.p63spot_entry,
        "p63spot2_entry": run.evidence.p63spot2_entry,
    })
}

fn interactive_dsky(rope: &Path, format: RopeFormat, quantum: u64) -> Result<()> {
    let image = load_file(rope, format)?;
    let mut runtime = Runtime::new(Cpu::new(image.into_memory()?));
    let mut dsky = DskyState::default();
    println!("Commands: 0-9 verb noun + - enter clear keyrel reset pro step run N status quit");
    println!("{}", dsky.render_text());
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("dsky> ");
        io::stdout().flush()?;
        let Some(line) = lines.next() else { break };
        let line = line?;
        let mut fields = line.split_whitespace();
        let Some(command) = fields.next() else {
            continue;
        };
        match command.to_ascii_lowercase().as_str() {
            "quit" | "q" => break,
            "status" => {}
            "step" => execute_dsky_steps(&mut runtime, &mut dsky, 1)?,
            "run" => {
                let count = fields
                    .next()
                    .map_or(Ok(quantum), str::parse::<u64>)
                    .context("run count must be decimal")?;
                execute_dsky_steps(&mut runtime, &mut dsky, count)?;
            }
            key => {
                let key = parse_key(key)?;
                if key == Key::Proceed {
                    runtime.schedule(
                        runtime.cpu().cycles(),
                        RuntimeEvent::Channel {
                            channel: 0o32,
                            value: agc_word::AgcWord::POSITIVE_ZERO,
                        },
                    );
                } else {
                    runtime.schedule(
                        runtime.cpu().cycles(),
                        RuntimeEvent::DskyKey { code: key.code()? },
                    );
                }
                execute_dsky_steps(&mut runtime, &mut dsky, quantum)?;
            }
        }
        println!("{}", dsky.render_text());
        println!(
            "PC {:04o}  instructions {}  cycles {}",
            runtime.cpu().program_counter(),
            runtime.cpu().instructions(),
            runtime.cpu().cycles()
        );
    }
    Ok(())
}

fn execute_dsky_steps(runtime: &mut Runtime, dsky: &mut DskyState, count: u64) -> Result<()> {
    for _ in 0..count {
        let outcome = runtime.step()?;
        dsky.consume_trace(&outcome.trace);
    }
    Ok(())
}

fn parse_key(input: &str) -> Result<Key> {
    if input.len() == 1 && input.as_bytes()[0].is_ascii_digit() {
        return Ok(Key::Digit(input.as_bytes()[0] - b'0'));
    }
    match input {
        "verb" | "v" => Ok(Key::Verb),
        "noun" | "n" => Ok(Key::Noun),
        "+" | "plus" => Ok(Key::Plus),
        "-" | "minus" => Ok(Key::Minus),
        "enter" | "e" => Ok(Key::Enter),
        "clear" | "c" => Ok(Key::Clear),
        "keyrel" | "k" => Ok(Key::KeyRelease),
        "reset" | "r" => Ok(Key::Reset),
        "pro" | "p" => Ok(Key::Proceed),
        _ => bail!("unknown DSKY command {input}"),
    }
}

fn default_overlay(repository: &Path, program: Program) -> Result<Option<Overlay>> {
    let filename = match program {
        Program::Comanche055 => "comanche055.json",
        Program::Luminary099 => "luminary099.json",
    };
    Ok(Some(Overlay::load(
        repository.join("overlays").join(filename),
    )?))
}

fn load_program_overlay(
    repository: &Path,
    program: Program,
    path: Option<&Path>,
) -> Result<Option<Overlay>> {
    if let Some(path) = path {
        Ok(Some(Overlay::load(resolve_from(repository, path))?))
    } else {
        default_overlay(repository, program)
    }
}

fn parse_octal_triplet(input: &str) -> Result<(u8, u16, u16)> {
    let fields = input.split(':').collect::<Vec<_>>();
    if fields.len() != 3 {
        bail!("rope fault must be BANK:OFFSET:MASK in octal");
    }
    let bank = u8::from_str_radix(fields[0], 8).context("invalid octal bank")?;
    let offset = u16::from_str_radix(fields[1], 8).context("invalid octal offset")?;
    let mask = u16::from_str_radix(fields[2], 8).context("invalid octal mask")?;
    if bank > 0o43 {
        bail!("rope fault bank must be in physical range 00..43 octal");
    }
    if offset >= 0o2000 {
        bail!("rope fault offset must be in physical range 0000..1777 octal");
    }
    if mask == 0 || mask > 0o77777 {
        bail!("rope fault mask must be a nonzero 15-bit octal word");
    }
    Ok((bank, offset, mask))
}

fn read_trace(path: &Path) -> Result<TraceLog> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    Ok(TraceLog::read_json_lines(BufReader::new(file))?)
}

fn trace_provenance_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.provenance.json", path.display()))
}

fn write_file_sidecar(
    path: &Path,
    artifact_kind: &str,
    provenance: Provenance,
    details: serde_json::Value,
) -> Result<()> {
    let payload = serde_json::json!({
        "path": path.display().to_string(),
        "bytes": fs::metadata(path)?.len(),
        "sha256": file_sha256(path)?,
        "details": details,
    });
    write_json(
        trace_provenance_path(path),
        &Envelope::new(artifact_kind, provenance, payload),
    )?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    Ok(())
}

fn absolute_from_current(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn resolve_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn dsky_digit_command_maps_to_keyboard_code() {
        assert_eq!(parse_key("7").unwrap(), Key::Digit(7));
        assert!(parse_key("launch").is_err());
    }

    #[test]
    fn rope_fault_parser_enforces_physical_ranges() {
        assert_eq!(
            parse_octal_triplet("32:0776:00001").unwrap(),
            (0o32, 0o776, 1)
        );
        assert!(parse_octal_triplet("44:0000:00001").is_err());
        assert!(parse_octal_triplet("32:2000:00001").is_err());
        assert!(parse_octal_triplet("32:0776:00000").is_err());
    }
}
