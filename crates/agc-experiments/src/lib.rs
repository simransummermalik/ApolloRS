#![forbid(unsafe_code)]
//! Reproducible paired mission experiments and fault-outcome classification.

use agc_faults::{AppliedFault, Fault, RecoveryComparison, compare_recovery};
use agc_loader::RopeImage;
use agc_mission::{MissionController, MissionError, MissionRun, MissionScenario, compare_missions};
use agc_trace::{MachineEventKind, TraceLog};
use agc_validation::ValidationReport;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Current fault-matrix specification and report schema.
pub const FAULT_MATRIX_SCHEMA_VERSION: u32 = 1;

/// Baseline milestone usable as a fault-injection anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissionAnchor {
    /// First operator key delivery.
    FirstKeyRequest,
    /// Final operator key delivery.
    FinalKeyRequest,
    /// First trace-backed selection of major mode 63.
    Program63Selected,
    /// First fetch of `P63LM`.
    P63Entry,
}

/// Reproducible injection timing resolved against the paired baseline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum InjectionPoint {
    /// Exact committed-instruction boundary.
    Instruction {
        /// Boundary before this instruction executes.
        instruction: u64,
    },
    /// Signed offset from a trace-backed mission milestone.
    Anchor {
        /// Baseline milestone.
        anchor: MissionAnchor,
        /// Signed instruction offset.
        offset: i64,
    },
    /// Signed offset from the first baseline write to a named guidance word.
    GuidanceWrite {
        /// Exact configured guidance-variable name.
        variable: String,
        /// Signed instruction offset.
        offset: i64,
    },
}

/// One predeclared paired fault experiment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FaultCaseSpec {
    /// Stable case identifier.
    pub id: String,
    /// Fault-injection boundary definition.
    pub injection: InjectionPoint,
    /// Deterministic fault payload.
    pub fault: Fault,
    /// Question declared before observing this campaign's result.
    pub research_question: String,
    /// Scope caveat specific to this case.
    pub limitation: String,
}

/// Complete declared fault matrix.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FaultMatrixSpec {
    /// Specification schema.
    pub schema_version: u32,
    /// Stable experiment name.
    pub name: String,
    /// Shared mission instruction horizon.
    pub instruction_limit: u64,
    /// Cases executed in declared order.
    pub cases: Vec<FaultCaseSpec>,
}

impl FaultMatrixSpec {
    /// Validates schema, identifiers, horizon, and fault ranges before execution.
    pub fn validate(&self) -> Result<(), ExperimentError> {
        if self.schema_version != FAULT_MATRIX_SCHEMA_VERSION {
            return Err(ExperimentError::Spec(format!(
                "fault matrix schema {} is unsupported; expected {}",
                self.schema_version, FAULT_MATRIX_SCHEMA_VERSION
            )));
        }
        if self.name.trim().is_empty() || self.instruction_limit == 0 || self.cases.is_empty() {
            return Err(ExperimentError::Spec(
                "name, nonzero instruction_limit, and at least one case are required".to_owned(),
            ));
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
                return Err(ExperimentError::Spec(format!(
                    "fault case identifier {:?} is empty or duplicated",
                    case.id
                )));
            }
            if case.research_question.trim().is_empty() || case.limitation.trim().is_empty() {
                return Err(ExperimentError::Spec(format!(
                    "fault case {} requires a research_question and limitation",
                    case.id
                )));
            }
            validate_fault(&case.fault).map_err(|reason| {
                ExperimentError::Spec(format!("fault case {} is invalid: {reason}", case.id))
            })?;
        }
        Ok(())
    }
}

/// Resolved baseline-relative injection timing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedInjection {
    /// Original declarative timing.
    pub declared: InjectionPoint,
    /// Exact committed-instruction boundary used by both audit and execution.
    pub instruction: u64,
    /// Baseline milestone used to resolve the boundary.
    pub basis: String,
}

/// Mission-level evidence distilled without copying frames or full traces.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionAcceptance {
    /// Instructions committed at the horizon.
    pub instructions: u64,
    /// Machine cycles at the horizon.
    pub cycles: u64,
    /// Final logical PC.
    pub final_pc: u16,
    /// All requested keys reached KEYRUPT and `CHARIN`.
    pub keyboard_sequence_verified: bool,
    /// Typed Pinball reconstruction agreed with rope `MODREG`.
    pub pinball_reconstruction_matches_rope: bool,
    /// Major mode 63 was selected and `P63LM` fetched.
    pub verified_p63_request: bool,
    /// P63 initialization writes matched the original rope.
    pub p63_initialization_matches_rope: bool,
    /// Landing-guidance writes continued beyond the initialization gate.
    pub landing_guidance_started: bool,
    /// Instruction at which major mode 63 was first observed.
    pub program_63_selected_instruction: Option<u64>,
    /// Instruction at which `P63LM` was first fetched.
    pub p63_entry_instruction: Option<u64>,
    /// Number of distinct configured guidance words first written after entry.
    pub guidance_writes: usize,
    /// Number of configured guidance words changed by the horizon.
    pub guidance_changes: usize,
}

impl MissionAcceptance {
    fn score(&self) -> usize {
        [
            self.keyboard_sequence_verified,
            self.pinball_reconstruction_matches_rope,
            self.verified_p63_request,
            self.p63_initialization_matches_rope,
            self.landing_guidance_started,
        ]
        .into_iter()
        .filter(|value| *value)
        .count()
    }
}

/// One changed mission-acceptance field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceDifference {
    /// Field name.
    pub field: String,
    /// Baseline value rendered without loss.
    pub baseline: String,
    /// Faulted value rendered without loss.
    pub faulted: String,
}

/// Outcome taxonomy for a completed paired campaign.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaultOutcomeClass {
    /// The scheduled operation did not alter its target (for example no pending interrupt).
    NotActivated,
    /// The target changed but no trace divergence was observed through the horizon.
    Masked,
    /// Traces diverged and final architectural registers reconverged with no evidence change.
    Recovered,
    /// Traces and final registers differ, but mission acceptance fields remain unchanged.
    PersistentStateDivergence,
    /// Mission evidence changed without reducing the five-field acceptance score.
    MissionEvidenceChanged,
    /// One or more mission acceptance obligations regressed.
    MissionDegraded,
}

/// Complete result for one paired campaign arm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FaultCaseResult {
    /// Stable case identifier.
    pub id: String,
    /// Resolved injection boundary.
    pub injection: ResolvedInjection,
    /// Scheduled deterministic fault.
    pub fault: Fault,
    /// Predeclared research question.
    pub research_question: String,
    /// Case-specific scope caveat.
    pub limitation: String,
    /// Exact applied-fault audit trail.
    pub applied_faults: Vec<AppliedFault>,
    /// Whether the target operation itself changed state.
    pub target_activated: bool,
    /// Faulted mission acceptance at the common horizon.
    pub faulted: MissionAcceptance,
    /// Exact full-schema trace comparison.
    pub trace_comparison: ValidationReport,
    /// Final-register and first-divergence summary.
    pub recovery: RecoveryComparison,
    /// Approximate committed-instruction boundary of first divergence.
    pub first_divergence_instruction: Option<u64>,
    /// Instructions from injection to first observed divergence.
    pub divergence_latency_instructions: Option<u64>,
    /// Changed acceptance fields.
    pub acceptance_differences: Vec<AcceptanceDifference>,
    /// Classified measured outcome.
    pub outcome: FaultOutcomeClass,
}

/// Aggregate matrix counts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FaultMatrixAggregate {
    /// Number of declared cases.
    pub cases: usize,
    /// Outcome histogram.
    pub outcomes: BTreeMap<FaultOutcomeClass, usize>,
    /// Cases with any trace divergence.
    pub diverged_cases: usize,
    /// Cases with regressed mission acceptance.
    pub degraded_cases: usize,
    /// Cases with exact final-register recovery after divergence.
    pub recovered_cases: usize,
}

/// Reproducible baseline and every paired fault arm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FaultMatrixReport {
    /// Report schema.
    pub schema_version: u32,
    /// Stable experiment name.
    pub name: String,
    /// Mission scenario name.
    pub scenario: String,
    /// Common instruction horizon.
    pub instruction_limit: u64,
    /// Baseline acceptance evidence.
    pub baseline: MissionAcceptance,
    /// Results in declaration order.
    pub cases: Vec<FaultCaseResult>,
    /// Aggregate outcome counts.
    pub aggregate: FaultMatrixAggregate,
}

/// Experiment declaration or execution failure.
#[derive(Debug, Error)]
pub enum ExperimentError {
    /// Invalid matrix declaration.
    #[error("invalid fault matrix: {0}")]
    Spec(String),
    /// Baseline milestone required by an injection point was absent.
    #[error("fault case {case} cannot resolve injection point: {reason}")]
    Injection {
        /// Fault case identifier.
        case: String,
        /// Missing or invalid baseline evidence.
        reason: String,
    },
    /// Mission setup or execution failed.
    #[error(transparent)]
    Mission(#[from] MissionError),
}

/// Runs one baseline and every declared fault arm from an identical initial controller.
pub fn run_luminary_p63_fault_matrix(
    rope: RopeImage,
    spec: &FaultMatrixSpec,
) -> Result<FaultMatrixReport, ExperimentError> {
    spec.validate()?;
    let mut scenario = MissionScenario::luminary_p63_landing();
    scenario.instruction_limit = spec.instruction_limit;
    let template = MissionController::from_rope(rope)?;
    let mut baseline_controller = template.clone();
    let baseline_run = baseline_controller.run(&scenario)?;
    let baseline = acceptance(&baseline_run, &baseline_controller);

    let resolved = spec
        .cases
        .iter()
        .map(|case| resolve_injection(case, &baseline_run, spec.instruction_limit))
        .collect::<Result<Vec<_>, _>>()?;
    let mut cases = Vec::with_capacity(spec.cases.len());
    for (case, injection) in spec.cases.iter().zip(resolved) {
        let mut faulted_controller = template.clone();
        faulted_controller.schedule_fault(injection.instruction, case.fault.clone());
        let faulted_run = faulted_controller.run(&scenario)?;
        let faulted = acceptance(&faulted_run, &faulted_controller);
        let trace_comparison = compare_missions(&baseline_controller, &faulted_controller);
        let recovery =
            compare_recovery(baseline_controller.runtime(), faulted_controller.runtime());
        let target_activated = fault_target_activated(&faulted_run.applied_faults);
        let first_divergence_instruction = recovery.first_divergence.map(|event| {
            instruction_boundary_at_event(faulted_controller.runtime().trace(), event)
        });
        let divergence_latency_instructions = first_divergence_instruction
            .map(|instruction| instruction.saturating_sub(injection.instruction));
        let acceptance_differences = acceptance_differences(&baseline, &faulted);
        let outcome = classify(
            target_activated,
            &trace_comparison,
            &recovery,
            &baseline,
            &faulted,
            &acceptance_differences,
        );
        cases.push(FaultCaseResult {
            id: case.id.clone(),
            injection,
            fault: case.fault.clone(),
            research_question: case.research_question.clone(),
            limitation: case.limitation.clone(),
            applied_faults: faulted_run.applied_faults,
            target_activated,
            faulted,
            trace_comparison,
            recovery,
            first_divergence_instruction,
            divergence_latency_instructions,
            acceptance_differences,
            outcome,
        });
    }

    let mut outcomes = BTreeMap::new();
    for case in &cases {
        *outcomes.entry(case.outcome).or_default() += 1;
    }
    let aggregate = FaultMatrixAggregate {
        cases: cases.len(),
        outcomes,
        diverged_cases: cases
            .iter()
            .filter(|case| !case.trace_comparison.equivalent)
            .count(),
        degraded_cases: cases
            .iter()
            .filter(|case| case.outcome == FaultOutcomeClass::MissionDegraded)
            .count(),
        recovered_cases: cases
            .iter()
            .filter(|case| case.outcome == FaultOutcomeClass::Recovered)
            .count(),
    };
    Ok(FaultMatrixReport {
        schema_version: FAULT_MATRIX_SCHEMA_VERSION,
        name: spec.name.clone(),
        scenario: scenario.name,
        instruction_limit: spec.instruction_limit,
        baseline,
        cases,
        aggregate,
    })
}

fn validate_fault(fault: &Fault) -> Result<(), String> {
    match fault {
        Fault::ErasableBitFlip { address, mask } => {
            if *address > 0o1777 || *mask == 0 || *mask > 0o77777 {
                return Err("logical erasable address or 15-bit mask is out of range".to_owned());
            }
        }
        Fault::PhysicalErasableBitFlip { bank, offset, mask } => {
            if *bank >= 8 || *offset >= 0o400 || *mask == 0 || *mask > 0o77777 {
                return Err(
                    "physical erasable bank, offset, or 15-bit mask is out of range".to_owned(),
                );
            }
        }
        Fault::RopeBitFlip { bank, offset, mask } => {
            if *bank >= 0o44 || *offset >= 0o2000 || *mask == 0 || *mask > 0o77777 {
                return Err("rope bank, offset, or 15-bit mask is out of range".to_owned());
            }
        }
        Fault::Register { index, mask } => {
            if *index >= 0o20 || *mask == 0 {
                return Err("central register index or full-width mask is out of range".to_owned());
            }
        }
        Fault::StuckChannel {
            channel,
            instructions,
            ..
        } => {
            if *channel > 0o777 || *instructions == 0 {
                return Err("channel is out of range or duration is zero".to_owned());
            }
        }
        Fault::TimerJump { address, delta } => {
            if !(0o24..=0o31).contains(address) || !(-16383..=16383).contains(delta) {
                return Err("timer address or one's-complement delta is out of range".to_owned());
            }
        }
        Fault::DropInterrupt { .. } | Fault::ImuBias { .. } | Fault::RadarSample { .. } => {}
    }
    Ok(())
}

fn resolve_injection(
    case: &FaultCaseSpec,
    baseline: &MissionRun,
    limit: u64,
) -> Result<ResolvedInjection, ExperimentError> {
    let (base, basis, offset) = match &case.injection {
        InjectionPoint::Instruction { instruction } => (
            *instruction,
            format!("explicit instruction {instruction}"),
            0,
        ),
        InjectionPoint::Anchor { anchor, offset } => {
            let instruction = match anchor {
                MissionAnchor::FirstKeyRequest => baseline
                    .evidence
                    .keys
                    .first()
                    .map(|key| key.requested_instruction),
                MissionAnchor::FinalKeyRequest => baseline
                    .evidence
                    .keys
                    .last()
                    .map(|key| key.requested_instruction),
                MissionAnchor::Program63Selected => baseline
                    .evidence
                    .program_63_selected
                    .as_ref()
                    .map(|milestone| milestone.instruction),
                MissionAnchor::P63Entry => baseline
                    .evidence
                    .p63lm_entry
                    .as_ref()
                    .map(|milestone| milestone.instruction),
            }
            .ok_or_else(|| ExperimentError::Injection {
                case: case.id.clone(),
                reason: format!("baseline did not produce {anchor:?}"),
            })?;
            (
                instruction,
                format!("baseline {anchor:?} at {instruction}"),
                *offset,
            )
        }
        InjectionPoint::GuidanceWrite { variable, offset } => {
            let instruction = baseline
                .evidence
                .guidance_writes
                .iter()
                .find(|write| write.name == *variable)
                .map(|write| write.milestone.instruction)
                .ok_or_else(|| ExperimentError::Injection {
                    case: case.id.clone(),
                    reason: format!("baseline did not write guidance variable {variable:?}"),
                })?;
            (
                instruction,
                format!("first baseline write to {variable} at {instruction}"),
                *offset,
            )
        }
    };
    let resolved = i128::from(base) + i128::from(offset);
    if resolved < 0 || resolved >= i128::from(limit) {
        return Err(ExperimentError::Injection {
            case: case.id.clone(),
            reason: format!("resolved boundary {resolved} is outside 0..{limit}"),
        });
    }
    Ok(ResolvedInjection {
        declared: case.injection.clone(),
        instruction: resolved as u64,
        basis,
    })
}

fn acceptance(run: &MissionRun, controller: &MissionController) -> MissionAcceptance {
    MissionAcceptance {
        instructions: run.instructions,
        cycles: run.cycles,
        final_pc: controller.runtime().cpu().program_counter(),
        keyboard_sequence_verified: run.evidence.keyboard_sequence_verified,
        pinball_reconstruction_matches_rope: run.evidence.pinball_reconstruction_matches_rope,
        verified_p63_request: run.evidence.verified_p63_request,
        p63_initialization_matches_rope: run.evidence.p63_initialization_matches_rope,
        landing_guidance_started: run.evidence.landing_guidance_started,
        program_63_selected_instruction: run
            .evidence
            .program_63_selected
            .as_ref()
            .map(|milestone| milestone.instruction),
        p63_entry_instruction: run
            .evidence
            .p63lm_entry
            .as_ref()
            .map(|milestone| milestone.instruction),
        guidance_writes: run.evidence.guidance_writes.len(),
        guidance_changes: run.evidence.guidance_changes.len(),
    }
}

fn fault_target_activated(applied: &[AppliedFault]) -> bool {
    !applied.is_empty()
        && applied.iter().any(|record| {
            !matches!(
                record.fault,
                Fault::DropInterrupt { .. } if record.resulting_word == Some(0)
            )
        })
}

fn classify(
    activated: bool,
    trace: &ValidationReport,
    recovery: &RecoveryComparison,
    baseline: &MissionAcceptance,
    faulted: &MissionAcceptance,
    differences: &[AcceptanceDifference],
) -> FaultOutcomeClass {
    if !activated {
        FaultOutcomeClass::NotActivated
    } else if trace.equivalent {
        FaultOutcomeClass::Masked
    } else if faulted.score() < baseline.score() {
        FaultOutcomeClass::MissionDegraded
    } else if !differences.is_empty() {
        FaultOutcomeClass::MissionEvidenceChanged
    } else if recovery.registers_recovered {
        FaultOutcomeClass::Recovered
    } else {
        FaultOutcomeClass::PersistentStateDivergence
    }
}

fn instruction_boundary_at_event(trace: &TraceLog, event: usize) -> u64 {
    trace.events[..event.min(trace.events.len())]
        .iter()
        .filter(|event| event.kind == MachineEventKind::Instruction)
        .count() as u64
}

fn acceptance_differences(
    baseline: &MissionAcceptance,
    faulted: &MissionAcceptance,
) -> Vec<AcceptanceDifference> {
    let mut differences = Vec::new();
    macro_rules! compare {
        ($field:ident) => {
            if baseline.$field != faulted.$field {
                differences.push(AcceptanceDifference {
                    field: stringify!($field).to_owned(),
                    baseline: format!("{:?}", baseline.$field),
                    faulted: format!("{:?}", faulted.$field),
                });
            }
        };
    }
    compare!(keyboard_sequence_verified);
    compare!(pinball_reconstruction_matches_rope);
    compare!(verified_p63_request);
    compare!(p63_initialization_matches_rope);
    compare!(landing_guidance_started);
    compare!(program_63_selected_instruction);
    compare!(p63_entry_instruction);
    compare!(guidance_writes);
    compare!(guidance_changes);
    differences
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_faults::Fault;

    #[test]
    fn matrix_validation_rejects_duplicate_cases_and_zero_masks() {
        let case = FaultCaseSpec {
            id: "duplicate".to_owned(),
            injection: InjectionPoint::Instruction { instruction: 0 },
            fault: Fault::ErasableBitFlip {
                address: 0o100,
                mask: 0,
            },
            research_question: String::new(),
            limitation: String::new(),
        };
        let spec = FaultMatrixSpec {
            schema_version: 1,
            name: "test".to_owned(),
            instruction_limit: 10,
            cases: vec![case.clone(), case],
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn tracked_p63_matrix_parses_and_validates() {
        let spec: FaultMatrixSpec =
            serde_json::from_str(include_str!("../../../experiments/p63-fault-matrix.json"))
                .unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.cases.len(), 11);
    }

    #[test]
    fn outcome_taxonomy_distinguishes_masking_and_degradation() {
        let acceptance = MissionAcceptance {
            instructions: 1,
            cycles: 1,
            final_pc: 0o4000,
            keyboard_sequence_verified: true,
            pinball_reconstruction_matches_rope: true,
            verified_p63_request: true,
            p63_initialization_matches_rope: true,
            landing_guidance_started: true,
            program_63_selected_instruction: Some(1),
            p63_entry_instruction: Some(1),
            guidance_writes: 1,
            guidance_changes: 1,
        };
        let trace_equal = ValidationReport {
            left_events: 1,
            right_events: 1,
            first: None,
            equivalent: true,
        };
        let recovery = RecoveryComparison {
            first_divergence: None,
            baseline_instructions: 1,
            faulted_instructions: 1,
            baseline_pc: 0o4000,
            faulted_pc: 0o4000,
            registers_recovered: true,
        };
        assert_eq!(
            classify(true, &trace_equal, &recovery, &acceptance, &acceptance, &[]),
            FaultOutcomeClass::Masked
        );
        let mut degraded = acceptance.clone();
        degraded.landing_guidance_started = false;
        let trace_different = ValidationReport {
            equivalent: false,
            first: None,
            ..trace_equal
        };
        assert_eq!(
            classify(
                true,
                &trace_different,
                &recovery,
                &acceptance,
                &degraded,
                &[AcceptanceDifference {
                    field: "landing_guidance_started".to_owned(),
                    baseline: "true".to_owned(),
                    faulted: "false".to_owned(),
                }],
            ),
            FaultOutcomeClass::MissionDegraded
        );
    }
}
