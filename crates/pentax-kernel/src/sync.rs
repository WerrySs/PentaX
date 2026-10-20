// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned bounded semaphore and event-flag state, tagged
// opaque handles, transactional outputs and cooperative synchronization waiters.
// See NOTICE and Kyty-MIT.txt.

//! Port of pinned `kernel/semaphore.cpp` and `kernel/eventFlag.cpp`.
//!
//! Kernel semaphores retain bounded cooperative waiters for the runtime. Raw
//! calls without continuation capture still refuse blocking waits. Each state
//! mutation happens only after
//! its guest output transaction committed, so a fault or budget refusal never
//! consumes counts or bits and never publishes a handle.
//!
//! Handles are PentaX policy: a kind tag in the upper 16 bits and a never
//! reused serial below it. They are not source pointer representations.

use crate::budget::{Budget, Limits};
use crate::catalog::Service;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite};
mod semaphore_wait;
pub use semaphore_wait::{SemaphoreWait, SemaphoreWaitAction};
mod event_flag_wait;
pub use event_flag_wait::{EventFlagWait, EventFlagWaitAction};

const TAG_MASK: u64 = 0xffff_0000_0000_0000;
const SEMAPHORE_TAG: u64 = 0x5345_0000_0000_0000;
const EVENT_FLAG_TAG: u64 = 0x4546_0000_0000_0000;
const SERIAL_LIMIT: u64 = 1 << 48;

const SEMA_ATTR_MAX: u32 = 2;
const SEMA_ATTR_FIFO: u32 = 1;
const EVENT_FLAG_ATTR_MASK: u32 = 0x33;
const EVENT_WAIT_MODE_BITS: u32 = 0xff;

struct Semaphore {
    id: u64,
    #[allow(dead_code)] // Retained resource identity for diagnostics.
    name: String,
    fifo: bool,
    count: i32,
    init_count: i32,
    max_count: i32,
}

struct EventFlag {
    id: u64,
    #[allow(dead_code)] // Retained resource identity for diagnostics.
    name: String,
    single_waiter: bool,
    #[allow(dead_code)] // Source accepts but ignores waiter order.
    fifo: bool,
    bits: u64,
    // Native Cancel/Delete must drain their original cohort before returning.
    draining: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitMode {
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClearMode {
    None,
    All,
    Bits,
}

pub(crate) struct SyncState {
    semaphores: Vec<Semaphore>,
    flags: Vec<EventFlag>,
    next_serial: u64,
    waiters: Vec<semaphore_wait::Waiter>,
    max_waiters: usize,
    flag_waiters: Vec<event_flag_wait::Waiter>,
    flag_operations: Vec<event_flag_wait::Operation>,
}

/// A 32-bit SysV integer argument occupies the low half of its register.
const fn int(value: u64) -> i32 {
    value as u32 as i32
}

/// Source `DecodeEventFlagWaitMode`. Encodings the source aborts on, and bits
/// outside its decoded byte, are unsupported rather than guessed errno values.
fn decode_wait_mode(mode: u32) -> Result<(WaitMode, ClearMode), Failure> {
    if mode & !EVENT_WAIT_MODE_BITS != 0 {
        return Err(Failure::Unsupported(
            "event flag wait mode bits outside source decoding",
        ));
    }
    let wait = match mode & 0x0f {
        0x01 => WaitMode::And,
        0x02 => WaitMode::Or,
        _ => return Err(Failure::Unsupported("unknown event flag wait mode")),
    };
    let clear = match mode & 0xf0 {
        0x00 => ClearMode::None,
        0x10 => ClearMode::All,
        0x20 => ClearMode::Bits,
        _ => return Err(Failure::Unsupported("unknown event flag clear mode")),
    };
    Ok((wait, clear))
}

impl SyncState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.semaphores.len() + self.flags.len()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut semaphores = Vec::new();
        semaphores
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        let mut flags = Vec::new();
        flags
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        let mut waiters = Vec::new();
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        let mut flag_waiters = Vec::new();
        flag_waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        let mut flag_operations = Vec::new();
        flag_operations
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            semaphores,
            flags,
            next_serial: 1,
            waiters,
            max_waiters: limits.max_thread_contexts,
            flag_waiters,
            flag_operations,
        })
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        match service {
            Service::CreateSema => self.create_sema(args, guest, budget),
            Service::DeleteSema => self.delete_sema(args, budget),
            Service::WaitSema => self.wait_sema(args, guest, budget),
            Service::PollSema => self.poll_sema(args, budget),
            Service::SignalSema => self.signal_sema(args, budget),
            Service::CancelSema => self.cancel_sema(args, guest, budget),
            Service::CreateEventFlag => self.create_flag(args, guest, budget),
            Service::DeleteEventFlag => self.delete_flag(args, budget),
            Service::WaitEventFlag => self.wait_flag(args, guest, budget, false),
            Service::PollEventFlag => self.wait_flag(args, guest, budget, true),
            Service::SetEventFlag => {
                let index = self.flag_index(args.0[0], budget)?;
                self.require_flag_live(index)?;
                self.flags[index].bits |= args.0[1];
                Ok(0)
            }
            // Source `Clear` keeps only the given mask: bits &= pattern.
            Service::ClearEventFlag => {
                let index = self.flag_index(args.0[0], budget)?;
                self.require_flag_live(index)?;
                self.flags[index].bits &= args.0[1];
                Ok(0)
            }
            Service::CancelEventFlag => self.cancel_flag(args, guest, budget),
            _ => Err(Failure::Unsupported("service is not owned by sync state")),
        }
    }

    fn next_id(&self, tag: u64) -> Result<u64, Failure> {
        if self.next_serial >= SERIAL_LIMIT {
            return Err(Failure::Limit("handle serials"));
        }
        Ok(tag | self.next_serial)
    }

    fn semaphore_index(&self, handle: u64, budget: &mut Budget) -> Result<usize, Failure> {
        if handle & TAG_MASK != SEMAPHORE_TAG {
            return Err(Errno::NoSuchResource.into());
        }
        budget.work(self.semaphores.len() as u64)?;
        self.semaphores
            .iter()
            .position(|entry| entry.id == handle)
            .ok_or(Failure::Errno(Errno::NoSuchResource))
    }

    fn flag_index(&self, handle: u64, budget: &mut Budget) -> Result<usize, Failure> {
        if handle & TAG_MASK != EVENT_FLAG_TAG {
            return Err(Errno::NoSuchResource.into());
        }
        budget.work(self.flags.len() as u64)?;
        self.flags
            .iter()
            .position(|entry| entry.id == handle)
            .ok_or(Failure::Errno(Errno::NoSuchResource))
    }

    fn create_sema<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [output, name, attr, init, max, option] = args.0;
        let (attr, init, max) = (attr as u32, int(init), int(max));
        if output == 0 {
            return Err(Failure::Unsupported(
                "null semaphore output (source aborts)",
            ));
        }
        if name == 0 || attr > SEMA_ATTR_MAX || init < 0 || max <= 0 || init > max {
            return Err(Errno::Invalid.into());
        }
        if option != 0 {
            return Err(Failure::Unsupported("semaphore creation option"));
        }
        budget.can_add_handle()?;
        if self.semaphores.len() >= self.semaphores.capacity() {
            return Err(Failure::Limit("handles"));
        }
        let id = self.next_id(SEMAPHORE_TAG)?;
        let name = budget.name(&*guest, name)?;
        budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &id.to_le_bytes(),
            }],
        )?;
        self.semaphores.push(Semaphore {
            id,
            name,
            fifo: attr == SEMA_ATTR_FIFO,
            count: init,
            init_count: init,
            max_count: max,
        });
        self.next_serial += 1;
        budget.added_handle();
        Ok(0)
    }

    fn delete_sema(&mut self, args: CallArgs, budget: &mut Budget) -> Result<u64, Failure> {
        let index = self.semaphore_index(args.0[0], budget)?;
        budget.work(self.waiters.len() as u64)?;
        self.finish_waiters(args.0[0], Errno::Access);
        self.semaphores.swap_remove(index);
        budget.removed_handle();
        Ok(0)
    }

    fn wait_sema<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let index = self.semaphore_index(args.0[0], budget)?;
        let (need, timeout) = (int(args.0[1]), args.0[2]);
        let (count, max) = (
            self.semaphores[index].count,
            self.semaphores[index].max_count,
        );
        if need < 1 || need > max {
            return Err(Errno::Invalid.into());
        }
        let micros = if timeout == 0 {
            None
        } else {
            Some(budget.u32(&*guest, timeout)?)
        };
        if count >= need {
            // Source writes the unchanged remaining time back on immediate success.
            if let Some(micros) = micros {
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: timeout,
                        bytes: &micros.to_le_bytes(),
                    }],
                )?;
            }
            self.semaphores[index].count = count - need;
            return Ok(0);
        }
        match micros {
            Some(0) => Err(Errno::TimedOut.into()),
            _ => Err(Failure::Unsupported("blocking semaphore wait")),
        }
    }

    fn poll_sema(&mut self, args: CallArgs, budget: &mut Budget) -> Result<u64, Failure> {
        let index = self.semaphore_index(args.0[0], budget)?;
        let need = int(args.0[1]);
        let entry = &mut self.semaphores[index];
        if need < 1 || need > entry.max_count {
            return Err(Errno::Invalid.into());
        }
        if entry.count < need {
            return Err(Errno::Busy.into());
        }
        entry.count -= need;
        Ok(0)
    }

    fn signal_sema(&mut self, args: CallArgs, budget: &mut Budget) -> Result<u64, Failure> {
        let index = self.semaphore_index(args.0[0], budget)?;
        budget.work(self.waiters.len() as u64)?;
        let count = int(args.0[1]);
        // The source only bounds the sum from above; a negative signal would
        // lower the count, which is not established firmware behavior.
        if count < 0 {
            return Err(Failure::Unsupported("negative semaphore signal count"));
        }
        let entry = &mut self.semaphores[index];
        let next = i64::from(entry.count) + i64::from(count);
        if next > i64::from(entry.max_count) {
            return Err(Errno::Invalid.into());
        }
        entry.count = next as i32;
        self.grant_waiters(index);
        Ok(0)
    }

    fn cancel_sema<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let index = self.semaphore_index(args.0[0], budget)?;
        let (set_count, threads) = (int(args.0[1]), args.0[2]);
        if set_count > self.semaphores[index].max_count {
            return Err(Errno::Invalid.into());
        }
        budget.work(self.waiters.len() as u64 * 2)?;
        let waiting = self.waiting_count(args.0[0]);
        if threads != 0 {
            budget.write(
                guest,
                &[GuestWrite {
                    address: threads,
                    bytes: &waiting.to_le_bytes(),
                }],
            )?;
        }
        let entry = &mut self.semaphores[index];
        entry.count = if set_count < 0 {
            entry.init_count
        } else {
            set_count
        };
        self.finish_waiters(args.0[0], Errno::Canceled);
        Ok(0)
    }

    fn create_flag<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [output, name, attr, init, param, _] = args.0;
        let attr = attr as u32;
        if output == 0 || name == 0 || param != 0 || attr & !EVENT_FLAG_ATTR_MASK != 0 {
            return Err(Errno::Invalid.into());
        }
        let fifo = match attr & 0x0f {
            0x00 | 0x01 => true,
            0x02 => false,
            _ => return Err(Errno::Invalid.into()),
        };
        let single_waiter = match attr & 0xf0 {
            0x00 | 0x10 => true,
            0x20 => false,
            _ => return Err(Errno::Invalid.into()),
        };
        budget.can_add_handle()?;
        if self.flags.len() >= self.flags.capacity() {
            return Err(Failure::Limit("handles"));
        }
        let id = self.next_id(EVENT_FLAG_TAG)?;
        let name = budget.name(&*guest, name)?;
        budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &id.to_le_bytes(),
            }],
        )?;
        self.flags.push(EventFlag {
            id,
            name,
            single_waiter,
            fifo,
            bits: init,
            draining: None,
        });
        self.next_serial += 1;
        budget.added_handle();
        Ok(0)
    }

    fn delete_flag(&mut self, args: CallArgs, budget: &mut Budget) -> Result<u64, Failure> {
        let index = self.flag_index(args.0[0], budget)?;
        self.require_flag_live(index)?;
        budget.work(self.flag_waiters.len() as u64 + 1)?;
        if self.flag_waiting_count(args.0[0]) != 0 {
            return Err(Failure::Unsupported("blocking event flag deletion"));
        }
        self.flags.swap_remove(index);
        budget.removed_handle();
        Ok(0)
    }

    /// Wait and poll share source `Wait`: the result pattern and remaining time
    /// are written before any clear, and also when the zero timeout expires.
    fn wait_flag<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
        poll: bool,
    ) -> Result<u64, Failure> {
        self.direct_event_flag_wait(args, guest, budget, poll)
    }

    fn cancel_flag<G: GuestAccess>(
        &mut self,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        self.direct_event_flag_cancel(args, guest, budget)
    }
}
