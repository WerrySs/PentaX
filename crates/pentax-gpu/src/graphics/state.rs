// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit finite known-state ownership, no reset guesses.

use super::{Error, IndexType, RegisterSpace};
use std::fmt;

/// Register values from admitted graphics queue history only. Every required
/// unknown remains a typed missing-register error; empty is not hardware reset.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GraphicsState {
    context: [Option<u32>; 0x400],
    saved_context: [Option<u32>; 0x400],
    context_pushed: bool,
    sh: [Option<u32>; 0x100],
    primitive: Option<u32>,
    index_offset: Option<u32>,
    primitive_restart: Option<u32>,
    ge_control: Option<u32>,
    ge_user_vgpr: Option<u32>,
    pub(super) instances: Option<u32>,
    pub(super) index_type: Option<IndexType>,
    pub(super) index_base: Option<u64>,
    pub(super) index_count: Option<u32>,
    pub(super) draw_indirect_base: Option<u64>,
}
impl Default for GraphicsState {
    fn default() -> Self {
        Self::empty()
    }
}
impl fmt::Debug for GraphicsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsState")
            .field("known_context", &self.context.iter().flatten().count())
            .field("context_pushed", &self.context_pushed)
            .field("known_sh", &self.sh.iter().flatten().count())
            .field("known_primitive", &self.primitive.is_some())
            .field("known_index_offset", &self.index_offset.is_some())
            .field("known_primitive_restart", &self.primitive_restart.is_some())
            .field("known_instances", &self.instances.is_some())
            .field("index_type", &self.index_type)
            .field("known_index_base", &self.index_base.is_some())
            .field("known_index_count", &self.index_count.is_some())
            .finish()
    }
}
impl GraphicsState {
    /// Only processor fields assigned by the public GuestGpu constructor and
    /// its SuspendPoint Reset. Detached state and packet reset contracts stay
    /// separate; this factory invents no other register or memory authority.
    pub const fn for_new_processor() -> Self {
        Self {
            instances: Some(1),
            index_type: Some(IndexType::U16),
            index_base: Some(0),
            index_count: Some(0),
            draw_indirect_base: Some(0),
            ..Self::empty()
        }
    }
    /// Private diagnostic format: context, saved context and SH entries each
    /// use one known byte plus little-endian u32. Ten scalar entries use one
    /// known byte plus u64, followed by the context-pushed byte. Unknown values
    /// have zero storage and remain explicitly unknown. This grants no access.
    pub const DIAGNOSTIC_SNAPSHOT_BYTES: usize = (0x400 * 2 + 0x100) * 5 + 10 * 9 + 1;

    /// Borrow the actual immutable draw snapshot, never a plan's future final state.
    pub fn diagnostic_draw_state(draw: &super::Draw) -> &Self {
        &draw.state
    }

    /// Candidates only, decoded by the same checked pure VS/PS address path
    /// used during draw preparation. These grant no GPU read authority and
    /// do not identify a failing stage or prove a complete shader extent.
    pub fn diagnostic_shader_addresses(&self) -> [Option<u64>; 2] {
        [
            super::prepare::shader_address(self, 0xc8).ok(),
            super::prepare::shader_address(self, 0x08).ok(),
        ]
    }

    pub fn diagnostic_snapshot(&self, output: &mut [u8]) -> bool {
        if output.len() != Self::DIAGNOSTIC_SNAPSHOT_BYTES {
            return false;
        }
        let mut at = 0;
        for bank in [&self.context[..], &self.saved_context[..], &self.sh[..]] {
            for value in bank {
                output[at] = u8::from(value.is_some());
                output[at + 1..at + 5].copy_from_slice(&value.unwrap_or(0).to_le_bytes());
                at += 5;
            }
        }
        // Scalar order is part of this versioned data-only format.
        for value in [
            self.primitive.map(u64::from),
            self.index_offset.map(u64::from),
            self.primitive_restart.map(u64::from),
            self.ge_control.map(u64::from),
            self.ge_user_vgpr.map(u64::from),
            self.instances.map(u64::from),
            self.index_type.map(|kind| match kind {
                IndexType::U16 => 0,
                IndexType::U32 => 1,
            }),
            self.index_base,
            self.index_count.map(u64::from),
            self.draw_indirect_base,
        ] {
            output[at] = u8::from(value.is_some());
            output[at + 1..at + 9].copy_from_slice(&value.unwrap_or(0).to_le_bytes());
            at += 9;
        }
        output[at] = u8::from(self.context_pushed);
        true
    }

    pub const fn empty() -> Self {
        Self {
            context: [None; 0x400],
            saved_context: [None; 0x400],
            context_pushed: false,
            sh: [None; 0x100],
            primitive: None,
            index_offset: None,
            primitive_restart: None,
            ge_control: None,
            ge_user_vgpr: None,
            instances: None,
            index_type: None,
            index_base: None,
            index_count: None,
            draw_indirect_base: None,
        }
    }
    pub(super) fn reset(&mut self, reset: crate::queue_reset::Reset) {
        match reset {
            crate::queue_reset::Reset::Context => {
                self.context = [None; 0x400];
                // Public Context::Reset -> ShaderRegisters default.
                self.context[0x29b] = Some(0);
            }
            crate::queue_reset::Reset::Processor => {
                // Pinned209dffe Reset does not assign these independent fields.
                let index_base = self.index_base;
                let instances = self.instances;
                *self = Self::empty();
                // Public CommandProcessor::Reset and UserConfig defaults.
                self.primitive = Some(0);
                self.index_offset = Some(0);
                self.primitive_restart = Some(0);
                self.ge_control = Some(0);
                self.ge_user_vgpr = Some(0);
                self.context[0x29b] = Some(0);
                self.instances = instances;
                self.index_type = Some(IndexType::U16);
                self.index_base = index_base;
                self.index_count = Some(0);
                self.draw_indirect_base = Some(0);
            }
        }
    }
    pub(super) fn context_operation(
        &mut self,
        operation: crate::context_state::Operation,
    ) -> Result<(), Error> {
        use crate::context_state::Operation;
        match operation {
            Operation::Clear => self.reset(crate::queue_reset::Reset::Context),
            Operation::Push | Operation::PushClear => {
                if self.context_pushed {
                    return Err(Error::Unsupported("context already pushed"));
                }
                self.saved_context = self.context;
                self.context_pushed = true;
                if operation == Operation::PushClear {
                    self.reset(crate::queue_reset::Reset::Context);
                }
            }
            Operation::Pop => {
                if !self.context_pushed {
                    return Err(Error::Unsupported("context not pushed"));
                }
                self.context = self.saved_context;
                self.saved_context = [None; 0x400];
                self.context_pushed = false;
            }
        }
        Ok(())
    }
    pub(super) fn primitive_profile(&self) -> Result<(), Error> {
        // Called only after the existing no-GS/no-amplification stage guard.
        // Absent legacy fields grant no new geometry authority. Known reset0
        // is inactive GS output, and2 is the admitted triangle output.
        if self.context[0x29b].is_some_and(|value| !matches!(value, 0 | 2)) {
            return Err(Error::Unsupported("graphics output primitive profile"));
        }
        // Source decodes only these named fields, ignoring reserved bits.
        if self.ge_control.is_some_and(|value| value & 0x3ffff != 0) {
            return Err(Error::Unsupported("graphics primitive grouping profile"));
        }
        if self.ge_user_vgpr.is_some_and(|value| value & 7 != 0) {
            return Err(Error::Unsupported("graphics extra user VGPR profile"));
        }
        Ok(())
    }
    pub(super) fn get(&self, space: RegisterSpace, register: u32) -> Result<u32, Error> {
        let value = match space {
            RegisterSpace::Context => self.context.get(register as usize).copied().flatten(),
            RegisterSpace::Sh => self.sh.get(register as usize).copied().flatten(),
            RegisterSpace::Uconfig => match register {
                0x242 => self.primitive,
                0x24a => self.index_offset,
                0x24b => self.primitive_restart,
                0x25b => self.ge_control,
                0x262 => self.ge_user_vgpr,
                _ => None,
            },
        };
        value.ok_or(Error::MissingRegister { space, register })
    }
    pub(super) fn write(
        &mut self,
        space: RegisterSpace,
        register: u32,
        value: u32,
    ) -> Result<(), Error> {
        match space {
            RegisterSpace::Context
                if matches!(register,
                    0x0c..=0x0d | 0x80..=0x82 | 0x8e..=0x91 | 0x94..=0x95 |
                    0x10f..=0x114 | 0x191..=0x1b1 | 0x1b3..=0x1b4 | 0x1b6 | 0x1b8 |
                    0x1c3..=0x1c5 | 0x1e0 | 0x200 | 0x202..=0x207 | 0x292..=0x293 | 0x29b |
                    0x2d5 | 0x2f8 | 0x318 | 0x31b..=0x31f | 0x321 | 0x325 |
                    0x390 | 0x398 | 0x3a0 | 0x3a8 | 0x3b0 | 0x3b8
                ) =>
            {
                self.context[register as usize] = Some(value)
            }
            RegisterSpace::Sh if matches!(register, 0x08..=0x1b | 0x88..=0x9b | 0xc8..=0xc9) => {
                self.sh[register as usize] = Some(value);
            }
            RegisterSpace::Uconfig if register == 0x242 => self.primitive = Some(value),
            RegisterSpace::Uconfig if register == 0x24a => self.index_offset = Some(value),
            RegisterSpace::Uconfig if register == 0x24b => self.primitive_restart = Some(value),
            RegisterSpace::Uconfig if register == 0x25b => self.ge_control = Some(value),
            RegisterSpace::Uconfig if register == 0x262 => self.ge_user_vgpr = Some(value),
            _ => return Err(Error::Unsupported("graphics register")),
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "state_initialization_tests.rs"]
mod initialization_tests;

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn diagnostic_state_preserves_unknown_zero_saved_banks_and_full_width_without_authority() {
        let mut state = GraphicsState::empty();
        state.write(RegisterSpace::Context, 0x80, 0).unwrap();
        state.write(RegisterSpace::Sh, 0x08, 0x1357_2468).unwrap();
        state
            .context_operation(crate::context_state::Operation::PushClear)
            .unwrap();
        state.index_base = Some(0xfedc_ba98_7654_3210);
        let before = state;
        let mut bytes = vec![0xa5; GraphicsState::DIAGNOSTIC_SNAPSHOT_BYTES];
        assert!(state.diagnostic_snapshot(&mut bytes));
        assert_eq!(&bytes[0x80 * 5..0x80 * 5 + 5], &[0; 5]);
        assert_eq!(
            &bytes[(0x400 + 0x80) * 5..(0x400 + 0x80) * 5 + 5],
            &[1, 0, 0, 0, 0]
        );
        let sh = (0x800 + 8) * 5;
        assert_eq!(&bytes[sh..sh + 5], &[1, 0x68, 0x24, 0x57, 0x13]);
        let base = (0x400 * 2 + 0x100) * 5 + 7 * 9;
        assert_eq!(bytes[base], 1);
        assert_eq!(
            &bytes[base + 1..base + 9],
            &0xfedc_ba98_7654_3210u64.to_le_bytes()
        );
        assert_eq!(bytes.last(), Some(&1));
        let mut short = [0xa5; 4];
        assert!(!state.diagnostic_snapshot(&mut short));
        assert_eq!(short, [0xa5; 4]);
        assert_eq!(state, before);
    }
}
