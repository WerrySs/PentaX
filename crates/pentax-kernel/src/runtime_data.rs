// SPDX-License-Identifier: GPL-2.0-only

//! Fixed copied outputs accompanying one new runtime-owned guest blob.

use crate::{Failure, GuestError, GuestWrite};

const OUTPUTS: usize = 3;
// FontWritingInit atomically publishes its complete256-byte caller object
// together with the new owned step/letter/metrics mapping.
const OUTPUT_BYTES: usize = 256;

#[derive(Clone, Copy)]
struct Output {
    address: u64,
    length: usize,
    bytes: [u8; OUTPUT_BYTES],
}

/// Bounded request data, not guest memory or publication authority. Each of up
/// to three external writes retains at most256 copied bytes; no heap allocation
/// or caller-owned slice survives preparation. The kernel adds its blob write.
#[derive(Clone, Copy)]
pub struct RuntimeDataOutputs {
    outputs: [Output; OUTPUTS],
    count: usize,
}

impl Default for RuntimeDataOutputs {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RuntimeDataOutputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeDataOutputs")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl RuntimeDataOutputs {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            outputs: [Output {
                address: 0,
                length: 0,
                bytes: [0; OUTPUT_BYTES],
            }; OUTPUTS],
            count: 0,
        }
    }

    /// Retain one complete copied output. Refusal leaves the builder unchanged.
    /// Cross-output overlap and actual permission/physical-alias validation
    /// remain mandatory at the one final guest transaction.
    pub fn push(&mut self, address: u64, bytes: &[u8]) -> Result<(), Failure> {
        if self.count == OUTPUTS {
            return Err(Failure::Limit("runtime data outputs"));
        }
        if bytes.len() > OUTPUT_BYTES {
            return Err(Failure::Limit("runtime data output bytes"));
        }
        address
            .checked_add(bytes.len() as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let output = &mut self.outputs[self.count];
        output.address = address;
        output.length = bytes.len();
        output.bytes[..bytes.len()].copy_from_slice(bytes);
        self.count += 1;
        Ok(())
    }

    pub(crate) const fn len(&self) -> usize {
        self.count
    }

    pub(crate) fn writes(&self) -> impl Iterator<Item = GuestWrite<'_>> {
        self.outputs[..self.count].iter().map(|output| GuestWrite {
            address: output.address,
            bytes: &output.bytes[..output.length],
        })
    }
}
