// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned fiber lifecycle and atomic guest ABI writes.
// Adapted from pinned src/libs/libKernel.cpp Fiber namespace; see NOTICE.

//! Source ABI and generation-bound fiber state. Run/switch/return admission
//! remains separate from authenticated native context exchange and completion.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Limits, Service};

const NULL: u64 = 0x8059_0001;
const ALIGNMENT: u64 = 0x8059_0002;
const RANGE: u64 = 0x8059_0003;
const INVALID: u64 = 0x8059_0004;
const PERMISSION: u64 = 0x8059_0005;
const RUNNING: u32 = 1;
const STATE: u64 = 0x8059_0006;
const MAGIC_START: u32 = 0xdef1_649c;
const MAGIC_END: u32 = 0xb375_92a0;
const OPT_MAGIC: u32 = 0xbb40_e64d;
const STACK_MAGIC: u64 = 0x7149_f2ca_7149_f2ca;
const IDLE: u32 = 2;
const TERMINATED: u32 = 3;
// Explicit x86-64 source ABI, including the bool's trailing padding. No host
// pointer, Rust bool/layout or native context is copied into a guest object.
const OBJECT_BYTES: usize = 208;
struct Record {
    address: u64,
    state: u32,
    generation: u64,
    entry: u64,
    argument: u64,
    context: u64,
    size: u64,
    resume_output: Option<u64>,
    execution_stack: Option<(u64, u64)>,
}
pub(crate) struct FiberState {
    records: Vec<Record>,
    maximum: usize,
    maximum_threads: usize,
    next_generation: u64,
    next_serial: u64,
    threads: Vec<runtime::Thread>,
    pending: Option<AdmittedFiberTransfer>,
}

fn end(address: u64, length: u64) -> Result<u64, Failure> {
    address
        .checked_add(length)
        .ok_or(GuestError::AddressOverflow.into())
}
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed ABI word"),
    )
}
fn name(
    guest: &impl GuestAccess,
    budget: &mut Budget,
    address: u64,
    cleared_object: Option<(u64, u64)>,
) -> Result<[u8; 32], Failure> {
    let mut result = [0; 32];
    for (index, byte) in result[..31].iter_mut().enumerate() {
        let pointer = end(address, index as u64)?;
        // Initialize clears the object before strncpy in the source. A name
        // overlapping that cleared span therefore terminates there.
        if cleared_object.is_some_and(|(start, end)| (start..end).contains(&pointer)) {
            break;
        }
        budget.read(guest, pointer, std::slice::from_mut(byte))?;
        if *byte == 0 {
            break;
        }
    }
    Ok(result)
}

impl FiberState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.records.len()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            records,
            maximum: limits.max_handles,
            maximum_threads: limits.max_thread_contexts,
            next_generation: 1,
            next_serial: 1,
            threads: {
                let mut v = Vec::new();
                v.try_reserve_exact(limits.max_thread_contexts)
                    .map_err(|_| Failure::Allocation)?;
                v
            },
            pending: None,
        })
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        thread: u64,
        args: CallArgs,
        stack: Option<u64>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        budget.work(self.records.len() as u64 + 1)?;
        let a = args.0;
        match service {
            FiberInitialize | FiberInitializeInternal => {
                let stack = stack.ok_or(Failure::Unsupported("missing fiber stack arguments"))?;
                let mut extra = [0; 24];
                let internal = service == FiberInitializeInternal;
                let length = if internal { 24 } else { 16 };
                budget.read(guest, end(stack, 8)?, &mut extra[..length])?;
                let option = u64::from_le_bytes(extra[..8].try_into().expect("fixed argument"));
                let flags = if internal { word(&extra, 8) } else { 0 };
                let version = word(&extra, if internal { 16 } else { 8 });
                self.initialize(a, option, flags, version, guest, budget)
            }
            FiberOptParamInitialize => {
                if a[0] == 0 {
                    return Ok(NULL);
                }
                if a[0] & 7 != 0 {
                    return Ok(ALIGNMENT);
                }
                let mut bytes = [0; 128];
                bytes[..4].copy_from_slice(&OPT_MAGIC.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            FiberFinalize | FiberGetInfo | FiberRename => {
                if a[0] == 0 || (service != FiberFinalize && a[1] == 0) {
                    return Ok(NULL);
                }
                let mut object = [0; OBJECT_BYTES];
                budget.read(guest, a[0], &mut object)?;
                if word(&object, 0) != MAGIC_START || word(&object, 204) != MAGIC_END {
                    return Ok(INVALID);
                }
                let Some(index) = self
                    .records
                    .iter()
                    .position(|record| record.address == a[0])
                else {
                    return Ok(INVALID);
                };
                match service {
                    FiberFinalize => {
                        if self.pending.is_some() {
                            return Err(Failure::Unsupported("pending fiber finalization"));
                        }
                        if self.records[index].state != IDLE || word(&object, 4) != IDLE {
                            return Ok(STATE);
                        }
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: end(a[0], 4)?,
                                bytes: &TERMINATED.to_le_bytes(),
                            }],
                        )?;
                        self.records[index].state = TERMINATED;
                    }
                    FiberGetInfo => {
                        let mut info = [0; 128];
                        info[..8].copy_from_slice(&128_u64.to_le_bytes());
                        info[8..71].copy_from_slice(&object[8..71]);
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: a[1],
                                bytes: &info,
                            }],
                        )?;
                    }
                    FiberRename => {
                        let copied = name(guest, budget, a[1], None)?;
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: end(a[0], 40)?,
                                bytes: &copied,
                            }],
                        )?;
                    }
                    _ => unreachable!(),
                }
                Ok(0)
            }
            FiberGetSelf | FiberGetThreadFramePointerAddress => {
                if a[0] == 0 {
                    return Ok(NULL);
                }
                let value = if service == FiberGetSelf {
                    self.threads
                        .iter()
                        .find(|row| row.thread == thread)
                        .and_then(|row| row.current)
                        .map_or(0, |key| key.address())
                } else {
                    0
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &value.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            FiberStartContextSizeCheck | FiberStopContextSizeCheck => Ok(0),
            _ => Err(Failure::Unsupported(
                "prepared fiber execution/context switching",
            )),
        }
    }

    fn initialize(
        &mut self,
        a: [u64; 6],
        option: u64,
        flags: u32,
        version: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if self.pending.is_some() {
            return Err(Failure::Unsupported("pending fiber initialization"));
        }
        let [address, name_pointer, entry, argument, context, size] = a;
        if address == 0 || name_pointer == 0 || entry == 0 {
            return Ok(NULL);
        }
        if address & 7 != 0 || context & 15 != 0 || option & 7 != 0 {
            return Ok(ALIGNMENT);
        }
        if size != 0 && size < 512 {
            return Ok(RANGE);
        }
        if size & 15 != 0 || (context == 0) != (size == 0) {
            return Ok(INVALID);
        }
        if option != 0 && budget.u32(guest, option)? != OPT_MAGIC {
            return Ok(INVALID);
        }
        let object_end = end(address, OBJECT_BYTES as u64)?;
        let context_end = end(context, size)?;
        let index = self
            .records
            .iter()
            .position(|record| record.address == address);
        if let Some(index) = index {
            if !matches!(self.records[index].state, IDLE | TERMINATED) {
                return Ok(STATE);
            }
        } else {
            if self.records.len() >= self.maximum {
                return Err(Failure::Limit("fiber records"));
            }
            budget.can_add_handle()?;
        }
        let generation = self.next_generation;
        let next_generation = generation
            .checked_add(1)
            .ok_or(Failure::Limit("fiber generation"))?;
        let copied_name = name(guest, budget, name_pointer, Some((address, object_end)))?;
        let mut object = [0; OBJECT_BYTES];
        object[..4].copy_from_slice(&MAGIC_START.to_le_bytes());
        object[4..8].copy_from_slice(&IDLE.to_le_bytes());
        for (offset, value) in [
            (8, entry),
            (16, argument),
            (24, context),
            (32, size),
            (88, context),
            (96, context_end),
        ] {
            object[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        object[40..72].copy_from_slice(&copied_name);
        object[80..84].copy_from_slice(
            &(flags | if version >= 0x0350_0000 { 0x100 } else { 0 }).to_le_bytes(),
        );
        object[204..208].copy_from_slice(&MAGIC_END.to_le_bytes());
        let stack_magic = STACK_MAGIC.to_le_bytes();
        let writes = [
            GuestWrite {
                address,
                bytes: &object,
            },
            GuestWrite {
                address: context,
                bytes: &stack_magic,
            },
        ];
        budget.write(guest, &writes[..if context == 0 { 1 } else { 2 }])?;
        // Both object and context marker are now published. Metadata capacity
        // and shared handle admission precede that transaction; no allocation
        // or fallible mutation follows it. Reinitialization replaces this record.
        if let Some(index) = index {
            self.records[index] = Record {
                address,
                state: IDLE,
                generation,
                entry,
                argument,
                context,
                size,
                resume_output: None,
                execution_stack: (context != 0).then_some((context, context_end)),
            };
        } else {
            self.records.push(Record {
                address,
                state: IDLE,
                generation,
                entry,
                argument,
                context,
                size,
                resume_output: None,
                execution_stack: (context != 0).then_some((context, context_end)),
            });
            budget.added_handle();
        }
        self.next_generation = next_generation;
        Ok(0)
    }
}

mod runtime;
pub use runtime::{AdmittedFiberTransfer, FiberAction, FiberKey, FiberTransferKind};
