#![forbid(unsafe_code)]
//! Typed, deterministic model of the AGC interpretive vector/scalar machine.

use agc_word::AgcDoubleWord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Interpretive scalar/vector value in signed 28-bit scaled-integer form.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum Value {
    /// Double-precision scalar numerator.
    Scalar(i64),
    /// Three double-precision scalar numerators.
    Vector([i64; 3]),
}

impl Value {
    fn checked_scalar(self) -> Result<i64, InterpreterError> {
        if let Self::Scalar(value) = self {
            Ok(value)
        } else {
            Err(InterpreterError::Type("expected scalar".to_owned()))
        }
    }

    fn checked_vector(self) -> Result<[i64; 3], InterpreterError> {
        if let Self::Vector(value) = self {
            Ok(value)
        } else {
            Err(InterpreterError::Type("expected vector".to_owned()))
        }
    }
}

/// Typed high-level interpretive instruction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Instruction {
    /// Load named storage into the accumulator.
    Load {
        /// Storage name.
        name: String,
    },
    /// Store accumulator into named storage.
    Store {
        /// Storage name.
        name: String,
    },
    /// Load an exact literal.
    Literal {
        /// Exact literal.
        value: Value,
    },
    /// Push accumulator.
    Push,
    /// Pop into accumulator.
    Pop,
    /// Add top of stack to accumulator.
    Add,
    /// Subtract accumulator from top of stack.
    Subtract,
    /// Multiply two scalars and shift by `fraction_bits`.
    Multiply {
        /// Binary fractional scale restored after the product.
        fraction_bits: u8,
    },
    /// Divide two scalars and shift by `fraction_bits`.
    Divide {
        /// Binary fractional scale applied before division.
        fraction_bits: u8,
    },
    /// Replace scalar with its magnitude.
    Absolute,
    /// One's-complement mathematical negation.
    Negate,
    /// Add two vectors.
    VectorAdd,
    /// Subtract vectors.
    VectorSubtract,
    /// Dot product.
    Dot,
    /// Cross product.
    Cross,
    /// Scale a vector by a scalar and shift by `fraction_bits`.
    Scale {
        /// Binary fractional scale restored after each product.
        fraction_bits: u8,
    },
    /// Integer square root of a non-negative scalar.
    SquareRoot,
    /// Unconditional branch.
    Branch {
        /// Interpretive instruction index.
        target: usize,
    },
    /// Branch when accumulator is either signed-zero representation mathematically.
    BranchZero {
        /// Interpretive instruction index.
        target: usize,
    },
    /// Branch when scalar accumulator is negative.
    BranchNegative {
        /// Interpretive instruction index.
        target: usize,
    },
    /// Call a subroutine.
    Call {
        /// Interpretive instruction index.
        target: usize,
    },
    /// Return from a subroutine.
    Return,
    /// Stop interpretive execution.
    Exit,
}

/// Complete interpretive program.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// Instructions in interpretive address order.
    pub instructions: Vec<Instruction>,
}

/// Machine state exposed for differential checking.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Interpretive program counter.
    pub pc: usize,
    /// Current accumulator.
    pub accumulator: Value,
    /// Operand stack.
    pub stack: Vec<Value>,
    /// Return stack.
    pub calls: Vec<usize>,
    /// Named semantic storage.
    pub memory: BTreeMap<String, Value>,
    /// Whether EXIT has committed.
    pub halted: bool,
    /// Number of interpretive instructions committed.
    pub steps: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            pc: 0,
            accumulator: Value::Scalar(0),
            stack: Vec::new(),
            calls: Vec::new(),
            memory: BTreeMap::new(),
            halted: false,
            steps: 0,
        }
    }
}

/// One interpretive transition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// Address executed.
    pub pc: usize,
    /// Instruction executed.
    pub instruction: Instruction,
    /// Accumulator before execution.
    pub before: Value,
    /// Accumulator after execution.
    pub after: Value,
}

/// Interpretive execution failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InterpreterError {
    /// PC or branch target outside program.
    #[error("interpretive address {0} is outside the program")]
    Address(usize),
    /// Stack underflow.
    #[error("interpretive operand stack underflow")]
    StackUnderflow,
    /// Return stack underflow.
    #[error("interpretive return stack underflow")]
    ReturnUnderflow,
    /// Missing named value.
    #[error("interpretive storage {0} is undefined")]
    Undefined(String),
    /// Operand type mismatch.
    #[error("interpretive type error: {0}")]
    Type(String),
    /// Arithmetic overflow or invalid operation.
    #[error("interpretive arithmetic error: {0}")]
    Arithmetic(String),
    /// Step budget exhausted without EXIT.
    #[error("interpretive step limit {0} exhausted")]
    StepLimit(u64),
}

/// Deterministic interpretive engine.
#[derive(Clone, Debug, Default)]
pub struct Interpreter {
    /// Public machine state for synchronized display/validation.
    pub state: State,
}

impl Interpreter {
    /// Creates an engine from explicit initial state.
    pub const fn new(state: State) -> Self {
        Self { state }
    }

    /// Executes one typed instruction.
    pub fn step(&mut self, program: &Program) -> Result<Transition, InterpreterError> {
        if self.state.halted {
            return Err(InterpreterError::Address(self.state.pc));
        }
        let pc = self.state.pc;
        let instruction = program
            .instructions
            .get(pc)
            .cloned()
            .ok_or(InterpreterError::Address(pc))?;
        let before = self.state.accumulator.clone();
        self.state.pc += 1;
        self.execute(&instruction, program.instructions.len())?;
        self.state.steps += 1;
        Ok(Transition {
            pc,
            instruction,
            before,
            after: self.state.accumulator.clone(),
        })
    }

    /// Runs to EXIT under an explicit instruction budget.
    pub fn run(
        &mut self,
        program: &Program,
        step_limit: u64,
    ) -> Result<Vec<Transition>, InterpreterError> {
        let mut transitions = Vec::new();
        while !self.state.halted && transitions.len() < step_limit as usize {
            transitions.push(self.step(program)?);
        }
        if self.state.halted {
            Ok(transitions)
        } else {
            Err(InterpreterError::StepLimit(step_limit))
        }
    }

    fn execute(
        &mut self,
        instruction: &Instruction,
        program_len: usize,
    ) -> Result<(), InterpreterError> {
        match instruction {
            Instruction::Load { name } => {
                self.state.accumulator = self
                    .state
                    .memory
                    .get(name)
                    .cloned()
                    .ok_or_else(|| InterpreterError::Undefined(name.clone()))?;
            }
            Instruction::Store { name } => {
                self.state
                    .memory
                    .insert(name.clone(), self.state.accumulator.clone());
            }
            Instruction::Literal { value } => self.state.accumulator = value.clone(),
            Instruction::Push => self.state.stack.push(self.state.accumulator.clone()),
            Instruction::Pop => {
                self.state.accumulator = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?;
            }
            Instruction::Add => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?;
                self.state.accumulator = add_values(left, self.state.accumulator.clone())?;
            }
            Instruction::Subtract => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?;
                self.state.accumulator = subtract_values(left, self.state.accumulator.clone())?;
            }
            Instruction::Multiply { fraction_bits } => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?
                    .checked_scalar()?;
                let right = self.state.accumulator.clone().checked_scalar()?;
                self.state.accumulator =
                    Value::Scalar(scaled_product(left, right, *fraction_bits)?);
            }
            Instruction::Divide { fraction_bits } => {
                let numerator = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?
                    .checked_scalar()?;
                let denominator = self.state.accumulator.clone().checked_scalar()?;
                if denominator == 0 {
                    return Err(InterpreterError::Arithmetic(
                        "division by signed zero".to_owned(),
                    ));
                }
                let scaled = i128::from(numerator)
                    .checked_shl(u32::from(*fraction_bits))
                    .ok_or_else(|| {
                        InterpreterError::Arithmetic("division scale overflow".to_owned())
                    })?;
                self.state.accumulator =
                    Value::Scalar(checked_i64(scaled / i128::from(denominator))?);
            }
            Instruction::Absolute => {
                let value = self.state.accumulator.clone().checked_scalar()?;
                self.state.accumulator = Value::Scalar(value.checked_abs().ok_or_else(|| {
                    InterpreterError::Arithmetic("absolute-value overflow".to_owned())
                })?);
            }
            Instruction::Negate => {
                self.state.accumulator = match self.state.accumulator.clone() {
                    Value::Scalar(value) => {
                        Value::Scalar(value.checked_neg().ok_or_else(|| {
                            InterpreterError::Arithmetic("negation overflow".to_owned())
                        })?)
                    }
                    Value::Vector(values) => Value::Vector([
                        checked_neg(values[0])?,
                        checked_neg(values[1])?,
                        checked_neg(values[2])?,
                    ]),
                };
            }
            Instruction::VectorAdd => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?;
                self.state.accumulator = add_values(left, self.state.accumulator.clone())?;
            }
            Instruction::VectorSubtract => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?;
                self.state.accumulator = subtract_values(left, self.state.accumulator.clone())?;
            }
            Instruction::Dot => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?
                    .checked_vector()?;
                let right = self.state.accumulator.clone().checked_vector()?;
                let sum = left
                    .iter()
                    .zip(right)
                    .try_fold(0_i128, |sum, (&left, right)| {
                        sum.checked_add(i128::from(left) * i128::from(right))
                    })
                    .ok_or_else(|| {
                        InterpreterError::Arithmetic("dot-product overflow".to_owned())
                    })?;
                self.state.accumulator = Value::Scalar(checked_i64(sum)?);
            }
            Instruction::Cross => {
                let left = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?
                    .checked_vector()?;
                let right = self.state.accumulator.clone().checked_vector()?;
                self.state.accumulator = Value::Vector([
                    cross_component(left[1], right[2], left[2], right[1])?,
                    cross_component(left[2], right[0], left[0], right[2])?,
                    cross_component(left[0], right[1], left[1], right[0])?,
                ]);
            }
            Instruction::Scale { fraction_bits } => {
                let vector = self
                    .state
                    .stack
                    .pop()
                    .ok_or(InterpreterError::StackUnderflow)?
                    .checked_vector()?;
                let scalar = self.state.accumulator.clone().checked_scalar()?;
                self.state.accumulator = Value::Vector([
                    scaled_product(vector[0], scalar, *fraction_bits)?,
                    scaled_product(vector[1], scalar, *fraction_bits)?,
                    scaled_product(vector[2], scalar, *fraction_bits)?,
                ]);
            }
            Instruction::SquareRoot => {
                let value = self.state.accumulator.clone().checked_scalar()?;
                if value < 0 {
                    return Err(InterpreterError::Arithmetic(
                        "square root of negative value".to_owned(),
                    ));
                }
                self.state.accumulator = Value::Scalar(integer_sqrt(value as u64) as i64);
            }
            Instruction::Branch { target } => set_target(&mut self.state.pc, *target, program_len)?,
            Instruction::BranchZero { target } => {
                if self.state.accumulator.clone().checked_scalar()? == 0 {
                    set_target(&mut self.state.pc, *target, program_len)?;
                }
            }
            Instruction::BranchNegative { target } => {
                if self.state.accumulator.clone().checked_scalar()? < 0 {
                    set_target(&mut self.state.pc, *target, program_len)?;
                }
            }
            Instruction::Call { target } => {
                let return_address = self.state.pc;
                set_target(&mut self.state.pc, *target, program_len)?;
                self.state.calls.push(return_address);
            }
            Instruction::Return => {
                self.state.pc = self
                    .state
                    .calls
                    .pop()
                    .ok_or(InterpreterError::ReturnUnderflow)?;
            }
            Instruction::Exit => self.state.halted = true,
        }
        ensure_value_range(&self.state.accumulator)
    }
}

fn add_values(left: Value, right: Value) -> Result<Value, InterpreterError> {
    match (left, right) {
        (Value::Scalar(left), Value::Scalar(right)) => Ok(Value::Scalar(checked_i64(
            i128::from(left) + i128::from(right),
        )?)),
        (Value::Vector(left), Value::Vector(right)) => Ok(Value::Vector([
            checked_i64(i128::from(left[0]) + i128::from(right[0]))?,
            checked_i64(i128::from(left[1]) + i128::from(right[1]))?,
            checked_i64(i128::from(left[2]) + i128::from(right[2]))?,
        ])),
        _ => Err(InterpreterError::Type(
            "ADD operands have unlike types".to_owned(),
        )),
    }
}

fn subtract_values(left: Value, right: Value) -> Result<Value, InterpreterError> {
    match (left, right) {
        (Value::Scalar(left), Value::Scalar(right)) => Ok(Value::Scalar(checked_i64(
            i128::from(left) - i128::from(right),
        )?)),
        (Value::Vector(left), Value::Vector(right)) => Ok(Value::Vector([
            checked_i64(i128::from(left[0]) - i128::from(right[0]))?,
            checked_i64(i128::from(left[1]) - i128::from(right[1]))?,
            checked_i64(i128::from(left[2]) - i128::from(right[2]))?,
        ])),
        _ => Err(InterpreterError::Type(
            "SUBTRACT operands have unlike types".to_owned(),
        )),
    }
}

fn scaled_product(left: i64, right: i64, fraction_bits: u8) -> Result<i64, InterpreterError> {
    checked_i64((i128::from(left) * i128::from(right)) >> fraction_bits)
}

fn cross_component(a: i64, b: i64, c: i64, d: i64) -> Result<i64, InterpreterError> {
    checked_i64(i128::from(a) * i128::from(b) - i128::from(c) * i128::from(d))
}

fn checked_neg(value: i64) -> Result<i64, InterpreterError> {
    value
        .checked_neg()
        .ok_or_else(|| InterpreterError::Arithmetic("vector negation overflow".to_owned()))
}

fn checked_i64(value: i128) -> Result<i64, InterpreterError> {
    i64::try_from(value)
        .map_err(|_| InterpreterError::Arithmetic("host intermediate overflow".to_owned()))
}

fn ensure_value_range(value: &Value) -> Result<(), InterpreterError> {
    let valid = |value: i64| {
        (-AgcDoubleWord::MAX_MAGNITUDE..=AgcDoubleWord::MAX_MAGNITUDE).contains(&value)
    };
    match value {
        Value::Scalar(value) if valid(*value) => Ok(()),
        Value::Vector(values) if values.iter().all(|&value| valid(value)) => Ok(()),
        _ => Err(InterpreterError::Arithmetic(
            "value exceeds AGC double-precision range".to_owned(),
        )),
    }
}

fn set_target(pc: &mut usize, target: usize, program_len: usize) -> Result<(), InterpreterError> {
    if target >= program_len {
        Err(InterpreterError::Address(target))
    } else {
        *pc = target;
        Ok(())
    }
}

fn integer_sqrt(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut current = value;
    let mut next = (current + value / current) / 2;
    while next < current {
        current = next;
        next = (current + value / current) / 2;
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_program_executes_with_exact_scaled_integer_math() {
        let program = Program {
            instructions: vec![
                Instruction::Literal {
                    value: Value::Scalar(3),
                },
                Instruction::Push,
                Instruction::Literal {
                    value: Value::Scalar(4),
                },
                Instruction::Multiply { fraction_bits: 0 },
                Instruction::SquareRoot,
                Instruction::Exit,
            ],
        };
        let mut interpreter = Interpreter::default();
        interpreter.run(&program, 10).unwrap();
        assert_eq!(interpreter.state.accumulator, Value::Scalar(3));
    }

    #[test]
    fn vector_cross_product_is_deterministic() {
        let program = Program {
            instructions: vec![
                Instruction::Literal {
                    value: Value::Vector([1, 0, 0]),
                },
                Instruction::Push,
                Instruction::Literal {
                    value: Value::Vector([0, 1, 0]),
                },
                Instruction::Cross,
                Instruction::Exit,
            ],
        };
        let mut interpreter = Interpreter::default();
        interpreter.run(&program, 10).unwrap();
        assert_eq!(interpreter.state.accumulator, Value::Vector([0, 0, 1]));
    }
}
