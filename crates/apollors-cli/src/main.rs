#![forbid(unsafe_code)]
//! ApolloRS command-line research, execution, validation, and DSKY interface.

use agc_assembler::{
    AssemblyError, ReferenceAssemblerConfig, assemble, assemble_binsource_reference,
    assemble_reference, expand_program,
};
use agc_cpu::Cpu;
use agc_dsky::{DskyState, Key};
use agc_faults::Fault;
use agc_loader::{RopeFormat, load_file};
use agc_mission::{MissionController, MissionScenario};
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
        /// Independently proofed VirtualAGC octal listing, parsed and checked in Rust.
        #[arg(long, conflicts_with = "reference_yayul")]
        reference_binsource: Option<PathBuf>,
        /// Exact reference toolchain commit/version recorded in the report.
        #[arg(long)]
        reference_toolchain: Option<String>,
        /// Ask yaYUL to emit despite its internal errors; ApolloRS still rejects them.
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
    /// Compare two ApolloRS JSON-lines traces under the complete trace schema.
    Validate {
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare an ApolloRS JSON-lines trace with pinned yaAGC exact-trace TSV.
    ValidateReference {
        /// ApolloRS architectural JSON-lines trace.
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
                        program.directory(),
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
                let report = report
                    .map(|path| resolve_from(&repository, &path))
                    .unwrap_or_else(|| output.with_extension("build.json"));
                let mut provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- assemble {} --entry {} --reference-yayul {} --output {}",
                        program.directory(),
                        entry,
                        config.executable.display(),
                        output.display()
                    ),
                    vec![
                        "This is an isolated external-reference build, not evidence that ApolloRS's native assembler emits the same rope.".to_owned(),
                    ],
                )?;
                provenance.reference_toolchain = config.toolchain.clone();
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
                let report = report
                    .map(|path| resolve_from(&repository, &path))
                    .unwrap_or_else(|| output.with_extension("build.json"));
                let mut provenance = capture_provenance(
                    &repository,
                    &corpus,
                    format!(
                        "cargo run -p apollors-cli -- assemble {} --reference-binsource {} --output {}",
                        program.directory(),
                        binsource.display(),
                        output.display()
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
                        program.directory(),
                        entry,
                        output.display()
                    ),
                    vec![
                        "Native assembly support is corpus-driven; equivalence to yaYUL must be established separately for this exact output.".to_owned(),
                    ],
                )?;
                provenance.reference_toolchain = "ApolloRS native assembler".to_owned();
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
                            "cargo run -p apollors-cli -- execute --rope {} --instructions {} --trace {}",
                            rope.display(),
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
                    "cargo run -p apollors-cli -- validate --left {} --right {}",
                    left.display(),
                    right.display()
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
                    "cargo run -p apollors-cli -- validate-reference --apollors {} --reference {}{}",
                    apollors.display(),
                    reference.display(),
                    if allow_prefix { " --allow-prefix" } else { "" }
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
                    "cargo run -p apollors-cli -- transpile {} --entry {} --output {}",
                    program.directory(),
                    entry,
                    output.display()
                ),
                vec![
                    "Generated instruction dispatch preserves source/word provenance but remains unverified until paired differential execution is recorded.".to_owned(),
                    "The readable typed Pinball V37 model is maintained and tested in agc-dsky rather than generated by this whole-program instruction dispatcher.".to_owned(),
                ],
            )?;
            provenance.reference_toolchain =
                "ApolloRS native parser/assembler/transpiler".to_owned();
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
                    "cargo run -p apollors-cli -- mission --rope {}{}{}{}",
                    rope.display(),
                    instructions.map_or(String::new(), |value| format!(" --instructions {value}")),
                    output.as_ref().map_or(String::new(), |path| format!(" --output {}", path.display())),
                    trace.as_ref().map_or(String::new(), |path| format!(" --trace {}", path.display()))
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
        controller.schedule_fault(
            0,
            Fault::RopeBitFlip {
                bank: bank as u8,
                offset,
                mask,
            },
        );
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

fn parse_octal_triplet(input: &str) -> Result<(u16, u16, u16)> {
    let fields = input.split(':').collect::<Vec<_>>();
    if fields.len() != 3 {
        bail!("rope fault must be BANK:OFFSET:MASK in octal");
    }
    Ok((
        u16::from_str_radix(fields[0], 8).context("invalid octal bank")?,
        u16::from_str_radix(fields[1], 8).context("invalid octal offset")?,
        u16::from_str_radix(fields[2], 8).context("invalid octal mask")?,
    ))
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
}
