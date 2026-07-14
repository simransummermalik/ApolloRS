#![forbid(unsafe_code)]
//! Trace differential analysis, divergence classification, and determinism checks.

use agc_runtime::{Runtime, RuntimeError, RuntimeEvent};
use agc_trace::{InterruptEvent, MachineEventKind, TraceEvent, TraceLog};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead};
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

/// Event kind emitted by ApolloRS's pinned yaAGC instrumentation patch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum YaAgcEventKind {
    /// State immediately before an ordinary instruction.
    Instruction,
    /// State after interrupt acceptance; yaAGC logs on the first of two MCTs.
    InterruptEntry,
}

/// Architectural subset exported by the pinned yaAGC differential oracle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct YaAgcReferenceEvent {
    /// yaAGC machine-cycle counter.
    pub cycle: u64,
    /// Transition kind.
    pub kind: YaAgcEventKind,
    /// Logical instruction address.
    pub pc: u16,
    /// Raw instruction or interrupted instruction.
    pub instruction: u16,
    /// Full accumulator, modulo the host C representation width.
    pub a: u16,
    /// Full L register.
    pub l: u16,
    /// Full Q register.
    pub q: u16,
    /// EBANK central register.
    pub eb: u16,
    /// FBANK central register.
    pub fb: u16,
    /// BBANK central register.
    pub bb: u16,
    /// Interrupt vector for an interrupt-entry row.
    pub interrupt_vector: Option<u16>,
    /// Interrupt priority number for an interrupt-entry row.
    pub interrupt_number: Option<u8>,
}

/// Parsed exact architectural trace from the pinned yaAGC oracle.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct YaAgcReferenceTrace {
    /// Events in oracle commit order.
    pub events: Vec<YaAgcReferenceEvent>,
}

impl YaAgcReferenceTrace {
    /// Parses the twelve-column TSV emitted by the documented instrumentation
    /// patch. Cycles are decimal; all architectural values are octal.
    pub fn read_tsv(reader: impl BufRead) -> Result<Self, ReferenceTraceError> {
        let mut events = Vec::new();
        for (line_index, line) in reader.lines().enumerate() {
            let line_number = line_index + 1;
            let line = line.map_err(ReferenceTraceError::Io)?;
            if line.trim().is_empty() {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 12 {
                return Err(ReferenceTraceError::Columns {
                    line: line_number,
                    actual: fields.len(),
                });
            }
            let cycle = fields[0]
                .parse::<u64>()
                .map_err(|_| ReferenceTraceError::Number {
                    line: line_number,
                    field: "cycle",
                    value: fields[0].to_owned(),
                })?;
            let kind = match fields[1] {
                "I" => YaAgcEventKind::Instruction,
                "R" => YaAgcEventKind::InterruptEntry,
                value => {
                    return Err(ReferenceTraceError::Kind {
                        line: line_number,
                        value: value.to_owned(),
                    });
                }
            };
            let octal = |index: usize, field: &'static str| {
                u64::from_str_radix(fields[index], 8)
                    .map(|value| value as u16)
                    .map_err(|_| ReferenceTraceError::Number {
                        line: line_number,
                        field,
                        value: fields[index].to_owned(),
                    })
            };
            let interrupt_vector = (kind == YaAgcEventKind::InterruptEntry)
                .then(|| octal(10, "interrupt-vector"))
                .transpose()?;
            let interrupt_number = (kind == YaAgcEventKind::InterruptEntry)
                .then(|| octal(11, "interrupt-number"))
                .transpose()?
                .map(|number| number as u8);
            events.push(YaAgcReferenceEvent {
                cycle,
                kind,
                pc: octal(2, "pc")?,
                instruction: octal(3, "instruction")?,
                a: octal(4, "a")?,
                l: octal(5, "l")?,
                q: octal(6, "q")?,
                eb: octal(7, "eb")?,
                fb: octal(8, "fb")?,
                bb: octal(9, "bb")?,
                interrupt_vector,
                interrupt_number,
            });
        }
        Ok(Self { events })
    }
}

/// Malformed or unreadable yaAGC oracle trace.
#[derive(Debug, Error)]
pub enum ReferenceTraceError {
    /// Input I/O failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A TSV row has the wrong field count.
    #[error("yaAGC trace line {line} has {actual} columns; expected 12")]
    Columns {
        /// One-based line number.
        line: usize,
        /// Observed field count.
        actual: usize,
    },
    /// Event-kind marker is unknown.
    #[error("yaAGC trace line {line} has unknown event kind {value:?}")]
    Kind {
        /// One-based line number.
        line: usize,
        /// Invalid marker.
        value: String,
    },
    /// Decimal or octal field is malformed.
    #[error("yaAGC trace line {line} has invalid {field} value {value:?}")]
    Number {
        /// One-based line number.
        line: usize,
        /// Field name.
        field: &'static str,
        /// Invalid text.
        value: String,
    },
}

/// Auditable result of comparing ApolloRS against the yaAGC oracle subset.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceValidationReport {
    /// Report schema.
    pub schema_version: u32,
    /// Qualified external implementation.
    pub oracle: String,
    /// ApolloRS events available.
    pub apollors_events: usize,
    /// yaAGC events available.
    pub reference_events: usize,
    /// Equal leading events before a divergence or stream end.
    pub matched_events: usize,
    /// Whether every event in both streams was compared.
    pub complete: bool,
    /// Whether all compared events were equal.
    pub equivalent: bool,
    /// First architectural divergence, if present.
    pub first: Option<Divergence>,
    /// Fields covered by this oracle export.
    pub compared_fields: Vec<String>,
    /// Documented interrupt-cycle normalization.
    pub timing_normalization: String,
}

/// Compares ApolloRS events with the exact yaAGC architectural TSV.
///
/// If `allow_stream_prefix` is true, either shorter stream is accepted when
/// every available event matches. The report remains explicitly incomplete.
pub fn compare_yaagc_reference(
    apollors: &TraceLog,
    reference: &YaAgcReferenceTrace,
    allow_stream_prefix: bool,
) -> ReferenceValidationReport {
    let shared = apollors.events.len().min(reference.events.len());
    let mut matched_events = 0;
    let mut first = None;
    for index in 0..shared {
        if let Some(divergence) =
            compare_yaagc_event(index, &apollors.events[index], &reference.events[index])
        {
            first = Some(divergence);
            break;
        }
        matched_events += 1;
    }
    if first.is_none() && apollors.events.len() != reference.events.len() {
        let shorter_stream_is_accepted = allow_stream_prefix && matched_events == shared;
        if !shorter_stream_is_accepted {
            first = Some(Divergence {
                event: shared,
                class: DivergenceClass::Timing,
                field: "trace-length".to_owned(),
                left: apollors.events.len().to_string(),
                right: reference.events.len().to_string(),
                explanation: "ApolloRS and yaAGC event streams have different lengths".to_owned(),
            });
        }
    }
    ReferenceValidationReport {
        schema_version: 1,
        oracle: "VirtualAGC yaAGC pinned exact-trace instrumentation".to_owned(),
        apollors_events: apollors.events.len(),
        reference_events: reference.events.len(),
        matched_events,
        complete: apollors.events.len() == reference.events.len(),
        equivalent: first.is_none(),
        first,
        compared_fields: [
            "event-kind",
            "cycle",
            "pc",
            "instruction",
            "a",
            "l",
            "q",
            "eb",
            "fb",
            "bb",
            "interrupt-vector",
            "interrupt-number",
        ]
        .map(str::to_owned)
        .to_vec(),
        timing_normalization: concat!(
            "yaAGC logs interrupt acceptance on the first of its two MCTs; ",
            "ApolloRS interrupt-entry cycle_end is therefore reference cycle + 1"
        )
        .to_owned(),
    }
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

fn compare_yaagc_event(
    index: usize,
    apollors: &TraceEvent,
    reference: &YaAgcReferenceEvent,
) -> Option<Divergence> {
    let expected_kind = match reference.kind {
        YaAgcEventKind::Instruction => MachineEventKind::Instruction,
        YaAgcEventKind::InterruptEntry => MachineEventKind::InterruptEntry,
    };
    let divergence = |class, field: &str, left: String, right: String, explanation: &str| {
        Some(Divergence {
            event: index,
            class,
            field: field.to_owned(),
            left,
            right,
            explanation: explanation.to_owned(),
        })
    };
    if apollors.kind != expected_kind {
        return divergence(
            DivergenceClass::Interrupt,
            "event-kind",
            format!("{:?}", apollors.kind),
            format!("{:?}", reference.kind),
            "instruction and interrupt-entry ordering differs",
        );
    }
    let expected_cycle = match reference.kind {
        YaAgcEventKind::Instruction => reference.cycle,
        YaAgcEventKind::InterruptEntry => reference.cycle + 1,
    };
    if apollors.cycle_end != expected_cycle {
        return divergence(
            DivergenceClass::Timing,
            "cycle-end",
            apollors.cycle_end.to_string(),
            expected_cycle.to_string(),
            "machine-cycle accounting differs after documented interrupt normalization",
        );
    }
    macro_rules! octal_difference {
        ($field:literal, $class:expr, $left:expr, $right:expr, $explanation:literal) => {
            if $left != $right {
                return divergence(
                    $class,
                    $field,
                    format!("{:o}", $left),
                    format!("{:o}", $right),
                    $explanation,
                );
            }
        };
    }
    octal_difference!(
        "pc",
        DivergenceClass::Decode,
        apollors.pc,
        reference.pc,
        "logical instruction address differs"
    );
    octal_difference!(
        "instruction",
        DivergenceClass::Decode,
        apollors.instruction.raw(),
        reference.instruction & 0o77777,
        "raw instruction differs"
    );
    let registers = match reference.kind {
        YaAgcEventKind::Instruction => apollors.before,
        YaAgcEventKind::InterruptEntry => apollors.after,
    };
    octal_difference!(
        "a",
        DivergenceClass::Arithmetic,
        registers.a,
        reference.a,
        "accumulator differs"
    );
    octal_difference!(
        "l",
        DivergenceClass::Arithmetic,
        registers.l,
        reference.l,
        "L register differs"
    );
    octal_difference!(
        "q",
        DivergenceClass::Arithmetic,
        registers.q,
        reference.q,
        "Q register differs"
    );
    octal_difference!(
        "eb",
        DivergenceClass::Banking,
        registers.eb,
        reference.eb,
        "EBANK differs"
    );
    octal_difference!(
        "fb",
        DivergenceClass::Banking,
        registers.fb,
        reference.fb,
        "FBANK differs"
    );
    octal_difference!(
        "bb",
        DivergenceClass::Banking,
        registers.bb,
        reference.bb,
        "BBANK differs"
    );
    if reference.kind == YaAgcEventKind::InterruptEntry {
        let entered = apollors.interrupts.iter().find_map(|event| match event {
            InterruptEvent::Entered { number, vector } => Some((*number, *vector)),
            _ => None,
        });
        let expected = reference.interrupt_number.zip(reference.interrupt_vector);
        if entered != expected {
            return divergence(
                DivergenceClass::Interrupt,
                "interrupt",
                format!("{entered:?}"),
                format!("{expected:?}"),
                "accepted interrupt number or vector differs",
            );
        }
    }
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

    #[test]
    fn parses_and_matches_pinned_yaagc_tsv_shape() {
        let reference =
            YaAgcReferenceTrace::read_tsv(b"1\tI\t4000\t4\t0\t0\t0\t0\t0\t0\t0\t0\n".as_slice())
                .unwrap();
        let mut event = TraceEvent::new(0, 0, 0o4000, AgcWord::from_raw_truncate(4));
        event.cycle_end = 1;
        event.before.z = 0o4000;
        let report = compare_yaagc_reference(
            &TraceLog {
                events: vec![event],
            },
            &reference,
            false,
        );
        assert!(report.equivalent);
        assert!(report.complete);
        assert_eq!(report.matched_events, 1);
    }

    #[test]
    fn normalizes_yaagc_interrupt_logging_cycle() {
        let reference = YaAgcReferenceTrace::read_tsv(
            b"564\tR\t5626\t50000\t3055\t0\t3055\t1400\t12000\t12003\t4014\t3\n".as_slice(),
        )
        .unwrap();
        let mut event = TraceEvent::new(0, 563, 0o5626, AgcWord::from_raw_truncate(0o50000));
        event.kind = MachineEventKind::InterruptEntry;
        event.cycle_end = 565;
        event.after.a = 0o3055;
        event.after.q = 0o3055;
        event.after.eb = 0o1400;
        event.after.fb = 0o12000;
        event.after.bb = 0o12003;
        event.interrupts.push(InterruptEvent::Entered {
            number: 3,
            vector: 0o4014,
        });
        let report = compare_yaagc_reference(
            &TraceLog {
                events: vec![event],
            },
            &reference,
            false,
        );
        assert!(report.equivalent);
    }
}
