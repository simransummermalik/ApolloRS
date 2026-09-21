#![forbid(unsafe_code)]
//! Streaming dynamic coverage metrics for long AGC architectural traces.

use agc_trace::{
    InterruptEvent, MachineEventKind, TRACE_SCHEMA_VERSION, TraceError, TraceEvent, TraceJsonReader,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use thiserror::Error;

/// Coverage artifact schema version.
pub const COVERAGE_SCHEMA_VERSION: u32 = 1;

/// Installed Apollo 11 Block II fixed-memory words.
pub const INSTALLED_ROPE_WORDS: usize = 36 * 1024;

/// Complete dynamic coverage report for one validated trace stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Coverage report schema.
    pub schema_version: u32,
    /// Input trace schema.
    pub trace_schema_version: u32,
    /// Event and timing totals.
    pub events: EventCoverage,
    /// Executed instruction forms and physical fetches.
    pub instructions: InstructionCoverage,
    /// Logical and physical memory traffic.
    pub memory: MemoryCoverage,
    /// Channel traffic.
    pub io: IoCoverage,
    /// Interrupt requests, entries, and resumes.
    pub interrupts: InterruptCoverage,
    /// Raw bank-register states observed before or after events.
    pub bank_registers: BankRegisterCoverage,
}

/// Event-kind and cycle coverage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventCoverage {
    /// All architectural events.
    pub total: u64,
    /// Committed ordinary instructions.
    pub instructions: u64,
    /// Accepted interrupt-entry events.
    pub interrupt_entries: u64,
    /// First sequence number.
    pub first_sequence: u64,
    /// Last sequence number.
    pub last_sequence: u64,
    /// First event start cycle.
    pub first_cycle: u64,
    /// Last event end cycle.
    pub last_cycle: u64,
    /// Cycle span from first start through last completion.
    pub cycle_span: u64,
}

/// Instruction and rope-fetch coverage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionCoverage {
    /// Distinct logical instruction addresses.
    pub unique_logical_pcs: usize,
    /// Distinct physical locations fetched and committed.
    pub unique_physical_fetches: usize,
    /// Distinct fixed-memory rope words fetched and committed.
    pub unique_rope_fetches: usize,
    /// Committed fetch events sourced from erasable memory.
    pub erasable_fetch_events: u64,
    /// Committed fetch events sourced from central/special registers.
    pub register_fetch_events: u64,
    /// Installed rope capacity used as the coverage denominator.
    pub installed_rope_words: usize,
    /// Unique fetches per million installed rope words.
    pub rope_fetch_coverage_ppm: u64,
    /// Distinct raw instruction words committed.
    pub unique_raw_words: usize,
    /// Distinct mnemonic/context pairs.
    pub unique_mnemonic_forms: usize,
    /// Basic-context events.
    pub basic_events: u64,
    /// Extracode-context events.
    pub extended_events: u64,
    /// Per mnemonic/context measurements in stable order.
    pub mnemonic_forms: Vec<MnemonicCoverage>,
    /// Physical fixed-bank fetch coverage.
    pub fixed_banks: Vec<BankCoverage>,
    /// Twenty most frequently committed physical words.
    pub hottest_fetch_locations: Vec<LocationCoverage>,
}

/// Measurements for one mnemonic in one decode context.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MnemonicCoverage {
    /// Canonical mnemonic.
    pub mnemonic: String,
    /// True when decoded under `EXTEND` context.
    pub extended: bool,
    /// Committed events.
    pub events: u64,
    /// Sum of event cycle durations.
    pub total_cycles: u64,
    /// Smallest observed event cycle duration.
    pub minimum_cycles: u64,
    /// Largest observed event cycle duration.
    pub maximum_cycles: u64,
    /// Distinct logical PCs for this form.
    pub unique_pcs: usize,
    /// Distinct raw instruction words for this form.
    pub unique_raw_words: usize,
}

/// One physical fetch location and its dynamic count.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocationCoverage {
    /// Canonical physical address.
    pub physical: String,
    /// Committed fetches.
    pub events: u64,
}

/// Aggregate memory coverage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryCoverage {
    /// All traced memory operations, including fetches.
    pub operations: u64,
    /// Instruction fetches.
    pub fetches: u64,
    /// Data reads.
    pub reads: u64,
    /// Data writes.
    pub writes: u64,
    /// Distinct logical addresses touched.
    pub unique_logical_addresses: usize,
    /// Distinct canonical physical addresses touched.
    pub unique_physical_addresses: usize,
    /// Fixed-bank traffic.
    pub fixed_banks: Vec<BankCoverage>,
    /// Erasable-bank traffic.
    pub erasable_banks: Vec<BankCoverage>,
    /// Central/special register traffic.
    pub registers: Vec<RegisterCoverage>,
}

/// Dynamic traffic within one physical memory bank.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BankCoverage {
    /// Physical bank.
    pub bank: u8,
    /// All accesses.
    pub operations: u64,
    /// Fetches.
    pub fetches: u64,
    /// Data reads.
    pub reads: u64,
    /// Data writes.
    pub writes: u64,
    /// Distinct offsets touched by any operation.
    pub unique_offsets: usize,
    /// Distinct offsets fetched as committed instructions.
    pub unique_fetch_offsets: usize,
}

/// Dynamic traffic for one central/special register index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegisterCoverage {
    /// Register index.
    pub index: u16,
    /// Reads.
    pub reads: u64,
    /// Writes.
    pub writes: u64,
}

/// Aggregate I/O-channel coverage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoCoverage {
    /// All channel operations.
    pub operations: u64,
    /// Channel reads.
    pub reads: u64,
    /// Channel writes.
    pub writes: u64,
    /// Distinct channel numbers touched.
    pub unique_channels: usize,
    /// Per-channel traffic.
    pub channels: Vec<ChannelCoverage>,
}

/// Dynamic traffic for one I/O channel.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChannelCoverage {
    /// Channel number.
    pub channel: u16,
    /// Reads.
    pub reads: u64,
    /// Writes.
    pub writes: u64,
}

/// Aggregate interrupt coverage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InterruptCoverage {
    /// Pending requests observed.
    pub requests: u64,
    /// Interrupt entries observed.
    pub entries: u64,
    /// `RESUME` transitions observed.
    pub resumes: u64,
    /// Request counts by interrupt priority number.
    pub requested_numbers: Vec<InterruptNumberCoverage>,
    /// Entry counts by interrupt priority number and vector.
    pub entered_vectors: Vec<InterruptVectorCoverage>,
}

/// Dynamic count for one requested interrupt number.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InterruptNumberCoverage {
    /// Interrupt priority number.
    pub number: u8,
    /// Requests.
    pub events: u64,
}

/// Dynamic count for one entered interrupt vector.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InterruptVectorCoverage {
    /// Interrupt priority number.
    pub number: u8,
    /// Vector address.
    pub vector: u16,
    /// Entries.
    pub events: u64,
}

/// Distinct raw bank-register states observed around events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BankRegisterCoverage {
    /// EB register words.
    pub eb_words: Vec<u16>,
    /// FB register words.
    pub fb_words: Vec<u16>,
    /// BB register words.
    pub bb_words: Vec<u16>,
}

/// Coverage analysis failure.
#[derive(Debug, Error)]
pub enum CoverageError {
    /// Trace input is malformed or violates ordering invariants.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// The stream contains no events.
    #[error("coverage requires at least one trace event")]
    Empty,
    /// A trace memory kind is unknown.
    #[error("unknown trace memory kind {0:?}")]
    MemoryKind(String),
    /// A trace physical address is malformed.
    #[error("malformed trace physical address {0:?}")]
    PhysicalAddress(String),
    /// An ordinary instruction has no physical fetch record.
    #[error("instruction event {0} has no physical fetch")]
    MissingInstructionFetch(u64),
}

#[derive(Clone, Debug, Default)]
struct MnemonicAccumulator {
    events: u64,
    total_cycles: u64,
    minimum_cycles: u64,
    maximum_cycles: u64,
    pcs: BTreeSet<u16>,
    raw_words: BTreeSet<u16>,
}

#[derive(Clone, Debug, Default)]
struct BankAccumulator {
    operations: u64,
    fetches: u64,
    reads: u64,
    writes: u64,
    offsets: BTreeSet<u16>,
    fetch_offsets: BTreeSet<u16>,
}

#[derive(Clone, Copy, Debug, Default)]
struct DirectionAccumulator {
    reads: u64,
    writes: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PhysicalAddress {
    Register(u16),
    Erasable(u8, u16),
    Fixed(u8, u16),
}

#[derive(Clone, Copy, Debug)]
enum AccessKind {
    Fetch,
    Read,
    Write,
}

#[derive(Debug, Default)]
struct CoverageAccumulator {
    total_events: u64,
    instruction_events: u64,
    interrupt_entries: u64,
    first_sequence: Option<u64>,
    last_sequence: u64,
    first_cycle: Option<u64>,
    last_cycle: u64,
    logical_pcs: BTreeSet<u16>,
    physical_fetches: BTreeMap<PhysicalAddress, u64>,
    raw_instruction_words: BTreeSet<u16>,
    basic_events: u64,
    extended_events: u64,
    mnemonic_forms: BTreeMap<(String, bool), MnemonicAccumulator>,
    memory_operations: u64,
    memory_fetches: u64,
    memory_reads: u64,
    memory_writes: u64,
    logical_addresses: BTreeSet<u16>,
    physical_addresses: BTreeSet<PhysicalAddress>,
    fixed_banks: BTreeMap<u8, BankAccumulator>,
    erasable_banks: BTreeMap<u8, BankAccumulator>,
    registers: BTreeMap<u16, DirectionAccumulator>,
    io_operations: u64,
    io_reads: u64,
    io_writes: u64,
    channels: BTreeMap<u16, DirectionAccumulator>,
    interrupt_requests: u64,
    interrupt_entries_seen: u64,
    interrupt_resumes: u64,
    requested_numbers: BTreeMap<u8, u64>,
    entered_vectors: BTreeMap<(u8, u16), u64>,
    eb_words: BTreeSet<u16>,
    fb_words: BTreeSet<u16>,
    bb_words: BTreeSet<u16>,
}

impl CoverageAccumulator {
    fn observe(&mut self, event: &TraceEvent) -> Result<(), CoverageError> {
        self.total_events += 1;
        self.first_sequence.get_or_insert(event.sequence);
        self.last_sequence = event.sequence;
        self.first_cycle.get_or_insert(event.cycle_start);
        self.last_cycle = event.cycle_end;

        for registers in [event.before, event.after] {
            self.eb_words.insert(registers.eb);
            self.fb_words.insert(registers.fb);
            self.bb_words.insert(registers.bb);
        }

        match event.kind {
            MachineEventKind::Instruction => self.observe_instruction(event)?,
            MachineEventKind::InterruptEntry => self.interrupt_entries += 1,
        }
        for access in &event.memory {
            self.observe_memory(&access.kind, access.logical, &access.physical)?;
        }
        for operation in &event.io {
            self.io_operations += 1;
            let channel = self.channels.entry(operation.channel).or_default();
            if operation.write {
                self.io_writes += 1;
                channel.writes += 1;
            } else {
                self.io_reads += 1;
                channel.reads += 1;
            }
        }
        for interrupt in &event.interrupts {
            match *interrupt {
                InterruptEvent::Requested { number } => {
                    self.interrupt_requests += 1;
                    *self.requested_numbers.entry(number).or_default() += 1;
                }
                InterruptEvent::Entered { number, vector } => {
                    self.interrupt_entries_seen += 1;
                    *self.entered_vectors.entry((number, vector)).or_default() += 1;
                }
                InterruptEvent::Resumed => self.interrupt_resumes += 1,
            }
        }
        Ok(())
    }

    fn observe_instruction(&mut self, event: &TraceEvent) -> Result<(), CoverageError> {
        self.instruction_events += 1;
        self.logical_pcs.insert(event.pc);
        self.raw_instruction_words.insert(event.instruction.raw());
        if event.extended {
            self.extended_events += 1;
        } else {
            self.basic_events += 1;
        }
        let cycles = event.cycle_end.saturating_sub(event.cycle_start);
        let form = self
            .mnemonic_forms
            .entry((event.mnemonic.clone(), event.extended))
            .or_default();
        form.events += 1;
        form.total_cycles += cycles;
        if form.events == 1 {
            form.minimum_cycles = cycles;
            form.maximum_cycles = cycles;
        } else {
            form.minimum_cycles = form.minimum_cycles.min(cycles);
            form.maximum_cycles = form.maximum_cycles.max(cycles);
        }
        form.pcs.insert(event.pc);
        form.raw_words.insert(event.instruction.raw());

        let fetch = event
            .memory
            .iter()
            .find(|access| access.kind == "fetch")
            .ok_or(CoverageError::MissingInstructionFetch(event.sequence))?;
        let physical = parse_physical(&fetch.physical)?;
        *self.physical_fetches.entry(physical).or_default() += 1;
        Ok(())
    }

    fn observe_memory(
        &mut self,
        kind: &str,
        logical: u16,
        physical: &str,
    ) -> Result<(), CoverageError> {
        let kind = match kind {
            "fetch" => AccessKind::Fetch,
            "read" => AccessKind::Read,
            "write" => AccessKind::Write,
            other => return Err(CoverageError::MemoryKind(other.to_owned())),
        };
        self.memory_operations += 1;
        match kind {
            AccessKind::Fetch => self.memory_fetches += 1,
            AccessKind::Read => self.memory_reads += 1,
            AccessKind::Write => self.memory_writes += 1,
        }
        self.logical_addresses.insert(logical);
        let physical = parse_physical(physical)?;
        self.physical_addresses.insert(physical);
        match physical {
            PhysicalAddress::Fixed(bank, offset) => {
                observe_bank(self.fixed_banks.entry(bank).or_default(), kind, offset);
            }
            PhysicalAddress::Erasable(bank, offset) => {
                observe_bank(self.erasable_banks.entry(bank).or_default(), kind, offset);
            }
            PhysicalAddress::Register(index) => {
                let register = self.registers.entry(index).or_default();
                match kind {
                    AccessKind::Fetch | AccessKind::Read => register.reads += 1,
                    AccessKind::Write => register.writes += 1,
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<CoverageReport, CoverageError> {
        let first_sequence = self.first_sequence.ok_or(CoverageError::Empty)?;
        let first_cycle = self.first_cycle.ok_or(CoverageError::Empty)?;
        let unique_physical_fetches = self.physical_fetches.len();
        let unique_rope_fetches = self
            .physical_fetches
            .keys()
            .filter(|address| matches!(address, PhysicalAddress::Fixed(_, _)))
            .count();
        let erasable_fetch_events = self
            .physical_fetches
            .iter()
            .filter_map(|(address, events)| {
                matches!(address, PhysicalAddress::Erasable(_, _)).then_some(events)
            })
            .sum();
        let register_fetch_events = self
            .physical_fetches
            .iter()
            .filter_map(|(address, events)| {
                matches!(address, PhysicalAddress::Register(_)).then_some(events)
            })
            .sum();
        let mut hottest_fetch_locations = self
            .physical_fetches
            .iter()
            .map(|(&physical, &events)| LocationCoverage {
                physical: format_physical(physical),
                events,
            })
            .collect::<Vec<_>>();
        hottest_fetch_locations.sort_by(|left, right| {
            right
                .events
                .cmp(&left.events)
                .then_with(|| left.physical.cmp(&right.physical))
        });
        hottest_fetch_locations.truncate(20);

        let mnemonic_forms = self
            .mnemonic_forms
            .into_iter()
            .map(|((mnemonic, extended), form)| MnemonicCoverage {
                mnemonic,
                extended,
                events: form.events,
                total_cycles: form.total_cycles,
                minimum_cycles: form.minimum_cycles,
                maximum_cycles: form.maximum_cycles,
                unique_pcs: form.pcs.len(),
                unique_raw_words: form.raw_words.len(),
            })
            .collect::<Vec<_>>();
        let unique_mnemonic_forms = mnemonic_forms.len();
        let instruction_fixed_banks = bank_coverage_from_fetches(&self.physical_fetches);
        let fixed_banks = bank_coverage(self.fixed_banks);
        let erasable_banks = bank_coverage(self.erasable_banks);
        let registers = self
            .registers
            .into_iter()
            .map(|(index, counts)| RegisterCoverage {
                index,
                reads: counts.reads,
                writes: counts.writes,
            })
            .collect();
        let channels = self
            .channels
            .into_iter()
            .map(|(channel, counts)| ChannelCoverage {
                channel,
                reads: counts.reads,
                writes: counts.writes,
            })
            .collect::<Vec<_>>();
        let unique_channels = channels.len();
        Ok(CoverageReport {
            schema_version: COVERAGE_SCHEMA_VERSION,
            trace_schema_version: TRACE_SCHEMA_VERSION,
            events: EventCoverage {
                total: self.total_events,
                instructions: self.instruction_events,
                interrupt_entries: self.interrupt_entries,
                first_sequence,
                last_sequence: self.last_sequence,
                first_cycle,
                last_cycle: self.last_cycle,
                cycle_span: self.last_cycle.saturating_sub(first_cycle),
            },
            instructions: InstructionCoverage {
                unique_logical_pcs: self.logical_pcs.len(),
                unique_physical_fetches,
                unique_rope_fetches,
                erasable_fetch_events,
                register_fetch_events,
                installed_rope_words: INSTALLED_ROPE_WORDS,
                rope_fetch_coverage_ppm: (unique_rope_fetches as u64).saturating_mul(1_000_000)
                    / INSTALLED_ROPE_WORDS as u64,
                unique_raw_words: self.raw_instruction_words.len(),
                unique_mnemonic_forms,
                basic_events: self.basic_events,
                extended_events: self.extended_events,
                mnemonic_forms,
                fixed_banks: instruction_fixed_banks,
                hottest_fetch_locations,
            },
            memory: MemoryCoverage {
                operations: self.memory_operations,
                fetches: self.memory_fetches,
                reads: self.memory_reads,
                writes: self.memory_writes,
                unique_logical_addresses: self.logical_addresses.len(),
                unique_physical_addresses: self.physical_addresses.len(),
                fixed_banks,
                erasable_banks,
                registers,
            },
            io: IoCoverage {
                operations: self.io_operations,
                reads: self.io_reads,
                writes: self.io_writes,
                unique_channels,
                channels,
            },
            interrupts: InterruptCoverage {
                requests: self.interrupt_requests,
                entries: self.interrupt_entries_seen,
                resumes: self.interrupt_resumes,
                requested_numbers: self
                    .requested_numbers
                    .into_iter()
                    .map(|(number, events)| InterruptNumberCoverage { number, events })
                    .collect(),
                entered_vectors: self
                    .entered_vectors
                    .into_iter()
                    .map(|((number, vector), events)| InterruptVectorCoverage {
                        number,
                        vector,
                        events,
                    })
                    .collect(),
            },
            bank_registers: BankRegisterCoverage {
                eb_words: self.eb_words.into_iter().collect(),
                fb_words: self.fb_words.into_iter().collect(),
                bb_words: self.bb_words.into_iter().collect(),
            },
        })
    }
}

/// Analyzes a validated JSON-lines trace without retaining its events.
pub fn analyze_json_lines(reader: impl BufRead) -> Result<CoverageReport, CoverageError> {
    let mut reader = TraceJsonReader::new(reader);
    let mut coverage = CoverageAccumulator::default();
    while let Some(event) = reader.next_event()? {
        coverage.observe(&event)?;
    }
    coverage.finish()
}

fn observe_bank(bank: &mut BankAccumulator, kind: AccessKind, offset: u16) {
    bank.operations += 1;
    bank.offsets.insert(offset);
    match kind {
        AccessKind::Fetch => {
            bank.fetches += 1;
            bank.fetch_offsets.insert(offset);
        }
        AccessKind::Read => bank.reads += 1,
        AccessKind::Write => bank.writes += 1,
    }
}

fn bank_coverage(banks: BTreeMap<u8, BankAccumulator>) -> Vec<BankCoverage> {
    banks
        .into_iter()
        .map(|(bank, counts)| BankCoverage {
            bank,
            operations: counts.operations,
            fetches: counts.fetches,
            reads: counts.reads,
            writes: counts.writes,
            unique_offsets: counts.offsets.len(),
            unique_fetch_offsets: counts.fetch_offsets.len(),
        })
        .collect()
}

fn bank_coverage_from_fetches(fetches: &BTreeMap<PhysicalAddress, u64>) -> Vec<BankCoverage> {
    let mut banks = BTreeMap::<u8, BankAccumulator>::new();
    for (&address, &events) in fetches {
        let PhysicalAddress::Fixed(bank, offset) = address else {
            continue;
        };
        let counts = banks.entry(bank).or_default();
        counts.operations += events;
        counts.fetches += events;
        counts.offsets.insert(offset);
        counts.fetch_offsets.insert(offset);
    }
    bank_coverage(banks)
}

fn format_physical(address: PhysicalAddress) -> String {
    match address {
        PhysicalAddress::Register(index) => format!("R:{index:02o}"),
        PhysicalAddress::Erasable(bank, offset) => format!("E{bank:o}:{offset:04o}"),
        PhysicalAddress::Fixed(bank, offset) => format!("F{bank:02o}:{offset:04o}"),
    }
}

fn parse_physical(value: &str) -> Result<PhysicalAddress, CoverageError> {
    let (region, offset) = value
        .split_once(':')
        .ok_or_else(|| CoverageError::PhysicalAddress(value.to_owned()))?;
    let offset = u16::from_str_radix(offset, 8)
        .map_err(|_| CoverageError::PhysicalAddress(value.to_owned()))?;
    if region == "R" {
        return Ok(PhysicalAddress::Register(offset));
    }
    let (prefix, bank) = region.split_at(1);
    let bank = u8::from_str_radix(bank, 8)
        .map_err(|_| CoverageError::PhysicalAddress(value.to_owned()))?;
    match prefix {
        "E" if bank < 8 && offset < 0o400 => Ok(PhysicalAddress::Erasable(bank, offset)),
        "F" if bank < 0o44 && offset < 0o2000 => Ok(PhysicalAddress::Fixed(bank, offset)),
        _ => Err(CoverageError::PhysicalAddress(value.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_trace::{IoEvent, MemoryEvent, RegisterSnapshot, TraceLog};
    use agc_word::AgcWord;

    #[test]
    fn streaming_coverage_counts_machine_surfaces() {
        let mut instruction = TraceEvent::new(0, 0, 0o4000, AgcWord::from_raw_truncate(0o30001));
        instruction.cycle_end = 2;
        instruction.mnemonic = "CA".to_owned();
        instruction.operand = 1;
        instruction.after = RegisterSnapshot {
            fb: 0o4000,
            bb: 0o4000,
            ..RegisterSnapshot::default()
        };
        instruction.memory = vec![
            MemoryEvent {
                kind: "fetch".to_owned(),
                logical: 0o4000,
                physical: "F02:0000".to_owned(),
                value: instruction.instruction,
            },
            MemoryEvent {
                kind: "read".to_owned(),
                logical: 1,
                physical: "R:01".to_owned(),
                value: AgcWord::POSITIVE_ZERO,
            },
        ];
        instruction.io.push(IoEvent {
            write: true,
            channel: 0o10,
            value: AgcWord::from_raw_truncate(1),
        });
        instruction
            .interrupts
            .push(InterruptEvent::Requested { number: 3 });

        let mut interrupt = TraceEvent::new(1, 2, 0o4001, AgcWord::POSITIVE_ZERO);
        interrupt.kind = MachineEventKind::InterruptEntry;
        interrupt.cycle_end = 4;
        interrupt.memory.push(MemoryEvent {
            kind: "fetch".to_owned(),
            logical: 0o4001,
            physical: "F02:0001".to_owned(),
            value: AgcWord::POSITIVE_ZERO,
        });
        interrupt.interrupts.push(InterruptEvent::Entered {
            number: 3,
            vector: 0o2014,
        });

        let mut trace = TraceLog::default();
        trace.push(instruction).unwrap();
        trace.push(interrupt).unwrap();
        let mut bytes = Vec::new();
        trace.write_json_lines(&mut bytes).unwrap();

        let report = analyze_json_lines(bytes.as_slice()).unwrap();
        assert_eq!(report.events.total, 2);
        assert_eq!(report.events.instructions, 1);
        assert_eq!(report.events.interrupt_entries, 1);
        assert_eq!(report.instructions.unique_physical_fetches, 1);
        assert_eq!(report.instructions.unique_rope_fetches, 1);
        assert_eq!(report.instructions.mnemonic_forms[0].mnemonic, "CA");
        assert_eq!(report.memory.fetches, 2);
        assert_eq!(report.memory.registers[0].reads, 1);
        assert_eq!(report.io.channels[0].writes, 1);
        assert_eq!(report.interrupts.requests, 1);
        assert_eq!(report.interrupts.entries, 1);
    }

    #[test]
    fn malformed_physical_addresses_fail_closed() {
        assert!(parse_physical("F44:0000").is_err());
        assert!(parse_physical("E8:0000").is_err());
        assert!(parse_physical("nowhere").is_err());
    }

    #[test]
    fn register_sourced_instruction_is_not_counted_as_rope_coverage() {
        let mut instruction = TraceEvent::new(0, 0, 2, AgcWord::from_raw_truncate(0o2704));
        instruction.cycle_end = 1;
        instruction.mnemonic = "TC".to_owned();
        instruction.memory.push(MemoryEvent {
            kind: "fetch".to_owned(),
            logical: 2,
            physical: "R:02".to_owned(),
            value: instruction.instruction,
        });
        let mut trace = TraceLog::default();
        trace.push(instruction).unwrap();
        let mut bytes = Vec::new();
        trace.write_json_lines(&mut bytes).unwrap();

        let report = analyze_json_lines(bytes.as_slice()).unwrap();
        assert_eq!(report.instructions.unique_physical_fetches, 1);
        assert_eq!(report.instructions.unique_rope_fetches, 0);
        assert_eq!(report.instructions.register_fetch_events, 1);
        assert!(report.instructions.fixed_banks.is_empty());
    }
}
