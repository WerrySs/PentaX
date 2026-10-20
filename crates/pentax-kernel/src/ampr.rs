// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned AMPR bindings and transactional guest headers.
// Adapted from pinned src/libs/libAmpr.cpp; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestError, GuestWrite, Service};

mod read;
mod submit;

const HEADER: u64 = 24;
const MAX_SIZE: u32 = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Snapshot {
    buffer: u64,
    size: u32,
    offset: u32,
}
struct Binding {
    object: u64,
    snapshot: Snapshot,
    alias: Option<u64>,
    reads: Vec<read::ReadCommand>,
    gather: Option<(u32, u64, u64)>,
}
#[derive(Default)]
pub(crate) struct AmprState {
    bindings: Vec<Binding>,
    submissions: submit::Submissions,
}

fn address(object: u64, offset: u64) -> Result<u64, Failure> {
    object
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}
fn valid_header(object: u64) -> bool {
    object != 0 && object.checked_add(HEADER).is_some()
}
fn write(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    outputs: &[GuestWrite<'_>],
) -> Result<(), Failure> {
    let scratch: usize = outputs.iter().map(|output| output.bytes.len()).sum();
    budget.scratch(scratch)?;
    budget.record_scratch(scratch);
    budget.write(guest, outputs)
}

impl AmprState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.bindings.len()
    }

    // Queued commands on the exact object take priority over its backing alias.
    // Append itself uses exact-object lookup; reset/submission use this order.
    fn resolve(&self, object: u64) -> Option<Snapshot> {
        self.resolve_index(object)
            .map(|index| self.bindings[index].snapshot)
    }
    fn resolve_index(&self, object: u64) -> Option<usize> {
        self.bindings
            .iter()
            .position(|binding| binding.object == object && !binding.reads.is_empty())
            .or_else(|| {
                self.bindings
                    .iter()
                    .position(|binding| binding.alias == Some(object))
            })
            .or_else(|| {
                self.bindings
                    .iter()
                    .position(|binding| binding.object == object)
            })
    }
    fn admit(&mut self, object: u64, budget: &Budget) -> Result<usize, Failure> {
        if let Some(index) = self
            .bindings
            .iter()
            .position(|binding| binding.object == object)
        {
            return Ok(index);
        }
        budget.can_add_handle()?;
        self.bindings
            .try_reserve_exact(1)
            .map_err(|_| Failure::Allocation)?;
        if self.bindings.capacity() > budget.limits.max_handles {
            return Err(Failure::Limit("AMPR bindings"));
        }
        Ok(self.bindings.len())
    }
    // Precharged metadata and capacity make publication infallible after the
    // guest transaction. Alias collisions replace the old owner, never revive
    // an earlier mapping when the newest binding is subsequently removed.
    fn publish(&mut self, object: u64, snapshot: Snapshot, index: usize, budget: &mut Budget) {
        for binding in &mut self.bindings {
            if binding.object == object
                || (snapshot.buffer != 0
                    && snapshot.buffer != object
                    && binding.alias == Some(snapshot.buffer))
            {
                binding.alias = None;
            }
        }
        let binding = Binding {
            object,
            snapshot,
            alias: (snapshot.buffer != 0 && snapshot.buffer != object).then_some(snapshot.buffer),
            reads: Vec::new(),
            gather: None,
        };
        if index == self.bindings.len() {
            self.bindings.push(binding);
            budget.added_handle();
        } else {
            budget.released_ampr_commands(self.bindings[index].reads.len());
            self.bindings[index].gather.take();
            self.bindings[index] = binding;
        }
    }
    fn erase(&mut self, object: u64, budget: &mut Budget) {
        if let Some(index) = self
            .bindings
            .iter()
            .position(|binding| binding.object == object)
        {
            let mut binding = self.bindings.swap_remove(index);
            budget.released_ampr_commands(binding.reads.len());
            binding.gather.take();
            budget.removed_handle();
        }
        for binding in &mut self.bindings {
            if binding.alias == Some(object) {
                binding.alias = None;
            }
        }
    }
    fn load(
        object: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<Snapshot, Failure> {
        if !valid_header(object) {
            return Err(Errno::Fault.into());
        }
        Ok(Snapshot {
            offset: budget.u32(guest, address(object, 4)?)?,
            size: budget.u32(guest, address(object, 12)?)?,
            buffer: budget.u64(guest, address(object, 16)?)?,
        })
    }
    fn bind(
        &mut self,
        object: u64,
        snapshot: Snapshot,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if !valid_header(object) {
            return Err(Errno::Fault.into());
        }
        let index = self.admit(object, budget)?;
        // Source reset preserves type, resets offset/count and keeps the actual
        // retained buffer/size. Reserved APR words and backing bytes are untouched.
        let mut fields = [0; 20];
        fields[..4].copy_from_slice(&snapshot.offset.to_le_bytes());
        fields[8..12].copy_from_slice(&snapshot.size.to_le_bytes());
        fields[12..20].copy_from_slice(&snapshot.buffer.to_le_bytes());
        write(
            guest,
            budget,
            &[GuestWrite {
                address: address(object, 4)?,
                bytes: &fields,
            }],
        )?;
        self.publish(object, snapshot, index, budget);
        Ok(0)
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Lookup, alias publication and a possible metadata-vector move are
        // bounded before allocation, guest output or retained-state effects.
        budget.work(self.bindings.len() as u64 * 6 + 1)?;
        let [object, buffer, size, ..] = args.0;
        use Service::*;
        let field = match service {
            AmprGetType => Some((0, 4)),
            AmprGetOffset => Some((4, 4)),
            AmprGetNum => Some((8, 4)),
            AmprGetSize => Some((12, 4)),
            AmprGetBase => Some((16, 8)),
            _ => None,
        };
        if let Some((offset, length)) = field {
            if object == 0 {
                return Ok(0);
            }
            let at = address(object, offset)?;
            return if length == 8 {
                budget.u64(guest, at)
            } else {
                budget.u32(guest, at).map(u64::from)
            };
        }
        match service {
            AmprConstruct => {
                if object == 0 {
                    return Ok(0);
                }
                if !valid_header(object) {
                    return Err(Errno::Fault.into());
                }
                write(
                    guest,
                    budget,
                    &[GuestWrite {
                        address: object,
                        bytes: &[0; 24],
                    }],
                )?;
                self.erase(object, budget);
                Ok(0)
            }
            AmprAprConstruct => {
                if object == 0 {
                    return Ok(0);
                }
                let first = if buffer == 0 {
                    object.checked_add(24)
                } else {
                    Some(buffer)
                };
                let second = if size == 0 {
                    object.checked_add(32)
                } else {
                    Some(size)
                };
                let (Some(first), Some(second)) = (first, second) else {
                    return Err(Errno::Fault.into());
                };
                if first.checked_add(8).is_none() || second.checked_add(8).is_none() {
                    return Err(Errno::Fault.into());
                }
                let zeros = [0; 16];
                let distance = first.abs_diff(second);
                // Duplicate, overlapping and adjacent zero words have one
                // exact contiguous union; disjoint outputs share one preflight.
                if distance <= 8 {
                    write(
                        guest,
                        budget,
                        &[GuestWrite {
                            address: first.min(second),
                            bytes: &zeros[..8 + distance as usize],
                        }],
                    )?;
                } else {
                    write(
                        guest,
                        budget,
                        &[
                            GuestWrite {
                                address: first,
                                bytes: &zeros[..8],
                            },
                            GuestWrite {
                                address: second,
                                bytes: &zeros[..8],
                            },
                        ],
                    )?;
                }
                Ok(0)
            }
            AmprAprDestruct => Ok(0), // Reviewed source ignores its input.
            AmprDestruct => {
                self.erase(object, budget);
                Ok(0)
            }
            AmprSetBuffer => {
                if object == 0 {
                    return Err(Errno::Invalid.into());
                }
                if !valid_header(object) {
                    return Err(Errno::Fault.into());
                }
                let size = size as u32;
                if buffer == 0 || buffer & 3 != 0 || size == 0 || size > MAX_SIZE || size & 3 != 0 {
                    return Err(Errno::Invalid.into());
                }
                if budget.u64(guest, address(object, 16)?)? != 0 {
                    return Err(Errno::Busy.into());
                }
                self.bind(
                    object,
                    Snapshot {
                        buffer,
                        size,
                        offset: 0,
                    },
                    guest,
                    budget,
                )
            }
            AmprReset => {
                if object == 0 {
                    return Err(Errno::Perm.into());
                }
                let snapshot = match self.resolve(object) {
                    Some(snapshot) => snapshot,
                    None => Self::load(object, guest, budget)?,
                };
                if snapshot.buffer == 0 {
                    // TryGetCommandBufferState retains a loaded empty snapshot
                    // even when Reset returns EPERM. Later direct guest edits
                    // must not silently replace that retained source state.
                    let index = self.admit(object, budget)?;
                    self.publish(object, snapshot, index, budget);
                    return Err(Errno::Perm.into());
                }
                self.bind(
                    object,
                    Snapshot {
                        offset: 0,
                        ..snapshot
                    },
                    guest,
                    budget,
                )
            }
            AmprClearBuffer => {
                if !valid_header(object) {
                    return Ok(0);
                }
                let buffer = budget.u64(guest, address(object, 16)?)?;
                let size = budget.u32(guest, address(object, 12)?)?;
                if size == 0 || buffer == 0 {
                    return Ok(0);
                }
                write(
                    guest,
                    budget,
                    &[GuestWrite {
                        address: address(object, 12)?,
                        bytes: &[0; 12],
                    }],
                )?;
                self.erase(object, budget);
                Ok(buffer)
            }
            _ => Err(Failure::Unsupported("AMPR lifecycle service")),
        }
    }
}
