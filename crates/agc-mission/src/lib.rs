#![forbid(unsafe_code)]
//! Real-execution mission scenarios, synchronized DSKY state, and visualization frames.

use agc_cpu::{Cpu, CpuError};
use agc_dsky::{DskyState, Key, V37ProgramChange};
use agc_faults::{Fault, FaultEngine, FaultError};
use agc_interpreter::State as InterpreterState;
use agc_loader::RopeImage;
use agc_memory::MemoryError;
use agc_runtime::{
    ExecutiveJob, FlightSoftwareObserver, HardwareState, Runtime, RuntimeError, RuntimeEvent,
    WaitlistTask,
};
use agc_trace::InterruptEvent;
use agc_validation::{ValidationReport, compare_traces};
use agc_word::AgcWord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Input scheduled by committed instruction rather than wall-clock time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionInput {
    /// Instruction boundary.
    pub instruction: u64,
    /// Runtime event.
    pub event: RuntimeEvent,
}

/// One retained or pad-loaded erasable word required by a mission fixture.
///
/// Initial values are deliberately data, not hidden emulator behavior.  The
/// provenance fields make the distinction between a historical precondition
/// and a value invented merely to drive a demonstration auditable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErasableInitialization {
    /// Historical symbol.
    pub symbol: String,
    /// Physical erasable bank.
    pub bank: u8,
    /// Offset within the physical bank.
    pub offset: u16,
    /// Raw one's-complement word loaded before execution.
    pub value: AgcWord,
    /// Historical source location or other primary evidence.
    pub provenance: String,
    /// Why the scenario requires this value.
    pub rationale: String,
}

/// A DSKY sequence paced by AGC cycles and software acknowledgement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DskySequence {
    /// Earliest machine cycle at which the first key can be delivered.
    pub start_cycle: u64,
    /// Minimum cycles between host key deliveries.
    pub minimum_gap_cycles: u64,
    /// Five-bit channel 015 key codes in operator order.
    pub keys: Vec<u8>,
}

/// Named erasable guidance value exposed in visualization frames.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuidanceVariable {
    /// Historical symbol or research name.
    pub name: String,
    /// Physical erasable bank.
    pub bank: u8,
    /// Offset within the physical bank.
    pub offset: u16,
}

/// Mission scenario supplied to the real runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionScenario {
    /// Stable scenario name.
    pub name: String,
    /// Exact instruction budget.
    pub instruction_limit: u64,
    /// Frame sampling interval in committed instructions.
    pub frame_interval: u64,
    /// Inputs in non-decreasing instruction order.
    pub inputs: Vec<MissionInput>,
    /// Explicit erasable state loaded before the first instruction.
    #[serde(default)]
    pub initialization: Vec<ErasableInitialization>,
    /// Optional software-paced DSKY operator sequence.
    #[serde(default)]
    pub dsky_sequence: Option<DskySequence>,
    /// Guidance variables read from real erasable state.
    pub guidance_variables: Vec<GuidanceVariable>,
    /// Optional Executive/Waitlist observer layout.
    pub observer: FlightSoftwareObserver,
}

impl MissionScenario {
    /// Real Luminary keyboard sequence requesting program 63 via V37E63E.
    ///
    /// Keypresses are separated so KEYRUPT and Pinball consume each one. The
    /// profile contains no scripted display or trajectory output; every frame
    /// comes from the loaded Luminary rope.
    pub fn luminary_p63_landing() -> Self {
        let keys = [
            Key::Verb,
            Key::Digit(3),
            Key::Digit(7),
            Key::Enter,
            Key::Digit(6),
            Key::Digit(3),
            Key::Enter,
        ];
        Self {
            name: "apollo11-lm5-padload-p63-entry".to_owned(),
            instruction_limit: 1_000_000,
            frame_interval: 5_000,
            inputs: Vec::new(),
            initialization: apollo11_lm5_p63_initialization(),
            dsky_sequence: Some(DskySequence {
                start_cycle: 40_000,
                minimum_gap_cycles: 32_768,
                keys: keys
                    .into_iter()
                    .map(|key| key.code().expect("built-in DSKY key is valid"))
                    .collect(),
            }),
            guidance_variables: luminary_p63_variables(),
            observer: FlightSoftwareObserver::default(),
        }
    }

    /// Validates deterministic ordering and non-zero limits.
    pub fn validate(&self) -> Result<(), MissionError> {
        if self.instruction_limit == 0 || self.frame_interval == 0 {
            return Err(MissionError::Scenario(
                "instruction_limit and frame_interval must be non-zero".to_owned(),
            ));
        }
        if self
            .inputs
            .windows(2)
            .any(|window| window[0].instruction > window[1].instruction)
        {
            return Err(MissionError::Scenario(
                "mission inputs must be ordered by instruction".to_owned(),
            ));
        }
        if let Some(sequence) = &self.dsky_sequence {
            if sequence.minimum_gap_cycles == 0 {
                return Err(MissionError::Scenario(
                    "DSKY minimum_gap_cycles must be non-zero".to_owned(),
                ));
            }
            if sequence.keys.iter().any(|code| *code > 0o37) {
                return Err(MissionError::Scenario(
                    "DSKY key codes must fit channel 015's five-bit field".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

fn apollo11_lm5_p63_initialization() -> Vec<ErasableInitialization> {
    let mut words = Vec::new();
    words.push(ErasableInitialization {
        symbol: "FLAGWRD3 (LM-5 padload + REFSMFLG)".to_owned(),
        bank: 0,
        offset: 0o077,
        value: AgcWord::try_from_raw(0o12000).expect("combined flag word is valid"),
        provenance: concat!(
            "LM-5 Data Book table LM5/4.5.1-1 page 2: FLAGWORD3=02000; ",
            "Luminary099/FLAGWORD_ASSIGNMENTS.agc: REFSMBIT=BIT13 (10000); ",
            "Luminary099/IMU_MODE_SWITCHING_ROUTINES.agc: R02BOTH requires known REFSMMAT"
        )
        .to_owned(),
        rationale: concat!(
            "Retain the documented Apollo 11 padload flag and the in-flight post-alignment ",
            "REFSMFLG required before selecting P63. This is an entry fixture, not a claim ",
            "that blank-memory startup reproduces the complete mission timeline."
        )
        .to_owned(),
    });

    const PADLOAD: &[(&str, u16, u16, u8)] = &[
        // Reference stable-member matrix: mission-tape first row and launch-tape remainder.
        ("REFSMMAT.0.HI", 0o1733, 0o12704, 2),
        ("REFSMMAT.0.LO", 0o1734, 0o06264, 2),
        ("REFSMMAT.1.HI", 0o1735, 0o12562, 18),
        ("REFSMMAT.1.LO", 0o1736, 0o10723, 18),
        ("REFSMMAT.2.HI", 0o1737, 0o01112, 18),
        ("REFSMMAT.2.LO", 0o1740, 0o25001, 18),
        // Apollo 11 landing time and landing-site vector.
        ("TLAND.HI", 0o2400, 0o04247, 4),
        ("TLAND.LO", 0o2401, 0o34030, 4),
        ("RLS.X.HI", 0o2022, 0o00301, 18),
        ("RLS.X.LO", 0o2023, 0o34760, 18),
        ("RLS.Y.HI", 0o2024, 0o00125, 18),
        ("RLS.Y.LO", 0o2025, 0o04627, 18),
        ("RLS.Z.HI", 0o2026, 0o00002, 18),
        ("RLS.Z.LO", 0o2027, 0o24342, 18),
        // P63 ignition targeting constants.
        ("VIGN.HI", 0o2472, 0o00416, 7),
        ("VIGN.LO", 0o2473, 0o16071, 7),
        ("RIGNX.HI", 0o2474, 0o77731, 7),
        ("RIGNX.LO", 0o2475, 0o44630, 7),
        ("RIGNZ.HI", 0o2476, 0o77125, 7),
        ("RIGNZ.LO", 0o2477, 0o62404, 7),
        ("KIGNX/B4.HI", 0o2500, 0o76607, 7),
        ("KIGNX/B4.LO", 0o2501, 0o61356, 7),
        ("KIGNY/B8.HI", 0o2502, 0o72634, 7),
        ("KIGNY/B8.LO", 0o2503, 0o51602, 7),
        ("KIGNV/B4.HI", 0o2504, 0o72775, 7),
        ("KIGNV/B4.LO", 0o2505, 0o57777, 7),
        // Landing-radar scales and filters.
        ("LRALPHA1", 0o2522, 0o01027, 8),
        ("LRBETA1", 0o2523, 0o04204, 8),
        ("LRALPHA2", 0o2524, 0o01022, 8),
        ("LRBETA2", 0o2525, 0o00004, 8),
        ("LRVMAX", 0o2526, 0o01414, 8),
        ("LRVF", 0o2527, 0o00116, 9),
        ("LRWVZ", 0o2530, 0o11463, 9),
        ("LRWVY", 0o2531, 0o11463, 9),
        ("LRWVX", 0o2532, 0o11463, 9),
        ("LRWVFZ", 0o2533, 0o06315, 9),
        ("LRWVFY", 0o2534, 0o06315, 9),
        ("LRWVFX", 0o2535, 0o06315, 9),
        ("LRWVFF", 0o2536, 0o03146, 9),
        ("RODSCALE", 0o2537, 0o14370, 9),
        // Landing phase timing criteria.
        ("LRHMAX", 0o3420, 0o35610, 14),
        ("LRWH", 0o3421, 0o13146, 14),
        ("ZOOMTIME", 0o3422, 0o05050, 14),
        ("TENDBRAK", 0o3423, 0o01407, 15),
        ("TENDAPPR", 0o3424, 0o00226, 15),
        ("DELTTFAP", 0o3425, 0o75240, 15),
        ("LEADTIME", 0o3426, 0o77743, 15),
        ("RPCRTIME", 0o3427, 0o01407, 15),
        ("RPCRTQSW", 0o3430, 0o37777, 15),
        ("TNEWA.HI", 0o3431, 0o20000, 15),
        ("TNEWA.LO", 0o3432, 0o00000, 15),
    ];
    words.extend(PADLOAD.iter().map(|&(symbol, address, value, page)| {
        ErasableInitialization {
            symbol: symbol.to_owned(),
            bank: (address / 0o400) as u8,
            offset: address % 0o400,
            value: AgcWord::try_from_raw(value).expect("transcribed padload word is valid"),
            provenance: format!(
                "LM-5 Data Book table LM5/4.5.1-1, PDF page {page}; https://www.ibiblio.org/apollo/Documents/Luminary99PadLoads.pdf"
            ),
            rationale: "Transcribed Apollo 11 LM-5 Luminary 99 erasable padload word".to_owned(),
        }
    }));
    words
}

fn luminary_p63_variables() -> Vec<GuidanceVariable> {
    const VARIABLES: &[(&str, u8, u16)] = &[
        ("MPAC", 0, 0o154),
        ("DSPCOUNT", 1, 0o377),
        ("DECBRNCH", 2, 0o000),
        ("VERBREG", 2, 0o001),
        ("NOUNREG", 2, 0o002),
        ("MODREG", 2, 0o011),
        ("DSPLOCK", 2, 0o012),
        ("CADRSTOR", 2, 0o042),
        ("DVTHRUSH", 2, 0o251),
        ("WCHPHASE", 2, 0o351),
        ("RLS.X.HI", 4, 0o022),
        ("RLS.X.LO", 4, 0o023),
        ("RLS.Y.HI", 4, 0o024),
        ("RLS.Y.LO", 4, 0o025),
        ("RLS.Z.HI", 4, 0o026),
        ("RLS.Z.LO", 4, 0o027),
        ("TLAND.HI", 5, 0o000),
        ("TLAND.LO", 5, 0o001),
        ("RANGEDSP.HI", 5, 0o224),
        ("RANGEDSP.LO", 5, 0o225),
        ("R60VSAVE.X.HI", 5, 0o230),
        ("R60VSAVE.X.LO", 5, 0o231),
        ("R60VSAVE.Y.HI", 5, 0o232),
        ("R60VSAVE.Y.LO", 5, 0o233),
        ("R60VSAVE.Z.HI", 5, 0o234),
        ("R60VSAVE.Z.LO", 5, 0o235),
        ("RGU.X.HI", 5, 0o236),
        ("RGU.X.LO", 5, 0o237),
        ("RGU.Y.HI", 5, 0o240),
        ("RGU.Y.LO", 5, 0o241),
        ("RGU.Z.HI", 5, 0o242),
        ("RGU.Z.LO", 5, 0o243),
        ("TIG.HI", 7, 0o041),
        ("TIG.LO", 7, 0o042),
        ("WHICH", 7, 0o055),
        ("DVCNTR", 7, 0o115),
        ("R.X.HI", 7, 0o120),
        ("R.X.LO", 7, 0o121),
        ("R.Y.HI", 7, 0o122),
        ("R.Y.LO", 7, 0o123),
        ("R.Z.HI", 7, 0o124),
        ("R.Z.LO", 7, 0o125),
        ("V.X.HI", 7, 0o126),
        ("V.X.LO", 7, 0o127),
        ("V.Y.HI", 7, 0o130),
        ("V.Y.LO", 7, 0o131),
        ("V.Z.HI", 7, 0o132),
        ("V.Z.LO", 7, 0o133),
        ("TTF/8TMP.HI", 7, 0o152),
        ("TTF/8TMP.LO", 7, 0o153),
        ("PIPTIME1.HI", 7, 0o160),
        ("PIPTIME1.LO", 7, 0o161),
        ("FLPASS0", 7, 0o223),
        ("TPIP.HI", 7, 0o224),
        ("TPIP.LO", 7, 0o225),
        ("VGU.X.HI", 7, 0o226),
        ("VGU.X.LO", 7, 0o227),
        ("VGU.Y.HI", 7, 0o230),
        ("VGU.Y.LO", 7, 0o231),
        ("VGU.Z.HI", 7, 0o232),
        ("VGU.Z.LO", 7, 0o233),
        ("LAND.X.HI", 7, 0o234),
        ("LAND.X.LO", 7, 0o235),
        ("LAND.Y.HI", 7, 0o236),
        ("LAND.Y.LO", 7, 0o237),
        ("LAND.Z.HI", 7, 0o240),
        ("LAND.Z.LO", 7, 0o241),
        ("TTF/8.HI", 7, 0o242),
        ("TTF/8.LO", 7, 0o243),
    ];
    VARIABLES
        .iter()
        .map(|&(name, bank, offset)| GuidanceVariable {
            name: name.to_owned(),
            bank,
            offset,
        })
        .collect()
}

/// Words initialized immediately after `R02BOTH` returns to `P63LM`.
/// Requiring the complete set prevents Pinball scratch activity from being
/// mistaken for landing-guidance execution.
const P63_INITIALIZATION_WORDS: [&str; 5] = ["WHICH", "DVTHRUSH", "DVCNTR", "WCHPHASE", "FLPASS0"];

/// Typed landing-guidance phase selected by the `P63LM` initialization block.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LandingGuidancePhase {
    /// `WCHPHASE = -1`, selecting the `IGNALG` flight-sequence row.
    IgnitionAlgorithm,
}

impl LandingGuidancePhase {
    /// Exact one's-complement word used by Luminary 099.
    pub const fn raw_word(self) -> AgcWord {
        match self {
            Self::IgnitionAlgorithm => AgcWord::from_raw_truncate(0o77776),
        }
    }
}

/// Readable Rust reconstruction of Luminary 099 `P63LM` lines 46–58.
///
/// This model deliberately covers only the five-word initialization after the
/// `R02BOTH` status check. It is compared with writes made by the original rope
/// and is not substituted into emulation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct P63Initialization {
    /// `P63ADRES`, consumed by the shared ignition routine through `WHICH`.
    pub burn_baby_address: AgcWord,
    /// `DPSTHRSH`, the velocity-change monitor threshold.
    pub delta_velocity_threshold: AgcWord,
    /// Number of velocity samples required by the monitor.
    pub delta_velocity_counter: AgcWord,
    /// Initial landing-guidance equation phase.
    pub guidance_phase: LandingGuidancePhase,
    /// Initial pass counter for the landing equations.
    pub first_pass: AgcWord,
}

impl P63Initialization {
    /// Reconstructs the constants encoded by flown Luminary 099.
    pub const fn luminary099() -> Self {
        Self {
            burn_baby_address: AgcWord::from_raw_truncate(0o02076),
            delta_velocity_threshold: AgcWord::from_raw_truncate(0o00044),
            delta_velocity_counter: AgcWord::from_raw_truncate(0o00004),
            guidance_phase: LandingGuidancePhase::IgnitionAlgorithm,
            first_pass: AgcWord::POSITIVE_ZERO,
        }
    }

    /// Requires the original rope to write every reconstructed value in source
    /// order. Additional later writes do not affect this initialization check.
    pub fn matches_rope_writes(&self, writes: &[GuidanceWrite]) -> bool {
        let expected = [
            ("WHICH", self.burn_baby_address),
            ("DVTHRUSH", self.delta_velocity_threshold),
            ("DVCNTR", self.delta_velocity_counter),
            ("WCHPHASE", self.guidance_phase.raw_word()),
            ("FLPASS0", self.first_pass),
        ];
        let mut previous_sequence = None;
        expected.into_iter().all(|(name, value)| {
            let Some(write) = writes.iter().find(|write| write.name == name) else {
                return false;
            };
            let ordered =
                previous_sequence.is_none_or(|sequence| write.milestone.trace_sequence > sequence);
            previous_sequence = Some(write.milestone.trace_sequence);
            ordered && write.value == value
        })
    }
}

/// One synchronized visualization/debugging frame backed by runtime state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionFrame {
    /// Committed instruction count.
    pub instruction: u64,
    /// Machine cycle.
    pub cycle: u64,
    /// Program counter.
    pub pc: u16,
    /// Full A.
    pub a: u16,
    /// L.
    pub l: u16,
    /// Full Q.
    pub q: u16,
    /// EBANK.
    pub eb: u16,
    /// FBANK.
    pub fb: u16,
    /// DSKY state after consuming this instruction's channel writes.
    pub dsky: DskyState,
    /// Configured real guidance words.
    pub guidance: BTreeMap<String, AgcWord>,
    /// Observed Executive jobs.
    pub executive: Vec<ExecutiveJob>,
    /// Observed Waitlist tasks.
    pub waitlist: Vec<WaitlistTask>,
    /// Physical sensor/telemetry state.
    pub hardware: HardwareState,
}

/// One trace-backed flight-software milestone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionMilestone {
    /// Human-readable historical symbol or event.
    pub name: String,
    /// Architectural trace sequence.
    pub trace_sequence: u64,
    /// Committed instruction count.
    pub instruction: u64,
    /// Machine cycle at commit.
    pub cycle: u64,
    /// Logical program counter.
    pub pc: u16,
    /// Physical fetch, interrupt vector, or erasable-write location.
    pub physical: String,
}

/// Auditable journey of one DSKY key through interrupt and Pinball.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeyEvidence {
    /// Five-bit channel 015 code.
    pub code: u8,
    /// Instruction boundary at which the host supplied the key.
    pub requested_instruction: u64,
    /// Machine cycle at which the host supplied the key.
    pub requested_cycle: u64,
    /// KEYRUPT1 acceptance, if observed.
    pub keyrupt: Option<MissionMilestone>,
    /// Pinball `CHARIN` execution, if observed.
    pub charin: Option<MissionMilestone>,
}

/// A named P63 erasable word that changed after entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuidanceChange {
    /// Historical symbol/component.
    pub name: String,
    /// Physical erasable bank.
    pub bank: u8,
    /// Offset within the bank.
    pub offset: u16,
    /// Value when `P63LM` was first fetched.
    pub at_p63_entry: AgcWord,
    /// Value at the end of the run.
    pub final_value: AgcWord,
}

/// First trace-backed write to a named landing-guidance word after `P63LM`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuidanceWrite {
    /// Historical symbol/component.
    pub name: String,
    /// Physical erasable bank.
    pub bank: u8,
    /// Offset within the bank.
    pub offset: u16,
    /// Value written by the rope instruction.
    pub value: AgcWord,
    /// Exact instruction and cycle that performed the write.
    pub milestone: MissionMilestone,
}

/// One explicit pre-execution erasable-memory load and the value it replaced.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppliedInitialization {
    /// Historical symbol.
    pub symbol: String,
    /// Physical erasable bank.
    pub bank: u8,
    /// Offset within the physical bank.
    pub offset: u16,
    /// Value before scenario setup.
    pub previous_value: AgcWord,
    /// Value loaded for the scenario.
    pub configured_value: AgcWord,
    /// Historical source location or other primary evidence.
    pub provenance: String,
    /// Why the scenario requires this value.
    pub rationale: String,
}

/// Evidence required before calling a run a verified P63 request.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionEvidence {
    /// Explicit pre-execution state changes; empty means no hidden setup.
    pub initialization: Vec<AppliedInitialization>,
    /// DSKY keys and their software acceptance path.
    pub keys: Vec<KeyEvidence>,
    /// First point at which `MODREG` contained decimal 63.
    pub program_63_selected: Option<MissionMilestone>,
    /// Readable Rust reconstruction fed only keys accepted by rope `CHARIN`.
    pub pinball_reconstruction: Option<V37ProgramChange>,
    /// True when reconstructed V37 output and rope `MODREG` select the same
    /// major mode.
    pub pinball_reconstruction_matches_rope: bool,
    /// First execution of `P63LM` at physical rope location 32,2776.
    pub p63lm_entry: Option<MissionMilestone>,
    /// First execution of `P63SPOT`, if reached.
    pub p63spot_entry: Option<MissionMilestone>,
    /// First execution of `P63SPOT2`, if reached.
    pub p63spot2_entry: Option<MissionMilestone>,
    /// P63-named erasable words changed after `P63LM` entry.
    pub guidance_changes: Vec<GuidanceChange>,
    /// First observed rope write to each configured guidance word after entry.
    pub guidance_writes: Vec<GuidanceWrite>,
    /// Readable reconstruction of the five-word `P63LM` initialization.
    pub p63_initialization: Option<P63Initialization>,
    /// True when original-rope writes match the typed P63 initialization in
    /// source order and raw word value.
    pub p63_initialization_matches_rope: bool,
    /// True when every supplied key traversed KEYRUPT1 and Pinball `CHARIN`.
    pub keyboard_sequence_verified: bool,
    /// True when the DSKY selected decimal program 63 and fetched `P63LM`.
    pub verified_p63_request: bool,
    /// True only when the P63 initialization words were all changed after
    /// entry, proving execution continued beyond the `R02BOTH` IMU gate.
    pub landing_guidance_started: bool,
}

/// Complete mission result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionRun {
    /// Scenario name.
    pub scenario: String,
    /// Sampled frames.
    pub frames: Vec<MissionFrame>,
    /// Final DSKY.
    pub final_dsky: DskyState,
    /// Instructions committed.
    pub instructions: u64,
    /// Cycles committed.
    pub cycles: u64,
    /// Fault audit trail length.
    pub faults_applied: usize,
    /// Trace-backed mission acceptance evidence.
    pub evidence: MissionEvidence,
}

/// Mission setup or execution failure.
#[derive(Debug, Error)]
pub enum MissionError {
    /// Invalid scenario.
    #[error("invalid mission scenario: {0}")]
    Scenario(String),
    /// Runtime failure.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Fault engine failure.
    #[error(transparent)]
    Fault(#[from] FaultError),
    /// CPU register inspection failed.
    #[error(transparent)]
    Cpu(#[from] CpuError),
    /// Physical memory setup failed.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// Rope cannot initialize memory.
    #[error("rope cannot initialize memory: {0}")]
    Rope(String),
}

/// Mission controller around one real emulated runtime.
#[derive(Clone, Debug)]
pub struct MissionController {
    runtime: Runtime,
    dsky: DskyState,
    faults: FaultEngine,
}

impl MissionController {
    /// Creates a controller from a decoded rope image.
    pub fn from_rope(rope: RopeImage) -> Result<Self, MissionError> {
        let memory = rope
            .into_memory()
            .map_err(|error| MissionError::Rope(error.to_string()))?;
        Ok(Self {
            runtime: Runtime::new(Cpu::new(memory)),
            dsky: DskyState::default(),
            faults: FaultEngine::default(),
        })
    }

    /// Creates a controller around a prepared runtime.
    pub fn from_runtime(runtime: Runtime) -> Self {
        Self {
            runtime,
            dsky: DskyState::default(),
            faults: FaultEngine::default(),
        }
    }

    /// Returns the live runtime.
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Returns the live runtime mutably for debugger setup.
    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// Schedules one fault by instruction boundary.
    pub fn schedule_fault(&mut self, instruction: u64, fault: Fault) {
        self.faults.schedule(instruction, fault);
    }

    /// Executes a scenario and emits only frames sampled from real state.
    pub fn run(&mut self, scenario: &MissionScenario) -> Result<MissionRun, MissionError> {
        scenario.validate()?;
        if !scenario.initialization.is_empty() && self.runtime.cpu().instructions() != 0 {
            return Err(MissionError::Scenario(
                "erasable initialization can only be applied before the first instruction"
                    .to_owned(),
            ));
        }
        self.runtime.set_observer(scenario.observer.clone());
        let mut input_index = 0;
        let mut dsky_index = 0;
        let mut next_dsky_cycle = scenario
            .dsky_sequence
            .as_ref()
            .map_or(u64::MAX, |sequence| sequence.start_cycle);
        let mut frames = Vec::new();
        let mut evidence = MissionEvidence::default();
        for initialization in &scenario.initialization {
            let previous_value = self
                .runtime
                .cpu()
                .memory()
                .read_erasable_physical(initialization.bank, initialization.offset)
                .ok_or_else(|| {
                    MissionError::Scenario(format!(
                        "initialization {} has invalid E{}:{:04o} address",
                        initialization.symbol, initialization.bank, initialization.offset
                    ))
                })?;
            self.runtime
                .cpu_mut()
                .memory_mut()
                .write_erasable_physical(
                    initialization.bank,
                    initialization.offset,
                    initialization.value,
                )?;
            evidence.initialization.push(AppliedInitialization {
                symbol: initialization.symbol.clone(),
                bank: initialization.bank,
                offset: initialization.offset,
                previous_value,
                configured_value: initialization.value,
                provenance: initialization.provenance.clone(),
                rationale: initialization.rationale.clone(),
            });
        }
        let mut p63_entry_values: Option<BTreeMap<String, AgcWord>> = None;
        let mut pinball_reconstruction = scenario
            .dsky_sequence
            .as_ref()
            .filter(|sequence| sequence.keys == [0o21, 3, 7, 0o34, 6, 3, 0o34])
            .map(|_| V37ProgramChange::default());
        while self.runtime.cpu().instructions() < scenario.instruction_limit {
            let instruction = self.runtime.cpu().instructions();
            while scenario
                .inputs
                .get(input_index)
                .is_some_and(|input| input.instruction <= instruction)
            {
                let input = &scenario.inputs[input_index];
                if let RuntimeEvent::DskyKey { code } = &input.event {
                    evidence.keys.push(KeyEvidence {
                        code: *code,
                        requested_instruction: instruction,
                        requested_cycle: self.runtime.cpu().cycles(),
                        keyrupt: None,
                        charin: None,
                    });
                }
                self.runtime
                    .schedule(self.runtime.cpu().cycles(), input.event.clone());
                input_index += 1;
            }
            if let Some(sequence) = &scenario.dsky_sequence
                && let Some(&code) = sequence.keys.get(dsky_index)
                && self.runtime.cpu().cycles() >= next_dsky_cycle
                && evidence
                    .keys
                    .last()
                    .is_none_or(|key| key.keyrupt.is_some() && key.charin.is_some())
            {
                let requested_cycle = self.runtime.cpu().cycles();
                evidence.keys.push(KeyEvidence {
                    code,
                    requested_instruction: instruction,
                    requested_cycle,
                    keyrupt: None,
                    charin: None,
                });
                self.runtime
                    .schedule(requested_cycle, RuntimeEvent::DskyKey { code });
                dsky_index += 1;
                next_dsky_cycle = requested_cycle + sequence.minimum_gap_cycles;
            }
            let outcome = self.faults.step(&mut self.runtime)?;
            self.dsky.consume_trace(&outcome.trace);
            self.observe_evidence(
                scenario,
                &outcome.trace,
                &mut evidence,
                &mut p63_entry_values,
                pinball_reconstruction.as_mut(),
            )?;
            if self.runtime.cpu().instructions() % scenario.frame_interval == 0
                && frames.last().is_none_or(|frame: &MissionFrame| {
                    frame.instruction != self.runtime.cpu().instructions()
                })
            {
                frames.push(self.frame(scenario)?);
            }
        }
        if frames
            .last()
            .is_none_or(|frame| frame.instruction != self.runtime.cpu().instructions())
        {
            frames.push(self.frame(scenario)?);
        }
        if let Some(at_entry) = p63_entry_values {
            let final_values = self.guidance_values(scenario)?;
            evidence.guidance_changes = scenario
                .guidance_variables
                .iter()
                .filter_map(|variable| {
                    let at_p63_entry = at_entry.get(&variable.name).copied()?;
                    let final_value = final_values.get(&variable.name).copied()?;
                    (at_p63_entry != final_value).then(|| GuidanceChange {
                        name: variable.name.clone(),
                        bank: variable.bank,
                        offset: variable.offset,
                        at_p63_entry,
                        final_value,
                    })
                })
                .collect();
        }
        evidence.keyboard_sequence_verified = !evidence.keys.is_empty()
            && evidence
                .keys
                .iter()
                .all(|key| key.keyrupt.is_some() && key.charin.is_some());
        evidence.pinball_reconstruction = pinball_reconstruction;
        evidence.pinball_reconstruction_matches_rope = evidence
            .pinball_reconstruction
            .as_ref()
            .and_then(|model| model.program_register)
            .is_some_and(|program| {
                self.runtime.cpu().memory().read_erasable_physical(2, 0o011)
                    == AgcWord::try_from_raw(u16::from(program)).ok()
            });
        evidence.verified_p63_request = evidence.keyboard_sequence_verified
            && evidence.pinball_reconstruction_matches_rope
            && evidence.program_63_selected.is_some()
            && evidence.p63lm_entry.is_some();
        let p63_initialization = P63Initialization::luminary099();
        evidence.p63_initialization_matches_rope =
            p63_initialization.matches_rope_writes(&evidence.guidance_writes);
        evidence.p63_initialization = Some(p63_initialization);
        evidence.landing_guidance_started = evidence.p63_initialization_matches_rope
            && P63_INITIALIZATION_WORDS.iter().all(|name| {
                evidence
                    .guidance_writes
                    .iter()
                    .any(|write| write.name == *name)
            });
        Ok(MissionRun {
            scenario: scenario.name.clone(),
            frames,
            final_dsky: self.dsky.clone(),
            instructions: self.runtime.cpu().instructions(),
            cycles: self.runtime.cpu().cycles(),
            faults_applied: self.faults.applied.len(),
            evidence,
        })
    }

    fn observe_evidence(
        &self,
        scenario: &MissionScenario,
        trace: &agc_trace::TraceEvent,
        evidence: &mut MissionEvidence,
        p63_entry_values: &mut Option<BTreeMap<String, AgcWord>>,
        pinball_reconstruction: Option<&mut V37ProgramChange>,
    ) -> Result<(), MissionError> {
        if let Some(vector) = trace
            .interrupts
            .iter()
            .find_map(|interrupt| match interrupt {
                InterruptEvent::Entered { number: 5, vector } => Some(*vector),
                _ => None,
            })
            && let Some(key) = evidence.keys.iter_mut().find(|key| key.keyrupt.is_none())
        {
            key.keyrupt = Some(self.milestone_at("KEYRUPT1", trace, format!("RUPT:{vector:04o}")));
        }

        let physical = trace
            .memory
            .first()
            .map_or("unknown", |access| access.physical.as_str());
        if physical == "F40:0077"
            && let Some(key_index) = evidence
                .keys
                .iter()
                .position(|key| key.keyrupt.is_some() && key.charin.is_none())
        {
            let code = evidence.keys[key_index].code;
            evidence.keys[key_index].charin = Some(self.milestone("CHARIN", trace));
            if let Some(model) = pinball_reconstruction {
                model.accept_code(code).map_err(|error| {
                    MissionError::Scenario(format!(
                        "typed Pinball reconstruction rejected rope-accepted key: {error}"
                    ))
                })?;
            }
        }

        if evidence.program_63_selected.is_none()
            && self.runtime.cpu().memory().read_erasable_physical(2, 0o011)
                == Some(AgcWord::try_from_raw(63).expect("63 is an AGC word"))
        {
            evidence.program_63_selected = Some(self.milestone("MODREG=63", trace));
        }
        match physical {
            "F32:0776" if evidence.p63lm_entry.is_none() => {
                evidence.p63lm_entry = Some(self.milestone("P63LM", trace));
                *p63_entry_values = Some(self.guidance_values(scenario)?);
            }
            "F36:0151" if evidence.p63spot_entry.is_none() => {
                evidence.p63spot_entry = Some(self.milestone("P63SPOT", trace));
            }
            "F32:1215" if evidence.p63spot2_entry.is_none() => {
                evidence.p63spot2_entry = Some(self.milestone("P63SPOT2", trace));
            }
            _ => {}
        }
        if evidence.p63lm_entry.is_some() {
            for access in trace.memory.iter().filter(|access| access.kind == "write") {
                if let Some(variable) = scenario.guidance_variables.iter().find(|variable| {
                    access.physical == format!("E{:o}:{:04o}", variable.bank, variable.offset)
                }) && !evidence
                    .guidance_writes
                    .iter()
                    .any(|write| write.name == variable.name)
                {
                    evidence.guidance_writes.push(GuidanceWrite {
                        name: variable.name.clone(),
                        bank: variable.bank,
                        offset: variable.offset,
                        value: access.value,
                        milestone: self.milestone_at(
                            &format!("write {}", variable.name),
                            trace,
                            access.physical.clone(),
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn milestone(&self, name: &str, trace: &agc_trace::TraceEvent) -> MissionMilestone {
        self.milestone_at(
            name,
            trace,
            trace
                .memory
                .first()
                .map_or_else(|| "unknown".to_owned(), |access| access.physical.clone()),
        )
    }

    fn milestone_at(
        &self,
        name: &str,
        trace: &agc_trace::TraceEvent,
        physical: String,
    ) -> MissionMilestone {
        MissionMilestone {
            name: name.to_owned(),
            trace_sequence: trace.sequence,
            instruction: self.runtime.cpu().instructions(),
            cycle: trace.cycle_end,
            pc: trace.pc,
            physical,
        }
    }

    fn guidance_values(
        &self,
        scenario: &MissionScenario,
    ) -> Result<BTreeMap<String, AgcWord>, MissionError> {
        scenario
            .guidance_variables
            .iter()
            .map(|variable| {
                let value = self
                    .runtime
                    .cpu()
                    .memory()
                    .read_erasable_physical(variable.bank, variable.offset)
                    .ok_or_else(|| {
                        MissionError::Scenario(format!(
                            "guidance variable {} has invalid E{}:{:04o} address",
                            variable.name, variable.bank, variable.offset
                        ))
                    })?;
                Ok((variable.name.clone(), value))
            })
            .collect()
    }

    fn frame(&self, scenario: &MissionScenario) -> Result<MissionFrame, MissionError> {
        let register = |index| {
            self.runtime
                .cpu()
                .central_register(index)
                .map(|value| value.raw())
        };
        let guidance = self.guidance_values(scenario)?;
        Ok(MissionFrame {
            instruction: self.runtime.cpu().instructions(),
            cycle: self.runtime.cpu().cycles(),
            pc: self.runtime.cpu().program_counter(),
            a: register(0)?,
            l: register(1)?,
            q: register(2)?,
            eb: register(3)?,
            fb: register(4)?,
            dsky: self.dsky.clone(),
            guidance,
            executive: self.runtime.observer().executive_jobs(self.runtime.cpu())?,
            waitlist: self.runtime.observer().waitlist_tasks(self.runtime.cpu())?,
            hardware: self.runtime.hardware().clone(),
        })
    }
}

/// Compares two completed mission controllers at trace level.
pub fn compare_missions(left: &MissionController, right: &MissionController) -> ValidationReport {
    compare_traces(left.runtime.trace(), right.runtime.trace())
}

/// Synchronized high-level interpreter state for interface consumers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InterpreterFrame {
    /// CPU instruction count when sampled.
    pub instruction: u64,
    /// Typed interpreter state.
    pub state: InterpreterState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_isa::{Mnemonic, encode};
    use agc_loader::{RopeFormat, decode_bytes};
    use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory};

    #[test]
    fn mission_frames_are_sampled_from_executed_cpu_state() {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        rope[2 * 1024] = encode(Mnemonic::Tcf, 0o4000).unwrap();
        let mut cpu = Cpu::new(Memory::with_rope(rope).unwrap());
        cpu.cancel_interrupt(agc_cpu::Interrupt::Downrupt);
        let runtime = Runtime::new(cpu);
        let mut controller = MissionController::from_runtime(runtime);
        let scenario = MissionScenario {
            name: "loop".to_owned(),
            instruction_limit: 5,
            frame_interval: 2,
            inputs: Vec::new(),
            initialization: Vec::new(),
            dsky_sequence: None,
            guidance_variables: Vec::new(),
            observer: FlightSoftwareObserver::default(),
        };
        let run = controller.run(&scenario).unwrap();
        assert_eq!(run.instructions, 5);
        assert_eq!(run.frames.last().unwrap().pc, 0o4000);
        assert_eq!(controller.runtime().trace().events.len(), 5);
    }

    #[test]
    fn p63_profile_exposes_its_non_blank_precondition_and_software_pacing() {
        let scenario = MissionScenario::luminary_p63_landing();
        assert!(scenario.initialization.len() > 50);
        let flagword = scenario
            .initialization
            .iter()
            .find(|word| word.symbol.starts_with("FLAGWRD3"))
            .unwrap();
        assert_eq!(flagword.value.raw(), 0o12000);
        assert!(
            scenario
                .initialization
                .iter()
                .any(|word| word.symbol == "TLAND.HI" && word.value.raw() == 0o04247)
        );
        assert!(
            scenario
                .initialization
                .iter()
                .any(|word| word.symbol == "RLS.X.HI" && word.value.raw() == 0o00301)
        );
        let sequence = scenario.dsky_sequence.as_ref().unwrap();
        assert_eq!(sequence.keys, [0o21, 3, 7, 0o34, 6, 3, 0o34]);
        assert!(scenario.inputs.is_empty());
    }

    #[test]
    fn typed_pinball_reconstruction_matches_real_luminary_rope() {
        let rope = decode_bytes(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../artifacts/generated/luminary099-reference.bin"
            )),
            RopeFormat::Yayul,
        )
        .unwrap();
        let mut controller = MissionController::from_rope(rope).unwrap();
        let mut scenario = MissionScenario::luminary_p63_landing();
        scenario.instruction_limit = 160_000;
        scenario.frame_interval = 40_000;
        let run = controller.run(&scenario).unwrap();
        assert!(run.evidence.keyboard_sequence_verified);
        assert!(run.evidence.pinball_reconstruction_matches_rope);
        assert_eq!(
            run.evidence
                .pinball_reconstruction
                .as_ref()
                .and_then(|model| model.program_register),
            Some(63)
        );
        assert!(run.evidence.verified_p63_request);
        assert!(run.evidence.p63_initialization_matches_rope);
        assert_eq!(
            run.evidence.p63_initialization,
            Some(P63Initialization::luminary099())
        );
        assert!(run.evidence.landing_guidance_started);
    }
}
