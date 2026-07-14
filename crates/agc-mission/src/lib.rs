#![forbid(unsafe_code)]
//! Real-execution mission scenarios, synchronized DSKY state, and visualization frames.

use agc_cpu::{Cpu, CpuError};
use agc_dsky::{DskyState, Key};
use agc_faults::{Fault, FaultEngine, FaultError};
use agc_interpreter::State as InterpreterState;
use agc_loader::RopeImage;
use agc_runtime::{
    ExecutiveJob, FlightSoftwareObserver, HardwareState, Runtime, RuntimeError, RuntimeEvent,
    WaitlistTask,
};
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

/// Named erasable guidance value exposed in visualization frames.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuidanceVariable {
    /// Historical symbol or research name.
    pub name: String,
    /// Logical erasable address.
    pub address: u16,
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
        let inputs = keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| MissionInput {
                instruction: 25_000 + index as u64 * 20_000,
                event: RuntimeEvent::DskyKey {
                    code: key.code().expect("built-in DSKY key is valid"),
                },
            })
            .collect();
        Self {
            name: "luminary099-p63-request".to_owned(),
            instruction_limit: 1_000_000,
            frame_interval: 5_000,
            inputs,
            guidance_variables: Vec::new(),
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
        Ok(())
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
        self.runtime.set_observer(scenario.observer.clone());
        let mut input_index = 0;
        let mut frames = Vec::new();
        while self.runtime.cpu().instructions() < scenario.instruction_limit {
            let instruction = self.runtime.cpu().instructions();
            while scenario
                .inputs
                .get(input_index)
                .is_some_and(|input| input.instruction <= instruction)
            {
                let input = &scenario.inputs[input_index];
                self.runtime
                    .schedule(self.runtime.cpu().cycles(), input.event.clone());
                input_index += 1;
            }
            let outcome = self.faults.step(&mut self.runtime)?;
            self.dsky.consume_trace(&outcome.trace);
            if self.runtime.cpu().instructions() % scenario.frame_interval == 0 {
                frames.push(self.frame(scenario)?);
            }
        }
        if frames
            .last()
            .is_none_or(|frame| frame.instruction != self.runtime.cpu().instructions())
        {
            frames.push(self.frame(scenario)?);
        }
        Ok(MissionRun {
            scenario: scenario.name.clone(),
            frames,
            final_dsky: self.dsky.clone(),
            instructions: self.runtime.cpu().instructions(),
            cycles: self.runtime.cpu().cycles(),
            faults_applied: self.faults.applied.len(),
        })
    }

    fn frame(&self, scenario: &MissionScenario) -> Result<MissionFrame, MissionError> {
        let register = |index| {
            self.runtime
                .cpu()
                .central_register(index)
                .map(|value| value.raw())
        };
        let mut guidance = BTreeMap::new();
        for variable in &scenario.guidance_variables {
            let value = self
                .runtime
                .cpu()
                .memory()
                .read(variable.address, agc_memory::AccessKind::Read)
                .map_err(RuntimeError::Memory)?
                .value;
            guidance.insert(variable.name.clone(), value);
        }
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
    use agc_memory::{FIXED_BANKS, FIXED_WORDS_PER_BANK, Memory};

    #[test]
    fn mission_frames_are_sampled_from_executed_cpu_state() {
        let mut rope = vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK];
        rope[2 * 1024] = encode(Mnemonic::Tcf, 0o4000).unwrap();
        let runtime = Runtime::new(Cpu::new(Memory::with_rope(rope).unwrap()));
        let mut controller = MissionController::from_runtime(runtime);
        let scenario = MissionScenario {
            name: "loop".to_owned(),
            instruction_limit: 5,
            frame_interval: 2,
            inputs: Vec::new(),
            guidance_variables: Vec::new(),
            observer: FlightSoftwareObserver::default(),
        };
        let run = controller.run(&scenario).unwrap();
        assert_eq!(run.instructions, 5);
        assert_eq!(run.frames.last().unwrap().pc, 0o4000);
        assert_eq!(controller.runtime().trace().events.len(), 5);
    }
}
