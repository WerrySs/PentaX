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
    sh: [Option<u32>; 0x100],
    primitive: Option<u32>,
    index_offset: Option<u32>,
    primitive_restart: Option<u32>,
    pub(super) instances: Option<u32>,
    pub(super) index_type: Option<IndexType>,
    pub(super) index_base: Option<u64>,
    pub(super) index_count: Option<u32>,
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
    pub const fn empty() -> Self {
        Self {
            context: [None; 0x400],
            sh: [None; 0x100],
            primitive: None,
            index_offset: None,
            primitive_restart: None,
            instances: None,
            index_type: None,
            index_base: None,
            index_count: None,
        }
    }
    pub(super) fn get(&self, space: RegisterSpace, register: u32) -> Result<u32, Error> {
        let value = match space {
            RegisterSpace::Context => self.context.get(register as usize).copied().flatten(),
            RegisterSpace::Sh => self.sh.get(register as usize).copied().flatten(),
            RegisterSpace::Uconfig => match register {
                0x242 => self.primitive,
                0x24a => self.index_offset,
                0x24b => self.primitive_restart,
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
                    0x1c3..=0x1c5 | 0x1e0 | 0x200 | 0x202..=0x207 | 0x292..=0x293 |
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
            _ => return Err(Error::Unsupported("graphics register")),
        }
        Ok(())
    }
}
