#![forbid(unsafe_code)]
//! Real channel-driven DSKY display, lamps, keyboard encoding, and text interface.

use agc_trace::TraceEvent;
use agc_word::AgcWord;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One DSKY keyboard key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Key {
    /// Numeric key.
    Digit(u8),
    /// VERB.
    Verb,
    /// NOUN.
    Noun,
    /// Plus sign.
    Plus,
    /// Minus sign.
    Minus,
    /// ENTER.
    Enter,
    /// CLEAR.
    Clear,
    /// KEY REL.
    KeyRelease,
    /// RSET.
    Reset,
    /// PRO.
    Proceed,
}

impl Key {
    /// Encodes the five-bit keyboard word used on channel 015.
    pub fn code(self) -> Result<u8, DskyError> {
        match self {
            Self::Digit(0) => Ok(0o20),
            Self::Digit(digit @ 1..=9) => Ok(digit),
            Self::Digit(digit) => Err(DskyError::Digit(digit)),
            Self::Verb => Ok(0o21),
            Self::Reset => Ok(0o22),
            Self::KeyRelease => Ok(0o31),
            Self::Plus => Ok(0o32),
            Self::Minus => Ok(0o33),
            Self::Enter => Ok(0o34),
            Self::Clear => Ok(0o36),
            Self::Noun => Ok(0o37),
            // PRO is carried as a discrete input rather than an ordinary keycode.
            Self::Proceed => Ok(0o20),
        }
    }

    /// Decodes an ordinary five-bit keyboard word from channel 015.
    ///
    /// The PRO key is a separate hardware discrete and is therefore not
    /// returned by this function; channel code 020 is the numeric zero key.
    pub fn from_code(code: u8) -> Result<Self, DskyError> {
        match code {
            0o20 => Ok(Self::Digit(0)),
            digit @ 1..=9 => Ok(Self::Digit(digit)),
            0o21 => Ok(Self::Verb),
            0o22 => Ok(Self::Reset),
            0o31 => Ok(Self::KeyRelease),
            0o32 => Ok(Self::Plus),
            0o33 => Ok(Self::Minus),
            0o34 => Ok(Self::Enter),
            0o36 => Ok(Self::Clear),
            0o37 => Ok(Self::Noun),
            _ => Err(DskyError::Code(code)),
        }
    }
}

/// Typed state of the Pinball V37 major-mode entry path.
///
/// This is a readable semantic reconstruction of the operator protocol, not
/// a replacement for executing Pinball in the rope. Mission validation feeds
/// it the exact keys accepted by rope routine `CHARIN` and compares its result
/// with Luminary's `MODREG`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum V37State {
    /// Waiting for the VERB key.
    #[default]
    AwaitVerb,
    /// Waiting for the tens digit of the verb.
    VerbTens,
    /// Waiting for the ones digit of the verb.
    VerbOnes {
        /// Entered tens digit.
        tens: u8,
    },
    /// Waiting for ENTER after a complete verb.
    VerbEnter {
        /// Two-digit verb value.
        verb: u8,
    },
    /// V37 accepted; waiting for the major-mode tens digit.
    ProgramTens,
    /// Waiting for the major-mode ones digit.
    ProgramOnes {
        /// Entered tens digit.
        tens: u8,
    },
    /// Waiting for ENTER after a complete major mode.
    ProgramEnter {
        /// Two-digit major mode.
        program: u8,
    },
    /// Program selection is complete.
    Complete {
        /// Selected major mode.
        program: u8,
    },
}

/// Readable, typed reconstruction of Pinball's `V37E nnE` protocol.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct V37ProgramChange {
    /// Current protocol state.
    pub state: V37State,
    /// Number of keys accepted by the reconstruction.
    pub accepted_keys: usize,
    /// Last complete verb register value.
    pub verb_register: Option<u8>,
    /// Selected major mode after final ENTER.
    pub program_register: Option<u8>,
}

impl V37ProgramChange {
    /// Applies one key that the original rope has accepted through `CHARIN`.
    pub fn accept(&mut self, key: Key) -> Result<V37State, PinballError> {
        self.state = match (self.state, key) {
            (V37State::AwaitVerb, Key::Verb) => V37State::VerbTens,
            (V37State::VerbTens, Key::Digit(tens)) => V37State::VerbOnes { tens },
            (V37State::VerbOnes { tens }, Key::Digit(ones)) => V37State::VerbEnter {
                verb: decimal_pair(tens, ones)?,
            },
            (V37State::VerbEnter { verb: 37 }, Key::Enter) => {
                self.verb_register = Some(37);
                V37State::ProgramTens
            }
            (V37State::VerbEnter { verb }, Key::Enter) => {
                return Err(PinballError::NotProgramChange(verb));
            }
            (V37State::ProgramTens, Key::Digit(tens)) => V37State::ProgramOnes { tens },
            (V37State::ProgramOnes { tens }, Key::Digit(ones)) => V37State::ProgramEnter {
                program: decimal_pair(tens, ones)?,
            },
            (V37State::ProgramEnter { program }, Key::Enter) => {
                self.program_register = Some(program);
                V37State::Complete { program }
            }
            (state, key) => return Err(PinballError::Unexpected { state, key }),
        };
        self.accepted_keys += 1;
        Ok(self.state)
    }

    /// Applies one channel 015 keyboard code.
    pub fn accept_code(&mut self, code: u8) -> Result<V37State, PinballError> {
        self.accept(Key::from_code(code)?)
    }
}

fn decimal_pair(tens: u8, ones: u8) -> Result<u8, PinballError> {
    if tens <= 9 && ones <= 9 {
        Ok(tens * 10 + ones)
    } else {
        Err(PinballError::InvalidDigit(tens.max(ones)))
    }
}

/// Sign lamps for one five-digit register.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Sign {
    /// Plus segment.
    pub plus: bool,
    /// Minus segment.
    pub minus: bool,
}

/// DSKY annunciator state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Lamps {
    /// UPLINK ACTY.
    pub uplink_activity: bool,
    /// NO ATT.
    pub no_attitude: bool,
    /// STBY.
    pub standby: bool,
    /// KEY REL.
    pub key_release: bool,
    /// OPR ERR.
    pub operator_error: bool,
    /// RESTART.
    pub restart: bool,
    /// TRACKER.
    pub tracker: bool,
    /// ALT.
    pub altitude: bool,
    /// VEL.
    pub velocity: bool,
    /// COMP ACTY.
    pub computer_activity: bool,
    /// TEMP.
    pub temperature: bool,
    /// GIMBAL LOCK.
    pub gimbal_lock: bool,
    /// PROG.
    pub program_alarm: bool,
}

/// Complete display state derived only from AGC output channels.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DskyState {
    /// PROG two digits.
    pub program: [Option<u8>; 2],
    /// VERB two digits.
    pub verb: [Option<u8>; 2],
    /// NOUN two digits.
    pub noun: [Option<u8>; 2],
    /// Three five-digit registers.
    pub registers: [[Option<u8>; 5]; 3],
    /// Register signs.
    pub signs: [Sign; 3],
    /// Annunciator lamps.
    pub lamps: Lamps,
    /// Flash command from channel 013.
    pub flash: bool,
    /// Last raw channel 010 relay word.
    pub last_relay_word: AgcWord,
    /// Last raw channel 011 lamp word.
    pub last_lamp_word: AgcWord,
}

impl Default for DskyState {
    fn default() -> Self {
        Self {
            program: [None; 2],
            verb: [None; 2],
            noun: [None; 2],
            registers: [[None; 5]; 3],
            signs: [Sign::default(); 3],
            lamps: Lamps::default(),
            flash: false,
            last_relay_word: AgcWord::POSITIVE_ZERO,
            last_lamp_word: AgcWord::POSITIVE_ZERO,
        }
    }
}

/// DSKY input/decode failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DskyError {
    /// Numeric key outside 0..9.
    #[error("invalid DSKY digit {0}")]
    Digit(u8),
    /// Five-bit code does not name an ordinary keyboard key.
    #[error("invalid DSKY channel 015 code {0:#o}")]
    Code(u8),
}

/// V37 semantic-reconstruction failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PinballError {
    /// Keyboard decoding failed.
    #[error(transparent)]
    Dsky(#[from] DskyError),
    /// A numeric key was outside 0..9.
    #[error("invalid V37 digit {0}")]
    InvalidDigit(u8),
    /// ENTER completed a verb other than V37.
    #[error("verb {0:02} is not the V37 program-change protocol")]
    NotProgramChange(u8),
    /// Key is not valid at this point in the protocol.
    #[error("unexpected key {key:?} in V37 state {state:?}")]
    Unexpected {
        /// State before the rejected key.
        state: V37State,
        /// Rejected key.
        key: Key,
    },
}

impl DskyState {
    /// Applies one channel write from the running AGC.
    pub fn write_channel(&mut self, channel: u16, value: AgcWord) {
        match channel {
            0o10 => self.write_relay(value),
            0o11 => self.write_lamps(value),
            0o13 => self.flash = value.raw() & 0o1000 != 0,
            0o163 => self.write_extended_lamps(value),
            _ => {}
        }
    }

    /// Consumes all output activity from one committed CPU step.
    pub fn consume_trace(&mut self, event: &TraceEvent) {
        for io in &event.io {
            if io.write {
                self.write_channel(io.channel, io.value);
            }
        }
    }

    /// Renders a compact terminal/research view from real display state.
    pub fn render_text(&self) -> String {
        let pair = |digits: &[Option<u8>; 2]| format_digits(digits);
        let mut output = format!(
            "+---------------- DSKY ----------------+\n| PROG {}   VERB {}   NOUN {}       |\n",
            pair(&self.program),
            pair(&self.verb),
            pair(&self.noun)
        );
        for (index, digits) in self.registers.iter().enumerate() {
            let sign = match self.signs[index] {
                Sign {
                    plus: true,
                    minus: false,
                } => '+',
                Sign {
                    plus: false,
                    minus: true,
                } => '-',
                Sign {
                    plus: true,
                    minus: true,
                } => '±',
                Sign {
                    plus: false,
                    minus: false,
                } => ' ',
            };
            output.push_str(&format!(
                "| R{}   {}{}                         |\n",
                index + 1,
                sign,
                format_digits(digits)
            ));
        }
        output.push_str(&format!(
            "| COMP ACTY {:3}  OPR ERR {:3}  PROG {:3} |\n+--------------------------------------\n",
            lamp(self.lamps.computer_activity),
            lamp(self.lamps.operator_error),
            lamp(self.lamps.program_alarm)
        ));
        output
    }

    fn write_relay(&mut self, value: AgcWord) {
        self.last_relay_word = value;
        let raw = value.raw();
        let relay = ((raw >> 11) & 0o17) as u8;
        let upper = decode_digit(((raw >> 5) & 0o37) as u8);
        let lower = decode_digit((raw & 0o37) as u8);
        let sign = raw & 0o2000 != 0;
        match relay {
            1..=7 => {
                let linear = 14 - usize::from(relay) * 2;
                self.set_linear_digit(linear + 1, upper);
                self.set_linear_digit(linear + 2, lower);
            }
            8 => {
                self.set_linear_digit(0, lower);
                self.signs[0].minus = sign;
            }
            9 => self.noun = [upper, lower],
            10 => self.verb = [upper, lower],
            11 => self.program = [upper, lower],
            12 => {
                self.lamps.velocity = raw & 0o4 != 0;
                self.lamps.no_attitude = raw & 0o10 != 0;
                self.lamps.altitude = raw & 0o20 != 0;
                self.lamps.gimbal_lock = raw & 0o40 != 0;
                self.lamps.tracker = raw & 0o100 != 0;
                self.lamps.program_alarm = raw & 0o200 != 0;
            }
            _ => {}
        }
    }

    fn set_linear_digit(&mut self, index: usize, digit: Option<u8>) {
        if index < 15 {
            self.registers[index / 5][index % 5] = digit;
        }
    }

    fn write_lamps(&mut self, value: AgcWord) {
        self.last_lamp_word = value;
        let raw = value.raw();
        self.lamps.iss_warning(raw);
        self.signs[0].plus = raw & 0o4 != 0;
        self.signs[0].minus = raw & 0o10 != 0;
        self.signs[1].plus = raw & 0o20 != 0;
        self.signs[1].minus = raw & 0o40 != 0;
        self.signs[2].plus = raw & 0o100 != 0;
        self.signs[2].minus = raw & 0o200 != 0;
    }

    fn write_extended_lamps(&mut self, value: AgcWord) {
        let raw = value.raw();
        self.lamps.computer_activity = raw & 0o1 != 0;
        self.lamps.uplink_activity = raw & 0o2 != 0;
        self.lamps.temperature = raw & 0o4 != 0;
        self.lamps.key_release = raw & 0o10 != 0;
        self.lamps.operator_error = raw & 0o20 != 0;
    }
}

impl Lamps {
    fn iss_warning(&mut self, raw: u16) {
        self.standby = raw & 0o400 != 0;
        self.restart = raw & 0o1000 != 0;
    }
}

fn decode_digit(code: u8) -> Option<u8> {
    match code {
        0 => None,
        0o25 => Some(0),
        0o03 => Some(1),
        0o31 => Some(2),
        0o33 => Some(3),
        0o17 => Some(4),
        0o36 => Some(5),
        0o34 => Some(6),
        0o23 => Some(7),
        0o35 => Some(8),
        0o37 => Some(9),
        _ => None,
    }
}

fn format_digits(digits: &[Option<u8>]) -> String {
    digits
        .iter()
        .map(|digit| digit.map_or(' ', |digit| char::from(b'0' + digit)))
        .collect()
}

const fn lamp(on: bool) -> &'static str {
    if on { "ON" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_words_drive_program_verb_noun_and_registers() {
        let mut dsky = DskyState::default();
        dsky.write_channel(
            0o10,
            AgcWord::from_raw_truncate((11 << 11) | (0o03 << 5) | 0o31),
        );
        dsky.write_channel(
            0o10,
            AgcWord::from_raw_truncate((10 << 11) | (0o33 << 5) | 0o17),
        );
        assert_eq!(dsky.program, [Some(1), Some(2)]);
        assert_eq!(dsky.verb, [Some(3), Some(4)]);
        assert!(dsky.render_text().contains("PROG 12"));
    }

    #[test]
    fn all_keyboard_codes_are_five_bit() {
        for digit in 0..=9 {
            assert!(Key::Digit(digit).code().unwrap() <= 0o37);
        }
        assert!(Key::Digit(10).code().is_err());
    }

    #[test]
    fn typed_v37_reconstruction_selects_program_63() {
        let mut change = V37ProgramChange::default();
        for code in [0o21, 3, 7, 0o34, 6, 3, 0o34] {
            change.accept_code(code).unwrap();
        }
        assert_eq!(change.verb_register, Some(37));
        assert_eq!(change.program_register, Some(63));
        assert_eq!(change.state, V37State::Complete { program: 63 });
        assert_eq!(change.accepted_keys, 7);
    }

    #[test]
    fn typed_v37_reconstruction_rejects_out_of_order_input() {
        let mut change = V37ProgramChange::default();
        assert!(matches!(
            change.accept(Key::Digit(3)),
            Err(PinballError::Unexpected { .. })
        ));
        assert_eq!(change, V37ProgramChange::default());
    }
}
