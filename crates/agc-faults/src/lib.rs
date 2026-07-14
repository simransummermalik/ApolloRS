#![forbid(unsafe_code)]
//! Deterministic register, memory, rope, channel, sensor, and interrupt fault injection.

use agc_cpu::{Interrupt, StepOutcome};
use agc_memory::{AccessKind, MemoryError};
use agc_runtime::{Runtime, RuntimeError};
use agc_word::{AgcRegister, AgcWord, ChannelAddress};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Fault applied at a defined instruction boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Fault {
    /// Flip bits in logical erasable memory.
    ErasableBitFlip {
        /// Logical address.
        address: u16,
        /// Fifteen-bit XOR mask.
        mask: u16,
    },
    /// Flip bits in physical fixed rope.
    RopeBitFlip {
        /// Physical bank.
        bank: u8,
        /// Bank offset.
        offset: u16,
        /// Fifteen-bit XOR mask.
        mask: u16,
    },
    /// Corrupt one full-width central register.
    Register {
        /// Register index.
        index: u16,
        /// Full 16-bit XOR mask.
        mask: u16,
    },
    /// Force a channel to a value before a number of instructions.
    StuckChannel {
        /// Channel number.
        channel: u16,
        /// Forced value.
        value: AgcWord,
        /// Number of instruction boundaries for which the fault remains active.
        instructions: u64,
    },
    /// Cancel one pending interrupt.
    DropInterrupt {
        /// Interrupt identity.
        interrupt: Interrupt,
    },
    /// Change one timer word by an exact one's-complement delta.
    TimerJump {
        /// Timer logical address 024..031.
        address: u16,
        /// Signed increment.
        delta: i32,
    },
    /// Inject IMU pulses through the physical runtime interface.
    ImuBias {
        /// X pulses.
        x: i16,
        /// Y pulses.
        y: i16,
        /// Z pulses.
        z: i16,
    },
    /// Replace radar input and request RADAR interrupt.
    RadarSample {
        /// Range.
        range: AgcWord,
        /// Range rate.
        rate: AgcWord,
    },
}

/// Fault scheduled by committed-instruction count.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledFault {
    /// CPU instruction count at which to apply it, before execution.
    pub instruction: u64,
    /// Stable same-boundary sequence.
    pub sequence: u64,
    /// Fault payload.
    pub fault: Fault,
}

/// Evidence that a fault was applied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppliedFault {
    /// Actual instruction boundary.
    pub instruction: u64,
    /// Actual cycle boundary.
    pub cycle: u64,
    /// Fault payload.
    pub fault: Fault,
    /// Raw result when the target produces one.
    pub resulting_word: Option<u16>,
}

/// Fault execution error.
#[derive(Debug, Error)]
pub enum FaultError {
    /// Runtime failure.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Memory failure.
    #[error(transparent)]
    Memory(#[from] MemoryError),
    /// Invalid timer address.
    #[error("timer fault address {0:#o} is not TIME1..TIME6")]
    TimerAddress(u16),
    /// Invalid channel.
    #[error("fault channel {0:#o} is outside nine bits")]
    Channel(u16),
    /// Delta cannot be represented in one word.
    #[error("fault delta {0} is outside one AGC word")]
    Delta(i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveChannelFault {
    channel: u16,
    value: AgcWord,
    remaining: u64,
}

/// Deterministic fault campaign around a real runtime.
#[derive(Clone, Debug, Default)]
pub struct FaultEngine {
    scheduled: BTreeMap<(u64, u64), Fault>,
    active_channels: Vec<ActiveChannelFault>,
    next_sequence: u64,
    /// Applied-fault audit trail.
    pub applied: Vec<AppliedFault>,
}

impl FaultEngine {
    /// Schedules a fault and returns its stable record.
    pub fn schedule(&mut self, instruction: u64, fault: Fault) -> ScheduledFault {
        let record = ScheduledFault {
            instruction,
            sequence: self.next_sequence,
            fault: fault.clone(),
        };
        self.scheduled
            .insert((instruction, self.next_sequence), fault);
        self.next_sequence += 1;
        record
    }

    /// Runs one fault-aware instruction boundary.
    pub fn step(&mut self, runtime: &mut Runtime) -> Result<StepOutcome, FaultError> {
        self.apply_due(runtime)?;
        self.enforce_channels(runtime)?;
        let outcome = runtime.step()?;
        self.active_channels.retain_mut(|fault| {
            fault.remaining = fault.remaining.saturating_sub(1);
            fault.remaining != 0
        });
        Ok(outcome)
    }

    /// Runs an instruction budget while preserving every applied-fault record.
    pub fn run(&mut self, runtime: &mut Runtime, instructions: u64) -> Result<(), FaultError> {
        for _ in 0..instructions {
            self.step(runtime)?;
        }
        Ok(())
    }

    fn apply_due(&mut self, runtime: &mut Runtime) -> Result<(), FaultError> {
        let instruction = runtime.cpu().instructions();
        let keys = self
            .scheduled
            .range(..=(instruction, u64::MAX))
            .map(|(&key, _)| key)
            .collect::<Vec<_>>();
        for key in keys {
            let fault = self
                .scheduled
                .remove(&key)
                .expect("collected fault key exists");
            self.apply(runtime, fault)?;
        }
        Ok(())
    }

    fn apply(&mut self, runtime: &mut Runtime, fault: Fault) -> Result<(), FaultError> {
        let resulting_word = match &fault {
            Fault::ErasableBitFlip { address, mask } => {
                let current = runtime
                    .cpu()
                    .memory()
                    .read(*address, AccessKind::Read)?
                    .value;
                let next = AgcWord::from_raw_truncate(current.raw() ^ mask);
                Some(
                    runtime
                        .cpu_mut()
                        .memory_mut()
                        .write(*address, next)?
                        .value
                        .raw(),
                )
            }
            Fault::RopeBitFlip { bank, offset, mask } => Some(
                runtime
                    .cpu_mut()
                    .memory_mut()
                    .inject_fixed_bit_flip(*bank, *offset, *mask)?
                    .raw(),
            ),
            Fault::Register { index, mask } => {
                let current = runtime
                    .cpu()
                    .central_register(*index)
                    .map_err(RuntimeError::Cpu)?;
                let next = AgcRegister::try_from_raw(u32::from(current.raw() ^ mask))
                    .expect("XOR of two u16 values is 16-bit");
                runtime
                    .cpu_mut()
                    .memory_mut()
                    .set_central_register(*index, next)?;
                Some(next.raw())
            }
            Fault::StuckChannel {
                channel,
                value,
                instructions,
            } => {
                ChannelAddress::new(*channel).map_err(|_| FaultError::Channel(*channel))?;
                self.active_channels.push(ActiveChannelFault {
                    channel: *channel,
                    value: *value,
                    remaining: *instructions,
                });
                Some(value.raw())
            }
            Fault::DropInterrupt { interrupt } => {
                Some(u16::from(runtime.cpu_mut().cancel_interrupt(*interrupt)))
            }
            Fault::TimerJump { address, delta } => {
                if !(0o24..=0o31).contains(address) {
                    return Err(FaultError::TimerAddress(*address));
                }
                let delta = AgcWord::from_i32(*delta).map_err(|_| FaultError::Delta(*delta))?;
                let current = runtime
                    .cpu()
                    .memory()
                    .read(*address, AccessKind::Read)?
                    .value;
                let next = current.wrapping_add(delta);
                Some(
                    runtime
                        .cpu_mut()
                        .memory_mut()
                        .write(*address, next)?
                        .value
                        .raw(),
                )
            }
            Fault::ImuBias { x, y, z } => {
                runtime.schedule(
                    runtime.cpu().cycles(),
                    agc_runtime::RuntimeEvent::ImuPulses {
                        x: *x,
                        y: *y,
                        z: *z,
                    },
                );
                None
            }
            Fault::RadarSample { range, rate } => {
                runtime.schedule(
                    runtime.cpu().cycles(),
                    agc_runtime::RuntimeEvent::Radar {
                        range: *range,
                        rate: *rate,
                    },
                );
                None
            }
        };
        self.applied.push(AppliedFault {
            instruction: runtime.cpu().instructions(),
            cycle: runtime.cpu().cycles(),
            fault,
            resulting_word,
        });
        Ok(())
    }

    fn enforce_channels(&self, runtime: &mut Runtime) -> Result<(), FaultError> {
        for fault in &self.active_channels {
            let channel = ChannelAddress::new(fault.channel)
                .map_err(|_| FaultError::Channel(fault.channel))?;
            runtime
                .cpu_mut()
                .memory_mut()
                .write_channel(channel, fault.value);
        }
        Ok(())
    }
}

/// Summary comparing a baseline and faulted campaign.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryComparison {
    /// First differing trace event, absent if traces match.
    pub first_divergence: Option<usize>,
    /// Baseline committed instructions.
    pub baseline_instructions: u64,
    /// Faulted committed instructions.
    pub faulted_instructions: u64,
    /// Baseline final program counter.
    pub baseline_pc: u16,
    /// Faulted final program counter.
    pub faulted_pc: u16,
    /// Whether final architectural registers match.
    pub registers_recovered: bool,
}

/// Compares completed baseline and faulted runtimes without rerunning either.
pub fn compare_recovery(baseline: &Runtime, faulted: &Runtime) -> RecoveryComparison {
    let first_divergence = baseline
        .trace()
        .first_difference(faulted.trace())
        .map(|difference| difference.index);
    let baseline_last = baseline.trace().events.last().map(|event| event.after);
    let faulted_last = faulted.trace().events.last().map(|event| event.after);
    RecoveryComparison {
        first_divergence,
        baseline_instructions: baseline.cpu().instructions(),
        faulted_instructions: faulted.cpu().instructions(),
        baseline_pc: baseline.cpu().program_counter(),
        faulted_pc: faulted.cpu().program_counter(),
        registers_recovered: baseline_last == faulted_last,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agc_cpu::Cpu;
    use agc_isa::{Mnemonic, encode};
    use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory};

    fn looping_runtime() -> Runtime {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        rope[2 * 1024] = encode(Mnemonic::Tcf, 0o4000).unwrap();
        Runtime::new(Cpu::new(Memory::with_rope(rope).unwrap()))
    }

    #[test]
    fn scheduled_bit_flip_changes_real_erasable_state() {
        let mut runtime = looping_runtime();
        let mut faults = FaultEngine::default();
        faults.schedule(
            0,
            Fault::ErasableBitFlip {
                address: 0o100,
                mask: 0o4,
            },
        );
        faults.step(&mut runtime).unwrap();
        assert_eq!(
            runtime
                .cpu()
                .memory()
                .read(0o100, AccessKind::Read)
                .unwrap()
                .value
                .raw(),
            4
        );
        assert_eq!(faults.applied.len(), 1);
    }
}
