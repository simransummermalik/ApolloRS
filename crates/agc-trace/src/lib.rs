#![forbid(unsafe_code)]
//! Deterministic instruction, memory, I/O, interrupt, and scheduler traces.

use agc_word::AgcWord;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, BufRead, Write};
use thiserror::Error;

/// Trace schema version written into every event.
pub const TRACE_SCHEMA_VERSION: u32 = 1;

/// Architectural register snapshot around an instruction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegisterSnapshot {
    /// Full 16-bit A register.
    pub a: u16,
    /// Full sign-extended L central register.
    pub l: u16,
    /// Full 16-bit Q register.
    pub q: u16,
    /// Twelve-bit Z register.
    pub z: u16,
    /// Erasable bank register.
    pub eb: u16,
    /// Fixed bank register.
    pub fb: u16,
    /// Combined bank register.
    pub bb: u16,
}

/// Memory access carried by a trace without depending on the memory crate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryEvent {
    /// `fetch`, `read`, or `write`.
    pub kind: String,
    /// Logical address.
    pub logical: u16,
    /// Human-readable physical address such as `E5:0377`.
    pub physical: String,
    /// Raw word.
    pub value: AgcWord,
}

/// I/O channel transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoEvent {
    /// True for a write, false for a read.
    pub write: bool,
    /// Nine-bit channel number.
    pub channel: u16,
    /// Raw value.
    pub value: AgcWord,
}

/// Interrupt activity associated with a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum InterruptEvent {
    /// Interrupt became pending.
    Requested {
        /// Interrupt number.
        number: u8,
    },
    /// Interrupt was accepted and vectored.
    Entered {
        /// Interrupt number.
        number: u8,
        /// Vector address.
        vector: u16,
    },
    /// RESUME restored interrupted state.
    Resumed,
}

/// One complete committed machine step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Trace format version.
    pub schema_version: u32,
    /// Monotonic instruction sequence.
    pub sequence: u64,
    /// Machine cycle at instruction start.
    pub cycle_start: u64,
    /// Machine cycle after commit.
    pub cycle_end: u64,
    /// Instruction address.
    pub pc: u16,
    /// Raw instruction word.
    pub instruction: AgcWord,
    /// Canonical mnemonic.
    pub mnemonic: String,
    /// Decoded operand.
    pub operand: u16,
    /// Whether instruction was extended.
    pub extended: bool,
    /// Registers before execution.
    pub before: RegisterSnapshot,
    /// Registers after execution.
    pub after: RegisterSnapshot,
    /// Ordered memory accesses.
    pub memory: Vec<MemoryEvent>,
    /// Ordered channel accesses.
    pub io: Vec<IoEvent>,
    /// Interrupt activity.
    pub interrupts: Vec<InterruptEvent>,
}

impl TraceEvent {
    /// Creates an event with required invariant fields initialized.
    pub fn new(sequence: u64, cycle_start: u64, pc: u16, instruction: AgcWord) -> Self {
        Self {
            schema_version: TRACE_SCHEMA_VERSION,
            sequence,
            cycle_start,
            cycle_end: cycle_start,
            pc,
            instruction,
            mnemonic: String::new(),
            operand: 0,
            extended: false,
            before: RegisterSnapshot::default(),
            after: RegisterSnapshot::default(),
            memory: Vec::new(),
            io: Vec::new(),
            interrupts: Vec::new(),
        }
    }
}

/// Append-only in-memory trace.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceLog {
    /// Events in commit order.
    pub events: Vec<TraceEvent>,
}

impl TraceLog {
    /// Appends while enforcing sequence and cycle monotonicity.
    pub fn push(&mut self, event: TraceEvent) -> Result<(), TraceError> {
        if event.schema_version != TRACE_SCHEMA_VERSION {
            return Err(TraceError::Schema(event.schema_version));
        }
        if let Some(previous) = self.events.last() {
            if event.sequence != previous.sequence + 1 || event.cycle_start < previous.cycle_end {
                return Err(TraceError::Ordering {
                    previous_sequence: previous.sequence,
                    sequence: event.sequence,
                });
            }
        } else if event.sequence != 0 {
            return Err(TraceError::Ordering {
                previous_sequence: 0,
                sequence: event.sequence,
            });
        }
        self.events.push(event);
        Ok(())
    }

    /// Writes deterministic newline-delimited JSON.
    pub fn write_json_lines(&self, mut writer: impl Write) -> Result<(), TraceError> {
        for event in &self.events {
            serde_json::to_writer(&mut writer, event)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Reads and validates newline-delimited JSON.
    pub fn read_json_lines(reader: impl BufRead) -> Result<Self, TraceError> {
        let mut trace = Self::default();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            trace.push(serde_json::from_str(&line)?)?;
        }
        Ok(trace)
    }

    /// Finds the first semantically unequal event.
    pub fn first_difference<'a>(&'a self, other: &'a Self) -> Option<TraceDifference<'a>> {
        let shared = self.events.len().min(other.events.len());
        for index in 0..shared {
            if self.events[index] != other.events[index] {
                return Some(TraceDifference {
                    index,
                    left: self.events.get(index),
                    right: other.events.get(index),
                });
            }
        }
        (self.events.len() != other.events.len()).then(|| TraceDifference {
            index: shared,
            left: self.events.get(shared),
            right: other.events.get(shared),
        })
    }
}

/// First divergent trace position.
#[derive(Clone, Copy, Debug)]
pub struct TraceDifference<'a> {
    /// Event index.
    pub index: usize,
    /// Left event, absent if left trace ended.
    pub left: Option<&'a TraceEvent>,
    /// Right event, absent if right trace ended.
    pub right: Option<&'a TraceEvent>,
}

impl fmt::Display for TraceDifference<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trace divergence at event {}", self.index)
    }
}

/// Trace serialization or invariant failure.
#[derive(Debug, Error)]
pub enum TraceError {
    /// I/O failure.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// JSON failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Unsupported schema.
    #[error("unsupported trace schema {0}")]
    Schema(u32),
    /// Non-monotonic event order.
    #[error("trace order violation after sequence {previous_sequence}: found {sequence}")]
    Ordering {
        /// Previous sequence.
        previous_sequence: u64,
        /// New sequence.
        sequence: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lines_round_trip() {
        let mut trace = TraceLog::default();
        trace
            .push(TraceEvent::new(0, 0, 0o4000, AgcWord::POSITIVE_ZERO))
            .unwrap();
        let mut bytes = Vec::new();
        trace.write_json_lines(&mut bytes).unwrap();
        let decoded = TraceLog::read_json_lines(bytes.as_slice()).unwrap();
        assert_eq!(decoded, trace);
    }
}
