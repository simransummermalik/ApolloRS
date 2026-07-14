#![forbid(unsafe_code)]
//! Deterministic hardware/runtime orchestration around the instruction-accurate CPU.

use agc_cpu::{Cpu, CpuError, Interrupt, RunOutcome, StepOutcome};
use agc_memory::{AccessKind, MemoryError};
use agc_trace::TraceLog;
use agc_word::{AgcWord, ChannelAddress};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Host-side event delivered at a deterministic machine cycle boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RuntimeEvent {
    /// Set an input/output channel before the next instruction.
    Channel {
        /// Nine-bit channel.
        channel: u16,
        /// Fifteen-bit value.
        value: AgcWord,
    },
    /// Request an architectural interrupt.
    Interrupt {
        /// Interrupt identity.
        interrupt: Interrupt,
    },
    /// Write physical erasable memory for scenario initialization or sensor DMA.
    Erasable {
        /// Bank.
        bank: u8,
        /// Offset.
        offset: u16,
        /// Value.
        value: AgcWord,
    },
    /// Apply IMU pulse increments in the three axes.
    ImuPulses {
        /// X-axis pulses.
        x: i16,
        /// Y-axis pulses.
        y: i16,
        /// Z-axis pulses.
        z: i16,
    },
    /// Supply radar range and range-rate words and request RADAR interrupt.
    Radar {
        /// Range measurement.
        range: AgcWord,
        /// Range-rate measurement.
        rate: AgcWord,
    },
    /// Inject a DSKY key code and request KEYRUPT1.
    DskyKey {
        /// Five-bit key code.
        code: u8,
    },
}

/// Event with a stable sequence for same-cycle ordering.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    /// Earliest machine cycle at which the event is applied.
    pub cycle: u64,
    /// Monotonic insertion sequence.
    pub sequence: u64,
    /// Event payload.
    pub event: RuntimeEvent,
}

/// Current inertial measurement hardware state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImuState {
    /// Accumulated X pulses.
    pub x: i32,
    /// Accumulated Y pulses.
    pub y: i32,
    /// Accumulated Z pulses.
    pub z: i32,
}

/// Current radar input state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadarState {
    /// Range word.
    pub range: AgcWord,
    /// Range-rate word.
    pub rate: AgcWord,
    /// Number of samples supplied.
    pub samples: u64,
}

impl Default for RadarState {
    fn default() -> Self {
        Self {
            range: AgcWord::POSITIVE_ZERO,
            rate: AgcWord::POSITIVE_ZERO,
            samples: 0,
        }
    }
}

/// Captured two-word downlink frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DownlinkFrame {
    /// Cycle of the completing write.
    pub cycle: u64,
    /// Channel 034 word.
    pub word1: AgcWord,
    /// Channel 035 word.
    pub word2: AgcWord,
}

/// Deterministic peripheral state independent of flight-program logic.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HardwareState {
    /// IMU pulse state.
    pub imu: ImuState,
    /// Radar state.
    pub radar: RadarState,
    /// Completed telemetry frames.
    pub downlink: Vec<DownlinkFrame>,
    pending_downlink_1: Option<AgcWord>,
    pending_downlink_2: Option<AgcWord>,
}

/// One configured Executive core-set slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutiveSlot {
    /// Slot index.
    pub slot: u8,
    /// Erasable logical address containing the job address.
    pub address_word: u16,
    /// Erasable logical address containing priority/state.
    pub priority_word: u16,
}

/// Observed Executive job state read from real erasable memory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutiveJob {
    /// Slot index.
    pub slot: u8,
    /// Job entry address.
    pub entry: u16,
    /// Priority/state word.
    pub priority: AgcWord,
}

/// One configured Waitlist task pair.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WaitlistSlot {
    /// Slot index.
    pub slot: u8,
    /// Address of delay word.
    pub delay_word: u16,
    /// Address of task entry word.
    pub task_word: u16,
}

/// Observed Waitlist task state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WaitlistTask {
    /// Slot index.
    pub slot: u8,
    /// Remaining delay word.
    pub delay: AgcWord,
    /// Task entry address.
    pub entry: u16,
}

/// Flight-software data-structure observer. It never schedules in place of the
/// historical Executive or Waitlist; it reads their real erasable records.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FlightSoftwareObserver {
    /// Configured Executive slots.
    pub executive_slots: Vec<ExecutiveSlot>,
    /// Configured Waitlist slots.
    pub waitlist_slots: Vec<WaitlistSlot>,
}

impl FlightSoftwareObserver {
    /// Reads currently active Executive jobs.
    pub fn executive_jobs(&self, cpu: &Cpu) -> Result<Vec<ExecutiveJob>, RuntimeError> {
        self.executive_slots
            .iter()
            .filter_map(|slot| {
                let entry = cpu
                    .memory()
                    .read(slot.address_word, AccessKind::Read)
                    .map(|access| access.value);
                let priority = cpu
                    .memory()
                    .read(slot.priority_word, AccessKind::Read)
                    .map(|access| access.value);
                match (entry, priority) {
                    (Ok(entry), Ok(priority)) if !entry.is_zero() => Some(Ok(ExecutiveJob {
                        slot: slot.slot,
                        entry: entry.raw() & 0o7777,
                        priority,
                    })),
                    (Ok(_), Ok(_)) => None,
                    (Err(error), _) | (_, Err(error)) => Some(Err(RuntimeError::Memory(error))),
                }
            })
            .collect()
    }

    /// Reads currently active Waitlist tasks.
    pub fn waitlist_tasks(&self, cpu: &Cpu) -> Result<Vec<WaitlistTask>, RuntimeError> {
        self.waitlist_slots
            .iter()
            .filter_map(|slot| {
                let delay = cpu
                    .memory()
                    .read(slot.delay_word, AccessKind::Read)
                    .map(|access| access.value);
                let entry = cpu
                    .memory()
                    .read(slot.task_word, AccessKind::Read)
                    .map(|access| access.value);
                match (delay, entry) {
                    (Ok(delay), Ok(entry)) if !entry.is_zero() => Some(Ok(WaitlistTask {
                        slot: slot.slot,
                        delay,
                        entry: entry.raw() & 0o7777,
                    })),
                    (Ok(_), Ok(_)) => None,
                    (Err(error), _) | (_, Err(error)) => Some(Err(RuntimeError::Memory(error))),
                }
            })
            .collect()
    }
}

/// Runtime orchestration failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// CPU failure.
    #[error(transparent)]
    Cpu(#[from] CpuError),
    /// Memory failure.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// Invalid channel in an external event.
    #[error("runtime event channel {0:#o} is outside nine bits")]
    Channel(u16),
}

/// CPU, event queue, physical peripherals, and flight-software observations.
#[derive(Clone, Debug)]
pub struct Runtime {
    cpu: Cpu,
    events: BTreeMap<(u64, u64), RuntimeEvent>,
    next_event_sequence: u64,
    hardware: HardwareState,
    observer: FlightSoftwareObserver,
}

impl Runtime {
    /// Creates a deterministic runtime around a loaded CPU.
    pub fn new(cpu: Cpu) -> Self {
        Self {
            cpu,
            events: BTreeMap::new(),
            next_event_sequence: 0,
            hardware: HardwareState::default(),
            observer: FlightSoftwareObserver::default(),
        }
    }

    /// Returns the CPU.
    pub const fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Returns the CPU mutably for debugger setup.
    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    /// Returns physical peripheral state.
    pub const fn hardware(&self) -> &HardwareState {
        &self.hardware
    }

    /// Returns flight-software observer configuration.
    pub const fn observer(&self) -> &FlightSoftwareObserver {
        &self.observer
    }

    /// Replaces flight-software observer configuration.
    pub fn set_observer(&mut self, observer: FlightSoftwareObserver) {
        self.observer = observer;
    }

    /// Schedules an external event with deterministic same-cycle ordering.
    pub fn schedule(&mut self, cycle: u64, event: RuntimeEvent) -> ScheduledEvent {
        let scheduled = ScheduledEvent {
            cycle,
            sequence: self.next_event_sequence,
            event,
        };
        self.events.insert((cycle, self.next_event_sequence), event);
        self.next_event_sequence += 1;
        scheduled
    }

    /// Executes one CPU instruction after applying all due events.
    pub fn step(&mut self) -> Result<StepOutcome, RuntimeError> {
        self.apply_due_events()?;
        let outcome = self.cpu.step()?;
        self.capture_output(&outcome);
        Ok(outcome)
    }

    /// Executes an instruction budget through the runtime event loop.
    pub fn run(&mut self, instruction_limit: u64) -> Result<RunOutcome, RuntimeError> {
        let start = self.cpu.instructions();
        while self.cpu.instructions() - start < instruction_limit {
            self.step()?;
        }
        Ok(RunOutcome {
            reason: agc_cpu::StopReason::InstructionLimit,
            instructions: self.cpu.instructions() - start,
            cycles: self.cpu.cycles(),
        })
    }

    /// Returns the committed trace.
    pub const fn trace(&self) -> &TraceLog {
        self.cpu.trace()
    }

    /// Returns queued events in delivery order.
    pub fn queued_events(&self) -> Vec<ScheduledEvent> {
        self.events
            .iter()
            .map(|(&(cycle, sequence), event)| ScheduledEvent {
                cycle,
                sequence,
                event: *event,
            })
            .collect()
    }

    fn apply_due_events(&mut self) -> Result<(), RuntimeError> {
        let cycle = self.cpu.cycles();
        let keys = self
            .events
            .range(..=(cycle, u64::MAX))
            .map(|(&key, _)| key)
            .collect::<Vec<_>>();
        for key in keys {
            let event = self
                .events
                .remove(&key)
                .expect("collected event key exists");
            self.apply_event(event)?;
        }
        Ok(())
    }

    fn apply_event(&mut self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        match event {
            RuntimeEvent::Channel { channel, value } => {
                let channel =
                    ChannelAddress::new(channel).map_err(|_| RuntimeError::Channel(channel))?;
                self.cpu.memory_mut().write_channel(channel, value);
            }
            RuntimeEvent::Interrupt { interrupt } => self.cpu.request_interrupt(interrupt),
            RuntimeEvent::Erasable {
                bank,
                offset,
                value,
            } => self
                .cpu
                .memory_mut()
                .write_erasable_physical(bank, offset, value)?,
            RuntimeEvent::ImuPulses { x, y, z } => {
                self.hardware.imu.x += i32::from(x);
                self.hardware.imu.y += i32::from(y);
                self.hardware.imu.z += i32::from(z);
                for (address, pulses) in [(0o32, x), (0o33, y), (0o34, z)] {
                    let current = self.cpu.memory().read(address, AccessKind::Read)?.value;
                    let delta = AgcWord::from_i32(i32::from(pulses))
                        .unwrap_or_else(|_| AgcWord::from_raw_truncate(pulses as u16));
                    self.cpu
                        .memory_mut()
                        .write(address, current.wrapping_add(delta))?;
                }
            }
            RuntimeEvent::Radar { range, rate } => {
                self.hardware.radar.range = range;
                self.hardware.radar.rate = rate;
                self.hardware.radar.samples += 1;
                self.cpu.memory_mut().write(0o46, range)?;
                self.cpu.memory_mut().write(0o45, rate)?;
                self.cpu.request_interrupt(Interrupt::Radar);
            }
            RuntimeEvent::DskyKey { code } => {
                self.cpu.memory_mut().write_channel(
                    ChannelAddress::new(0o15).expect("channel 15 is valid"),
                    AgcWord::from_raw_truncate(u16::from(code & 0o37)),
                );
                self.cpu.request_interrupt(Interrupt::Key1);
            }
        }
        Ok(())
    }

    fn capture_output(&mut self, outcome: &StepOutcome) {
        for io in &outcome.trace.io {
            if !io.write {
                continue;
            }
            match io.channel {
                0o34 => self.hardware.pending_downlink_1 = Some(io.value),
                0o35 => self.hardware.pending_downlink_2 = Some(io.value),
                _ => {}
            }
            if let (Some(word1), Some(word2)) = (
                self.hardware.pending_downlink_1.take(),
                self.hardware.pending_downlink_2.take(),
            ) {
                self.hardware.downlink.push(DownlinkFrame {
                    cycle: outcome.trace.cycle_end,
                    word1,
                    word2,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_isa::{Mnemonic, encode};
    use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory};

    #[test]
    fn same_cycle_events_are_replayed_in_insertion_order() {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        rope[2 * 1024] = encode(Mnemonic::Read, 0o20).unwrap();
        let mut runtime = Runtime::new(Cpu::new(Memory::with_rope(rope).unwrap()));
        runtime.schedule(
            0,
            RuntimeEvent::Channel {
                channel: 0o20,
                value: AgcWord::from_raw_truncate(1),
            },
        );
        runtime.schedule(
            0,
            RuntimeEvent::Channel {
                channel: 0o20,
                value: AgcWord::from_raw_truncate(2),
            },
        );
        // Event application itself is the invariant under test.
        runtime.apply_due_events().unwrap();
        assert_eq!(
            runtime
                .cpu()
                .memory()
                .read_channel(ChannelAddress::new(0o20).unwrap())
                .raw(),
            2
        );
    }
}
