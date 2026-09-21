#![forbid(unsafe_code)]
//! Executable Block II semantic conformance rope and measured result model.

use agc_cpu::{Cpu, CpuError, StopReason};
use agc_isa::{EncodeError, Mnemonic, decode, encode_with_context};
use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory, MemoryError, register};
use agc_trace::{InterruptEvent, MachineEventKind, TraceLog};
use agc_word::{AgcWord, ChannelAddress, WordError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Stable identifier for the built-in Block II semantic suite.
pub const SUITE_ID: &str = "apollors-block-ii-semantic-conformance-v1";
/// Suite manifest/report schema.
pub const SUITE_SCHEMA_VERSION: u32 = 1;

const RESTART_PC: u16 = 0o4000;
const DOWNRUPT_VECTOR: u16 = 0o4040;
const START_PC: u16 = 0o4100;
const FAILURE_PC: u16 = 0o5700;
const CONSTANT_PC: u16 = 0o6000;
const MAX_INSTRUCTIONS: u64 = 2_000;

/// One independently named semantic region in the generated rope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaseDefinition {
    /// Stable case identifier.
    pub id: String,
    /// Architectural behavior under test.
    pub objective: String,
    /// First logical PC in the case.
    pub start_pc: u16,
    /// First logical PC after the case.
    pub end_pc_exclusive: u16,
    /// Mnemonics that must be observed inside this PC region.
    pub required_mnemonics: Vec<Mnemonic>,
    /// Boundary classes deliberately represented by the case.
    pub boundaries: Vec<String>,
}

/// Declarative location checked after the suite reaches its terminal breakpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "region", rename_all = "kebab-case")]
pub enum AssertionLocation {
    /// Physical erasable memory, independent of final EBANK selection.
    Erasable {
        /// Physical erasable bank.
        bank: u8,
        /// Offset within the bank.
        offset: u16,
    },
    /// Central register.
    Register {
        /// Register index.
        index: u16,
    },
    /// I/O channel.
    Channel {
        /// Nine-bit channel address.
        channel: u16,
    },
}

/// One specification-derived final-state expectation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExpectedAssertion {
    /// Stable assertion identifier.
    pub id: String,
    /// Architectural location sampled.
    pub location: AssertionLocation,
    /// Expected raw AGC word/register value.
    pub expected_raw: u16,
    /// Human-readable reason for the expected value.
    pub rationale: String,
}

/// Reproducible description of the generated rope and its obligations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuiteManifest {
    /// Manifest schema.
    pub schema_version: u32,
    /// Stable suite identifier.
    pub suite_id: String,
    /// Initial hardware restart PC.
    pub restart_pc: u16,
    /// Entry point after the startup interrupt/resume sequence.
    pub start_pc: u16,
    /// Success breakpoint; the instruction at this address is not executed.
    pub terminal_pc: u16,
    /// Failure breakpoint used by in-rope control-flow checks.
    pub failure_pc: u16,
    /// Upper execution bound guarding malformed control flow.
    pub maximum_instructions: u64,
    /// Named semantic cases.
    pub cases: Vec<CaseDefinition>,
    /// Final-state obligations.
    pub assertions: Vec<ExpectedAssertion>,
    /// Every canonical mnemonic required somewhere in the trace.
    pub required_mnemonics: Vec<Mnemonic>,
    /// Explicit required decode contexts. `INDEX` appears in both contexts.
    pub required_forms: Vec<InstructionForm>,
}

/// Mnemonic plus basic/extracode decode context.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct InstructionForm {
    /// Canonical mnemonic.
    pub mnemonic: Mnemonic,
    /// Whether the word was decoded in extracode context.
    pub extended: bool,
}

/// Generated physical rope plus its serializable manifest.
#[derive(Clone, Debug)]
pub struct GeneratedSuite {
    /// Physical fixed-memory words in bank order.
    pub rope_words: Vec<AgcWord>,
    /// Machine-readable suite definition.
    pub manifest: SuiteManifest,
}

/// One sampled final-state obligation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    /// Stable assertion identifier.
    pub id: String,
    /// Sampled architectural location.
    pub location: AssertionLocation,
    /// Expected raw value.
    pub expected_raw: u16,
    /// Observed raw value.
    pub actual_raw: u16,
    /// Expected six-digit octal value.
    pub expected_octal: String,
    /// Observed six-digit octal value.
    pub actual_octal: String,
    /// Whether the values are exactly equal.
    pub passed: bool,
    /// Specification rationale copied from the manifest.
    pub rationale: String,
}

/// Measured reachability and instruction coverage for one case.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    /// Stable case identifier.
    pub id: String,
    /// Architectural behavior under test.
    pub objective: String,
    /// Events committed in the case's logical PC region.
    pub observed_events: usize,
    /// Required mnemonics observed in that region.
    pub observed_required_mnemonics: Vec<Mnemonic>,
    /// Required mnemonics absent from that region.
    pub missing_required_mnemonics: Vec<Mnemonic>,
    /// Whether the case region and all of its required mnemonics were exercised.
    pub passed: bool,
    /// Boundary classes represented by the case.
    pub boundaries: Vec<String>,
}

/// Local execution report. External-oracle comparison is attached by the CLI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConformanceReport {
    /// Report schema.
    pub schema_version: u32,
    /// Stable suite identifier.
    pub suite_id: String,
    /// Committed instruction count.
    pub instructions: u64,
    /// Committed trace event count, including interrupt entry.
    pub events: usize,
    /// Final machine-cycle count.
    pub cycles: u64,
    /// Actual stop reason.
    pub stop_reason: String,
    /// Final logical PC.
    pub final_pc: u16,
    /// Whether execution stopped at the success breakpoint.
    pub terminal_reached: bool,
    /// Whether execution stopped at the failure breakpoint.
    pub failure_reached: bool,
    /// Canonical mnemonics observed as committed semantics.
    pub observed_mnemonics: Vec<Mnemonic>,
    /// Canonical mnemonics not observed.
    pub missing_mnemonics: Vec<Mnemonic>,
    /// Mnemonic/context forms observed.
    pub observed_forms: Vec<InstructionForm>,
    /// Required mnemonic/context forms not observed.
    pub missing_forms: Vec<InstructionForm>,
    /// Per-case reachability results.
    pub cases: Vec<CaseResult>,
    /// Specification-derived final-state checks.
    pub assertions: Vec<AssertionResult>,
    /// True only if terminal, coverage, case, and final-state obligations pass.
    pub passed: bool,
}

/// Local report paired with the full architectural trace for oracle comparison.
#[derive(Clone, Debug)]
pub struct SuiteExecution {
    /// Measured conformance report.
    pub report: ConformanceReport,
    /// Complete trace through the terminal breakpoint.
    pub trace: TraceLog,
}

/// Conformance generation or execution failure.
#[derive(Debug, Error)]
pub enum ConformanceError {
    /// Instruction encoding failed.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// AGC word construction failed.
    #[error(transparent)]
    Word(#[from] WordError),
    /// Memory construction or sampling failed.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// CPU execution failed.
    #[error(transparent)]
    Cpu(#[from] CpuError),
    /// A generated label is missing.
    #[error("conformance generator has unresolved label {0}")]
    MissingLabel(String),
    /// Two generated records occupy one rope word.
    #[error("conformance generator overlaps physical bank {bank:02o} offset {offset:04o}")]
    Overlap {
        /// Physical fixed bank.
        bank: u8,
        /// Physical offset.
        offset: u16,
    },
    /// Generated code crossed a fixed-memory window.
    #[error("conformance generator cannot place logical PC {0:04o} in fixed-fixed bank 2")]
    CodeAddress(u16),
    /// A declarative assertion names an unavailable physical erasable word.
    #[error("conformance assertion cannot sample erasable bank {bank:o} offset {offset:04o}")]
    AssertionLocation {
        /// Physical erasable bank.
        bank: u8,
        /// Physical erasable offset.
        offset: u16,
    },
}

/// Generates the deterministic Block II semantic conformance rope.
pub fn generate_block_ii_suite() -> Result<GeneratedSuite, ConformanceError> {
    let mut builder = Builder::new();

    builder.label_at("restart", RESTART_PC);
    builder.place_instruction(2, 0, Mnemonic::Tcf, Operand::Label("start", 0), false);
    builder.place_instruction(
        2,
        DOWNRUPT_VECTOR & 0o1777,
        Mnemonic::Resume,
        Operand::Literal(0),
        false,
    );
    builder.pc = START_PC;

    builder.start_case(
        "interrupt-control",
        "Accept the reset DOWNRUPT, resume the interrupted transfer, and protect RELINT/INHINT boundaries",
        vec![Mnemonic::Resume, Mnemonic::Tcf, Mnemonic::Inhint, Mnemonic::Relint],
        &["reset DOWNRUPT", "interrupt resume", "interrupt inhibition"],
    );
    builder.label("start");
    builder.emit(Mnemonic::Inhint, 0);
    builder.emit(Mnemonic::Relint, 0);
    builder.emit(Mnemonic::Inhint, 0);
    builder.end_case();

    builder.start_case(
        "ones-complement-arithmetic",
        "Exercise signed zero, both overflow directions, storage correction, magnitude operations, and modular arithmetic",
        vec![
            Mnemonic::Ca,
            Mnemonic::Ad,
            Mnemonic::Ads,
            Mnemonic::Cs,
            Mnemonic::Mask,
            Mnemonic::Su,
            Mnemonic::Msu,
            Mnemonic::Incr,
            Mnemonic::Aug,
            Mnemonic::Dim,
            Mnemonic::Ts,
        ],
        &[
            "positive zero",
            "negative zero",
            "positive overflow",
            "negative overflow",
            "end-around carry",
        ],
    );
    builder.emit_label(Mnemonic::Ca, "max-positive", 0);
    builder.emit_label(Mnemonic::Ad, "one", 0);
    builder.emit(Mnemonic::Ts, 0o100);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.emit(Mnemonic::Ts, 0o101);

    builder.emit_label(Mnemonic::Ca, "min-negative", 0);
    builder.emit_label(Mnemonic::Ad, "negative-one", 0);
    builder.emit(Mnemonic::Ts, 0o102);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.emit(Mnemonic::Ts, 0o103);

    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Ts, 0o220);
    builder.emit_label(Mnemonic::Ca, "two", 0);
    builder.emit(Mnemonic::Ads, 0o220);
    builder.emit(Mnemonic::Ca, 0o220);
    builder.emit(Mnemonic::Ts, 0o104);

    builder.emit_label(Mnemonic::Cs, "one", 0);
    builder.emit(Mnemonic::Ts, 0o105);
    builder.emit_label(Mnemonic::Ca, "mask-left", 0);
    builder.emit_label(Mnemonic::Mask, "mask-right", 0);
    builder.emit(Mnemonic::Ts, 0o106);

    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit(Mnemonic::Ts, 0o227);
    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit_extended(Mnemonic::Su, 0o227);
    builder.emit(Mnemonic::Ts, 0o107);
    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Ts, 0o227);
    builder.emit_label(Mnemonic::Ca, "five", 0);
    builder.emit_extended(Mnemonic::Msu, 0o227);
    builder.emit(Mnemonic::Ts, 0o227);
    builder.emit(Mnemonic::Ca, 0o227);
    builder.emit(Mnemonic::Ts, 0o110);
    builder.emit(Mnemonic::Incr, 0o227);
    builder.emit(Mnemonic::Ca, 0o227);
    builder.emit(Mnemonic::Ts, 0o111);

    builder.emit_label(Mnemonic::Ca, "positive-zero", 0);
    builder.emit(Mnemonic::Ts, 0o221);
    builder.emit_extended(Mnemonic::Aug, 0o221);
    builder.emit_extended(Mnemonic::Dim, 0o221);
    builder.emit(Mnemonic::Ca, 0o221);
    builder.emit(Mnemonic::Ts, 0o112);

    builder.emit_label(Mnemonic::Ca, "negative-zero", 0);
    builder.emit(Mnemonic::Ts, 0o222);
    builder.emit_extended(Mnemonic::Aug, 0o222);
    builder.emit_extended(Mnemonic::Dim, 0o222);
    builder.emit(Mnemonic::Ca, 0o222);
    builder.emit(Mnemonic::Ts, 0o113);
    builder.end_case();

    builder.start_case(
        "branch-boundaries",
        "Drive all four CCS classes and taken/not-taken BZF/BZMF paths",
        vec![Mnemonic::Ccs, Mnemonic::Bzf, Mnemonic::Bzmf, Mnemonic::Tcf],
        &["positive", "positive zero", "negative", "negative zero"],
    );
    emit_ccs_path(&mut builder, "ccs-positive", "one", 0, 0o223);
    emit_ccs_path(&mut builder, "ccs-positive-zero", "positive-zero", 1, 0o224);
    emit_ccs_path(&mut builder, "ccs-negative", "negative-one", 2, 0o225);
    emit_ccs_path(&mut builder, "ccs-negative-zero", "negative-zero", 3, 0o226);

    builder.emit_label(Mnemonic::Ca, "positive-zero", 0);
    builder.emit_extended_label(Mnemonic::Bzf, "bzf-zero-taken", 0);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.label("bzf-zero-taken");
    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit_extended_label(Mnemonic::Bzf, "failure", 0);

    builder.emit_label(Mnemonic::Ca, "negative-zero", 0);
    builder.emit_extended_label(Mnemonic::Bzf, "bzf-negative-zero-taken", 0);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.label("bzf-negative-zero-taken");

    builder.emit_label(Mnemonic::Ca, "negative-one", 0);
    builder.emit_extended_label(Mnemonic::Bzmf, "bzmf-negative-taken", 0);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.label("bzmf-negative-taken");
    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit_extended_label(Mnemonic::Bzmf, "failure", 0);
    builder.end_case();

    builder.start_case(
        "exchange-and-index",
        "Exercise single/double exchanges, Q/L special paths, and basic plus extended INDEX",
        vec![
            Mnemonic::Xch,
            Mnemonic::Lxch,
            Mnemonic::Qxch,
            Mnemonic::Dxch,
            Mnemonic::Index,
            Mnemonic::Read,
        ],
        &[
            "central registers",
            "erasable exchange",
            "indexed basic word",
            "indexed extracode word",
        ],
    );
    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Ts, 0o230);
    builder.emit_label(Mnemonic::Ca, "five", 0);
    builder.emit(Mnemonic::Xch, 0o230);
    builder.emit(Mnemonic::Ts, 0o114);

    builder.emit_label(Mnemonic::Ca, "two", 0);
    builder.emit(Mnemonic::Ts, 0o231);
    builder.emit(Mnemonic::Lxch, 0o231);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o115);

    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Ts, 0o232);
    builder.emit_extended(Mnemonic::Qxch, 0o232);
    builder.emit(Mnemonic::Ca, register::Q);
    builder.emit(Mnemonic::Ts, 0o116);

    builder.emit_label(Mnemonic::Ca, "eight", 0);
    builder.emit(Mnemonic::Ts, 0o233);
    builder.emit_label(Mnemonic::Ca, "nine", 0);
    builder.emit(Mnemonic::Ts, 0o234);
    builder.emit_label(Mnemonic::Ca, "four", 0);
    builder.emit(Mnemonic::Ts, 0o235);
    builder.emit(Mnemonic::Lxch, 0o235);
    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Dxch, 0o234);
    builder.emit(Mnemonic::Ts, 0o117);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o120);
    builder.emit(Mnemonic::Ca, 0o233);
    builder.emit(Mnemonic::Ts, 0o121);
    builder.emit(Mnemonic::Ca, 0o234);
    builder.emit(Mnemonic::Ts, 0o122);

    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit(Mnemonic::Ts, 0o236);
    builder.emit(Mnemonic::Index, 0o236);
    builder.emit_label(Mnemonic::Tcf, "basic-index-target", -1);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.label("basic-index-target");

    builder.emit_label(Mnemonic::Ca, "io-index-value", 0);
    builder.emit_extended(Mnemonic::Write, 0o41);
    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit(Mnemonic::Ts, 0o237);
    builder.emit(Mnemonic::Extend, 0);
    builder.emit_context(Mnemonic::Index, Operand::Literal(0o237), true);
    builder.emit_context(Mnemonic::Read, Operand::Literal(0o40), true);
    builder.emit(Mnemonic::Ts, 0o123);
    builder.end_case();

    builder.start_case(
        "double-precision-arithmetic",
        "Cover double load/complement/add, multiply, and divide with exact quotient and remainder signatures",
        vec![Mnemonic::Dca, Mnemonic::Dcs, Mnemonic::Das, Mnemonic::Mp, Mnemonic::Dv],
        &["double word", "negative product", "nonzero divide remainder"],
    );
    builder.emit_extended_label(Mnemonic::Dca, "double-low", 0);
    builder.emit(Mnemonic::Ts, 0o124);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o125);
    builder.emit_extended_label(Mnemonic::Dcs, "double-low", 0);
    builder.emit(Mnemonic::Ts, 0o126);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o127);

    builder.emit_label(Mnemonic::Ca, "one", 0);
    builder.emit(Mnemonic::Ts, 0o240);
    builder.emit_label(Mnemonic::Ca, "two", 0);
    builder.emit(Mnemonic::Ts, 0o241);
    builder.emit_label(Mnemonic::Ca, "four", 0);
    builder.emit(Mnemonic::Ts, 0o242);
    builder.emit(Mnemonic::Lxch, 0o242);
    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Das, 0o241);
    builder.emit(Mnemonic::Ca, 0o240);
    builder.emit(Mnemonic::Ts, 0o130);
    builder.emit(Mnemonic::Ca, 0o241);
    builder.emit(Mnemonic::Ts, 0o131);

    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit_extended_label(Mnemonic::Mp, "negative-two", 0);
    builder.emit(Mnemonic::Ts, 0o132);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o133);

    builder.emit_label(Mnemonic::Ca, "three", 0);
    builder.emit(Mnemonic::Ts, 0o243);
    builder.emit_extended_label(Mnemonic::Dca, "dividend-low", 0);
    builder.emit_extended(Mnemonic::Dv, 0o243);
    builder.emit(Mnemonic::Ts, 0o134);
    builder.emit(Mnemonic::Ca, register::L);
    builder.emit(Mnemonic::Ts, 0o135);
    builder.end_case();

    builder.start_case(
        "channel-logic",
        "Exercise every extracode channel read/write Boolean operation against a deterministic channel",
        vec![
            Mnemonic::Write,
            Mnemonic::Read,
            Mnemonic::Rand,
            Mnemonic::Wand,
            Mnemonic::Ror,
            Mnemonic::Wor,
            Mnemonic::Rxor,
        ],
        &["nine-bit channel address", "AND", "OR", "XOR", "read-modify-write"],
    );
    emit_channel_reset(&mut builder);
    builder.emit_extended(Mnemonic::Read, 0o40);
    builder.emit(Mnemonic::Ts, 0o136);

    builder.emit_label(Mnemonic::Ca, "io-right", 0);
    builder.emit_extended(Mnemonic::Rand, 0o40);
    builder.emit(Mnemonic::Ts, 0o137);

    emit_channel_reset(&mut builder);
    builder.emit_label(Mnemonic::Ca, "io-right", 0);
    builder.emit_extended(Mnemonic::Wand, 0o40);
    builder.emit_extended(Mnemonic::Read, 0o40);
    builder.emit(Mnemonic::Ts, 0o140);

    emit_channel_reset(&mut builder);
    builder.emit_label(Mnemonic::Ca, "io-right", 0);
    builder.emit_extended(Mnemonic::Ror, 0o40);
    builder.emit(Mnemonic::Ts, 0o141);

    emit_channel_reset(&mut builder);
    builder.emit_label(Mnemonic::Ca, "io-right", 0);
    builder.emit_extended(Mnemonic::Wor, 0o40);
    builder.emit_extended(Mnemonic::Read, 0o40);
    builder.emit(Mnemonic::Ts, 0o142);

    emit_channel_reset(&mut builder);
    builder.emit_label(Mnemonic::Ca, "io-right", 0);
    builder.emit_extended(Mnemonic::Rxor, 0o40);
    builder.emit(Mnemonic::Ts, 0o143);
    builder.end_case();

    builder.start_case(
        "bank-selection",
        "Prove switched erasable isolation, BB synchronization, fixed-bank calls, and superbank selection",
        vec![Mnemonic::Ts, Mnemonic::Ca, Mnemonic::Tc, Mnemonic::Write],
        &["EBANK 1/6", "FBANK 4/30 octal", "BB synchronization", "superbank 40 octal"],
    );
    builder.emit_label(Mnemonic::Ca, "ebank-six", 0);
    builder.emit(Mnemonic::Ts, register::EB);
    builder.emit_label(Mnemonic::Ca, "bank-six-value", 0);
    builder.emit(Mnemonic::Ts, 0o1400);
    builder.emit_label(Mnemonic::Ca, "ebank-one", 0);
    builder.emit(Mnemonic::Ts, register::EB);
    builder.emit_label(Mnemonic::Ca, "bank-one-value", 0);
    builder.emit(Mnemonic::Ts, 0o1400);
    builder.emit_label(Mnemonic::Ca, "ebank-six", 0);
    builder.emit(Mnemonic::Ts, register::EB);
    builder.emit(Mnemonic::Ca, 0o1400);
    builder.emit(Mnemonic::Ts, 0o144);

    builder.emit_label(Mnemonic::Ca, "fbank-four", 0);
    builder.emit(Mnemonic::Ts, register::FB);
    builder.emit(Mnemonic::Ca, register::BB);
    builder.emit(Mnemonic::Ts, 0o145);
    builder.emit(Mnemonic::Tc, 0o2000);

    builder.emit_label(Mnemonic::Ca, "superbank-enable", 0);
    builder.emit_extended(Mnemonic::Write, 0o7);
    builder.emit_label(Mnemonic::Ca, "fbank-thirty", 0);
    builder.emit(Mnemonic::Ts, register::FB);
    builder.emit(Mnemonic::Tc, 0o2000);
    builder.end_case();

    builder.start_case(
        "software-interrupt",
        "Force EDRUPT with no pending maskable request and recover through an instruction fetched from A",
        vec![Mnemonic::EdrupT, Mnemonic::Tcf, Mnemonic::Ca, Mnemonic::Ts],
        &["vector zero", "instruction fetch from A", "unmaskable software interrupt"],
    );
    builder.emit_label(Mnemonic::Ca, "edrupt-recovery-word", 0);
    builder.emit_extended(Mnemonic::EdrupT, 0);
    builder.emit_label(Mnemonic::Tcf, "failure", 0);
    builder.label("edrupt-recovery");
    builder.emit_label(Mnemonic::Ca, "edrupt-marker", 0);
    builder.emit(Mnemonic::Ts, 0o150);
    builder.emit_label(Mnemonic::Tcf, "terminal", 0);
    builder.end_case();

    builder.label_at("failure", FAILURE_PC);
    builder.place_instruction(
        2,
        FAILURE_PC & 0o1777,
        Mnemonic::Tcf,
        Operand::Label("failure", 0),
        false,
    );
    builder.label_at("terminal", FAILURE_PC + 1);
    builder.place_instruction(
        2,
        (FAILURE_PC + 1) & 0o1777,
        Mnemonic::Tcf,
        Operand::Label("terminal", 0),
        false,
    );

    add_constants(&mut builder)?;
    add_fixed_bank_routines(&mut builder);
    add_assertions(&mut builder);
    builder.finish()
}

/// Runs the generated suite to its terminal or failure breakpoint.
pub fn execute_suite(suite: &GeneratedSuite) -> Result<SuiteExecution, ConformanceError> {
    let memory = Memory::with_rope(suite.rope_words.clone())?;
    let mut cpu = Cpu::new(memory);
    cpu.add_breakpoint(suite.manifest.terminal_pc)?;
    cpu.add_breakpoint(suite.manifest.failure_pc)?;
    let outcome = cpu.run(suite.manifest.maximum_instructions)?;
    let final_pc = cpu.program_counter();
    let terminal_reached = matches!(
        outcome.reason,
        StopReason::Breakpoint(pc) if pc == suite.manifest.terminal_pc
    );
    let failure_reached = matches!(
        outcome.reason,
        StopReason::Breakpoint(pc) if pc == suite.manifest.failure_pc
    );

    let mut observed_mnemonics = BTreeSet::new();
    let mut observed_forms = BTreeSet::new();
    for event in &cpu.trace().events {
        if event.kind == MachineEventKind::Instruction {
            if let Some(mnemonic) = Mnemonic::parse(&event.mnemonic) {
                observed_mnemonics.insert(mnemonic);
                observed_forms.insert(InstructionForm {
                    mnemonic,
                    extended: event.extended,
                });
            }
        } else if event.interrupts.iter().any(|interrupt| {
            matches!(
                interrupt,
                InterruptEvent::Entered {
                    number: 0,
                    vector: 0
                }
            )
        }) {
            let instruction = decode(event.instruction, event.extended);
            if instruction.mnemonic == Mnemonic::EdrupT {
                observed_mnemonics.insert(Mnemonic::EdrupT);
                observed_forms.insert(InstructionForm {
                    mnemonic: Mnemonic::EdrupT,
                    extended: true,
                });
            }
        }
    }

    let missing_mnemonics = suite
        .manifest
        .required_mnemonics
        .iter()
        .copied()
        .filter(|mnemonic| !observed_mnemonics.contains(mnemonic))
        .collect::<Vec<_>>();
    let missing_forms = suite
        .manifest
        .required_forms
        .iter()
        .filter(|form| !observed_forms.contains(*form))
        .cloned()
        .collect::<Vec<_>>();

    let cases = suite
        .manifest
        .cases
        .iter()
        .map(|case| case_result(case, cpu.trace()))
        .collect::<Vec<_>>();
    let assertions = suite
        .manifest
        .assertions
        .iter()
        .map(|assertion| sample_assertion(&cpu, assertion))
        .collect::<Result<Vec<_>, _>>()?;
    let passed = terminal_reached
        && !failure_reached
        && missing_mnemonics.is_empty()
        && missing_forms.is_empty()
        && cases.iter().all(|case| case.passed)
        && assertions.iter().all(|assertion| assertion.passed);
    let stop_reason = match outcome.reason {
        StopReason::InstructionLimit => "instruction-limit".to_owned(),
        StopReason::Breakpoint(pc) => format!("breakpoint-{pc:04o}"),
        StopReason::Watchpoint(address) => format!("watchpoint-{address:04o}"),
    };
    let report = ConformanceReport {
        schema_version: SUITE_SCHEMA_VERSION,
        suite_id: suite.manifest.suite_id.clone(),
        instructions: outcome.instructions,
        events: cpu.trace().events.len(),
        cycles: outcome.cycles,
        stop_reason,
        final_pc,
        terminal_reached,
        failure_reached,
        observed_mnemonics: observed_mnemonics.into_iter().collect(),
        missing_mnemonics,
        observed_forms: observed_forms.into_iter().collect(),
        missing_forms,
        cases,
        assertions,
        passed,
    };
    Ok(SuiteExecution {
        report,
        trace: cpu.take_trace(),
    })
}

fn case_result(case: &CaseDefinition, trace: &TraceLog) -> CaseResult {
    let events = trace
        .events
        .iter()
        .filter(|event| {
            (case.start_pc..case.end_pc_exclusive).contains(&event.pc)
                || (case.id == "interrupt-control"
                    && matches!(event.pc, RESTART_PC | DOWNRUPT_VECTOR))
        })
        .collect::<Vec<_>>();
    let mut observed = BTreeSet::new();
    for event in &events {
        if let Some(mnemonic) = Mnemonic::parse(&event.mnemonic) {
            observed.insert(mnemonic);
        }
        if event.interrupts.iter().any(|interrupt| {
            matches!(
                interrupt,
                InterruptEvent::Entered {
                    number: 0,
                    vector: 0
                }
            )
        }) {
            observed.insert(Mnemonic::EdrupT);
        }
    }
    let observed_required_mnemonics = case
        .required_mnemonics
        .iter()
        .copied()
        .filter(|mnemonic| observed.contains(mnemonic))
        .collect::<Vec<_>>();
    let missing_required_mnemonics = case
        .required_mnemonics
        .iter()
        .copied()
        .filter(|mnemonic| !observed.contains(mnemonic))
        .collect::<Vec<_>>();
    CaseResult {
        id: case.id.clone(),
        objective: case.objective.clone(),
        observed_events: events.len(),
        observed_required_mnemonics,
        passed: !events.is_empty() && missing_required_mnemonics.is_empty(),
        missing_required_mnemonics,
        boundaries: case.boundaries.clone(),
    }
}

fn sample_assertion(
    cpu: &Cpu,
    assertion: &ExpectedAssertion,
) -> Result<AssertionResult, ConformanceError> {
    let actual_raw = match assertion.location {
        AssertionLocation::Erasable { bank, offset } => cpu
            .memory()
            .read_erasable_physical(bank, offset)
            .ok_or(ConformanceError::AssertionLocation { bank, offset })?
            .raw(),
        AssertionLocation::Register { index } => cpu.central_register(index)?.raw(),
        AssertionLocation::Channel { channel } => cpu
            .memory()
            .read_channel(ChannelAddress::new(channel)?)
            .raw(),
    };
    Ok(AssertionResult {
        id: assertion.id.clone(),
        location: assertion.location,
        expected_raw: assertion.expected_raw,
        actual_raw,
        expected_octal: format!("{:06o}", assertion.expected_raw),
        actual_octal: format!("{actual_raw:06o}"),
        passed: actual_raw == assertion.expected_raw,
        rationale: assertion.rationale.clone(),
    })
}

fn emit_ccs_path(
    builder: &mut Builder,
    id: &'static str,
    constant: &'static str,
    skips: usize,
    scratch: u16,
) {
    builder.emit_label(Mnemonic::Ca, constant, 0);
    builder.emit(Mnemonic::Ts, scratch);
    builder.emit(Mnemonic::Ccs, scratch);
    for _ in 0..skips {
        builder.emit_label(Mnemonic::Tcf, "failure", 0);
    }
    builder.emit_label(Mnemonic::Tcf, id, 0);
    builder.label(id);
}

fn emit_channel_reset(builder: &mut Builder) {
    builder.emit_label(Mnemonic::Ca, "io-left", 0);
    builder.emit_extended(Mnemonic::Write, 0o40);
}

fn add_constants(builder: &mut Builder) -> Result<(), ConformanceError> {
    builder.data_label("positive-zero", AgcWord::POSITIVE_ZERO);
    builder.data_label("negative-zero", AgcWord::NEGATIVE_ZERO);
    builder.data_label("one", AgcWord::from_i32(1)?);
    builder.data_label("two", AgcWord::from_i32(2)?);
    builder.data_label("three", AgcWord::from_i32(3)?);
    builder.data_label("four", AgcWord::from_i32(4)?);
    builder.data_label("five", AgcWord::from_i32(5)?);
    builder.data_label("eight", AgcWord::from_i32(8)?);
    builder.data_label("nine", AgcWord::from_i32(9)?);
    builder.data_label("negative-one", AgcWord::from_i32(-1)?);
    builder.data_label("negative-two", AgcWord::from_i32(-2)?);
    builder.data_label("max-positive", AgcWord::from_raw_truncate(0o37777));
    builder.data_label("min-negative", AgcWord::from_raw_truncate(0o40000));
    builder.data_label("mask-left", AgcWord::from_raw_truncate(0o52525));
    builder.data_label("mask-right", AgcWord::from_raw_truncate(0o33663));
    builder.data_label("io-left", AgcWord::from_raw_truncate(0o52525));
    builder.data_label("io-right", AgcWord::from_raw_truncate(0o33663));
    builder.data_label("io-index-value", AgcWord::from_raw_truncate(0o12345));

    builder.data_label("double-high", AgcWord::from_i32(1)?);
    builder.data_label("double-low", AgcWord::from_i32(2)?);
    builder.data_label("dividend-high", AgcWord::POSITIVE_ZERO);
    builder.data_label("dividend-low", AgcWord::from_i32(10)?);

    builder.data_label("ebank-six", AgcWord::from_raw_truncate(0o03000));
    builder.data_label("ebank-one", AgcWord::from_raw_truncate(0o00400));
    builder.data_label("fbank-four", AgcWord::from_raw_truncate(0o10000));
    builder.data_label("fbank-thirty", AgcWord::from_raw_truncate(0o60000));
    builder.data_label("superbank-enable", AgcWord::from_raw_truncate(0o00100));
    builder.data_label("bank-six-value", AgcWord::from_raw_truncate(0o60606));
    builder.data_label("bank-one-value", AgcWord::from_raw_truncate(0o10101));
    builder.data_label("edrupt-marker", AgcWord::from_raw_truncate(0o45454));
    builder.data_instruction_label(
        "edrupt-recovery-word",
        Mnemonic::Tcf,
        Operand::Label("edrupt-recovery", 0),
        false,
    );
    Ok(())
}

fn add_fixed_bank_routines(builder: &mut Builder) {
    builder.place_instruction(0o04, 0, Mnemonic::Ca, Operand::Literal(0o2003), false);
    builder.place_instruction(0o04, 1, Mnemonic::Ts, Operand::Literal(0o146), false);
    builder.place_instruction(0o04, 2, Mnemonic::Tc, Operand::Literal(register::Q), false);
    builder.place_data(0o04, 3, AgcWord::from_raw_truncate(0o44444));

    builder.place_instruction(0o40, 0, Mnemonic::Ca, Operand::Literal(0o2003), false);
    builder.place_instruction(0o40, 1, Mnemonic::Ts, Operand::Literal(0o147), false);
    builder.place_instruction(0o40, 2, Mnemonic::Tc, Operand::Literal(register::Q), false);
    builder.place_data(0o40, 3, AgcWord::from_raw_truncate(0o40404));
}

#[allow(clippy::too_many_lines)]
fn add_assertions(builder: &mut Builder) {
    let e0 = |offset| AssertionLocation::Erasable { bank: 0, offset };
    let expected = [
        (
            "positive-overflow-store",
            e0(0o100),
            0o00000,
            "TS writes overflow-corrected positive zero",
        ),
        (
            "positive-overflow-residual",
            e0(0o101),
            0o00001,
            "positive overflow leaves +1 in A after the skip",
        ),
        (
            "negative-overflow-store",
            e0(0o102),
            0o77777,
            "TS writes overflow-corrected negative zero",
        ),
        (
            "negative-overflow-residual",
            e0(0o103),
            0o77776,
            "negative overflow leaves -1 in A after the skip",
        ),
        ("ads-result", e0(0o104), 0o00005, "2 + 3 is stored by ADS"),
        (
            "cs-result",
            e0(0o105),
            0o77776,
            "CS 1 produces one's-complement -1",
        ),
        (
            "mask-result",
            e0(0o106),
            0o52525 & 0o33663,
            "MASK is a fifteen-bit bitwise AND",
        ),
        ("su-result", e0(0o107), 0o00002, "SU computes 3 - 1"),
        (
            "msu-result",
            e0(0o110),
            0o00002,
            "MSU computes the modular 5 - 3 result",
        ),
        (
            "incr-result",
            e0(0o111),
            0o00003,
            "INCR advances the prior modular result",
        ),
        (
            "dim-positive-zero-crossing",
            e0(0o112),
            0o77777,
            "DIM of +1 reaches the one's-complement negative-zero encoding",
        ),
        (
            "dim-negative-zero",
            e0(0o113),
            0o77777,
            "AUG then DIM returns -0 through -1",
        ),
        (
            "xch-result",
            e0(0o114),
            0o00003,
            "XCH places the old erasable value in A",
        ),
        (
            "lxch-result",
            e0(0o115),
            0o00002,
            "LXCH places the erasable value in L",
        ),
        (
            "qxch-result",
            e0(0o116),
            0o00003,
            "QXCH places the erasable value in Q",
        ),
        (
            "dxch-a",
            e0(0o117),
            0o00010,
            "DXCH loads the high erasable word into A",
        ),
        (
            "dxch-l",
            e0(0o120),
            0o00011,
            "DXCH loads the low erasable word into L",
        ),
        (
            "dxch-memory-high",
            e0(0o121),
            0o00003,
            "DXCH stores the old A high word",
        ),
        (
            "dxch-memory-low",
            e0(0o122),
            0o00004,
            "DXCH stores the old L low word",
        ),
        (
            "extended-index-read",
            e0(0o123),
            0o12345,
            "extended INDEX changes READ 40 to READ 41",
        ),
        (
            "dca-high",
            e0(0o124),
            0o00001,
            "DCA loads the high word into A",
        ),
        (
            "dca-low",
            e0(0o125),
            0o00002,
            "DCA loads the low word into L",
        ),
        (
            "dcs-high",
            e0(0o126),
            0o77776,
            "DCS complements the high word",
        ),
        (
            "dcs-low",
            e0(0o127),
            0o77775,
            "DCS complements the low word",
        ),
        (
            "das-high",
            e0(0o130),
            0o00004,
            "DAS stores 3 + 1 in the high word",
        ),
        (
            "das-low",
            e0(0o131),
            0o00006,
            "DAS stores 4 + 2 in the low word",
        ),
        (
            "mp-high",
            e0(0o132),
            0o77777,
            "the high word of 3 * -2 is negative sign extension",
        ),
        (
            "mp-low",
            e0(0o133),
            0o77771,
            "the low word of 3 * -2 encodes -6",
        ),
        (
            "dv-quotient",
            e0(0o134),
            0o00003,
            "double-word 10 divided by 3 has quotient 3",
        ),
        (
            "dv-remainder",
            e0(0o135),
            0o00001,
            "double-word 10 divided by 3 has remainder 1",
        ),
        (
            "read-result",
            e0(0o136),
            0o52525,
            "READ returns the channel word",
        ),
        (
            "rand-result",
            e0(0o137),
            0o52525 & 0o33663,
            "RAND reads channel AND A",
        ),
        (
            "wand-result",
            e0(0o140),
            0o52525 & 0o33663,
            "WAND writes channel AND A",
        ),
        (
            "ror-result",
            e0(0o141),
            0o52525 | 0o33663,
            "ROR reads channel OR A",
        ),
        (
            "wor-result",
            e0(0o142),
            0o52525 | 0o33663,
            "WOR writes channel OR A",
        ),
        (
            "rxor-result",
            e0(0o143),
            0o52525 ^ 0o33663,
            "RXOR reads channel XOR A",
        ),
        (
            "selected-ebank-value",
            e0(0o144),
            0o60606,
            "switched erasable bank 6 retains its own value",
        ),
        (
            "combined-bank-register",
            e0(0o145),
            0o10006,
            "BB combines FBANK 4 with EBANK 6",
        ),
        (
            "fixed-bank-four",
            e0(0o146),
            0o44444,
            "TC through FBANK 4 executes bank 4",
        ),
        (
            "fixed-superbank-forty",
            e0(0o147),
            0o40404,
            "channel 7 maps FBANK 30 to physical superbank 40",
        ),
        (
            "edrupt-recovery",
            e0(0o150),
            0o45454,
            "EDRUPT vector-zero recovery reached the marker",
        ),
    ];
    for (id, location, value, rationale) in expected {
        builder.assertion(id, location, value, rationale);
    }
    builder.assertion(
        "physical-ebank-one",
        AssertionLocation::Erasable { bank: 1, offset: 0 },
        0o10101,
        "physical EBANK 1 remains isolated from EBANK 6",
    );
    builder.assertion(
        "physical-ebank-six",
        AssertionLocation::Erasable { bank: 6, offset: 0 },
        0o60606,
        "physical EBANK 6 retains its selected-window write",
    );
    builder.assertion(
        "superbank-channel",
        AssertionLocation::Channel { channel: 0o7 },
        0o00100,
        "channel 7 retains the superbank selector bit",
    );
    builder.assertion(
        "final-ebank",
        AssertionLocation::Register {
            index: register::EB,
        },
        0o03000,
        "the final selected erasable bank is 6",
    );
    builder.assertion(
        "final-fbank",
        AssertionLocation::Register {
            index: register::FB,
        },
        0o60000,
        "the final pre-superbank FBANK selection is 30 octal",
    );
}

#[derive(Clone, Copy, Debug)]
enum Operand {
    Literal(u16),
    Label(&'static str, i16),
}

#[derive(Clone, Copy, Debug)]
struct InstructionSpec {
    bank: u8,
    offset: u16,
    mnemonic: Mnemonic,
    operand: Operand,
    extended_context: bool,
}

#[derive(Clone, Copy, Debug)]
struct DataInstructionSpec {
    bank: u8,
    offset: u16,
    mnemonic: Mnemonic,
    operand: Operand,
    extended_context: bool,
}

#[derive(Clone, Debug)]
struct PendingCase {
    id: &'static str,
    objective: &'static str,
    start_pc: u16,
    required_mnemonics: Vec<Mnemonic>,
    boundaries: Vec<String>,
}

#[derive(Debug)]
struct Builder {
    pc: u16,
    constant_pc: u16,
    labels: BTreeMap<&'static str, u16>,
    instructions: Vec<InstructionSpec>,
    data_instructions: Vec<DataInstructionSpec>,
    data: Vec<(u8, u16, AgcWord)>,
    cases: Vec<CaseDefinition>,
    current_case: Option<PendingCase>,
    assertions: Vec<ExpectedAssertion>,
}

impl Builder {
    fn new() -> Self {
        Self {
            pc: START_PC,
            constant_pc: CONSTANT_PC,
            labels: BTreeMap::new(),
            instructions: Vec::new(),
            data_instructions: Vec::new(),
            data: Vec::new(),
            cases: Vec::new(),
            current_case: None,
            assertions: Vec::new(),
        }
    }

    fn label(&mut self, label: &'static str) {
        self.label_at(label, self.pc);
    }

    fn label_at(&mut self, label: &'static str, address: u16) {
        assert!(self.labels.insert(label, address).is_none());
    }

    fn emit(&mut self, mnemonic: Mnemonic, operand: u16) {
        self.emit_context(mnemonic, Operand::Literal(operand), false);
    }

    fn emit_label(&mut self, mnemonic: Mnemonic, label: &'static str, delta: i16) {
        self.emit_context(mnemonic, Operand::Label(label, delta), false);
    }

    fn emit_extended(&mut self, mnemonic: Mnemonic, operand: u16) {
        self.emit(Mnemonic::Extend, 0);
        self.emit_context(mnemonic, Operand::Literal(operand), true);
    }

    fn emit_extended_label(&mut self, mnemonic: Mnemonic, label: &'static str, delta: i16) {
        self.emit(Mnemonic::Extend, 0);
        self.emit_context(mnemonic, Operand::Label(label, delta), true);
    }

    fn emit_context(&mut self, mnemonic: Mnemonic, operand: Operand, extended_context: bool) {
        let pc = self.pc;
        assert!((0o4000..=0o5777).contains(&pc));
        self.place_instruction(2, pc & 0o1777, mnemonic, operand, extended_context);
        self.pc += 1;
    }

    fn place_instruction(
        &mut self,
        bank: u8,
        offset: u16,
        mnemonic: Mnemonic,
        operand: Operand,
        extended_context: bool,
    ) {
        self.instructions.push(InstructionSpec {
            bank,
            offset,
            mnemonic,
            operand,
            extended_context,
        });
    }

    fn place_data(&mut self, bank: u8, offset: u16, word: AgcWord) {
        self.data.push((bank, offset, word));
    }

    fn data_label(&mut self, label: &'static str, word: AgcWord) {
        let address = self.constant_pc;
        self.label_at(label, address);
        self.place_data(3, address & 0o1777, word);
        self.constant_pc += 1;
    }

    fn data_instruction_label(
        &mut self,
        label: &'static str,
        mnemonic: Mnemonic,
        operand: Operand,
        extended_context: bool,
    ) {
        let address = self.constant_pc;
        self.label_at(label, address);
        self.data_instructions.push(DataInstructionSpec {
            bank: 3,
            offset: address & 0o1777,
            mnemonic,
            operand,
            extended_context,
        });
        self.constant_pc += 1;
    }

    fn start_case(
        &mut self,
        id: &'static str,
        objective: &'static str,
        required_mnemonics: Vec<Mnemonic>,
        boundaries: &[&str],
    ) {
        assert!(self.current_case.is_none());
        self.current_case = Some(PendingCase {
            id,
            objective,
            start_pc: self.pc,
            required_mnemonics,
            boundaries: boundaries.iter().map(ToString::to_string).collect(),
        });
    }

    fn end_case(&mut self) {
        let case = self.current_case.take().expect("a case is open");
        self.cases.push(CaseDefinition {
            id: case.id.to_owned(),
            objective: case.objective.to_owned(),
            start_pc: case.start_pc,
            end_pc_exclusive: self.pc,
            required_mnemonics: case.required_mnemonics,
            boundaries: case.boundaries,
        });
    }

    fn assertion(
        &mut self,
        id: &str,
        location: AssertionLocation,
        expected_raw: u16,
        rationale: &str,
    ) {
        self.assertions.push(ExpectedAssertion {
            id: id.to_owned(),
            location,
            expected_raw,
            rationale: rationale.to_owned(),
        });
    }

    fn finish(self) -> Result<GeneratedSuite, ConformanceError> {
        assert!(self.current_case.is_none());
        let mut words = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        let mut occupied = vec![false; words.len()];
        for spec in self.instructions {
            let operand = resolve_operand(spec.operand, &self.labels)?;
            let word = encode_with_context(spec.mnemonic, operand, spec.extended_context)?;
            place_word(&mut words, &mut occupied, spec.bank, spec.offset, word)?;
        }
        for spec in self.data_instructions {
            let operand = resolve_operand(spec.operand, &self.labels)?;
            let word = encode_with_context(spec.mnemonic, operand, spec.extended_context)?;
            place_word(&mut words, &mut occupied, spec.bank, spec.offset, word)?;
        }
        for (bank, offset, word) in self.data {
            place_word(&mut words, &mut occupied, bank, offset, word)?;
        }

        let mut required_forms = Mnemonic::ALL
            .iter()
            .copied()
            .map(|mnemonic| InstructionForm {
                mnemonic,
                extended: mnemonic.is_extended(),
            })
            .collect::<Vec<_>>();
        required_forms.push(InstructionForm {
            mnemonic: Mnemonic::Index,
            extended: true,
        });
        required_forms.sort();
        let terminal_pc = *self
            .labels
            .get("terminal")
            .ok_or_else(|| ConformanceError::MissingLabel("terminal".to_owned()))?;
        let failure_pc = *self
            .labels
            .get("failure")
            .ok_or_else(|| ConformanceError::MissingLabel("failure".to_owned()))?;
        Ok(GeneratedSuite {
            rope_words: words,
            manifest: SuiteManifest {
                schema_version: SUITE_SCHEMA_VERSION,
                suite_id: SUITE_ID.to_owned(),
                restart_pc: RESTART_PC,
                start_pc: START_PC,
                terminal_pc,
                failure_pc,
                maximum_instructions: MAX_INSTRUCTIONS,
                cases: self.cases,
                assertions: self.assertions,
                required_mnemonics: Mnemonic::ALL.to_vec(),
                required_forms,
            },
        })
    }
}

fn resolve_operand(
    operand: Operand,
    labels: &BTreeMap<&'static str, u16>,
) -> Result<u16, ConformanceError> {
    match operand {
        Operand::Literal(value) => Ok(value),
        Operand::Label(label, delta) => {
            let base = labels
                .get(label)
                .copied()
                .ok_or_else(|| ConformanceError::MissingLabel(label.to_owned()))?;
            Ok(base.wrapping_add_signed(delta) & 0o7777)
        }
    }
}

fn place_word(
    words: &mut [AgcWord],
    occupied: &mut [bool],
    bank: u8,
    offset: u16,
    word: AgcWord,
) -> Result<(), ConformanceError> {
    let index = usize::from(bank) * FIXED_WORDS_PER_BANK + usize::from(offset);
    if occupied[index] {
        return Err(ConformanceError::Overlap { bank, offset });
    }
    occupied[index] = true;
    words[index] = word;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_loader::{RopeFormat, decode_bytes, encode_yayul};

    #[test]
    fn generated_rope_round_trips_through_yayul_order() {
        let suite = generate_block_ii_suite().unwrap();
        let bytes = encode_yayul(&suite.rope_words).unwrap();
        let decoded = decode_bytes(&bytes, RopeFormat::Yayul).unwrap();
        assert_eq!(decoded.words, suite.rope_words);
    }

    #[test]
    fn suite_reaches_terminal_and_covers_every_required_form() {
        let suite = generate_block_ii_suite().unwrap();
        let execution = execute_suite(&suite).unwrap();
        assert!(execution.report.terminal_reached, "{:#?}", execution.report);
        assert!(!execution.report.failure_reached);
        assert!(
            execution.report.missing_mnemonics.is_empty(),
            "{:?}",
            execution.report.missing_mnemonics
        );
        assert!(
            execution.report.missing_forms.is_empty(),
            "{:?}",
            execution.report.missing_forms
        );
        let failed = execution
            .report
            .assertions
            .iter()
            .filter(|assertion| !assertion.passed)
            .collect::<Vec<_>>();
        assert!(failed.is_empty(), "{failed:#?}");
        assert!(execution.report.passed, "{:#?}", execution.report);
    }

    #[test]
    fn canonical_instruction_list_has_no_duplicates() {
        let unique = Mnemonic::ALL.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), Mnemonic::ALL.len());
    }
}
