#![forbid(unsafe_code)]
//! Block II logical address translation, banked memory, channels, and edit registers.

use agc_word::{AgcAddress, AgcRegister, AgcWord, ChannelAddress};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Number of erasable banks.
pub const ERASABLE_BANKS: usize = 8;
/// Words per erasable bank.
pub const ERASABLE_WORDS_PER_BANK: usize = 256;
/// Number of installed fixed banks in an Apollo 11 Block II rope.
pub const FIXED_BANKS: usize = 36;
/// Words per fixed bank.
pub const FIXED_WORDS_PER_BANK: usize = 1024;
/// Number of I/O channels addressable by the instruction format.
pub const CHANNELS: usize = 512;

/// Central-register addresses.
pub mod register {
    /// Accumulator.
    pub const A: u16 = 0o0;
    /// Lower product register.
    pub const L: u16 = 0o1;
    /// Return-address register.
    pub const Q: u16 = 0o2;
    /// Erasable bank selector.
    pub const EB: u16 = 0o3;
    /// Fixed bank selector.
    pub const FB: u16 = 0o4;
    /// Program counter.
    pub const Z: u16 = 0o5;
    /// Combined bank register.
    pub const BB: u16 = 0o6;
    /// Hardwired positive zero.
    pub const ZERO: u16 = 0o7;
    /// Accumulator interrupt shadow.
    pub const ARUPT: u16 = 0o10;
    /// L interrupt shadow.
    pub const LRUPT: u16 = 0o11;
    /// Q interrupt shadow.
    pub const QRUPT: u16 = 0o12;
    /// First spare central register.
    pub const SPARE13: u16 = 0o13;
    /// Second spare central register.
    pub const SPARE14: u16 = 0o14;
    /// Z interrupt shadow.
    pub const ZRUPT: u16 = 0o15;
    /// BB interrupt shadow.
    pub const BBRUPT: u16 = 0o16;
    /// Saved instruction for RESUME.
    pub const BRUPT: u16 = 0o17;
}

/// Logical address translation result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "region", rename_all = "kebab-case")]
pub enum PhysicalAddress {
    /// Central or special erasable register.
    Register {
        /// Register index.
        index: u16,
    },
    /// Banked erasable memory.
    Erasable {
        /// Physical bank.
        bank: u8,
        /// Offset within bank.
        offset: u16,
    },
    /// Fixed rope memory.
    Fixed {
        /// Physical bank.
        bank: u8,
        /// Offset within bank.
        offset: u16,
    },
}

/// Kind of traced memory access.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessKind {
    /// Instruction fetch.
    Fetch,
    /// Data read.
    Read,
    /// Data write.
    Write,
}

/// One completed logical memory access.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryAccess {
    /// Access kind.
    pub kind: AccessKind,
    /// Logical address supplied by the instruction.
    pub logical: u16,
    /// Resolved physical address.
    pub physical: PhysicalAddress,
    /// Value read or written.
    pub value: AgcWord,
}

/// Memory-map failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MemoryError {
    /// An address is outside the twelve-bit logical space.
    #[error("logical address {0:#o} is outside the AGC address space")]
    AddressOutOfRange(u16),
    /// A physical bank is not installed.
    #[error("fixed bank {0:#o} is not installed")]
    FixedBankUnavailable(u8),
    /// Rope memory is read-only during execution.
    #[error("attempted write to fixed memory at logical address {0:#o}")]
    FixedWrite(u16),
    /// Rope image has the wrong number of words.
    #[error("rope image has {actual} words; expected {expected}")]
    RopeSize {
        /// Required words.
        expected: usize,
        /// Supplied words.
        actual: usize,
    },
}

/// Complete deterministic machine memory state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Memory {
    central: [u16; 0o61],
    erasable: Vec<AgcWord>,
    fixed: Vec<AgcWord>,
    channels: Vec<AgcWord>,
    superbank: bool,
}

impl Default for Memory {
    fn default() -> Self {
        Self::blank()
    }
}

impl Memory {
    /// Creates zero-filled erasable and rope memory.
    pub fn blank() -> Self {
        let mut channels = vec![AgcWord::POSITIVE_ZERO; CHANNELS];
        initialize_input_channels(&mut channels);
        Self {
            central: [0; 0o61],
            erasable: vec![AgcWord::POSITIVE_ZERO; ERASABLE_BANKS * ERASABLE_WORDS_PER_BANK],
            fixed: vec![AgcWord::POSITIVE_ZERO; FIXED_BANKS * FIXED_WORDS_PER_BANK],
            channels,
            superbank: false,
        }
    }

    /// Creates memory with a bank-order rope image.
    pub fn with_rope(words: Vec<AgcWord>) -> Result<Self, MemoryError> {
        let expected = FIXED_BANKS * FIXED_WORDS_PER_BANK;
        if words.len() != expected {
            return Err(MemoryError::RopeSize {
                expected,
                actual: words.len(),
            });
        }
        let mut memory = Self::blank();
        memory.fixed = words;
        Ok(memory)
    }

    /// Replaces the rope image in physical bank order.
    pub fn load_rope(&mut self, words: &[AgcWord]) -> Result<(), MemoryError> {
        if words.len() != self.fixed.len() {
            return Err(MemoryError::RopeSize {
                expected: self.fixed.len(),
                actual: words.len(),
            });
        }
        self.fixed.copy_from_slice(words);
        Ok(())
    }

    /// Returns an immutable physical rope image.
    pub fn rope(&self) -> &[AgcWord] {
        &self.fixed
    }

    /// Resolves a logical address through current bank registers.
    pub fn translate(&self, logical: u16) -> Result<PhysicalAddress, MemoryError> {
        if logical > 0o7777 {
            return Err(MemoryError::AddressOutOfRange(logical));
        }
        match logical {
            0o0000..=0o0060 => Ok(PhysicalAddress::Register { index: logical }),
            0o0061..=0o1377 => Ok(PhysicalAddress::Erasable {
                bank: (logical >> 8) as u8,
                offset: logical & 0o377,
            }),
            0o1400..=0o1777 => Ok(PhysicalAddress::Erasable {
                bank: self.ebank(),
                offset: logical & 0o377,
            }),
            0o2000..=0o3777 => Ok(PhysicalAddress::Fixed {
                bank: self.selected_fixed_bank(),
                offset: logical & 0o1777,
            }),
            0o4000..=0o5777 => Ok(PhysicalAddress::Fixed {
                bank: 2,
                offset: logical & 0o1777,
            }),
            _ => Ok(PhysicalAddress::Fixed {
                bank: 3,
                offset: logical & 0o1777,
            }),
        }
    }

    /// Reads a logical word and returns the resolved access record.
    pub fn read(&self, logical: u16, kind: AccessKind) -> Result<MemoryAccess, MemoryError> {
        let physical = self.translate(logical)?;
        let value = self.read_physical(physical)?;
        Ok(MemoryAccess {
            kind,
            logical,
            physical,
            value,
        })
    }

    /// Performs a CPU operand read, including the read-and-rewrite cycle of
    /// the four editing registers. The returned access contains the value
    /// before the edit; subsequent reads observe the transformed value.
    pub fn read_and_edit(
        &mut self,
        logical: u16,
        kind: AccessKind,
    ) -> Result<MemoryAccess, MemoryError> {
        let access = self.read(logical, kind)?;
        let edit_index = match access.physical {
            PhysicalAddress::Register { index } if (0o20..=0o23).contains(&index) => Some(index),
            PhysicalAddress::Erasable { bank: 0, offset } if (0o20..=0o23).contains(&offset) => {
                Some(offset)
            }
            _ => None,
        };
        if let Some(index) = edit_index {
            self.write_special(index, access.value);
        }
        Ok(access)
    }

    /// Writes a logical erasable word and returns the resolved access record.
    pub fn write(&mut self, logical: u16, value: AgcWord) -> Result<MemoryAccess, MemoryError> {
        let physical = self.translate(logical)?;
        match physical {
            PhysicalAddress::Register { index } => self.write_special(index, value),
            PhysicalAddress::Erasable { bank: 0, offset } if offset <= 0o60 => {
                self.write_special(offset, value);
            }
            PhysicalAddress::Erasable { bank, offset } => {
                self.erasable[usize::from(bank) * ERASABLE_WORDS_PER_BANK + usize::from(offset)] =
                    value;
            }
            PhysicalAddress::Fixed { .. } => return Err(MemoryError::FixedWrite(logical)),
        }
        Ok(MemoryAccess {
            kind: AccessKind::Write,
            logical,
            physical,
            value: self.read_physical(physical)?,
        })
    }

    /// Reads one central register at full 16-bit width.
    pub fn central_register(&self, index: u16) -> Result<AgcRegister, MemoryError> {
        if index >= 16 {
            return Err(MemoryError::AddressOutOfRange(index));
        }
        AgcRegister::try_from_raw(u32::from(self.central[usize::from(index)]))
            .map_err(|_| MemoryError::AddressOutOfRange(index))
    }

    /// Writes one central register at full architectural width.
    pub fn set_central_register(
        &mut self,
        index: u16,
        value: AgcRegister,
    ) -> Result<(), MemoryError> {
        if index >= 16 {
            return Err(MemoryError::AddressOutOfRange(index));
        }
        self.write_central_raw(index, value.raw());
        Ok(())
    }

    /// Reads a nine-bit I/O channel.
    pub fn read_channel(&self, channel: ChannelAddress) -> AgcWord {
        match channel.get() {
            1 => self.read_special(register::L),
            2 => self.read_special(register::Q),
            _ => self.channels[usize::from(channel.get())],
        }
    }

    /// Writes a nine-bit I/O channel and handles bank/timer control side effects.
    pub fn write_channel(&mut self, channel: ChannelAddress, value: AgcWord) {
        match channel.get() {
            1 => self.write_special(register::L, value),
            2 => self.write_special(register::Q, value),
            0o7 => {
                self.superbank = value.raw() & 0o100 != 0;
                self.channels[7] = value;
            }
            _ => self.channels[usize::from(channel.get())] = value,
        }
    }

    /// Returns the currently selected erasable bank.
    pub const fn ebank(&self) -> u8 {
        ((self.central[register::EB as usize] >> 8) & 0o7) as u8
    }

    /// Returns the five-bit FBANK selection before superbank translation.
    pub const fn fbank(&self) -> u8 {
        ((self.central[register::FB as usize] >> 10) & 0o37) as u8
    }

    /// Returns whether channel 7 selects fixed superbanks.
    pub const fn superbank(&self) -> bool {
        self.superbank
    }

    /// Clears erasable, registers, channels, and bank selectors, preserving rope.
    pub fn reset_volatile(&mut self) {
        self.central.fill(0);
        self.erasable.fill(AgcWord::POSITIVE_ZERO);
        self.channels.fill(AgcWord::POSITIVE_ZERO);
        initialize_input_channels(&mut self.channels);
        self.superbank = false;
    }

    /// Reads physical erasable memory for diagnostics and fault injection.
    pub fn read_erasable_physical(&self, bank: u8, offset: u16) -> Option<AgcWord> {
        if usize::from(bank) >= ERASABLE_BANKS || usize::from(offset) >= ERASABLE_WORDS_PER_BANK {
            return None;
        }
        Some(if bank == 0 && offset <= 0o60 {
            self.read_special(offset)
        } else {
            self.erasable[usize::from(bank) * ERASABLE_WORDS_PER_BANK + usize::from(offset)]
        })
    }

    /// Writes physical erasable memory for deterministic initialization.
    pub fn write_erasable_physical(
        &mut self,
        bank: u8,
        offset: u16,
        value: AgcWord,
    ) -> Result<(), MemoryError> {
        if usize::from(bank) >= ERASABLE_BANKS || usize::from(offset) >= ERASABLE_WORDS_PER_BANK {
            return Err(MemoryError::AddressOutOfRange(
                (u16::from(bank) << 8) | offset,
            ));
        }
        if bank == 0 && offset <= 0o60 {
            self.write_special(offset, value);
        } else {
            self.erasable[usize::from(bank) * ERASABLE_WORDS_PER_BANK + usize::from(offset)] =
                value;
        }
        Ok(())
    }

    /// Flips selected bits in physical rope memory for an explicit fault-injection run.
    /// Normal logical writes remain read-only.
    pub fn inject_fixed_bit_flip(
        &mut self,
        bank: u8,
        offset: u16,
        mask: u16,
    ) -> Result<AgcWord, MemoryError> {
        if usize::from(bank) >= FIXED_BANKS || usize::from(offset) >= FIXED_WORDS_PER_BANK {
            return Err(MemoryError::FixedBankUnavailable(bank));
        }
        let index = usize::from(bank) * FIXED_WORDS_PER_BANK + usize::from(offset);
        self.fixed[index] = AgcWord::from_raw_truncate(self.fixed[index].raw() ^ mask);
        Ok(self.fixed[index])
    }

    fn selected_fixed_bank(&self) -> u8 {
        let bank = self.fbank();
        if self.superbank && (0o30..=0o33).contains(&bank) {
            bank + 0o10
        } else {
            bank
        }
    }

    fn read_physical(&self, physical: PhysicalAddress) -> Result<AgcWord, MemoryError> {
        match physical {
            PhysicalAddress::Register { index } => Ok(self.read_special(index)),
            PhysicalAddress::Erasable { bank: 0, offset } if offset <= 0o60 => {
                Ok(self.read_special(offset))
            }
            PhysicalAddress::Erasable { bank, offset } => Ok(
                self.erasable[usize::from(bank) * ERASABLE_WORDS_PER_BANK + usize::from(offset)]
            ),
            PhysicalAddress::Fixed { bank, offset } => {
                let bank_index = usize::from(bank);
                if bank_index >= FIXED_BANKS {
                    return Err(MemoryError::FixedBankUnavailable(bank));
                }
                Ok(self.fixed[bank_index * FIXED_WORDS_PER_BANK + usize::from(offset)])
            }
        }
    }

    fn read_special(&self, index: u16) -> AgcWord {
        match index {
            register::A | register::L | register::Q => {
                AgcRegister::try_from_raw(u32::from(self.central[usize::from(index)]))
                    .expect("stored central register is 16-bit")
                    .overflow_correct()
            }
            register::Z => AgcWord::from_raw_truncate(self.central[register::Z as usize] & 0o7777),
            register::ZERO => AgcWord::POSITIVE_ZERO,
            _ => AgcWord::from_raw_truncate(self.central[usize::from(index)]),
        }
    }

    fn write_special(&mut self, index: u16, value: AgcWord) {
        match index {
            0o20 => self.central[index as usize] = rotate_right(value.raw()),
            0o21 => self.central[index as usize] = shift_right(value.raw()),
            0o22 => self.central[index as usize] = rotate_left(value.raw()),
            0o23 => self.central[index as usize] = (value.raw() >> 7) & 0o177,
            0o24..=0o60 => self.central[index as usize] = value.raw(),
            register::ZERO => {}
            register::A | register::L | register::Q => {
                self.write_central_raw(index, value.sign_extend().raw());
            }
            index if index < 0o20 => self.write_central_raw(index, value.raw()),
            _ => self.central[index as usize] = value.raw(),
        }
    }

    fn write_central_raw(&mut self, index: u16, raw: u16) {
        match index {
            register::A | register::L | register::Q => self.central[index as usize] = raw,
            register::Z => self.central[index as usize] = raw & 0o7777,
            register::EB => {
                self.central[index as usize] = raw & 0o03400;
                self.synchronize_banks(index);
            }
            register::FB => {
                self.central[index as usize] = raw & 0o76000;
                self.synchronize_banks(index);
            }
            register::BB => {
                self.central[index as usize] = raw & 0o76007;
                self.synchronize_banks(index);
            }
            register::ZERO => {}
            index if index < 0o20 => self.central[index as usize] = raw,
            _ => self.central[index as usize] = raw & 0o77777,
        }
    }

    fn synchronize_banks(&mut self, written: u16) {
        if written == register::BB {
            let bb = self.central[register::BB as usize];
            self.central[register::EB as usize] = (bb & 0o7) << 8;
            self.central[register::FB as usize] = bb & 0o76000;
        } else {
            self.central[register::BB as usize] = self.central[register::FB as usize]
                | ((self.central[register::EB as usize] >> 8) & 0o7);
        }
    }
}

fn rotate_right(raw: u16) -> u16 {
    ((raw >> 1) | ((raw & 1) << 14)) & 0o77777
}

fn rotate_left(raw: u16) -> u16 {
    ((raw << 1) | ((raw >> 14) & 1)) & 0o77777
}

fn shift_right(raw: u16) -> u16 {
    ((raw >> 1) | (raw & 0o40000)) & 0o77777
}

fn initialize_input_channels(channels: &mut [AgcWord]) {
    channels[0o30] = AgcWord::from_raw_truncate(0o37777);
    for channel in [0o31, 0o32, 0o33] {
        channels[channel] = AgcWord::NEGATIVE_ZERO;
    }
}

/// Converts a checked address to its raw logical value.
pub const fn logical(address: AgcAddress) -> u16 {
    address.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_windows_and_fixed_fixed_are_distinct() {
        let mut memory = Memory::blank();
        memory
            .write(0o3, AgcWord::try_from_raw(0o2400).unwrap())
            .unwrap();
        memory
            .write(0o4, AgcWord::try_from_raw(0o14000).unwrap())
            .unwrap();
        assert_eq!(
            memory.translate(0o1400).unwrap(),
            PhysicalAddress::Erasable { bank: 5, offset: 0 }
        );
        assert_eq!(
            memory.translate(0o2000).unwrap(),
            PhysicalAddress::Fixed { bank: 6, offset: 0 }
        );
        assert_eq!(
            memory.translate(0o4000).unwrap(),
            PhysicalAddress::Fixed { bank: 2, offset: 0 }
        );
    }

    #[test]
    fn editing_registers_transform_on_write() {
        let mut memory = Memory::blank();
        memory
            .write(0o20, AgcWord::try_from_raw(1).unwrap())
            .unwrap();
        assert_eq!(
            memory.read(0o20, AccessKind::Read).unwrap().value.raw(),
            0o40000
        );
        memory
            .write(0o22, AgcWord::try_from_raw(0o40001).unwrap())
            .unwrap();
        assert_eq!(memory.read(0o22, AccessKind::Read).unwrap().value.raw(), 3);
    }

    #[test]
    fn editing_registers_transform_again_on_operand_read() {
        let mut memory = Memory::blank();
        memory
            .write(0o20, AgcWord::try_from_raw(0o152).unwrap())
            .unwrap();
        assert_eq!(
            memory.read(0o20, AccessKind::Read).unwrap().value.raw(),
            0o65
        );

        let access = memory.read_and_edit(0o20, AccessKind::Read).unwrap();

        assert_eq!(access.value.raw(), 0o65);
        assert_eq!(
            memory.read(0o20, AccessKind::Read).unwrap().value.raw(),
            0o40032
        );
    }

    #[test]
    fn bank_zero_edit_register_alias_transforms_on_operand_read() {
        let mut memory = Memory::blank();
        memory
            .write(0o20, AgcWord::try_from_raw(0o152).unwrap())
            .unwrap();

        let access = memory.read_and_edit(0o1420, AccessKind::Read).unwrap();

        assert_eq!(access.value.raw(), 0o65);
        assert_eq!(
            memory.read(0o20, AccessKind::Read).unwrap().value.raw(),
            0o40032
        );
    }

    #[test]
    fn l_register_preserves_sign_extension() {
        let mut memory = Memory::blank();
        let negative = AgcWord::from_i32(-7).unwrap();
        memory
            .set_central_register(register::L, negative.sign_extend())
            .unwrap();
        assert_eq!(
            memory.central_register(register::L).unwrap(),
            negative.sign_extend()
        );
        assert_eq!(
            memory.read(register::L, AccessKind::Read).unwrap().value,
            negative
        );
    }

    #[test]
    fn ordinary_word_write_sign_extends_central_registers() {
        let mut memory = Memory::blank();
        let negative = AgcWord::from_i32(-7).unwrap();
        memory.write(register::L, negative).unwrap();
        assert_eq!(
            memory.central_register(register::L).unwrap(),
            negative.sign_extend()
        );
    }

    #[test]
    fn bank_zero_window_aliases_central_and_timer_registers() {
        let mut memory = Memory::blank();
        let first = AgcWord::try_from_raw(0o151).unwrap();
        let second = AgcWord::try_from_raw(0o252).unwrap();

        memory.write(0o25, first).unwrap();
        let window_read = memory.read(0o1425, AccessKind::Read).unwrap();
        assert_eq!(
            window_read.physical,
            PhysicalAddress::Erasable {
                bank: 0,
                offset: 0o25
            }
        );
        assert_eq!(window_read.value, first);

        memory.write(0o1425, second).unwrap();
        assert_eq!(memory.read(0o25, AccessKind::Read).unwrap().value, second);
        assert_eq!(memory.read_erasable_physical(0, 0o25), Some(second));
    }

    #[test]
    fn physical_bank_zero_writes_alias_special_registers() {
        let mut memory = Memory::blank();
        let value = AgcWord::try_from_raw(0o321).unwrap();

        memory.write_erasable_physical(0, 0o25, value).unwrap();

        assert_eq!(memory.read(0o25, AccessKind::Read).unwrap().value, value);
        assert_eq!(memory.read(0o1425, AccessKind::Read).unwrap().value, value);
    }

    #[test]
    fn nonzero_erasable_banks_do_not_alias_central_registers() {
        let mut memory = Memory::blank();
        let central = AgcWord::try_from_raw(0o151).unwrap();
        let banked = AgcWord::try_from_raw(0o252).unwrap();
        memory.write(0o25, central).unwrap();
        memory
            .write(register::EB, AgcWord::try_from_raw(0o400).unwrap())
            .unwrap();

        memory.write(0o1425, banked).unwrap();

        assert_eq!(memory.read(0o25, AccessKind::Read).unwrap().value, central);
        assert_eq!(memory.read_erasable_physical(1, 0o25), Some(banked));
    }

    #[test]
    fn bank_zero_alias_write_reports_original_physical_location() {
        let mut memory = Memory::blank();
        let access = memory
            .write(0o1403, AgcWord::try_from_raw(0o1400).unwrap())
            .unwrap();

        assert_eq!(
            access.physical,
            PhysicalAddress::Erasable { bank: 0, offset: 3 }
        );
        assert_eq!(access.value.raw(), 0o1400);
        assert_eq!(memory.ebank(), 3);
    }

    #[test]
    fn ordinary_shadow_register_writes_remain_fifteen_bit() {
        let mut memory = Memory::blank();
        let negative = AgcWord::from_i32(-7).unwrap();

        memory.write(register::ARUPT, negative).unwrap();

        assert_eq!(
            memory.central_register(register::ARUPT).unwrap().raw(),
            negative.raw()
        );
    }

    #[test]
    fn central_register_reads_overflow_correct_l() {
        let mut memory = Memory::blank();
        memory
            .set_central_register(register::L, AgcRegister::try_from_raw(0o040000).unwrap())
            .unwrap();
        assert_eq!(
            memory.read(register::L, AccessKind::Read).unwrap().value,
            AgcWord::POSITIVE_ZERO
        );
    }

    #[test]
    fn reset_uses_block_ii_discrete_input_defaults() {
        let mut memory = Memory::blank();
        assert_eq!(
            memory
                .read_channel(ChannelAddress::new(0o30).unwrap())
                .raw(),
            0o37777
        );
        for channel in [0o31, 0o32, 0o33] {
            assert_eq!(
                memory.read_channel(ChannelAddress::new(channel).unwrap()),
                AgcWord::NEGATIVE_ZERO
            );
        }
        memory.write_channel(ChannelAddress::new(0o31).unwrap(), AgcWord::POSITIVE_ZERO);
        memory.reset_volatile();
        assert_eq!(
            memory.read_channel(ChannelAddress::new(0o31).unwrap()),
            AgcWord::NEGATIVE_ZERO
        );
    }
}
