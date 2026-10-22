// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: retained save initialization and bounded source events.
// Adapted from pinned libSaveData.cpp; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};
use std::collections::VecDeque;

const PARAMETER: u64 = 0x809f_0000;
const NOT_FOUND: u64 = 0x809f_0008;
const EVENT_BYTES: usize = 104;
const EVENT_COUNT: usize = 20;
mod directories;
mod memory;
mod metadata;
mod storage;
pub use storage::SaveStorage;

pub(crate) struct SaveState {
    initialized: bool,
    next_transaction: u32,
    events: VecDeque<[u8; EVENT_BYTES]>,
    memory: memory::SaveMemory,
    directories: directories::DirectoryState,
}
impl SaveState {
    pub(crate) fn new() -> Result<Self, Failure> {
        let mut events = VecDeque::new();
        events
            .try_reserve_exact(EVENT_COUNT)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            initialized: false,
            next_transaction: 1,
            events,
            memory: memory::SaveMemory::new()?,
            directories: directories::DirectoryState::new(),
        })
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        use Service::*;
        match service {
            SaveInitialize => {
                // Source initialization ignores the optional pointer. It does
                // not mount storage, clear events or reset transaction IDs.
                self.initialized = true;
                Ok(0)
            }
            SaveTerminate => {
                if self.directories.live() {
                    return Ok(directories::BUSY);
                }
                // A live directory mount blocks cleanup before events, memory,
                // initialization or the filesystem namespace can be changed.
                budget.work(self.events.len() as u64)?;
                self.memory.clear(budget)?;
                self.events.clear();
                self.initialized = false;
                Ok(0)
            }
            SaveCreateTransaction => {
                // The pinned function only issues monotonic identifiers; its
                // size argument does not allocate guest or host storage.
                if self.next_transaction > i32::MAX as u32 {
                    return Err(Failure::Limit("save transaction IDs"));
                }
                let id = self.next_transaction;
                self.next_transaction += 1;
                Ok(u64::from(id))
            }
            SaveDeleteTransaction => {
                // Source deliberately ignores the identifier. Preserve this
                // reviewed no-op without claiming resource or mount deletion.
                Ok(0)
            }
            SavePrepare => {
                if args.0[0] == 0 || args.0[1] == 0 {
                    return Ok(PARAMETER);
                }
                // Check the fixed mount-point and the actual resource/mode
                // fields. Source prepare performs no persistent write or mount.
                budget.read(guest, args.0[0], &mut [0; 16])?;
                budget.read(guest, args.0[1], &mut [0; 8])?;
                Ok(0)
            }
            SaveCommit => {
                if args.0[0] == 0 {
                    return Ok(PARAMETER);
                }
                let mut header = [0; 8];
                budget.read(guest, args.0[0], &mut header)?;
                let mode = u32::from_le_bytes(header[4..].try_into().unwrap());
                if mode & 1 != 0 {
                    budget.scratch(EVENT_BYTES)?;
                    budget.work(EVENT_BYTES as u64 + 1)?;
                    budget.record_scratch(EVENT_BYTES);
                    let mut event = [0; EVENT_BYTES];
                    event[..4].copy_from_slice(&4_u32.to_le_bytes());
                    // Source retains the newest twenty events. Pop before push
                    // to use preallocated capacity; no observer can interleave
                    // and no fallible operation follows admission.
                    if self.events.len() == EVENT_COUNT {
                        self.events.pop_front();
                    }
                    self.events.push_back(event);
                }
                Ok(0)
            }
            SaveGetEvent => {
                // event_param is only diagnostic data in the source, ignored.
                if args.0[1] == 0 {
                    return Ok(PARAMETER);
                }
                let Some(event) = self.events.front() else {
                    return Ok(NOT_FOUND);
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: args.0[1],
                        bytes: event,
                    }],
                )?;
                self.events.pop_front();
                Ok(0)
            }
            SaveMemorySetup | SaveMemoryGet | SaveMemorySet | SaveMemorySync => self.memory.invoke(
                service,
                args,
                guest,
                budget,
                &mut self.events,
                &self.directories,
            ),
            _ => Err(Failure::Unsupported("save state service")),
        }
    }
    pub(crate) fn invoke_directory(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        files: &mut crate::filesystem::FileState,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        match self.directories.invoke(
            service,
            args,
            guest,
            budget,
            self.memory.storage(),
            &self.memory,
            &mut self.events,
            files,
        ) {
            Ok(value) | Err(memory::Error::Code(value)) => Ok(value),
            Err(memory::Error::Policy(error)) => Err(error),
        }
    }
}
impl crate::Kernel {
    /// Initialization receipt only; no persistent save or mounted storage claim.
    #[must_use]
    pub const fn save_data_initialized(&self) -> bool {
        self.save.initialized
    }
}
