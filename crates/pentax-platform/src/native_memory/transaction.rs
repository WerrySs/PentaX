// SPDX-License-Identifier: GPL-2.0-only

//! Typed requests for one exclusively paused native-memory transaction.
//! Effects describe guest addresses and logical permissions, never host pointers.

use super::NativePermissions;

/// One supported actual backing effect. Direct views share retained physical
/// sections. Unmapping retains addresses; physical release removes affected views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMemoryEffect {
    None,
    AllocatePhysical {
        physical: u64,
        size: u64,
    },
    ReleasePhysical {
        physical: u64,
        size: u64,
    },
    MapPhysical {
        address: u64,
        size: u64,
        physical: u64,
        permissions: NativePermissions,
    },
    MapZeroed {
        address: u64,
        size: u64,
        permissions: NativePermissions,
    },
    Protect {
        address: u64,
        size: u64,
        permissions: NativePermissions,
    },
    Unmap {
        address: u64,
        size: u64,
    },
}

/// One borrowed output, validated against the effect's prospective permissions.
/// Safe Rust slices cannot alias the owner's private native pages.
#[derive(Clone, Copy, Debug)]
pub struct NativeMemoryWrite<'a> {
    pub address: u64,
    pub bytes: &'a [u8],
}

/// Trusted loader request, not guest write authority. The exclusive owner
/// compares all old bytes before replacing any eight-byte relocation word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWordReplacement {
    pub address: u64,
    pub expected: u64,
    pub replacement: u64,
}

/// Request data only, not an unchecked prepared capability. Applying a request
/// always revalidates the exact live, exclusively borrowed owner.
#[derive(Clone, Copy, Debug)]
pub struct NativeMemoryTransaction<'a> {
    pub effect: NativeMemoryEffect,
    pub writes: &'a [NativeMemoryWrite<'a>],
}

/// Admission bounds enforced before metadata allocation or OS mutation.
/// `max_preflight_steps` counts conservative page/reservation visits and output
/// overlap comparisons, not CPU instructions or allocator bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTransactionLimits {
    pub max_outputs: usize,
    pub max_output_bytes: u64,
    pub max_preflight_steps: u64,
    /// Physical-release split/rollback metadata, admitted before any OS effect.
    pub max_scratch_bytes: usize,
}

impl Default for NativeTransactionLimits {
    fn default() -> Self {
        Self {
            max_outputs: 64,
            max_output_bytes: 16 * 1024 * 1024,
            max_preflight_steps: 1024 * 1024,
            max_scratch_bytes: 1024 * 1024,
        }
    }
}
