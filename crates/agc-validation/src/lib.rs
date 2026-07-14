#![forbid(unsafe_code)]
//! Trace differential analysis, divergence classification, and determinism checks.

use agc_runtime::{Runtime, RuntimeError, RuntimeEvent};
use agc_trace::{TraceEvent, TraceLog};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Required divergence taxonomy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DivergenceClass {
    /// Source parser or provenance mismatch.
    Parsing,
    /// Program-image mismatch.
    Assembly,
    /// Opcode, operand, or instruction-sequence mismatch.
    Decode,
    /// Register result consistent with arithmetic disagreement.
    Arithmetic,
    /// Logical or physical memory access mismatch.
    Addressing,
    /// EBANK, FBANK, BB, or physical-bank mismatch.
    Banking,
    /// Cycle count or external-event timing mismatch.
    Timing,
    /// Interrupt request/entry/order mismatch.
    Interrupt,
    /// Channel operation mismatch.
    Io,
    /// Interpretive state mismatch.
    Interpreter,
    /// Executive or Waitlist state mismatch.
    Scheduler,
    /// Generated-code behavior mismatch.
    Transpilation,
    /// Evidence is insufficient for a narrower class.
    Unknown,
}

/// One precisely located semantic divergence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// Event index.
    pub event: usize,
    /// Classification.
    pub class: DivergenceClass,
    /// Compared field.
    pub field: String,
    /// Left value.
    pub left: String,
    /// Right value.
    pub right: String,
    /// Human-readable explanation.
    pub explanation: String,
}

/// Full differential result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Events in the left trace.
    pub left_events: usize,
    /// Events in the right trace.
    pub right_events: usize,
    /// First divergence, if any.
    pub first: Option<Divergence>,
    /// Whether traces are semantically identical under this schema.
    pub equivalent: bool,
}

/// Differential runner failure.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// Runtime execution failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Event count differs before traces can be compared to completion.
    #[error("scenario has unequal event streams")]
    Scenario,
}

/// Compares all required architectural fields in commit order.
pub fn compare_traces(left: &TraceLog, right: &TraceLog) -> ValidationReport {
    let shared = left.events.len().min(right.events.len());
    let first = (0..shared)
        .find_map(|index| compare_event(index, &left.events[index], &right.events[index]))
        .or_else(|| {
            (left.events.len() != right.events.len()).then(|| Divergence {
                event: shared,
                class: DivergenceClass::Timing,
                field: "trace-length".to_owned(),
                left: left.events.len().to_string(),
                right: right.events.len().to_string(),
                explanation: "one implementation committed additional instructions".to_owned(),
            })
        });
    ValidationReport {
        left_events: left.events.len(),
        right_events: right.events.len(),
        equivalent: first.is_none(),
        first,
    }
}

/// Runs two implementations under matching scheduled events and instruction budget.
pub fn differential_run(
    left: &mut Runtime,
    right: &mut Runtime,
    events: &[(u64, RuntimeEvent)],
    instructions: u64,
) -> Result<ValidationReport, ValidationError> {
    for (cycle, event) in events {
        left.schedule(*cycle, event.clone());
        right.schedule(*cycle, event.clone());
    }
    left.run(instructions)?;
    right.run(instructions)?;
    Ok(compare_traces(left.trace(), right.trace()))
}

/// Runs two clones of one initialized runtime to prove host-level determinism.
pub fn determinism_check(
    initialized: &Runtime,
    events: &[(u64, RuntimeEvent)],
    instructions: u64,
) -> Result<ValidationReport, ValidationError> {
    let mut left = initialized.clone();
    let mut right = initialized.clone();
    differential_run(&mut left, &mut right, events, instructions)
}

fn compare_event(index: usize, left: &TraceEvent, right: &TraceEvent) -> Option<Divergence> {
    macro_rules! difference {
        ($field:literal, $class:expr, $left:expr, $right:expr, $explanation:literal) => {
            if $left != $right {
                return Some(Divergence {
                    event: index,
                    class: $class,
                    field: $field.to_owned(),
                    left: format!("{:?}", $left),
                    right: format!("{:?}", $right),
                    explanation: $explanation.to_owned(),
                });
            }
        };
    }
    difference!(
        "sequence",
        DivergenceClass::Timing,
        left.sequence,
        right.sequence,
        "commit sequence differs"
    );
    difference!(
        "pc",
        DivergenceClass::Decode,
        left.pc,
        right.pc,
        "instruction address differs"
    );
    difference!(
        "instruction",
        DivergenceClass::Decode,
        left.instruction,
        right.instruction,
        "raw instruction differs"
    );
    difference!(
        "mnemonic",
        DivergenceClass::Decode,
        left.mnemonic,
        right.mnemonic,
        "decoded operation differs"
    );
    difference!(
        "operand",
        DivergenceClass::Decode,
        left.operand,
        right.operand,
        "decoded operand differs"
    );
    difference!(
        "extended",
        DivergenceClass::Decode,
        left.extended,
        right.extended,
        "extracode context differs"
    );
    difference!(
        "cycle-start",
        DivergenceClass::Timing,
        left.cycle_start,
        right.cycle_start,
        "instruction begins at a different machine cycle"
    );
    difference!(
        "cycle-end",
        DivergenceClass::Timing,
        left.cycle_end,
        right.cycle_end,
        "instruction consumes a different cycle count"
    );
    difference!(
        "before.eb",
        DivergenceClass::Banking,
        left.before.eb,
        right.before.eb,
        "erasable-bank state differs before execution"
    );
    difference!(
        "before.fb",
        DivergenceClass::Banking,
        left.before.fb,
        right.before.fb,
        "fixed-bank state differs before execution"
    );
    difference!(
        "before.bb",
        DivergenceClass::Banking,
        left.before.bb,
        right.before.bb,
        "combined-bank state differs before execution"
    );
    difference!(
        "before",
        DivergenceClass::Arithmetic,
        left.before,
        right.before,
        "architectural register state differs before execution"
    );
    difference!(
        "memory",
        DivergenceClass::Addressing,
        left.memory,
        right.memory,
        "ordered logical/physical memory accesses differ"
    );
    difference!(
        "io",
        DivergenceClass::Io,
        left.io,
        right.io,
        "ordered channel operations differ"
    );
    difference!(
        "interrupts",
        DivergenceClass::Interrupt,
        left.interrupts,
        right.interrupts,
        "interrupt activity differs"
    );
    difference!(
        "after.eb",
        DivergenceClass::Banking,
        left.after.eb,
        right.after.eb,
        "erasable-bank state differs after execution"
    );
    difference!(
        "after.fb",
        DivergenceClass::Banking,
        left.after.fb,
        right.after.fb,
        "fixed-bank state differs after execution"
    );
    difference!(
        "after.bb",
        DivergenceClass::Banking,
        left.after.bb,
        right.after.bb,
        "combined-bank state differs after execution"
    );
    difference!(
        "after",
        DivergenceClass::Arithmetic,
        left.after,
        right.after,
        "architectural result registers differ"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_trace::TraceEvent;
    use agc_word::AgcWord;

    #[test]
    fn cycle_disagreement_is_classified_as_timing() {
        let mut left = TraceEvent::new(0, 0, 0o4000, AgcWord::POSITIVE_ZERO);
        let mut right = left.clone();
        left.cycle_end = 1;
        right.cycle_end = 2;
        let report = compare_traces(
            &TraceLog { events: vec![left] },
            &TraceLog {
                events: vec![right],
            },
        );
        assert_eq!(report.first.unwrap().class, DivergenceClass::Timing);
    }
}
