#![forbid(unsafe_code)]
//! Deterministic Block II CPU with complete basic/extracode dispatch and trace coverage.

use agc_isa::{DecodedInstruction, Mnemonic, decode};
use agc_memory::{AccessKind, Memory, MemoryAccess, MemoryError, PhysicalAddress, register};
use agc_trace::{
    InterruptEvent, IoEvent, MemoryEvent, RegisterSnapshot, TraceError, TraceEvent, TraceLog,
};
use agc_word::{AgcDoubleWord, AgcRegister, AgcWord, ChannelAddress, SignClass};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Interrupt request identifiers and priority order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum Interrupt {
    /// TIME6 counter overflow.
    Time6 = 1,
    /// TIME5 counter overflow.
    Time5 = 2,
    /// TIME3 counter overflow.
    Time3 = 3,
    /// TIME4 counter overflow.
    Time4 = 4,
    /// First keyboard interrupt.
    Key1 = 5,
    /// Second keyboard interrupt.
    Key2 = 6,
    /// Uplink interrupt.
    Uprupt = 7,
    /// Downlink interrupt.
    Downrupt = 8,
    /// Radar data interrupt.
    Radar = 9,
    /// Manual interrupt.
    Handrupt = 10,
}

impl Interrupt {
    /// Four-word interrupt vector in fixed-fixed memory.
    pub const fn vector(self) -> u16 {
        0o4000 + (self as u16) * 4
    }
}

/// Reason execution stopped before reaching an instruction budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    /// Requested instruction budget was consumed.
    InstructionLimit,
    /// Program counter matched a breakpoint.
    Breakpoint(u16),
    /// An instruction wrote a watched address.
    Watchpoint(u16),
}

/// Result of one committed CPU step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    /// Decoded operation.
    pub instruction: DecodedInstruction,
    /// Committed trace event.
    pub trace: TraceEvent,
    /// Watched address written during the step.
    pub watchpoint: Option<u16>,
}

/// Result of bounded execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    /// Why execution stopped.
    pub reason: StopReason,
    /// Number of instructions committed by this run.
    pub instructions: u64,
    /// Total machine cycle counter at stop.
    pub cycles: u64,
}

/// CPU execution failure.
#[derive(Debug, Error)]
pub enum CpuError {
    /// Memory map failure.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// Trace invariant failure.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// Architecturally illegal arithmetic condition.
    #[error("{0}")]
    Arithmetic(String),
}

/// Full deterministic CPU and architectural control state.
#[derive(Clone, Debug)]
pub struct Cpu {
    memory: Memory,
    extended: bool,
    indexed: Option<AgcWord>,
    substitute_instruction: bool,
    interrupt_enabled: bool,
    in_interrupt: bool,
    pending_interrupts: BTreeSet<Interrupt>,
    cycles: u64,
    sequence: u64,
    breakpoints: BTreeSet<u16>,
    watchpoints: BTreeSet<u16>,
    trace: TraceLog,
    scaler_mcts: u16,
    scaler: u32,
    downrupt_mcts: u32,
}

impl Cpu {
    /// Creates a CPU and resets volatile machine state to the restart vector.
    pub fn new(mut memory: Memory) -> Self {
        memory.reset_volatile();
        let mut cpu = Self {
            memory,
            extended: false,
            indexed: None,
            substitute_instruction: false,
            interrupt_enabled: true,
            in_interrupt: false,
            pending_interrupts: BTreeSet::new(),
            cycles: 0,
            sequence: 0,
            breakpoints: BTreeSet::new(),
            watchpoints: BTreeSet::new(),
            trace: TraceLog::default(),
            scaler_mcts: 0,
            scaler: 0,
            downrupt_mcts: 0,
        };
        cpu.set_z(0o4000);
        cpu
    }

    /// Returns machine memory for read-only inspection.
    pub const fn memory(&self) -> &Memory {
        &self.memory
    }

    /// Returns machine memory for deterministic peripheral or test setup.
    pub fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }

    /// Returns the accumulated deterministic trace.
    pub const fn trace(&self) -> &TraceLog {
        &self.trace
    }

    /// Removes and returns the trace accumulated so far.
    pub fn take_trace(&mut self) -> TraceLog {
        std::mem::take(&mut self.trace)
    }

    /// Returns the total machine-cycle count.
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Returns the number of committed instructions.
    pub const fn instructions(&self) -> u64 {
        self.sequence
    }

    /// Returns whether maskable interrupts are enabled.
    pub const fn interrupt_enabled(&self) -> bool {
        self.interrupt_enabled
    }

    /// Returns whether execution is inside an interrupt handler.
    pub const fn in_interrupt(&self) -> bool {
        self.in_interrupt
    }

    /// Returns whether the next fetched word is decoded as an extracode.
    pub const fn extended_pending(&self) -> bool {
        self.extended
    }

    /// Returns the current program counter.
    pub fn program_counter(&self) -> u16 {
        self.memory
            .central_register(register::Z)
            .expect("Z register exists")
            .raw()
            & 0o7777
    }

    /// Returns a full central register for debugger display.
    pub fn central_register(&self, index: u16) -> Result<AgcRegister, CpuError> {
        Ok(self.memory.central_register(index)?)
    }

    /// Adds a breakpoint.
    pub fn add_breakpoint(&mut self, address: u16) -> Result<(), CpuError> {
        if address > 0o7777 {
            return Err(MemoryError::AddressOutOfRange(address).into());
        }
        self.breakpoints.insert(address);
        Ok(())
    }

    /// Removes a breakpoint.
    pub fn remove_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    /// Adds an erasable/logical write watchpoint.
    pub fn add_watchpoint(&mut self, address: u16) -> Result<(), CpuError> {
        if address > 0o7777 {
            return Err(MemoryError::AddressOutOfRange(address).into());
        }
        self.watchpoints.insert(address);
        Ok(())
    }

    /// Requests a maskable interrupt. Duplicate requests coalesce in hardware order.
    pub fn request_interrupt(&mut self, interrupt: Interrupt) {
        self.pending_interrupts.insert(interrupt);
    }

    /// Cancels a pending interrupt for explicit hardware fault injection.
    pub fn cancel_interrupt(&mut self, interrupt: Interrupt) -> bool {
        self.pending_interrupts.remove(&interrupt)
    }

    /// Resets volatile state while preserving the loaded rope and debugger controls.
    pub fn reset(&mut self) {
        self.memory.reset_volatile();
        self.extended = false;
        self.indexed = None;
        self.substitute_instruction = false;
        self.interrupt_enabled = true;
        self.in_interrupt = false;
        self.pending_interrupts.clear();
        self.cycles = 0;
        self.sequence = 0;
        self.trace.events.clear();
        self.scaler_mcts = 0;
        self.scaler = 0;
        self.downrupt_mcts = 0;
        self.set_z(0o4000);
    }

    /// Executes one instruction and commits a complete trace event.
    pub fn step(&mut self) -> Result<StepOutcome, CpuError> {
        let mut interrupt_events = Vec::new();
        self.enter_pending_interrupt(&mut interrupt_events)?;
        let pc = self.program_counter();
        let before = self.snapshot();
        let fetch_address = if std::mem::take(&mut self.substitute_instruction) {
            register::BRUPT
        } else {
            pc
        };
        let fetch = self.memory.read(fetch_address, AccessKind::Fetch)?;
        let fetched_word = fetch.value;
        self.set_z(pc.wrapping_add(1) & 0o7777);
        let actual_word = self
            .indexed
            .take()
            .map_or(fetched_word, |index| fetched_word.wrapping_add(index));
        let used_extended = self.extended;
        self.extended = false;
        let instruction = decode(actual_word, used_extended);
        let mut event = TraceEvent::new(self.sequence, self.cycles, pc, actual_word);
        event.before = before;
        event.mnemonic = instruction.mnemonic.to_string();
        event.operand = instruction.operand;
        event.extended = instruction.extended;
        event.memory.push(memory_event(fetch));
        event.interrupts = interrupt_events;
        let mut watchpoint = None;
        self.execute(instruction, &mut event, &mut watchpoint)?;
        self.cycles += u64::from(instruction.cycles);
        self.tick_timers(instruction.cycles, &mut event)?;
        event.cycle_end = self.cycles;
        event.after = self.snapshot();
        self.trace.push(event.clone())?;
        self.sequence += 1;
        Ok(StepOutcome {
            instruction,
            trace: event,
            watchpoint,
        })
    }

    /// Runs until a breakpoint, watchpoint, or instruction budget.
    pub fn run(&mut self, instruction_limit: u64) -> Result<RunOutcome, CpuError> {
        let start = self.sequence;
        while self.sequence - start < instruction_limit {
            let pc = self.program_counter();
            if self.breakpoints.contains(&pc) {
                return Ok(RunOutcome {
                    reason: StopReason::Breakpoint(pc),
                    instructions: self.sequence - start,
                    cycles: self.cycles,
                });
            }
            let outcome = self.step()?;
            if let Some(address) = outcome.watchpoint {
                return Ok(RunOutcome {
                    reason: StopReason::Watchpoint(address),
                    instructions: self.sequence - start,
                    cycles: self.cycles,
                });
            }
        }
        Ok(RunOutcome {
            reason: StopReason::InstructionLimit,
            instructions: self.sequence - start,
            cycles: self.cycles,
        })
    }

    fn execute(
        &mut self,
        instruction: DecodedInstruction,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        let k = instruction.operand;
        match instruction.mnemonic {
            Mnemonic::Ad => {
                let value = self.read_register_operand(k, event)?;
                self.add_register_to_a(value);
            }
            Mnemonic::Ads => {
                let value = self.read_register_operand(k, event)?;
                self.add_register_to_a(value);
                self.write_register_operand(k, self.a_register(), event, watchpoint)?;
            }
            Mnemonic::Aug => {
                let value = self.read_register_operand(k, event)?;
                let increment = if value.raw() & 0o100000 == 0 {
                    AgcWord::from_raw_truncate(1).sign_extend()
                } else {
                    AgcWord::from_raw_truncate(0o77776).sign_extend()
                };
                self.write_register_operand(k, value.wrapping_add(increment), event, watchpoint)?;
            }
            Mnemonic::Bzf => {
                if matches!(self.a_raw(), 0 | 0o177777) {
                    self.set_z(k);
                }
            }
            Mnemonic::Bzmf => {
                let a = self.a_raw();
                if a == 0 || a & 0o100000 != 0 {
                    self.set_z(k);
                }
            }
            Mnemonic::Ca => {
                let value = self.read_register_operand(k, event)?;
                self.set_a_register(value);
            }
            Mnemonic::Ccs => self.ccs(k, event)?,
            Mnemonic::Cs => {
                let value = self.read_register_operand(k, event)?;
                self.set_a_raw(!value.raw());
            }
            Mnemonic::Das => self.das(k, event, watchpoint)?,
            Mnemonic::Dca => self.double_load(k, false, event)?,
            Mnemonic::Dcs => self.double_load(k, true, event)?,
            Mnemonic::Dim => {
                let value = self.read_register_operand(k, event)?;
                if !matches!(value.raw(), 0 | 0o177777) {
                    let increment = if value.raw() & 0o100000 == 0 {
                        AgcWord::from_raw_truncate(0o77776).sign_extend()
                    } else {
                        AgcWord::from_raw_truncate(1).sign_extend()
                    };
                    self.write_register_operand(
                        k,
                        value.wrapping_add(increment),
                        event,
                        watchpoint,
                    )?;
                }
            }
            Mnemonic::Dv => self.divide(k, event)?,
            Mnemonic::Dxch => self.double_exchange(k, event, watchpoint)?,
            Mnemonic::EdrupT => {
                self.interrupt_enabled = false;
                self.software_interrupt(event)?;
            }
            Mnemonic::Extend => self.extended = true,
            Mnemonic::Incr => {
                let value = self.read_register_operand(k, event)?;
                let result = value.wrapping_add(AgcWord::from_raw_truncate(1).sign_extend());
                self.write_register_operand(k, result, event, watchpoint)?;
            }
            Mnemonic::Index => {
                self.indexed = Some(self.read_word(k, event)?);
                self.extended = instruction.extended;
            }
            Mnemonic::Inhint => self.interrupt_enabled = false,
            Mnemonic::Lxch => self.exchange_register(k, register::L, event, watchpoint)?,
            Mnemonic::Mask => self.mask(k, event)?,
            Mnemonic::Mp => self.multiply(k, event)?,
            Mnemonic::Msu => self.modular_subtract(k, event)?,
            Mnemonic::Qxch => self.exchange_register(k, register::Q, event, watchpoint)?,
            Mnemonic::Rand => self.channel_logic(k, ChannelLogic::AndRead, event)?,
            Mnemonic::Read => {
                let value = self.read_channel(k, event)?;
                self.set_a_word(value);
            }
            Mnemonic::Relint => self.interrupt_enabled = true,
            Mnemonic::Resume => self.resume(event)?,
            Mnemonic::Ror => self.channel_logic(k, ChannelLogic::OrRead, event)?,
            Mnemonic::Rxor => self.channel_logic(k, ChannelLogic::XorRead, event)?,
            Mnemonic::Su => {
                let value = self.read_register_operand(k, event)?;
                self.add_register_to_a(register_from_raw(!value.raw()));
            }
            Mnemonic::Tc => {
                let return_address = self.program_counter();
                if k != register::Q {
                    self.set_q_raw(return_address);
                }
                self.set_z(k);
            }
            Mnemonic::Tcf => self.set_z(k),
            Mnemonic::Ts => self.transfer_to_storage(k, event, watchpoint)?,
            Mnemonic::Wand => self.channel_logic(k, ChannelLogic::AndWrite, event)?,
            Mnemonic::Wor => self.channel_logic(k, ChannelLogic::OrWrite, event)?,
            Mnemonic::Write => {
                let value = self.a_word();
                self.write_channel(k, value, event)?;
            }
            Mnemonic::Xch => self.exchange_register(k, register::A, event, watchpoint)?,
        }
        Ok(())
    }

    fn ccs(&mut self, k: u16, event: &mut TraceEvent) -> Result<(), CpuError> {
        let value = self.read_register_operand(k, event)?;
        let raw = value.raw();
        let magnitude = if raw & 0o100000 == 0 { raw } else { !raw };
        self.set_a_raw(if magnitude > 1 { magnitude - 1 } else { 0 });

        if k < 0o20 {
            match raw & 0o140000 {
                0o040000 => return Ok(()),
                0o100000 => {
                    self.skip(2);
                    return Ok(());
                }
                _ => {}
            }
        }
        match value.overflow_correct().sign_class() {
            SignClass::PositiveZero => self.skip(1),
            SignClass::NegativeZero => self.skip(3),
            SignClass::Negative => self.skip(2),
            SignClass::Positive => {}
        }
        Ok(())
    }

    fn transfer_to_storage(
        &mut self,
        k: u16,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        let accumulator = self.a_register();
        let overflow = accumulator.raw() & 0o140000;
        match k {
            register::A => {
                if overflow != 0 && matches!(overflow, 0o040000 | 0o100000) {
                    self.skip(1);
                }
                return Ok(());
            }
            register::Z => {
                self.set_z(accumulator.raw() & 0o7777);
                match overflow {
                    0o040000 => self.set_a_word(AgcWord::from_raw_truncate(1)),
                    0o100000 => self.set_a_word(AgcWord::from_raw_truncate(0o77776)),
                    _ => {}
                }
                return Ok(());
            }
            _ => self.write_register_operand(k, accumulator, event, watchpoint)?,
        }
        match overflow {
            0o040000 => {
                self.set_a_word(AgcWord::from_raw_truncate(1));
                self.skip(1);
            }
            0o100000 => {
                self.set_a_word(AgcWord::from_raw_truncate(0o77776));
                self.skip(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn das(
        &mut self,
        k: u16,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        if k == register::L {
            let mut low = self.l_register().wrapping_add(self.l_register());
            let mut high = self.a_register().wrapping_add(self.a_register());
            high = add_double_precision_carry(high, low);
            low = low.overflow_correct().sign_extend();
            self.set_a_register(high);
            self.set_l_register(low);
            return Ok(());
        }

        // The encoded address is pre-incremented: A addresses K-1 and L K.
        let high_address = k.saturating_sub(1);
        let low_address = k;
        let memory_low = self.read_register_operand(low_address, event)?;
        let memory_high = self.read_register_operand(high_address, event)?;
        let mut high = self.a_register().wrapping_add(memory_high);
        let mut low = self.l_register().wrapping_add(memory_low);
        if low.has_overflow() {
            high = add_double_precision_carry(high, low);
        }
        low = low.overflow_correct().sign_extend();
        self.set_a_raw(match high.raw() & 0o140000 {
            0o100000 => 0o177776,
            0o040000 => 1,
            _ => 0,
        });
        self.set_l_word(AgcWord::POSITIVE_ZERO);
        self.write_register_operand(low_address, low, event, watchpoint)?;
        self.write_register_operand(high_address, high, event, watchpoint)?;
        Ok(())
    }

    fn double_load(
        &mut self,
        k: u16,
        complement: bool,
        event: &mut TraceEvent,
    ) -> Result<(), CpuError> {
        if k == register::L {
            if complement {
                self.set_a_raw(!self.a_raw());
                self.set_l_register(
                    register_from_raw(!self.l_register().raw())
                        .overflow_correct()
                        .sign_extend(),
                );
            } else {
                self.set_l_register(self.l_register().overflow_correct().sign_extend());
            }
            return Ok(());
        }

        let low = self.read_register_operand(k, event)?;
        let high = self.read_register_operand(k.saturating_sub(1), event)?;
        let low = if complement {
            register_from_raw(!low.raw())
        } else {
            low
        };
        let high = if complement {
            register_from_raw(!high.raw())
        } else {
            high
        };
        self.set_l_register(low.overflow_correct().sign_extend());
        self.set_a_register(high);
        Ok(())
    }

    fn double_exchange(
        &mut self,
        k: u16,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        if k == register::L {
            self.set_l_register(self.l_register().overflow_correct().sign_extend());
            return Ok(());
        }

        let memory_low = self.read_register_operand(k, event)?;
        let old_l = self.l_register();
        self.write_register_operand(k, old_l, event, watchpoint)?;
        self.set_l_register(memory_low.overflow_correct().sign_extend());

        let high_address = k.saturating_sub(1);
        let memory_high = self.read_register_operand(high_address, event)?;
        let old_a = self.a_register();
        self.write_register_operand(high_address, old_a, event, watchpoint)?;
        self.set_a_register(memory_high);
        Ok(())
    }

    fn exchange_register(
        &mut self,
        k: u16,
        register_index: u16,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        if k == register_index {
            return Ok(());
        }
        if k == register::ZERO && register_index != register::A {
            self.set_central_register(register_index, register_from_raw(0));
            return Ok(());
        }
        let memory_value = self.read_register_operand(k, event)?;
        let register_value = self.memory.central_register(register_index)?;
        self.write_register_operand(k, register_value, event, watchpoint)?;
        self.set_central_register(register_index, memory_value);
        Ok(())
    }

    fn mask(&mut self, k: u16, event: &mut TraceEvent) -> Result<(), CpuError> {
        if k < 0o20 {
            let rhs = self.read_register_operand(k, event)?.raw();
            self.set_a_raw(self.a_raw() & rhs);
        } else {
            let rhs = self.read_word(k, event)?;
            self.set_a_word(AgcWord::from_raw_truncate(self.a_word().raw() & rhs.raw()));
        }
        Ok(())
    }

    fn multiply(&mut self, k: u16, event: &mut TraceEvent) -> Result<(), CpuError> {
        let a = self.a_word();
        let rhs = self.read_word(k, event)?;
        let negative = a.is_negative() ^ rhs.is_negative();
        let magnitude = i64::from(a.to_i32_lossy_zero().unsigned_abs())
            * i64::from(rhs.to_i32_lossy_zero().unsigned_abs());
        if magnitude == 0 && negative && a.is_zero() && !rhs.is_zero() {
            self.set_a_word(AgcWord::NEGATIVE_ZERO);
            self.set_l_word(AgcWord::NEGATIVE_ZERO);
            return Ok(());
        }
        let result = AgcDoubleWord::from_i64(if negative { -magnitude } else { magnitude })
            .map_err(|error| CpuError::Arithmetic(error.to_string()))?;
        self.set_a_word(result.high);
        self.set_l_word(result.low);
        Ok(())
    }

    fn divide(&mut self, k: u16, event: &mut TraceEvent) -> Result<(), CpuError> {
        let divisor_word = self.read_word(k, event)?;
        let divisor = i64::from(divisor_word.to_i32_lossy_zero());
        if divisor == 0 {
            return Err(CpuError::Arithmetic("DV divisor is signed zero".to_owned()));
        }
        let dividend = AgcDoubleWord {
            high: self.a_word(),
            low: self.l_word(),
        }
        .to_i64_lossy_zero();
        let quotient = dividend / divisor;
        let remainder = dividend % divisor;
        if !(-i64::from(agc_word::MAX_MAGNITUDE)..=i64::from(agc_word::MAX_MAGNITUDE))
            .contains(&quotient)
        {
            return Err(CpuError::Arithmetic(format!(
                "DV quotient {quotient} exceeds a single AGC word"
            )));
        }
        self.set_a_word(
            AgcWord::from_i32(quotient as i32)
                .map_err(|error| CpuError::Arithmetic(error.to_string()))?,
        );
        self.set_l_word(
            AgcWord::from_i32(remainder as i32)
                .map_err(|error| CpuError::Arithmetic(error.to_string()))?,
        );
        Ok(())
    }

    fn modular_subtract(&mut self, k: u16, event: &mut TraceEvent) -> Result<(), CpuError> {
        let rhs = self.read_word(k, event)?.raw();
        let lhs = self.a_word().raw();
        let twos = lhs.wrapping_sub(rhs) & 0o77777;
        let ones = if twos & 0o40000 != 0 {
            twos.wrapping_sub(1) & 0o77777
        } else {
            twos
        };
        self.set_a_word(AgcWord::from_raw_truncate(ones));
        Ok(())
    }

    fn channel_logic(
        &mut self,
        channel: u16,
        operation: ChannelLogic,
        event: &mut TraceEvent,
    ) -> Result<(), CpuError> {
        let current = self.read_channel(channel, event)?;
        let a = self.a_word();
        let result = match operation {
            ChannelLogic::AndRead | ChannelLogic::AndWrite => {
                AgcWord::from_raw_truncate(a.raw() & current.raw())
            }
            ChannelLogic::OrRead | ChannelLogic::OrWrite => {
                AgcWord::from_raw_truncate(a.raw() | current.raw())
            }
            ChannelLogic::XorRead => AgcWord::from_raw_truncate(a.raw() ^ current.raw()),
        };
        self.set_a_word(result);
        if matches!(operation, ChannelLogic::AndWrite | ChannelLogic::OrWrite) {
            self.write_channel(channel, result, event)?;
        }
        Ok(())
    }

    fn software_interrupt(&mut self, event: &mut TraceEvent) -> Result<(), CpuError> {
        let zrupt = self.program_counter();
        self.memory
            .set_central_register(register::ZRUPT, register_from_raw(zrupt))?;
        self.memory
            .set_central_register(register::BRUPT, event.instruction.sign_extend())?;
        self.set_z(0);
        self.indexed = None;
        self.in_interrupt = true;
        Ok(())
    }

    fn resume(&mut self, event: &mut TraceEvent) -> Result<(), CpuError> {
        // The interrupt routine restores A, L, Q, and BB in software.  RESUME
        // backs up to the interrupted address and substitutes BRUPT once.
        let zrupt = self.memory.central_register(register::ZRUPT)?.raw() & 0o7777;
        self.set_z(zrupt.wrapping_sub(1) & 0o7777);
        self.substitute_instruction = true;
        self.in_interrupt = false;
        self.extended = false;
        event.interrupts.push(InterruptEvent::Resumed);
        Ok(())
    }

    fn enter_pending_interrupt(
        &mut self,
        events: &mut Vec<InterruptEvent>,
    ) -> Result<(), CpuError> {
        if !self.interrupt_enabled
            || self.in_interrupt
            || self.extended
            || self.a_register().has_overflow()
        {
            return Ok(());
        }
        let pc = self.program_counter();
        let fetched = if self.substitute_instruction {
            self.memory
                .central_register(register::BRUPT)?
                .overflow_correct()
        } else {
            self.memory.read(pc, AccessKind::Read)?.value
        };
        let interrupted = self
            .indexed
            .map_or(fetched, |index| fetched.wrapping_add(index));
        // RELINT, INHINT, and EXTEND are protected from interrupt entry.
        if matches!(interrupted.raw(), 3 | 4 | 6) {
            return Ok(());
        }
        let Some(interrupt) = self.pending_interrupts.pop_first() else {
            return Ok(());
        };
        let zrupt = pc.wrapping_add(1) & 0o7777;
        self.memory
            .set_central_register(register::ZRUPT, register_from_raw(zrupt))?;
        self.memory
            .set_central_register(register::BRUPT, interrupted.sign_extend())?;
        self.set_z(interrupt.vector());
        self.interrupt_enabled = false;
        self.in_interrupt = true;
        self.indexed = None;
        self.substitute_instruction = false;
        self.extended = false;
        events.push(InterruptEvent::Entered {
            number: interrupt as u8,
            vector: interrupt.vector(),
        });
        Ok(())
    }

    fn tick_timers(&mut self, cycles: u8, event: &mut TraceEvent) -> Result<(), CpuError> {
        self.downrupt_mcts += u32::from(cycles);
        while self.downrupt_mcts >= 1706 {
            self.downrupt_mcts -= 1706;
            self.pending_interrupts.insert(Interrupt::Downrupt);
            event.interrupts.push(InterruptEvent::Requested {
                number: Interrupt::Downrupt as u8,
            });
        }

        self.scaler_mcts += u16::from(cycles) * 3;
        while self.scaler_mcts >= 80 {
            self.scaler_mcts -= 80;
            self.scaler = self.scaler.wrapping_add(1);
            match self.scaler & 0o37 {
                0 => self.increment_timer(0o30, Interrupt::Time5, event)?,
                8 => self.increment_timer(0o27, Interrupt::Time4, event)?,
                16 => {
                    self.increment_time1(event)?;
                    self.increment_timer(0o26, Interrupt::Time3, event)?;
                }
                _ => {}
            }
            let channel13 = self
                .memory
                .read_channel(ChannelAddress::new(0o13).expect("channel 13 is valid"));
            if self.scaler & 1 == 0 && channel13.raw() & 0o40000 != 0 {
                let time6 = self.memory.read(0o31, AccessKind::Read)?.value;
                if time6.is_zero() {
                    self.pending_interrupts.insert(Interrupt::Time6);
                    event.interrupts.push(InterruptEvent::Requested {
                        number: Interrupt::Time6 as u8,
                    });
                    self.memory.write_channel(
                        ChannelAddress::new(0o13).expect("channel 13 is valid"),
                        AgcWord::from_raw_truncate(channel13.raw() & !0o40000),
                    );
                } else {
                    let next = if time6.is_negative() {
                        AgcWord::from_raw_truncate(time6.raw() + 1)
                    } else {
                        AgcWord::from_raw_truncate(time6.raw() - 1)
                    };
                    let access = self.memory.write(0o31, next)?;
                    event.memory.push(memory_event(access));
                }
            }
        }
        Ok(())
    }

    fn increment_time1(&mut self, event: &mut TraceEvent) -> Result<(), CpuError> {
        let time1 = self.memory.read(0o25, AccessKind::Read)?.value;
        let incremented = AgcWord::from_raw_truncate((time1.raw() + 1) & 0o37777);
        let access = self.memory.write(0o25, incremented)?;
        event.memory.push(memory_event(access));
        if incremented.is_positive_zero() {
            let time2 = self.memory.read(0o24, AccessKind::Read)?.value;
            let access = self.memory.write(
                0o24,
                AgcWord::from_raw_truncate((time2.raw() + 1) & 0o37777),
            )?;
            event.memory.push(memory_event(access));
        }
        Ok(())
    }

    fn increment_timer(
        &mut self,
        address: u16,
        interrupt: Interrupt,
        event: &mut TraceEvent,
    ) -> Result<(), CpuError> {
        let current = self.memory.read(address, AccessKind::Read)?.value;
        let mut next = AgcWord::from_raw_truncate(current.raw().wrapping_add(1));
        if next.raw() == 0o40000 {
            next = AgcWord::POSITIVE_ZERO;
            self.pending_interrupts.insert(interrupt);
            event.interrupts.push(InterruptEvent::Requested {
                number: interrupt as u8,
            });
        }
        let access = self.memory.write(address, next)?;
        event.memory.push(memory_event(access));
        Ok(())
    }

    fn read_word(&mut self, address: u16, event: &mut TraceEvent) -> Result<AgcWord, CpuError> {
        let access = self.memory.read(address, AccessKind::Read)?;
        let value = access.value;
        event.memory.push(memory_event(access));
        Ok(value)
    }

    fn read_register_operand(
        &mut self,
        address: u16,
        event: &mut TraceEvent,
    ) -> Result<AgcRegister, CpuError> {
        let access = self.memory.read(address, AccessKind::Read)?;
        let value = if address < 0o20 {
            self.memory.central_register(address)?
        } else {
            access.value.sign_extend()
        };
        event.memory.push(memory_event(access));
        Ok(value)
    }

    fn write_word(
        &mut self,
        address: u16,
        value: AgcWord,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        let access = self.memory.write(address, value)?;
        event.memory.push(memory_event(access));
        if self.watchpoints.contains(&address) {
            *watchpoint = Some(address);
        }
        Ok(())
    }

    fn write_register_operand(
        &mut self,
        address: u16,
        value: AgcRegister,
        event: &mut TraceEvent,
        watchpoint: &mut Option<u16>,
    ) -> Result<(), CpuError> {
        if address < 0o20 {
            self.memory.set_central_register(address, value)?;
            let mut access = self.memory.read(address, AccessKind::Read)?;
            access.kind = AccessKind::Write;
            event.memory.push(memory_event(access));
        } else {
            self.write_word(address, value.overflow_correct(), event, watchpoint)?;
            return Ok(());
        }
        if self.watchpoints.contains(&address) {
            *watchpoint = Some(address);
        }
        Ok(())
    }

    fn read_channel(&mut self, channel: u16, event: &mut TraceEvent) -> Result<AgcWord, CpuError> {
        let channel_address =
            ChannelAddress::new(channel).map_err(|_| MemoryError::AddressOutOfRange(channel))?;
        let value = self.memory.read_channel(channel_address);
        event.io.push(IoEvent {
            write: false,
            channel,
            value,
        });
        Ok(value)
    }

    fn write_channel(
        &mut self,
        channel: u16,
        value: AgcWord,
        event: &mut TraceEvent,
    ) -> Result<(), CpuError> {
        let channel_address =
            ChannelAddress::new(channel).map_err(|_| MemoryError::AddressOutOfRange(channel))?;
        self.memory.write_channel(channel_address, value);
        event.io.push(IoEvent {
            write: true,
            channel,
            value,
        });
        Ok(())
    }

    fn snapshot(&self) -> RegisterSnapshot {
        let raw = |index| {
            self.memory
                .central_register(index)
                .expect("architectural register exists")
                .raw()
        };
        RegisterSnapshot {
            a: raw(register::A),
            l: raw(register::L),
            q: raw(register::Q),
            z: raw(register::Z),
            eb: raw(register::EB),
            fb: raw(register::FB),
            bb: raw(register::BB),
        }
    }

    fn a_register(&self) -> AgcRegister {
        self.memory
            .central_register(register::A)
            .expect("A register exists")
    }

    fn a_raw(&self) -> u16 {
        self.a_register().raw()
    }

    fn a_word(&self) -> AgcWord {
        self.a_register().overflow_correct()
    }

    fn l_register(&self) -> AgcRegister {
        self.memory
            .central_register(register::L)
            .expect("L register exists")
    }

    fn l_word(&self) -> AgcWord {
        self.memory
            .central_register(register::L)
            .expect("L register exists")
            .overflow_correct()
    }

    fn set_a_raw(&mut self, raw: u16) {
        self.set_a_register(register_from_raw(raw));
    }

    fn set_a_register(&mut self, value: AgcRegister) {
        self.set_central_register(register::A, value);
    }

    fn set_a_word(&mut self, word: AgcWord) {
        self.memory
            .set_central_register(register::A, word.sign_extend())
            .expect("A register exists");
    }

    fn set_l_word(&mut self, word: AgcWord) {
        self.set_l_register(word.sign_extend());
    }

    fn set_l_register(&mut self, value: AgcRegister) {
        self.set_central_register(register::L, value);
    }

    fn set_central_register(&mut self, index: u16, value: AgcRegister) {
        self.memory
            .set_central_register(index, value)
            .expect("central register exists");
    }

    fn set_q_raw(&mut self, raw: u16) {
        self.memory
            .set_central_register(
                register::Q,
                AgcRegister::try_from_raw(u32::from(raw)).expect("u16 is valid register"),
            )
            .expect("Q register exists");
    }

    fn set_z(&mut self, address: u16) {
        self.memory
            .set_central_register(
                register::Z,
                AgcRegister::try_from_raw(u32::from(address & 0o7777))
                    .expect("12-bit Z is a register"),
            )
            .expect("Z register exists");
    }

    fn skip(&mut self, words: u16) {
        self.set_z(self.program_counter().wrapping_add(words) & 0o7777);
    }

    fn add_register_to_a(&mut self, value: AgcRegister) {
        self.set_a_register(self.a_register().wrapping_add(value));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChannelLogic {
    AndRead,
    OrRead,
    XorRead,
    AndWrite,
    OrWrite,
}

fn memory_event(access: MemoryAccess) -> MemoryEvent {
    MemoryEvent {
        kind: match access.kind {
            AccessKind::Fetch => "fetch",
            AccessKind::Read => "read",
            AccessKind::Write => "write",
        }
        .to_owned(),
        logical: access.logical,
        physical: match access.physical {
            PhysicalAddress::Register { index } => format!("R:{index:02o}"),
            PhysicalAddress::Erasable { bank, offset } => format!("E{bank:o}:{offset:04o}"),
            PhysicalAddress::Fixed { bank, offset } => format!("F{bank:02o}:{offset:04o}"),
        },
        value: access.value,
    }
}

fn register_from_raw(raw: u16) -> AgcRegister {
    AgcRegister::try_from_raw(u32::from(raw)).expect("u16 is a valid AGC register")
}

fn add_double_precision_carry(high: AgcRegister, low: AgcRegister) -> AgcRegister {
    match low.raw() & 0o140000 {
        0o040000 => high.wrapping_add(AgcWord::from_raw_truncate(1).sign_extend()),
        0o100000 => high.wrapping_add(AgcWord::from_raw_truncate(0o77776).sign_extend()),
        _ => high,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_isa::encode;

    fn cpu_with_program(program: &[(Mnemonic, u16)]) -> Cpu {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; agc_memory::FIXED_BANKS * 1024];
        for (offset, &(mnemonic, operand)) in program.iter().enumerate() {
            rope[2 * 1024 + offset] = encode(mnemonic, operand).unwrap();
        }
        Cpu::new(Memory::with_rope(rope).unwrap())
    }

    #[test]
    fn transfer_add_store_and_trace_are_real_execution() {
        let mut cpu = cpu_with_program(&[
            (Mnemonic::Ca, 0o100),
            (Mnemonic::Ad, 0o101),
            (Mnemonic::Ts, 0o102),
            (Mnemonic::Tcf, 0o4003),
        ]);
        cpu.memory_mut()
            .write(0o100, AgcWord::from_i32(2).unwrap())
            .unwrap();
        cpu.memory_mut()
            .write(0o101, AgcWord::from_i32(3).unwrap())
            .unwrap();
        cpu.run(3).unwrap();
        assert_eq!(
            cpu.memory().read(0o102, AccessKind::Read).unwrap().value,
            AgcWord::from_i32(5).unwrap()
        );
        assert_eq!(cpu.trace().events.len(), 3);
        assert!(
            cpu.trace().events[2]
                .memory
                .iter()
                .any(|access| access.kind == "write" && access.logical == 0o102)
        );
    }

    #[test]
    fn interrupt_entry_and_resume_restore_context() {
        let mut cpu = cpu_with_program(&[(Mnemonic::Relint, 0), (Mnemonic::Tcf, 0o4001)]);
        cpu.step().unwrap();
        cpu.request_interrupt(Interrupt::Time6);
        let outcome = cpu.step().unwrap();
        assert_eq!(outcome.trace.pc, Interrupt::Time6.vector());
        assert!(cpu.in_interrupt());
    }

    #[test]
    fn decoded_double_load_address_is_preincremented() {
        let mut cpu = cpu_with_program(&[(Mnemonic::Extend, 0), (Mnemonic::Dca, 0o100)]);
        cpu.memory_mut()
            .write(0o77, AgcWord::from_i32(12).unwrap())
            .unwrap();
        cpu.memory_mut()
            .write(0o100, AgcWord::from_i32(-7).unwrap())
            .unwrap();
        cpu.run(2).unwrap();
        assert_eq!(cpu.a_word(), AgcWord::from_i32(12).unwrap());
        assert_eq!(cpu.l_word(), AgcWord::from_i32(-7).unwrap());
    }

    #[test]
    fn decoded_double_exchange_address_is_preincremented() {
        let mut cpu = cpu_with_program(&[(Mnemonic::Dxch, 0o100)]);
        cpu.set_a_word(AgcWord::from_i32(3).unwrap());
        cpu.set_l_word(AgcWord::from_i32(4).unwrap());
        cpu.memory_mut()
            .write(0o77, AgcWord::from_i32(8).unwrap())
            .unwrap();
        cpu.memory_mut()
            .write(0o100, AgcWord::from_i32(9).unwrap())
            .unwrap();
        cpu.step().unwrap();
        assert_eq!(cpu.a_word(), AgcWord::from_i32(8).unwrap());
        assert_eq!(cpu.l_word(), AgcWord::from_i32(9).unwrap());
        assert_eq!(
            cpu.memory().read(0o77, AccessKind::Read).unwrap().value,
            AgcWord::from_i32(3).unwrap()
        );
        assert_eq!(
            cpu.memory().read(0o100, AccessKind::Read).unwrap().value,
            AgcWord::from_i32(4).unwrap()
        );
    }

    #[test]
    fn return_preserves_q_for_indirect_transfer() {
        let mut cpu = cpu_with_program(&[(Mnemonic::Tc, register::Q)]);
        cpu.set_q_raw(0o4321);
        cpu.step().unwrap();
        assert_eq!(cpu.program_counter(), register::Q);
        assert_eq!(cpu.central_register(register::Q).unwrap().raw(), 0o4321);
        cpu.step().unwrap();
        assert_eq!(cpu.program_counter(), 0o4321);
    }

    #[test]
    fn resume_substitutes_brupt_instead_of_indexing_memory() {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; agc_memory::FIXED_BANKS * 1024];
        rope[2 * 1024] = encode(Mnemonic::Relint, 0).unwrap();
        rope[2 * 1024 + 1] = encode(Mnemonic::Tcf, 0o4001).unwrap();
        rope[2 * 1024 + 4] = encode(Mnemonic::Resume, 0).unwrap();
        let mut cpu = Cpu::new(Memory::with_rope(rope).unwrap());
        cpu.step().unwrap();
        cpu.request_interrupt(Interrupt::Time6);
        cpu.memory_mut()
            .set_central_register(register::Q, register_from_raw(0o4321))
            .unwrap();
        cpu.memory_mut()
            .set_central_register(register::QRUPT, register_from_raw(0o1234))
            .unwrap();
        cpu.step().unwrap();
        assert_eq!(cpu.central_register(register::Q).unwrap().raw(), 0o4321);
        let resumed = cpu.step().unwrap();
        assert_eq!(resumed.trace.pc, 0o4001);
        assert_eq!(resumed.trace.memory[0].logical, register::BRUPT);
        assert_eq!(resumed.instruction.mnemonic, Mnemonic::Tcf);
        assert_eq!(cpu.program_counter(), 0o4001);
        assert_eq!(cpu.central_register(register::Q).unwrap().raw(), 0o4321);
    }

    #[test]
    fn hardware_interrupt_only_captures_zrupt_and_brupt() {
        let mut cpu = cpu_with_program(&[(Mnemonic::Relint, 0), (Mnemonic::Tcf, 0o4001)]);
        cpu.memory_mut()
            .set_central_register(register::ARUPT, register_from_raw(0o7654))
            .unwrap();
        cpu.step().unwrap();
        cpu.request_interrupt(Interrupt::Time6);
        cpu.step().unwrap();
        assert_eq!(cpu.central_register(register::ARUPT).unwrap().raw(), 0o7654);
        assert_eq!(cpu.central_register(register::ZRUPT).unwrap().raw(), 0o4002);
        assert_eq!(
            cpu.central_register(register::BRUPT)
                .unwrap()
                .overflow_correct(),
            encode(Mnemonic::Tcf, 0o4001).unwrap()
        );
    }

    #[test]
    fn interrupt_after_resume_captures_substitute_then_fetches_vector() {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; agc_memory::FIXED_BANKS * 1024];
        rope[2 * 1024] = encode(Mnemonic::Relint, 0).unwrap();
        rope[2 * 1024 + 1] = encode(Mnemonic::Tcf, 0o4001).unwrap();
        rope[2 * 1024 + 4] = encode(Mnemonic::Relint, 0).unwrap();
        rope[2 * 1024 + 5] = encode(Mnemonic::Resume, 0).unwrap();
        rope[2 * 1024 + 0o40] = encode(Mnemonic::Tcf, 0o4321).unwrap();
        let mut cpu = Cpu::new(Memory::with_rope(rope).unwrap());
        cpu.step().unwrap();
        cpu.request_interrupt(Interrupt::Time6);
        cpu.request_interrupt(Interrupt::Downrupt);
        cpu.step().unwrap();
        cpu.step().unwrap();
        let nested = cpu.step().unwrap();
        assert_eq!(nested.trace.pc, Interrupt::Downrupt.vector());
        assert_eq!(nested.trace.memory[0].logical, Interrupt::Downrupt.vector());
        assert_eq!(nested.instruction.mnemonic, Mnemonic::Tcf);
        assert_eq!(nested.instruction.operand, 0o4321);
        assert_eq!(
            cpu.central_register(register::BRUPT)
                .unwrap()
                .overflow_correct(),
            encode(Mnemonic::Tcf, 0o4001).unwrap()
        );
    }
}
